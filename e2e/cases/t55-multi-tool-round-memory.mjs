import './_selfrun.mjs';
// T55 多工具轮的记忆与末轮思考：一次请求工具两次 ⇒ 第三次请求必须**两条结果都在**，
// 末轮只有思考没有正文 ⇒ 收束成答案而不是兜底。
//
// ## 本用例钉的是什么
//
// 一次真实事故（会话 `f1566610`，18 轮 / 23 次工具调用 / **零** `chat.assistant.final`）
// 的两种失效形态，各自对应一条已修的缺陷：
//
// | 缺陷 | 症状（当时实测） | 本用例的判据 |
// |---|---|---|
// | 交换每轮清空（`exchange.clear()`） | 第 3 次请求看不见第 1 次的工具结果 ⇒ 模型以为没读过，**把同一个文件再读一遍**（同一 `vdfs_read docs/006-ref/006-当前事实表.md` 出现 4 次） | 第 3 次请求里 `role=tool` **两条都在**，正文分别是第 1、第 2 轮的结果 |
// | 末轮只读 `text` 不读 `reasoning` | reasoning 模型「想清楚了不复述」⇒ 末轮被判 `model returned empty text`，整轮落兜底格，界面上是**一串没有解释的连续思考** | 收束格是 `chat.assistant.final`（状态 completed），正文 = 那段思考 |
// | 收束帧把「思考即正文」那一段标成 `Reasoning` | 同一段内容落库与网格重建都是 `Text`、流式层却是 `Reasoning` ⇒ live 与刷新后长得不一样；CLI 非交互只回显 `Text`，于是正文一个字没打，还以「模型没有产出文本内容」退出 1 | `runCli` 的 stdout **含那段思考**且退出码 0 |
//
// ## 为什么单测盖不住、必须端到端
//
// `turn_runner.test.rs` 里那两条守卫（`two_tool_rounds_accumulate_…`、
// `reasoning_only_final_round_closes_as_answer_not_fallback`）用假分发端口，它们证的是
// **运行器给了什么**。而「结果到底有没有送到模型眼前」中间还隔着两道人不知道的东西：
// 协议适配器按 `tool_call_id` 取「同一次调用的最终结果」（`last_result_of`），以及
// 请求包清洗会**丢弃没有对应 `tool_calls` 声明的 tool 结果**。这两道都在把交换里的
// 第 1 轮结果吃掉的可能路径上——只看运行器的断言，它们全是绿的，模型却什么也没收到。
// 唯一能证「第三次请求体里两条结果都在」的地方是 mock-llm 的 `/_requests` 回读。
//
// ## 为什么断言 id 逐调用互不相同
//
// 结果与调用**配对**靠 id。合成 id 若按工具名生成，两次调用同一个工具就是同一个 id ⇒
// `last_result_of` 把两条结果并成一条，第 1 轮的真实结果被第 2 轮顶掉。错配比缺结果
// 更坏：模型据别人的正文改道。所以除了「两条都在」，还钉「两条的 `tool_call_id`
// 分别等于两次调用声明的 id」。
//
// ## 为什么把对话面钉死、为什么工具场景带 `once`
//
// 与 [t26](t26-v2-tool-round.mjs) / [t46](t46-prompt-shape-baseline.mjs) 同一取舍：
// 断言按**精确请求数**（3 次）写，对话面的静默请求会让下标依赖分类器的路由结果；
// `once` 烧掉重入名额，否则带 `match` 的工具场景在工具结果轮再次命中 ⇒ 无限工具循环。
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

/** 会话 id 固定（与 t46 同一理由：它出现在 prompt 的记忆地址里） */
const SID = 'e2e-t55';

/** 第 1 轮工具的回显标记：必须一路走到**第三次**请求体（第 2 轮的清空缺陷就是吃掉它） */
const ECHO_A = '第一轮回显内容';
/** 第 2 轮工具的回显标记 */
const ECHO_B = '第二轮回显内容';
/** 第 1 轮的中途正文：跨轮累加后仍要在第三次请求里看得见（清空 = 模型丢掉自己的计划） */
const ROUND1_TEXT = '先读第一个。';
/** 末轮只有思考、没有正文——收束答案应当是这段思考 */
const REASONING_ANSWER = '两轮都读到了，可以收束。';

