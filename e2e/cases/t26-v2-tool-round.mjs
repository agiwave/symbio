import './_selfrun.mjs';
// T26 工具轮 v2 化：工具调用真的执行、结果真的回灌下一次请求。
//
// ## 本用例钉的是什么
//
// [10 §3 批 1](../../symbio/src/plugins/session/docs/102-core-loop.md "PL-008-工具轮v2化实施方案") 的出口判据原文：
// 「mock-llm `toolCalls` → 工具执行 → **`/_requests` 回读**：工具结果进了下一次请求；
// 网格有 `artifact.added` 且带溯源」。
//
// 这条判据之所以必须端到端，是因为它要证的三件事**各自只能在一处被看见**：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 工具**真的执行**了 | mock-mcp 的 `--record`（进程外证据，不看后端自述） |
// | 结果**真的回了模型** | mock-llm 的 `/_requests` 回读**第二次请求体** |
// | 转写里 tool_call 与结果**成对** | 生产读出口 `vdfs/read` 会话叶子（与前端 `loadMessages` 同一出口） |
//
// 单测（`turn_exec.test.rs::tool_round_lands_artifact_and_feeds_next_request`）用假 provider
// 覆盖了同一形状，但它证不了"真实 provider + 真实 MCP 工具 + 真实落盘"这条链——
// 单测里的 provider 是假件，工具分发走的是「没有父插件」的诚实失败路径。
//
// ## 为什么用 MCP 工具而不是内置工具
//
// 内置 `vdfs_read` 的路径要经「虚拟根 → 物理工作目录」两层解析（相对路径落在
// `<workdir>` 下），断言结果正文就得先知道这两层怎么拼——那是**另一个**主题。
// MCP `echo` 的返回是确定的（`JSON.stringify(参数)`），参数里放一个标记串即可，
// 断言「标记串出现在第二次请求里」不依赖任何路径解析。
//
// ## 为什么工具调用场景要 `once: true`（与 v1 的 T2 不同）
//
// T2（v1）里工具结果轮的**最后一条消息 role = tool**，mock 靠 `afterTool` 场景区分。
// v2 同样是结构化消息（末条也是 `role: tool`），但**本用例不声明 `afterTool` 场景**：
// mock 的 `afterTool` 是**优先池**而非硬过滤，池子选不出就退回全池，于是带 `match` 的
// 工具场景会在工具结果轮**再次命中** ⇒ 无限工具循环。所以给工具场景加 `once: true`：
// 它只烧一次，第二次请求落到下一条无 `match` 场景（`after-echo`）上。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 对话面（`classify` 判决 / `compose` 措辞）出厂默认开启，会给每一轮多加一次静默
// 请求。本用例的断言按**精确请求数**（2 次）写，且验的是工具链路——与对话面正交，
// 故按 `DIALOG_FACE_OFF` 的约定把它钉死。
import { join } from 'node:path';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  readFileSyncSafe,
  startReadbackCli,
  readSessionMessages,
  nextPort,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  waitFor,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t26';

/** mock-mcp 回显的内容标记：它必须从工具结果一路走到第二次请求体 */
const ECHO_TEXT = 'mock 回显内容';

