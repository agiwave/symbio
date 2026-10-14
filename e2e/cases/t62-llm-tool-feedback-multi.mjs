import './_selfrun.mjs';
// T62 多工具反馈契约（LC-004）：同一轮里的每次调用都有配对结果，错误路径也不例外。
//
// ## 本用例钉的是什么
//
// 「有请求必有响应」在模型眼前的形态：一轮里模型连发两次工具调用（一成一败），
// 收尾轮请求必须带回**两条** `role=tool` 结果，各自 `tool_call_id` 与调用严格
// 配对——串格（A 的结果配到 B 名下）在多工具轮里最危险：模型按错误的事实继续
// 推理，而两侧单看都对。
//
// 本用例同时钉三条只有请求体能判的事实：
// ① 两次调用都真的执行了（mock-mcp 的 `--record` NDJSON：进程外证据）；
// ② 错误工具的失败信息**原样回灌**（模型必须看见失败才能换路——t6 判会话
//    照常收敛的行为面，这里判请求面的回灌保真）；
// ③ 调用 id 互不相同的两次调用产生两条结果（id 归并退化 ⇒ 只剩一格，
//    另一格的结果凭空消失——mock-llm `buildToolCallId` 的注释记过这个症状）。
//
// ## 场景
//
// 一轮两调用：`echo`（成功，回显固定文本）+ `add`（mock-actions.json 注入
// JSON-RPC 错误）。工具的真实产出分别从两处取：echo 的结果形态由 mock-mcp
// 默认行为定（`content = JSON.stringify(args)`），错误的 message 由脚本给定
// ——用例不自己编「工具会返回什么」。
import { join } from 'node:path';
import { readFileSync } from 'node:fs';
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
  readFileSyncSafe,
} from '../helpers.mjs';
import {
  assertToolFeedback,
  toolCallsOf,
  toolResultsOf,
  dialogOf,
  textOf,
} from '../llm-contract.mjs';

const U = '同一轮里调两个工具';
const ECHO_ARGS = { text: 'T62回显内容' };
const ERR_MSG = 'mock 注入的工具执行失败'; // mock-actions.json 的 onCall.add.error

