import './_selfrun.mjs';
// T5 LLM HTTP 500：失败路径收敛，存储中无停在 streaming/pending 的节点。
import {
  MockLlm,
  makeHomedir,
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

export default defineCase('T5 LLM HTTP 500：失败路径收敛，无「永远运行中」', async () => {
  const llm = await new MockLlm([
    { id: 'boom', match: '触发故障', status: 500, error: 'mock 注入的服务端错误' },
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
  let api = null;
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '触发故障', provider: PROVIDER_ID, session: 'e2e-t5', timeoutMs: 60_000 });
    assert(r.code !== 0, '失败轮 CLI 应以非零退出码结束');
    assert(
      r.stderr.includes('mock') || r.stderr.includes('失败') || r.stderr.includes('错误'),
      `stderr 应有错误信息（实际: ${r.stderr.slice(0, 400)}）`,
    );

    // 关键不变量：转写里不得有停在 streaming/pending 的节点（终态收敛）
    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t5',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t5');
    assertTranscriptInvariants(msgs, 'T5');
    const nonTerminal = (msgs ?? []).filter((m) => ['streaming', 'pending'].includes(m.status));
    assertEq(
      nonTerminal.length, 0,
      `不得有停在非终态的节点（实际: ${JSON.stringify(nonTerminal.map((m) => ({ id: m.id, status: m.status })))}）`,
    );
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
