import './_selfrun.mjs';
// T46 prompt 形状基线：出厂路径**实际送给模型的是什么形状**，与基线逐字段比。
//
// ## 本用例钉的是什么
//
// [plan/13 终点第 2 条](../../docs/archive/13-质量改进总纲.md)：「有一份 prompt 形状基线，
// 能回答『这样对模型是变好还是变坏』」。[ADR-048a](../../docs/registry/adr/ADR-048a.md)
// 判过「拍平是退步」，但那次判定依据的是**论证**，不是记录——连「现在送出去的是什么形状」
// 都没有一份可核对的东西，「结构化一定更好」同样未经测量（登记条目
// [S-end①](../../docs/registry/task/S-end①.md)）。
//
// 基线本体在 [`docs/reference/PROMPT_SHAPE.md`](../../docs/reference/PROMPT_SHAPE.md)
// （生成物，唯一写者是本用例带 `PROMPT_SHAPE_UPDATE=1`），**判据口径**（什么进基线、
// 什么只作观测、为什么规模不进判据）在 [ADR-057](../../docs/registry/adr/ADR-057.md)，
// **提取与比对实现**在 `scripts/prompt-shape-audit.mjs`（PS-001…003）。
// 本用例只做三件事：跑一次真实链路 ⇒ 把 mock-llm 收到的原样请求体落盘 ⇒ 交给脚本判。
//
// ## 为什么从运行时抓，不静态读装配代码
//
// 静态读 `render_messages` / `request_view_prefix` 只能读出**打算**发什么。中间隔着投影、
// 截断、记忆召回与协议适配器（四种 wire 形态各有一套 flatten）——ADR-048a 判的是
// 「送进模型的是消息数组还是拼平的文本」，这是一条**运行时事实**，只有真实请求体能回答。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 本基线的主题是**主线装配的形状**。对话面（`classify` / `compose`）出厂默认开启，
// 每轮多插静默请求，那些请求的形状是 [t21](t21-classify.mjs) / [t22](t22-compose.mjs) /
// [t23](t23-progress-report.mjs) 的主题；混进同一份基线 ⇒ 请求下标依赖分类器的路由结果，
// 一次对话面调优会让「主线形状」整块变红而它没有诊断价值。基线文件的场景名明写了这个口径。
//
// ## 为什么场景里必须带一次真实工具轮
//
// ADR-048a 争的就是 `role` 与 `tool_call_id` 这两个**结构载体**。只有纯文本轮次的基线
// 拍不出它们——把工具结果降级成一段正文，形状里必须看得见 `tool_calls` 计数与
// `tool_call_id` 的消失。工具走 mock-mcp 的 `echo`（返回确定，与 t26 同一取舍）。
import { join } from 'node:path';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  nextPort,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  waitFor,
} from '../helpers.mjs';
import { extractObservations } from '../../scripts/prompt-shape-audit.mjs';

/** 会话 id 固定：它出现在 prompt 的记忆地址里，随机 id 会让基线每次都不一致 */
const SID = 'e2e-t46';
/** 基线场景名（写进基线文件，也是判据的一部分——换场景不得顶替旧基线） */
const SCENARIO = 'full-主线装配-工具轮-对话面关闭';
/** 工具回显内容：固定文本，让「观测列」在两次重生成之间可比 */
const ECHO_TEXT = '形状基线回显内容';

const AUDIT_SCRIPT = join(E2E_ROOT, 'scripts', 'prompt-shape-audit.mjs');

export default defineCase(
  'T46 prompt 形状基线：真实链路送出的形状与基线逐字段一致（PS-001…003）',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'call-echo',
        match: '帮我回显',
        once: true,
        content: '我先回显一下。',
        toolCalls: [{ id: 'call_1', name: 'mcp__mockserv__echo', arguments: { text: ECHO_TEXT } }],
      },
      // 工具结果轮：不再请求工具 ⇒ 本轮收束（`once` 已烧掉，落到这条无 match 的兜底）。
      { id: 'after-echo', content: '工具调用完成，回显成功。' },
      // 第二轮：历史回灌后的请求形状（多两条历史消息，system 前缀里多出召回小节）。
      { id: 'turn2', match: '第二轮', content: '第二轮回复。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        session: { ...DIALOG_FACE_OFF },
      },
    });
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, 'mcp-record.ndjson')],
      enabled: true,
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: SID,
        stdinText: '帮我回显一下\n第二轮问题\n',
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      // 链路没跑通时「空形状与空基线一致」是假绿灯——脚本用 PS-003 兜最后一道，
      // 这里先给出**有诊断价值**的一条：请求数与场景预期不符，说明红在本用例不在基线。
      assertEq(reqs.length, 3, '应恰好 3 次请求（工具轮 / 工具结果轮 / 第二轮）');

      // 观测列：规模随每次运行打印，改动前后的差值就是「这次结构变动花了多少」。
      const obs = extractObservations(reqs);
      for (const [i, o] of obs.entries()) {
        console.log(`  req${i}: 消息 ${o.messages} 条 · 正文 ${o.message_bytes}B · 请求体 ${o.request_bytes}B`);
      }

      const dir = mkdtempSync(join(tmpdir(), 'symbio-prompt-shape-'));
      const livePath = join(dir, 'live.json');
      writeFileSync(livePath, JSON.stringify(reqs));

      const update = process.env.PROMPT_SHAPE_UPDATE === '1';
      const args = [AUDIT_SCRIPT, `--from=${livePath}`, `--scenario=${SCENARIO}`];
      if (update) args.push('--write');
      const a = spawnSync(process.execPath, args, { encoding: 'utf8', cwd: E2E_ROOT });
      const out = `${a.stdout ?? ''}${a.stderr ?? ''}`;
      if (update) {
        assertEq(a.status, 0, `基线重生成失败：\n${out}`);
        console.log(`  已重生成 docs/reference/PROMPT_SHAPE.md（${SCENARIO}）`);
      } else {
        assertEq(a.status, 0, `prompt 形状与基线不一致（重生成：PROMPT_SHAPE_UPDATE=1 node e2e/cases/t46-prompt-shape-baseline.mjs）：\n${out}`);
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm / mock-mcp 子进程退出',
      });
    }
  },
);
