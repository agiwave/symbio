import './_selfrun.mjs';
// T38 工具事实的投影消费：跨轮的工具结果进 `transcript` 投影 ⇒ **第二轮**请求的
// prompt 里带得着**第一轮**的工具结果。
//
// ## 本用例钉的是什么
//
// [10 §3 批 3](../../docs/004-plan/008-工具轮v2化实施方案.md "PL-008-工具轮v2化实施方案") 的出口判据原文：
// 「两轮工具轮的会话，第二轮请求包里带第一轮的工具结果」。
//
// 为什么必须端到端：单测（`actors/mod.test.rs::transcript_includes_artifact_as_tool_line`）
// 已经证了「`artifact.added` 进投影 + 渲染成工具行」，但它证不了**这条事实真的
// 由一次真实工具调用产生、并真的走到下一轮的请求体里**——单测里的网格是手搭的。
//
// ## 为什么「跨轮」才测得出投影消费
//
// 工具轮**当轮**的交换由运行器**就地追加**（不经投影）——所以**轮 0 的第二次请求**
// 带工具结果是「本来就对」的（T26 已证）。本用例要证的是**跨轮**：轮 1 的 prompt
// 只由投影渲染（基线），若 `artifact.added` 不进投影，轮 1 就看不见轮 0 的工具结果
// ⇒ 模型会重复调用同一个工具。三处请求因此各有分工：
//
// | 请求 | 工具结果从哪来 | 证什么 |
// |---|---|---|
// | ① 轮 0 首轮 | ——（还没有） | 反向：此刻不该有工具结果 |
// | ② 轮 0 收尾轮 | 轮内交换（运行器就地追加） | 当轮路径照常（与 T26 同形） |
// | ③ 轮 1（新用户发言） | **投影**（`transcript`） | **本批的主题**：跨轮带得着 |
//
// ## 为什么两轮用两次 `runCli`（而不是一轮里塞两次发言）
//
// 两次 `runCli` 是**两条独立用户发言**（各自开一轮），共享同一个会话目录 ⇒ 同一份
// 事实源。这正是「跨轮」的定义。同一轮内不会有第二个用户格（续写走 resume）。
//
// ## 为什么工具调用场景要 `once: true`（与 T26 同因）
//
// 本用例不声明 `afterTool` 场景，而 mock 的 `afterTool` 是**优先池**而非硬过滤——
// 池子选不出就退回全池，带 `match` 的工具场景会在工具结果轮**再次命中** ⇒ 无限工具
// 循环。加 `once: true` 后它只烧一次，第二次请求落到下一条无 `match` 场景上。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  startReadbackCli,
  readSessionMessages,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  E2E_ROOT,
} from '../helpers.mjs';

/** 本用例的会话 id（两轮共享） */
const SID = 'e2e-t38';

/** mock-mcp 回显的内容标记：它必须从轮 0 的工具结果一路走到轮 1 的请求体 */
const ECHO_TEXT = '跨轮回显内容';

/** 被调工具名（MCP 前缀 + 服务名 + 工具名，与 T26 同一口径） */
const ECHO_TOOL = 'mcp__mockserv__echo';

/** 取请求体的**未转义 prompt 正文**（摊所有 message 的字符串 content）。 */
function promptOf(body) {
  return (body?.messages ?? [])
    .map((m) => (typeof m.content === 'string' ? m.content : ''))
    .join('\n');
}

