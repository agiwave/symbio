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
  nextPort,
  startReadbackCli,
  readSessionMessages,
  waitFor,
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
    },
  });
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
  let api = null;
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '算一下 1+2', provider: PROVIDER_ID, session: 'e2e-t6' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(r.stdout.includes('工具出错了'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);

    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t6',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t6');
    assertTranscriptInvariants(msgs, 'T6');
    const tc = msgs.find((m) => m.type === 'tool_call');
    assert(tc, '应有 tool_call 节点');
    // auto 模式：错误结果传回 LLM 继续生成，tool_call 本身仍应收敛到终态
    assert(['completed', 'failed'].includes(tc.status), `tool_call 应为终态（实际: ${tc.status}）`);
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});