export default defineCase(
  'T62 多工具反馈契约：同轮两调用各自配对、错误路径照样回灌、无 id 归并（LC-004）',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'dual',
        match: U,
        once: true,
        content: '两个工具都调了。',
        toolCalls: [
          { id: 'call_a', name: 'mcp__mockserv__echo', arguments: ECHO_ARGS },
          { id: 'call_b', name: 'mcp__mockserv__add', arguments: { a: 1, b: 2 } },
        ],
      },
      { id: 'after-tool', afterTool: true, content: '收尾：结果都拿到了。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
    });
    const recordPath = join(hd.homedir, 'mcp-record.ndjson');
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [
        join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'),
        '--script',
        join(E2E_ROOT, 'e2e', 'mock-actions.json'),
        '--record',
        recordPath,
      ],
      enabled: true,
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: U,
        provider: PROVIDER_ID,
        session: 'e2e-t62',
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      assertEq(reqs.length, 2, '应两条请求（工具轮 + 收尾轮）');

      // ── ① 配对契约：两次调用各有结果，id 严格对应 ───────────────────────
      assertToolFeedback(reqs[1], 'T62① 收尾轮配对');
      const calls = toolCallsOf(reqs[1]);
      assertEq(calls.length, 2, `收尾轮应重放两次调用（实际 ${calls.length}——id 归并会少一格）`);
      const ids = calls.map((c) => c.id);
      assertEq(new Set(ids).size, 2, `两次调用的 id 必须不同（实际: ${JSON.stringify(ids)}）`);
      const results = toolResultsOf(reqs[1]);
      assertEq(results.length, 2, `应带回两条 role=tool 结果（实际 ${results.length}）`);
      // 配对方向也判：每条结果的 id 都能在调用里找到（反向防止「多出幽灵结果」）
      for (const res of results) {
        assert(
          ids.includes(res.tool_call_id),
          `结果 ${res.tool_call_id} 没有对应调用（幽灵结果）`,
        );
      }

      // ── ② 两条结果各自的内容保真 ────────────────────────────────────────
      const echoRes = results.find((x) => x.tool_call_id === 'call_a');
      const addRes = results.find((x) => x.tool_call_id === 'call_b');
      assert(echoRes && addRes, '两条结果都应在场（按 id 取）');
      // echo 的真实产出 = mock-mcp 默认行为 `JSON.stringify(args)`
      assert(
        textOf(echoRes).includes(JSON.stringify(ECHO_ARGS)),
        `echo 结果应含工具真实产出 ${JSON.stringify(ECHO_ARGS)}（实际: ${textOf(echoRes)}）`,
      );
      // 错误路径：失败信息原样回灌（模型看得见失败才会换路）
      assert(
        textOf(addRes).includes(ERR_MSG),
        `add 的失败信息应原样回灌（实际: ${textOf(addRes)}）`,
      );

      // ── ③ 进程外证据：两次调用都真的执行了 ──────────────────────────────
      const record = readFileSyncSafe(recordPath);
      const calls1 = record
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l))
        .filter((e) => e.kind === 'call');
      assertEq(calls1.length, 2, `mock-mcp 应收到两次 tools/call（实际 ${calls1.length}）`);
      assertEq(calls1.map((c) => c.name).sort().join(','), 'add,echo', '被调工具名（两个都执行）');
      const echoCall = calls1.find((c) => c.name === 'echo');
      assertEq(echoCall.args, ECHO_ARGS, 'echo 的入参应完整到达工具');

      // ── ④ 同轮请求里的形态：每次调用后紧跟自己的结果（相邻配对）──────────
      // 实测形态（2026-10-14 探针钉下）：多工具轮在 wire 上不是「一条 assistant
      // 带两个 tool_calls」，而是**每次调用各一条 assistant 消息、后随自己的
      // role=tool 结果**——相邻配对。判据钉这个相邻性：调用与结果之间插进别
      // 的东西（另一调用的结果 / 正文）就是串格的前兆。
      const seq = (reqs[1].body.messages ?? []).map((m) => m.role);
      assertEq(
        seq.join(','),
        'system,user,assistant,assistant,tool,assistant,tool',
        `收尾轮的消息序列应为「用户 → 模型正文 → (调用,结果) × 2」（实际: ${seq.join(',')}）`,
      );
      const withCalls = dialogOf(reqs[1]).filter((m) => (m.tool_calls ?? []).length > 0);
      assertEq(withCalls.length, 2, '两次调用应各占一条 assistant 消息');
      for (const m of withCalls) {
        assertEq(m.tool_calls.length, 1, '每条带调用的 assistant 消息应恰含一个 tool_calls 条目');
      }
      // 相邻性：每条「调用消息」的下一条必须是对应 id 的 tool 结果
      const msgs1 = reqs[1].body.messages ?? [];
      for (const [i, m] of msgs1.entries()) {
        if (!(m.tool_calls ?? []).length) continue;
        const next = msgs1[i + 1];
        assert(
          next && next.role === 'tool' && next.tool_call_id === m.tool_calls[0].id,
          `调用 ${m.tool_calls[0].id} 的下一条必须是对应 id 的 tool 结果（实际: ${next?.role}/${next?.tool_call_id}）`,
        );
      }
      // 参数完整性（流式两片聚合的终点）：add 的入参可解析且值正确
      const addCall = calls.find((c) => c.function?.name === 'mcp__mockserv__add');
      const addArgs = JSON.parse(addCall.function?.arguments ?? '{}');
      assertEq(addArgs, { a: 1, b: 2 }, 'add 入参应完整聚合（不是半截 JSON）');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