export default defineCase(
  'T38 工具事实的投影消费：跨轮工具结果进 transcript 投影 ⇒ 第二轮请求带第一轮的工具结果',
  async () => {
    const llm = await new MockLlm([
      // ① 轮 0 首轮：模型请求工具（带一段中途正文，逼出「定格并切节点」路径）。
      {
        id: 'echo',
        match: '帮我回显',
        once: true,
        content: '我先回显一下。',
        toolCalls: [{ id: 'call_1', name: ECHO_TOOL, arguments: { text: ECHO_TEXT } }],
      },
      // ② 轮 0 收尾轮（工具结果已回灌）：不再请求工具 ⇒ 本轮收束。
      { id: 'r0-final', once: true, content: '第一答：回显完成。' },
      // ③ 轮 1（新的用户发言）：收束。它的 prompt 只由**投影**渲染。
      { id: 'r1-final', content: '第二答。' },
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
    // MCP stdio 工具：`echo` 的返回是确定的（`JSON.stringify(参数)`），断言
    // 「标记串进了请求体」不依赖任何路径解析（与 T26 同因）。
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs')],
      enabled: true,
    });

    try {
      // ── 轮 0：工具轮（工具真的执行、结果真的回灌当轮请求）────────────────────
      const r0 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我回显一下',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r0.code, 0, `轮 0 CLI 退出码（stderr: ${r0.stderr.slice(0, 600)}）`);

      // ── 轮 1：新的用户发言（同一会话 ⇒ 同一份事实源）────────────────────────
      const r1 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '再问一句',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r1.code, 0, `轮 1 CLI 退出码（stderr: ${r1.stderr.slice(0, 600)}）`);

      // ── 证据 ①：三次请求（轮 0 × 2 + 轮 1 × 1）──────────────────────────────
      const reqs = await llm.requests();
      assertEq(reqs.length, 3, `mock-llm 应收到三次请求（实得 ${reqs.length}）`);

      const p0 = promptOf(reqs[0].body);
      const p1 = promptOf(reqs[1].body);
      const p2 = promptOf(reqs[2].body);

      // ⚠️ 判据在**消息层**上：验「这条内容在**哪个角色**的消息里」（ADR-048a）
      // ——`role: user` / `role: assistant` / `role: tool` 三种消息，而不是文本前缀；
      // 「把用户发言标成 assistant」这类退化因此会红。
      // ⚠️ 只读 **LLM 请求包**里的消息——「用户如何对话就如何读」：
      // 请求消息 = 模型真正看到的那一份（前端发什么，这里就断言什么）。
      const msgsOf = (i) => reqs[i].body.messages ?? [];
      const textsOf = (i, role) =>
        msgsOf(i)
          .filter((m) => m.role === role)
          .map((m) => (typeof m.content === 'string' ? m.content : ''));
      const toolTexts = (i) => textsOf(i, 'tool');
      // 跨轮工具结果在请求里是 **assistant 消息**，不是 `role: tool`——
      // 见 `provider_adapter` 里的降级与理由（协议要求 tool 消息配对**本次**请求
      // 的 tool_calls，跨轮的调用在上一轮，于是那条消息会被清洗段当孤儿丢掉）。
      const crossTurnTexts = (i) => textsOf(i, 'assistant');
      const saw = (i, role, needle) => textsOf(i, role).some((t) => t.includes(needle));

      // 反向：轮 0 首轮请求里不该有工具结果（那时还没执行）。
      assert(
        toolTexts(0).length === 0,
        `轮 0 首轮请求不该出现 role=tool 消息（实际: ${JSON.stringify(msgsOf(0).map((m) => m.role))}）`,
      );

      // 当轮路径照常（与 T26 同形）：轮 0 收尾轮的工具结果来自**轮内交换**。
      assert(
        toolTexts(1).some((t) => t.includes(ECHO_TEXT)),
        `轮 0 收尾轮应带 role=tool 的结果（轮内交换）（实际: ${JSON.stringify(toolTexts(1)).slice(0, 400)}）`,
      );

      // ── 证据 ②：**本用例的主题**——轮 1 的请求带轮 0 的工具结果（投影消费）──
      // 轮 1 的消息由 `render_messages`（= `transcript` 投影 → `to_messages`）产出，
      // 轮内交换**不跨轮**。它带得着，只可能是因为 `artifact.added` 进了投影。
      assert(
        crossTurnTexts(2).some((t) => t.includes(ECHO_TEXT)),
        `轮 1 请求应带**第一轮**的工具结果（跨轮投影消费）（实际: ${JSON.stringify(crossTurnTexts(2)).slice(0, 400)}）`,
      );
      // 轮 1 的历史里也看得见轮 0 的问与答（投影的多轮形态没被工具消息挤掉），
      // 且**角色正确**——问是 user 说的、答是 assistant 说的。
      assert(saw(2, 'user', '帮我回显一下'), '轮 1 历史应含轮 0 的用户发言（role=user）');
      assert(
        saw(2, 'assistant', '第一答：回显完成。'),
        '轮 1 历史应含轮 0 的收束正文（role=assistant）',
      );
      assert(
        textsOf(2, 'user').at(-1)?.includes('再问一句') ?? false,
        `当前发言在历史之外且是最后一条 user（实际末条: ${JSON.stringify(textsOf(2, 'user').at(-1))}）`,
      );

      // ── 转写不变量（经生产读出口读回：与前端 loadMessages 同一出口）────────
      const api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T38');
        const tc = msgs.find((m) => (m.type ?? m.msg_type) === 'tool_call');
        assert(tc, `转写里应有 tool_call 节点（实得类型: ${msgs.map((m) => m.type).join(',')}）`);
        assert(
          msgs.some((m) => m.role === 'tool' && m.parent_id === tc.id),
          '工具结果必须挂在 tool_call 节点之下（有请求必有响应）',
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t26 同一约定：Windows 上带活子进程退出会撞
      // libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '读回 CLI / mock-llm 子进程退出',
      });
    }
  },
);
