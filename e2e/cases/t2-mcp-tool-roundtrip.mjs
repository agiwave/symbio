import './_selfrun.mjs';
// T2 MCP stdio 工具回路：tool_calls → mock-mcp 执行 → 结果回灌 → 收尾。
import { join } from 'node:path';
import { readFileSync } from 'node:fs';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  addMcpServer,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T2 工具回路（MCP stdio）：tool_calls → 执行 → 结果回灌 → 收尾', async () => {
  const llm = await new MockLlm([
    {
      id: 'call-echo',
      match: '帮我回显',
      toolCalls: [{ id: 'call_1', name: 'mcp__mockserv__echo', arguments: { text: 'mock 回显内容' } }],
    },
    { id: 'after-echo', afterTool: true, content: '工具调用完成，回显成功。' },
  ]).start();
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
  addMcpServer(hd, 'mockserv', {
    type: 'stdio',
    command: process.execPath,
    args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, 'mcp-record.ndjson')],
    enabled: true,
  });
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '帮我回显一下', provider: PROVIDER_ID, session: 'e2e-t2' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(r.stdout.includes('工具调用完成，回显成功'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);
    assert(r.stderr.includes('调用工具'), `stderr 应播报工具调用（实际: ${r.stderr.slice(0, 400)}）`);

    // mock-mcp 真被 spawn 且执行了调用
    const record = readFileSyncSafe(join(hd.homedir, 'mcp-record.ndjson'));
    const calls = record.split('\n').filter(Boolean).map((l) => JSON.parse(l)).filter((e) => e.kind === 'call');
    assertEq(calls.length, 1, 'mock-mcp 应恰好收到一次 tools/call');
    assertEq(calls[0].name, 'echo', '被调工具名');
    assertEq(calls[0].args, { text: 'mock 回显内容' }, '工具参数');

    // 两次 LLM 请求：第一次带工具清单与 tool_calls 回复，第二次带工具结果
    const reqs = await llm.requests();
    assertEq(reqs.length, 2, 'mock-llm 应收到两次请求（工具轮 + 收尾轮）');
    const toolWireNames = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
    assert(toolWireNames.includes('mcp__mockserv__echo'), `MCP 工具应注册进请求（实际: ${toolWireNames.join(',')}）`);
    const toolMsg = reqs[1].body.messages.find((m) => m.role === 'tool');
    assert(toolMsg, '第二次请求应携带 role=tool 的结果消息');
    assert(JSON.stringify(toolMsg).includes('mock 回显内容'), '工具结果应回灌给模型');

    // 转写落盘：tool_call 与结果成对
    const msgs = readMessagesJson(hd.homedir, 'e2e-t2');
    assertTranscriptInvariants(msgs, 'T2');
    const tc = msgs.find((m) => m.type === 'tool_call');
    assert(tc, '应有 tool_call 节点');
    assertEq(tc.status, 'completed', 'tool_call 终态应为 completed');
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});
