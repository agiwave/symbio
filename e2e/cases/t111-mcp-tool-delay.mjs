import './_selfrun.mjs';
// T111 MCP 工具执行超时：读超时按 server 配置生效，失败回灌 LLM，会话照常收敛（auto 模式）。
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

export default defineCase('T111 MCP 工具超时：按 server 配置的读超时收敛，失败回灌 LLM', async () => {
  const llm = await new MockLlm([
    {
      id: 'call-slow',
      match: '慢一下',
      toolCalls: [{ id: 'call_s1', name: 'mcp__delayserv__slow', arguments: {} }],
    },
    { id: 'after-timeout', afterTool: true, content: '等太久了，先停在这。' },
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
  addMcpServer(hd, 'delayserv', {
    type: 'stdio',
    command: process.execPath,
    args: [
      join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'),
      '--script', join(E2E_ROOT, 'e2e', 'mock-actions-delay.json'),
    ],
    // 默认 30s 会让这条用例等半分钟；配置值让读超时降到 3s（mock 睡 8s）。
    timeout_secs: 3,
    enabled: true,
  });
  let api = null;
  try {
    const r = await runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '慢一下 mock 服务', provider: PROVIDER_ID, session: 'e2e-t111' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(r.stdout.includes('等太久'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);

    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t111',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t111');
    assertTranscriptInvariants(msgs, 'T111');
    const tc = msgs.find((m) => msgTypeOf(m) === 'tool_call');
    assert(tc, '应有 tool_call 节点');
    assert(['completed', 'failed'].includes(tc.status), `tool_call 应为终态（实际: ${tc.status}）`);

    // 超时经客户端失败路径回灌：第二轮请求里模型看到的工具结果是读超时。
    const reqs = await llm.requests();
    assert(reqs.length >= 2, `应有第二轮请求（实际 ${reqs.length} 条）`);
    const last = JSON.stringify(reqs[reqs.length - 1]);
    assert(last.includes('timed out'), `工具结果应带读超时正文（实际: ${last.slice(0, 500)}）`);
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
