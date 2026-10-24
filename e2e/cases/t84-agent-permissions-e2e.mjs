import './_selfrun.mjs';
// T84 Agent 权限执行面端到端：选中智能体后，出站域名与自有包路径按**它的声明**判（D-153）。
//
// ## 这条用例补的是什么
//
// `permissions` 四字段的执行方（`D-153`）在各执行面有单测（`net_guard` / `vdfs`
// 路径入口 / 装载期）钉了判据，但有一条**只有端到端能证**的链：作用域从收集期
// 发布（`agent` 插件）经 `collect_capabilities` 挂回轮 ctx，再一路 fork 进工具
// 执行期。任一环断了，表现都是「闸门恒过」——单测里手工构造的 ctx 看不出来，
// 因为它们是直接往 ctx 里塞槽的。本用例让真 CLI 跑真会话：选中智能体 ⇒ 模型
// 调工具 ⇒ 结果按声明放行 / 拒绝。
//
// ## 判据怎么取
//
// 全部取**模型实际收到的那段工具结果文本**（`role=tool` 消息正文）：它是「用户
// 看得见的结果」在模型侧的唯一投影。四条判据各对一种形状：
//
// 1. **越界必拒**：零授权智能体出站被拒，错误文案点名 `permissions.network.domains`；
// 2. **自有包按声明判**：同一个智能体读自己的包被拒，文案点名 `permissions.fs.read`；
// 3. **不归它管的一律放行**：同一个智能体读**工作区**文件照常放行——管辖范围
//    只管自己的包，用户工作目录是宿主策略的地界（D-153）；
// 4. **声明了真放行**：另一个声明了 `fs.read: ["**"]` 的智能体读自己的包成功——
//    这条钉住「槽里带的是真声明」：槽到了但权限是空的（改名回归）会在 1–3 全绿
//    的情况下在这里露出来。
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  cleanupHomedir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  DIALOG_FACE_OFF,
  waitFor,
  msgRoleOf,
} from '../helpers.mjs';

/** 零授权智能体（夹具默认 manifest 不带权限段 ⇒ grant 语义下的零授权） */
const ZERO = 't84-zero';
/** 显式声明「自己的包整个可读」的智能体 */
const GRANTED = 't84-granted';
const ZERO_PERSONA = '你是零授权智能体：什么都不许做。';
const GRANTED_PERSONA = '你是持权智能体：自己的包可读。';
const GRANTED_PERMS = `permissions:
  fs:
    read:
      - "**"`;

// `<根>/agent/<id>/memory/AGENTS.md`：子树 memory 挂载点的虚拟地址（根名 `.vdfsv2`
// 按协议地址写字面量，与 T57 同一约定）。
const OWN_ADDR_ZERO = '.vdfsv2/agent/t84-zero/memory/AGENTS.md';
const OWN_ADDR_GRANTED = '.vdfsv2/agent/t84-granted/memory/AGENTS.md';

const WS_MARKER = 'T84-WORKSPACE-MARKER：这一行在工作区，不归智能体声明管。';
const T_WEB = '试一下外取那个文档页面';
const T_OWN = '读一下你自己的记忆文件';
const T_WS = '读一下工作区的 README';
// 会话 B 的用户轮刻意用**另一句**：两个会话共用一份 mock 夹具，同名 match 会让
// 「哪一发命中哪条」依赖声明顺序（今天成立，重构时不一定）。
const T_GRANTED = '再读一次你自己的记忆文件';

/** 取某发请求里**最后一条** `role=tool` 消息正文（历史里带着前面每一轮的结果）。 */
function toolResultText(req) {
  const msgs = req?.body?.messages ?? [];
  const tool = [...msgs].reverse().find((m) => msgRoleOf(m) === 'tool');
  if (!tool) return '';
  return typeof tool.content === 'string' ? tool.content : JSON.stringify(tool.content);
}

