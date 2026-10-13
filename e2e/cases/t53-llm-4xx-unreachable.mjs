import './_selfrun.mjs';
// T53 LLM 不可用（黑盒）：两类「服务根本没法用」的形态——
// ① 凭证被拒（401，不可重试的错误态）；② 起点压根不可达（端口没人听）。
// 用户视角判据：这一轮得到**可见的失败**并当场收敛（不悬挂、不假成功）；
// 服务恢复后的正常轮照常工作。断言只走用户可见出口。
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
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T53 LLM 不可用：401 错误态与起点不可达都可见地失败，恢复轮照常', async () => {
  const llm = await new MockLlm([
    { id: 'unauthorized', match: '凭证测试', status: 401, retryAfterSec: null, error: 'mock 注入的 401（凭证无效）' },
    { id: 'answer', match: '恢复测试', content: '回复：服务恢复后的正常回答。' },
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
    },
  });
  // 起点不可达：把一个克隆 provider 指到本机没有监听的端口（同协议配置）。
  const deadPort = nextPort();
  const hdDead = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(deadPort) }],
    pluginConfigs: {
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
    // ① 401：不可重试的错误态——立刻给用户可见的失败。
    const r401 = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '凭证测试', provider: PROVIDER_ID, session: 'e2e-t53', timeoutMs: 60_000 });
    assert(r401.code !== 0, `401 轮应失败退出（实际 code=${r401.code}）`);
    assert(/401|凭证|密钥|API|错误|失败/i.test(r401.stderr), `401 失败应可见（stderr 头部: ${r401.stderr.slice(0, 300)}）`);

    // ② 起点不可达：连接级失败——同样可见并收敛。
    const rDead = runCli({ homedir: hdDead.homedir, workdir: hdDead.workdir, message: '起点不可达测试', provider: PROVIDER_ID, session: 'e2e-t53-dead', timeoutMs: 60_000 });
    assert(rDead.code !== 0, `不可达轮应失败退出（实际 code=${rDead.code}）`);
    assert(/错误|失败|error|connect|拒绝|不可达/i.test(rDead.stderr), `连接失败应可见（stderr 头部: ${rDead.stderr.slice(0, 300)}）`);

    // ③ 收敛：两个失败轮之后，转写里没有停在 streaming/pending 的节点。
    api = await startReadbackCli({
      homedir: hd.homedir, workdir: hd.workdir, session: 'e2e-t53', provider: PROVIDER_ID, gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t53');
    assertTranscriptInvariants(msgs, 'T53');
    const nonTerminal = (msgs ?? []).filter((m) => ['streaming', 'pending'].includes(m.status));
    assert(
      nonTerminal.length === 0,
      `401 轮后不得有停在非终态的节点（实际: ${JSON.stringify(nonTerminal.map((m) => ({ id: m.id, status: m.status })))}）`,
    );

    // ④ 恢复轮：服务正常后，同一会话照常工作。
    const rOk = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '恢复测试', provider: PROVIDER_ID, session: 'e2e-t53', timeoutMs: 90_000 });
    assert(rOk.code === 0, `恢复轮应正常完成（code=${rOk.code}, stderr: ${rOk.stderr.slice(0, 300)}）`);
    const texts = (await readSessionMessages(api, 'e2e-t53') ?? []).map((m) => (typeof m.content === 'string' ? m.content : '')).join('\n');
    assert(texts.includes('服务恢复后的正常回答'), '恢复轮回复应在转写里');
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    cleanupHomedir(hdDead);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});
