import './_selfrun.mjs';
// T58 多轮历史保真（LC-003）：第 N 轮请求 = 前 N-1 轮完整重放 + 本轮发言。
//
// ## 本用例钉的是什么
//
// 历史重放是「用户请求 → 模型调用」里最长的一段中间态：转写里存了什么、
// 请求视图就重放什么，中间任何一处丢 / 重 / 错位，模型看到的都是一个
// **被剪裁过的过去**——而它回答时引用「我们刚才说的」，用户才发现对不上。
// 这类失效在响应侧没有形态（回答照常产出），判据只能从请求体取。
//
// 三轮场景（第 1 轮带一次 MCP 工具轮 ⇒ 产生 4 条请求），逐条判角色序列：
//   req0  [user1]
//   req1  [user1, assistant, assistant(tool_calls), tool]      ← 工具结果回灌轮
//   req2  [user1, assistant(工具轮塌缩), assistant, user2]      ← 下一轮的重放
//   req3  [user1, assistant(塌缩), assistant, user2, assistant, user3]
//
// 两个塌缩契约同场钉：① 同一轮内工具轮是三条消息（正文 + tool_calls + 结果），
// ② 后续轮里它塌成**一条**带「（此前调用工具 X 的结果）」标记的 assistant
// 消息——不塌（每轮重放三连）会撑爆上下文，塌丢（结果没了）就是 T-095 的失忆。
//
// ## 与 t55 的分工
//
// t55 判「多工具轮的结果累进 + 末轮思考收束」（T-095 的单点守卫）；本用例判
// **逐请求的角色序列与发言进场时序**——重放结构的全量形状，不含工具轮细节的
// 语义（那归 t55 / t26）。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  addMcpServer,
  runCli,
  E2E_ROOT,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import {
  assertHistoryReplay,
  assertUtteranceReplay,
  assertToolResultCarriedForward,
  assertNoToolResultsInFirstTurn,
  assertToolFeedback,
  dialogOf,
  textOf,
  toolCallsOf,
} from '../llm-contract.mjs';

const ECHO_TEXT = 'T58回显内容';
const U1 = '第一轮：帮我回显一下';
const U2 = '第二轮：问点别的';
const U3 = '第三轮：再问一个';
const TOOL = 'mcp__mockserv__echo';

/** 塌缩标记（`flatten_chat_messages` 对历史工具轮的既定形态，镜像 Rust 侧常量口径） */
const COLLAPSE_MARK = '（此前调用工具';

export default defineCase(
  'T58 多轮历史保真：逐请求角色序列 + 发言进场时序 + 工具轮塌缩契约（LC-003）',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'tool',
        match: U1,
        once: true,
        content: '我先回显。',
        toolCalls: [{ id: 'call_1', name: TOOL, arguments: { text: ECHO_TEXT } }],
      },
      { id: 'after-tool', afterTool: true, content: '工具调用完成。' },
      { id: 't2', match: U2, content: '第二轮回答。' },
      { id: 't3', match: U3, content: '第三轮回答。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
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
        session: 'e2e-t58',
        stdinText: `${U1}\n${U2}\n${U3}\n`,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      assertEq(reqs.length, 4, '三轮对话应产生 4 条请求（第 1 轮工具轮 + 结果轮，第 2/3 轮各一条）');

      // ── ① 逐请求角色序列（历史保主的主体判据）──────────────────────────
      assertHistoryReplay(
        reqs,
        [
          ['user'], // req0：第 1 轮首请求
          ['user', 'assistant', 'assistant', 'tool'], // req1：工具结果回灌轮
          ['user', 'assistant', 'assistant', 'user'], // req2：第 2 轮（工具轮已塌缩）
          ['user', 'assistant', 'assistant', 'user', 'assistant', 'user'], // req3：第 3 轮
        ],
        'T58① 角色序列',
      );

      // ── ② 发言进场时序：原话不重不漏、不提前─────────────────────────────
      // introAt：U1 从 req0 起在场；U2 从 req2（第 2 轮首请求）起；U3 从 req3 起。
      assertUtteranceReplay(
        reqs,
        { utterances: [U1, U2, U3], introAt: [0, 2, 3] },
        'T58② 进场时序',
      );

      // ── ③ 工具反馈契约（模型眼前：调用 ⇐⇒ 结果）────────────────────────
      assertToolFeedback(reqs[1], 'T58③ 工具结果轮');
      // req1 里那次调用必须带参数（模型意图的完整传递）
      const calls = toolCallsOf(reqs[1]);
      assertEq(calls.length, 1, 'req1 应恰有一次工具调用');
      assertEq(calls[0].function?.name, TOOL, '被调工具名');
      const args = JSON.parse(calls[0].function?.arguments ?? '{}');
      assertEq(args.text, ECHO_TEXT, '工具参数应完整流式聚合（不是半截 JSON）');

      // ── ④ 工具轮塌缩契约：同一轮内三连、后续轮里一条带标记────────────────
      const req2ToolMsgs = dialogOf(reqs[2]).filter((m) => textOf(m).includes(ECHO_TEXT));
      assertEq(
        req2ToolMsgs.length,
        1,
        `req2 里工具结果应塌缩成一条 assistant 消息（实际 ${req2ToolMsgs.length} 条）`,
      );
      assertEq(req2ToolMsgs[0].role, 'assistant', '塌缩后是 assistant 消息（不是 tool——tool 消息只属于它自己的轮次）');
      assert(
        textOf(req2ToolMsgs[0]).includes(COLLAPSE_MARK),
        `塌缩消息应带「${COLLAPSE_MARK}」标记（实际: ${textOf(req2ToolMsgs[0]).slice(0, 80)}）`,
      );
      assert(
        textOf(req2ToolMsgs[0]).includes(TOOL),
        '塌缩消息应带工具名（结果与调用对得上）',
      );

      // ── ⑤ 结果跨轮存活（T-095 失忆族的结构面）────────────────────────────
      assertToolResultCarriedForward(
        reqs,
        { toolName: TOOL, resultText: ECHO_TEXT, fromIndex: 1 },
        'T58⑤ 结果跨轮存活',
      );
      assertNoToolResultsInFirstTurn(reqs, 'T58⑤');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
