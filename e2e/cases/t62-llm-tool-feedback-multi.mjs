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
  dialogFaceOff,
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
      pluginConfigs: { ...dialogFaceOff() },
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
      const r = await runCli({
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

      // ── ④ 同轮请求里的形态：**一 assistant 可承载多个 tool_calls**，其后紧跟
      // 同数量的 role=tool 结果，`tool_call_id` 与 `tool_calls[].id` 逐一配对。
      //
      // ⚠️ 这是 provider 协议的**标准形态**，不是放宽：两种承载方式（一调用一
      // assistant / 一 assistant 多调用）都合法，判据钉的是**配对本身**。钉
      // 「恰好一调用一 assistant」等于钉当年探针看到的那一种写法，换个合法承载
      // 方式就假红——那正是「钉了绕开问题时采用的形态」。
      const msgs1 = reqs[1].body.messages ?? [];
      const callMsgs = msgs1.filter((m) => (m.tool_calls ?? []).length > 0);
      assert(
        callMsgs.length >= 1,
        `收尾轮应重放至少一条带 tool_calls 的 assistant（实际 ${callMsgs.length}）`,
      );
      for (const [i, m] of msgs1.entries()) {
        const ids = (m.tool_calls ?? []).map((c) => c.id);
        if (!ids.length) continue;
        const after = msgs1.slice(i + 1, i + 1 + ids.length);
        assert(
          after.length === ids.length && after.every((n) => n.role === 'tool'),
          `调用 ${ids.join('/')} 之后必须紧跟 ${ids.length} 条 role=tool 结果（实际: ${JSON.stringify(after.map((n) => n.role))}）`,
        );
        assertEq(
          after.map((n) => n.tool_call_id).sort().join(','),
          ids.slice().sort().join(','),
          `调用与结果必须逐一配对（tool_call_id ↔ tool_calls[].id；实际调用 ${JSON.stringify(ids)} / 结果 ${JSON.stringify(after.map((n) => n.tool_call_id))}）`,
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
