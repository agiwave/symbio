import './_selfrun.mjs';
// T39 子智能体的**单槽注册不覆盖父会话**（plan/11 §2-F / 批 2 ②）。
//
// ## 本用例钉的是什么
//
// [plan/11 §2-F](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的缺口条目：
// 「`register_vdfs_root` / `register_model_providers` 单槽 ⇒ 子智能体的注册会覆盖父的」。
// 核实后这条**在现仓不成立**（每次 `collect_capabilities` 各建一个 visitor +
// `SubAgentVisitor` 在父会话收集期**丢弃**这两项）。所以本用例证的不是"新机制能工作"，
// 而是**旧的静默破裂路径被堵上了**——plan/11 §4 的「反向用例是硬要求」。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 会话的 `<根>` 是**系统树**的 | `vdfs_read` 的工具结果（回灌进第二发请求） |
// | 它**不是**子智能体自己那棵树的 | 同一份工具结果里没有子智能体目录的正文 |
//
// ## 为什么读 `<根>/memory/AGENTS.md` 能分辨两棵树
//
// 同一个挂载名在**两棵树里各有一个实例**，而 `AGENTS.md` 是「**本插件目录的父目录**下的
// 文件」：系统树的 `memory` 读 `{homedir}/AGENTS.md`，子智能体树的 `memory` 读
// `<agentdir>/AGENTS.md`。于是"哪一棵树的根生效"在这一个地址上**可判**。
//
// ## 为什么不能拿整个请求体当判据（本用例只认工具结果）
//
// 两份 `AGENTS.md` **都会**出现在请求里——子树的注册走 `agent/<id>/` 前缀并集
// （`SubAgentVisitor` 只丢弃单槽那两项，提示词段是转发的），所以「请求体里出现了
// 子智能体的正文」是**设计**（`t30` 正是在断言这一点）。能分辨"根被劫持"的只有
// **那一次 `vdfs_read` 的结果**。
//
// ## 为什么只钉根、不钉模型（诚实边界）
//
// 根那一半的顺序是**确定**的：容器在自己的 `traverse` 里**先**登记自己的根、**再**遍历
// 子插件（`plugins/composite/composite.rs` 的 `register_vdfs_root` 在子循环之前），
// 而子树的根只能在那次遍历里被登记 ⇒ 一旦把丢弃改成转发，**子树的根必然最后写入**，
// 本用例必然变红。
//
// 模型那一半没有这个性质：系统 `model` 与 `agent` 都是容器的**子插件**，谁后注册谁生效，
// 而子插件的遍历序来自 `HashMap`（逐进程随机）⇒ 端到端钉不住（实测：把丢弃改成转发后
// 本用例仍通过，因为系统 `model` 恰好后注册）。模型侧的判据因此落在单测
// （`plugins/agent/host/scope.test.rs` 的 `sub_agent_model_registration_is_discarded`）。
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

const SID = 'e2e-t39';
const AGENT_ID = 'reviewer';

/** 系统树那份记忆（`{homedir}/AGENTS.md`）——会话的根生效时读到的就是它 */
const SYS_MEMORY = '系统侧记忆标记：SYS-ROOT-ONLY。';
/** 子智能体目录那份记忆（`<agentdir>/AGENTS.md`）——根被劫持时读到的会是它 */
const AGENT_MEMORY = '子智能体记忆标记：AGENT-ROOT-ONLY。';

const ASK = '读一下你的记忆文件。';
/** `<根>/memory/AGENTS.md`（根名 `.vdfsv2` 是 vdfs 插件的运行时数据，此处按协议地址写） */
const READ_PATH = '.vdfsv2/memory/AGENTS.md';
const FINAL = '读完了。';

/**
 * 从请求体里取出**工具结果**正文。
 *
 * 两种形状都认，因为执行路径决定承载方式、与「结果回灌了没有」无关：
 * - v1：独立的 `role=tool` 消息；
 * - v2（`full` 档，出厂）：整段请求是一条 user 消息，工具交换渲染成
 *   `工具结果(名): …` 行（`render_tool_exchange`），**永远没有 `role=tool` 消息**。
 *
 * 只认这一处内容：两份 `AGENTS.md` 都会以提示词段的形式进请求（设计如此），
 * 能把"根被劫持"分辨出来的只有这一次 `vdfs_read` 的返回。
 */
function toolResultText(body) {
  const msgs = body?.messages ?? [];
  const asTool = msgs
    .filter((m) => m.role === 'tool')
    .map((m) => (typeof m.content === 'string' ? m.content : JSON.stringify(m.content)));
  if (asTool.length) return asTool.join('\n');
  // v2：从 prompt 文本里取「工具结果(…)」那些行（跨行正文也一并带上）
  const text = msgs.map((m) => String(m.content ?? '')).join('\n');
  const lines = text.split('\n');
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].startsWith('工具结果(')) out.push(lines.slice(i, i + 12).join('\n'));
  }
  return out.join('\n');
}

export default defineCase(
  'T39 子 Agent 的 VDFS 根注册不劫持父会话的 <根>（单槽丢弃的反向判据）',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'read-memory',
        match: ASK,
        once: true,
        toolCalls: [
          { id: 'call_m1', name: 'vdfs_read', arguments: { path: READ_PATH } },
        ],
      },
      // 工具结果回灌后的第二发（无 `match` ⇒ 兜底命中）
      { id: 'final', content: FINAL },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
    });
    writeFileSync(join(hd.homedir, 'AGENTS.md'), SYS_MEMORY);
    // 子智能体目录自己的 `AGENTS.md`：两棵树的同一条路径指向两份不同的正文。
    makeAgentDir(hd.homedir, {
      id: AGENT_ID,
      persona: AGENT_MEMORY,
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: SID,
        agent: AGENT_ID,
        message: ASK,
      });
      assertEq(r.code, 0, `退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      assertEq(
        reqs.length,
        2,
        `应恰好两发请求（工具轮 + 结果回灌），实际 ${reqs.length}`,
      );

      const toolText = toolResultText(reqs[1].body);
      assert(
        toolText.length > 0,
        `第二发请求里应带工具结果（判据落在这里；形状变了要当场说出来）：` +
          `${JSON.stringify(reqs[1].body?.messages ?? []).slice(0, 600)}`,
      );
      assert(
        toolText.includes(SYS_MEMORY),
        `会话的 <根> 必须是**系统树**的——工具结果应读到 {homedir}/AGENTS.md：` +
          `${toolText.slice(0, 400)}`,
      );
      assert(
        !toolText.includes(AGENT_MEMORY),
        `工具结果不得来自子智能体自己那棵树（子 Agent 的根注册必须被丢弃，` +
          `否则它劫持整个会话的 <根>）：${toolText.slice(0, 400)}`,
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28 / t30 同一约定：Windows 上带活子进程退出
      // 会撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