export default defineCase(
  'T26 工具轮 v2 化：工具调用真的执行 → 结果回灌下一次请求 → 转写里 tool_call 与结果成对',
  async () => {
    const llm = await new MockLlm([
      // 第一次请求：模型请求一个工具（带一段中途正文，逼出「定格并切节点」路径）。
      // `once: true` —— 第二次请求（工具结果轮）若不声明 `afterTool` 场景，优先池选不出
      // 就退回全池，本场景（带 `match`）会**再次命中** ⇒ 无限工具循环；`once` 烧一次断开
      // 重入（见文件头）。
      {
        id: 'call-echo',
        match: '帮我回显',
        once: true,
        content: '我先回显一下。',
        toolCalls: [
          { id: 'call_1', name: 'mcp__mockserv__echo', arguments: { text: ECHO_TEXT } },
        ],
      },
      // 第二次请求（工具结果已回灌）：不再请求工具 ⇒ 本轮收束。
      { id: 'after-echo', content: '工具调用完成，回显成功。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 转写经生产读出口读（gateway `invoke` → `vdfs/read`），入站口必须开着。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 本用例的主题与对话面正交 ⇒ 把三个开关钉死（理由见 `DIALOG_FACE_OFF`）。
        session: { ...DIALOG_FACE_OFF },
      },
    });
    // MCP stdio 工具：`--record` 是「工具真的执行过」的进程外证据。
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
        message: '帮我回显一下',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);
      assert(
        r.stdout.includes('工具调用完成，回显成功'),
        `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`,
      );

      // ── 证据 ①：工具真的执行了（进程外，不看后端自述）────────────────────
      const record = readFileSyncSafe(join(hd.homedir, 'mcp-record.ndjson'));
      const calls = record
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l))
        .filter((e) => e.kind === 'call');
      assertEq(calls.length, 1, `mock-mcp 应恰好收到一次 tools/call（实际 ${calls.length} 次）`);
      assertEq(calls[0].name, 'echo', '被调工具名');
      assertEq(calls[0].args, { text: ECHO_TEXT }, '工具参数');

      // ── 证据 ②：工具结果进了**下一次请求**（只能靠回读请求体证明）─────────
      const reqs = await llm.requests();
      assertEq(reqs.length, 2, 'mock-llm 应收到两次请求（工具轮 + 收尾轮）');

      // 工具清单确实下行（批 1 的 §2.1 工具通道）：第一次请求里看得见 MCP 工具。
      const tools0 = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      assert(
        tools0.includes('mcp__mockserv__echo'),
        `MCP 工具应注册进第一次请求（实际: ${tools0.join(',')}）`,
      );

      // 第二次请求：模型请求过的工具 + 工具结果都在请求里。
      //
      // ⚠️ 判据在**消息层**上：验「调用在」「结果在」「结果是 `tool` 角色」「工具名
      // 对得上」（D-048a）——而不是「某个字符串出现过」。
      const secondBody = reqs[1].body;
      const secondMsgs = secondBody.messages ?? [];
      const llmCalls = secondMsgs.flatMap((m) => m.tool_calls ?? []);
      assert(
        llmCalls.some((c) => c.function?.name === 'mcp__mockserv__echo'),
        `第二次请求应带上模型请求过的工具（实际: ${JSON.stringify(llmCalls).slice(0, 400)}）`,
      );
      const toolResults = secondMsgs.filter((m) => m.role === 'tool');
      assert(
        toolResults.length > 0,
        `第二次请求应带上 role=tool 的结果（实际: ${JSON.stringify(secondMsgs.map((m) => m.role)).slice(0, 300)}）`,
      );
      const second = JSON.stringify(secondBody);
      assert(
        second.includes(ECHO_TEXT),
        '工具结果正文（回显内容）应原样回灌给模型',
      );
      // 反向：第一次请求里不该有工具结果（那时还没执行）。
      assert(
        (reqs[0].body.messages ?? []).every((m) => m.role !== 'tool'),
        '第一次请求不该出现 role=tool 消息（结果尚未产生）',
      );

      // ── 转写不变量：tool_call 与结果成对（经生产读出口读回）────────────────
      const api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T26');
        const tc = msgs.find((m) => msgTypeOf(m) === 'tool_call');
        assert(tc, `应有 tool_call 节点（实得类型: ${msgs.map((m) => m.type).join(',')}）`);
        assert(
          msgs.some((m) => msgRoleOf(m) === 'tool' && m.parent_id === tc.id),
          '工具结果必须挂在 tool_call 节点之下',
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t4 同一约定：Windows 上带活子进程退出会撞
      // libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
