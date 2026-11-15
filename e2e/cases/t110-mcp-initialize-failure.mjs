import './_selfrun.mjs';
// T110 MCP 握手失败：单个 server 装配失败只跳过它，会话照常收敛（auto 模式）。
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
  msgTypeOf,
} from '../helpers.mjs';

export default defineCase('T110 MCP 握手失败：坏 server 被跳过，会话照常收敛（auto 模式）', async () => {
  const llm = await new MockLlm([
    { id: 'no-mcp', match: '握手', content: 'MCP 服务没起来，但会话还在。' },
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
  addMcpServer(hd, 'badsrv', {
    type: 'stdio',
    command: process.execPath,
    args: [
      join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'),
      '--script', join(E2E_ROOT, 'e2e', 'mock-actions-fail-init.json'),
    ],
    enabled: true,
  });
  let api = null;
  try {
    const r = await runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '握手失败也不影响我', provider: PROVIDER_ID, session: 'e2e-t110' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(r.stdout.includes('会话还在'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);

    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t110',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t110');
    assertTranscriptInvariants(msgs, 'T110');
    assert(!msgs.some((m) => msgTypeOf(m) === 'tool_call'), '坏 server 不应产出 tool_call');
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
