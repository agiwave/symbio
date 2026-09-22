import './_selfrun.mjs';
// T6 MCP 工具 JSON-RPC 错误：错误结果回灌 LLM，会话照常收敛（auto 模式）。
import { join } from 'node:path';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  addMcpServer,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T6 MCP 工具报错：错误结果回灌 LLM，会话照常收敛（auto 模式）', async () => {
  const llm = await new MockLlm([
    {
      id: 'call-add',
      match: '算一下',
      toolCalls: [{ id: 'call_a1', name: 'mcp__mockserv__add', arguments: { a: 1, b: 2 } }],
    },
    { id: 'after-add', afterTool: true, content: '工具出错了，但我还在。' },
  ]).start();
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
  addMcpServer(hd, 'mockserv', {
    type: 'stdio',
    command: process.execPath,
    args: [
      join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'),
      '--script', join(E2E_ROOT, 'e2e', 'mock-actions.json'),
      '--record', join(hd.homedir, 'mcp-record.ndjson'),
    ],
    enabled: true,
  });
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '算一下 1+2', provider: PROVIDER_ID, session: 'e2e-t6' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(r.stdout.includes('工具出错了'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);

    const msgs = readMessagesJson(hd.homedir, 'e2e-t6');
    assertTranscriptInvariants(msgs, 'T6');
    const tc = msgs.find((m) => m.type === 'tool_call');
    assert(tc, '应有 tool_call 节点');
    // auto 模式：错误结果传回 LLM 继续生成，tool_call 本身仍应收敛到终态
    assert(['completed', 'failed'].includes(tc.status), `tool_call 应为终态（实际: ${tc.status}）`);
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});