export default defineCase(
  'T84 Agent 权限执行面端到端：出站与自有包路径按声明判，工作区不归它管（D-153）',
  async () => {
    const llm = await new MockLlm([
      // ── 会话 A（零授权智能体）三轮 ──
      {
        id: 'a1-web',
        match: T_WEB,
        once: true,
        // 公网主机：过得了 scheme 与 SSRF 两道闸，第三道（Agent 声明）才轮到它——
        // 这样拒绝文案必然来自权限判据，不会与「阻止本地/私有主机」混形。
        toolCalls: [
          { id: 'c_a1', name: 'web_fetch', arguments: { url: 'https://docs.example.com/page' } },
        ],
      },
      { id: 'a1-final', afterTool: true, once: true, content: '外取这一步结束了。' },
      {
        id: 'a2-own',
        match: T_OWN,
        once: true,
        toolCalls: [{ id: 'c_a2', name: 'vdfs_read', arguments: { path: OWN_ADDR_ZERO } }],
      },
      { id: 'a2-final', afterTool: true, once: true, content: '自有包读取这一步结束了。' },
      {
        id: 'a3-ws',
        match: T_WS,
        once: true,
        toolCalls: [{ id: 'c_a3', name: 'vdfs_read', arguments: { path: 'README.md' } }],
      },
      { id: 'a3-final', afterTool: true, once: true, content: '工作区读取这一步结束了。' },
      // ── 会话 B（持权智能体）一轮 ──
      {
        id: 'b1-own',
        match: T_GRANTED,
        once: true,
        toolCalls: [{ id: 'c_b1', name: 'vdfs_read', arguments: { path: OWN_ADDR_GRANTED } }],
      },
      { id: 'b1-final', afterTool: true, once: true, content: '持权读取这一步结束了。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
    });
    // 工作区文件：判据 3 的靶子（会话 A 的智能体读它必须照常放行）
    writeFileSync(join(hd.workdir, 'README.md'), `${WS_MARKER}\n`);
    makeAgentDir(hd.homedir, {
      id: ZERO,
      name: '零授权',
      persona: ZERO_PERSONA,
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });
    makeAgentDir(hd.homedir, {
      id: GRANTED,
      name: '持权',
      persona: GRANTED_PERSONA,
      providerId: PROVIDER_ID,
      providerPort: llm.port,
      permissions: GRANTED_PERMS,
    });

    try {
      // ── 会话 A：零授权智能体，三轮 ──
      const rA = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: 'e2e-t84-a',
        agent: ZERO,
        stdinText: `${T_WEB}\n${T_OWN}\n${T_WS}\n`,
      });
      assertEq(rA.code, 0, `会话 A 退出码（stderr: ${rA.stderr.slice(0, 600)}）`);

      // ── 会话 B：持权智能体，一轮 ──
      const rB = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: 'e2e-t84-b',
        agent: GRANTED,
        stdinText: `${T_GRANTED}\n`,
      });
      assertEq(rB.code, 0, `会话 B 退出码（stderr: ${rB.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      // 会话 A：3 个用户轮 + 3 个工具结果轮；会话 B：1 + 1 ⇒ 共 8 发
      assertEq(reqs.length, 8, `应八发请求（A 六 + B 二），实际 ${reqs.length}`);

      // 工具面里两个工具都注册了（否则后面的判据是「工具不存在」而不是「被闸拒」）
      const wireNames = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      for (const name of ['web_fetch', 'vdfs_read']) {
        assert(
          wireNames.includes(name),
          `${name} 应注册进模型可见工具面（实际: ${wireNames.join(',')}）`,
        );
      }

      // ── 判据 1：出站被拒，文案点名 permissions.network.domains ──
      //
      // 宿主态（槽没到）时这一发会**放行**并撞上连接失败——文案完全不同，
      // 所以「点名了权限字段」就是「槽带着作用域到了工具执行期」的判据。
      const webText = toolResultText(reqs[1]);
      assert(
        webText.includes('permissions.network.domains'),
        `零授权智能体出站应被拒并点名 permissions.network.domains（实际: ${webText.slice(0, 400)}）`,
      );

      // ── 判据 2：读自有包被拒，文案点名 permissions.fs.read ──
      const ownText = toolResultText(reqs[3]);
      assert(
        ownText.includes('permissions.fs.read'),
        `零授权智能体读自己的包应被拒并点名 permissions.fs.read（实际: ${ownText.slice(0, 400)}）`,
      );

      // ── 判据 3：工作区文件照常放行（管辖范围只管自己的包） ──
      const wsText = toolResultText(reqs[5]);
      assert(
        wsText.includes(WS_MARKER),
        `同一智能体读工作区文件应放行——用户工作目录不归 Agent 声明管（实际: ${wsText.slice(0, 400)}）`,
      );

      // ── 判据 4：声明了真放行（槽里带的是真声明，不是空权限） ──
      const grantedText = toolResultText(reqs[7]);
      assert(
        grantedText.includes(GRANTED_PERSONA),
        `声明了 fs.read 的智能体读自己的包应放行并读到人格正文（实际: ${grantedText.slice(0, 400)}）`,
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