export default defineCase(
  'T55 多工具轮跨轮累加 + 末轮思考即答案：第三次请求带齐两条结果，收束不落兜底',
  async () => {
    const llm = await new MockLlm([
      // 第 1 次请求：正文 + 一次工具调用（`once` 只烧一次，见文件头）。
      {
        id: 'round1',
        match: '连读两次',
        once: true,
        content: ROUND1_TEXT,
        toolCalls: [{ id: 'tc-1', name: 'mcp__mockserv__echo', arguments: { text: ECHO_A } }],
      },
      // 第 2 次请求（末条是 tool 结果 ⇒ `afterTool` 池）：再调一次**同名**工具。
      // 场景顺序即优先级：`choose` 取池子里第一个无 `match` 的，名额烧完才轮到末轮。
      {
        id: 'round2',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'tc-2', name: 'mcp__mockserv__echo', arguments: { text: ECHO_B } }],
      },
      // 第 3 次请求：不再请求工具，且**只给思考不给正文**（真实 reasoning 模型的形态）。
      { id: 'final-reasoning-only', afterTool: true, reasoning: [REASONING_ANSWER] },
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
        message: '帮我连读两次再收束',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);
      // 缺陷 B 的用户可见面：答案就是那段思考。落到兜底时这里是兜底话术，永远不含它。
      assert(
        r.stdout.includes(REASONING_ANSWER),
        `stdout 应为末轮思考充当成答（实际: ${JSON.stringify(r.stdout)}）`,
      );

      // ── 证据 ①：工具真的执行了两次，且**不是**同一次调用被重放 ──────────────
      const calls = readFileSyncSafe(join(hd.homedir, 'mcp-record.ndjson'))
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l))
        .filter((e) => e.kind === 'call');
      assertEq(calls.length, 2, `mock-mcp 应恰好收到两次 tools/call（实际 ${calls.length} 次）`);
      assertEq(
        calls.map((c) => c.args?.text),
        [ECHO_A, ECHO_B],
        '两次调用应按轮次各执行一次（第 1 轮的结果被吃掉 ⇒ 模型重读 ⇒ 这里是重复的同一个参数）',
      );

      // ── 证据 ②：第三次请求体带齐**两轮**的结果 ────────────────────────────
      const reqs = await llm.requests();
      assertEq(reqs.length, 3, 'mock-llm 应收到三次请求（工具轮 / 第二个工具轮 / 收束轮）');

      const third = (reqs[2].body.messages ?? []);
      const toolResults = third.filter((m) => msgRoleOf(m) === 'tool');
      assertEq(toolResults.length, 2, `第三次请求应带两条 role=tool 的结果（实际 ${toolResults.length} 条：第 1 轮的被清空了）`);
      assertEq(
        toolResults.map((m) => m.content).sort(),
        [JSON.stringify({ text: ECHO_A }), JSON.stringify({ text: ECHO_B })].sort(),
        '两条结果正文应分别是第 1、第 2 轮的回显（mock-mcp 的 echo 回显参数 JSON）',
      );
      // 配对靠 id：逐调用互不相同，且各自对得上自己那一次调用的声明。
      const resultIds = toolResults.map((m) => m.tool_call_id);
      assertEq(new Set(resultIds).size, 2, `两条结果的 tool_call_id 必须互不相同（实际: ${resultIds.join(',')})`);
      const declaredIds = third.flatMap((m) => (m.tool_calls ?? []).map((c) => c.id));
      assertEq(
        [...declaredIds].sort(),
        ['tc-1', 'tc-2'],
        'assistant 消息应声明两次调用（tool 结果没有对应声明会被协议层丢弃 ⇒ 无限工具循环）',
      );
      for (const tr of toolResults) {
        assert(
          declaredIds.includes(tr.tool_call_id),
          `tool 结果 ${tr.tool_call_id} 在本次请求里没有对应的调用声明（会被协议层丢弃）`,
        );
      }
      // 第 1 轮的中途正文也在：清空交换连模型自己的计划一起清掉。
      const assistantTexts = third.filter((m) => msgRoleOf(m) === 'assistant').map((m) => m.content ?? '');
      assert(
        assistantTexts.some((t) => t.includes(ROUND1_TEXT)),
        `第三次请求应保留第 1 轮的中途正文（实际 assistant 正文: ${JSON.stringify(assistantTexts).slice(0, 400)}）`,
      );
      // 反向：第 2 次请求只有第 1 条结果（还没执行第 2 轮）。
      assertEq(
        (reqs[1].body.messages ?? []).filter((m) => m.role === 'tool').length,
        1,
        '第二次请求只应带第 1 轮的结果',
      );

      // ── 证据 ③：转写里这一轮是**收束**，不是兜底 ──────────────────────────
      const api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T55');
        const answer = msgs.find(
          (m) => msgRoleOf(m) === 'assistant' && msgTypeOf(m) === 'text' && (m.content ?? '') === REASONING_ANSWER,
        );
        assert(
          answer,
          `应有一条正文节点就是末轮思考（实得: ${msgs.map((m) => `${m.type}:${JSON.stringify(m.content ?? '').slice(0, 40)}`).join(' | ')})`,
        );
        assertEq(answer.status, 'completed', '收束格的状态（兜底是 failed）');
        assert(
          !msgs.some((m) => m.status === 'failed'),
          '本轮不该有任何失败格（末轮只有思考被判成 empty text 时整轮落兜底）',
        );
        const toolCalls = msgs.filter((m) => msgTypeOf(m) === 'tool_call');
        assertEq(toolCalls.length, 2, '两次调用应有两张工具卡（同 id 被归并 ⇒ 只剩一张）');
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t4 / t26 同一约定）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
