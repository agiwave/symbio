import './_selfrun.mjs';
// T52 网络拟真（黑盒）：真实网络有首字节延迟、每帧抖动、流中途停顿（连接在、数据停一会）。
// 用户视角判据：这些「慢但不坏」的形态下对话照常成功——用户拿到完整回复，会话收敛。
// 断言只走用户可见出口：stdout 正文 + 转写不变量 + 用例整体耗时的粗对表（延迟与停顿真的发生了）。
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

export default defineCase('T52 网络拟真：首字节延迟 / 逐帧抖动 / 中途停顿下完整回复照常到达', async () => {
  const CONTENT = '回复：网络慢但没坏，停顿之后正文还是一字不少。';
  // 内容拆成**真实多分片**（停顿位次是「按事件」锚定的，单个 content 只有一个事件，
  // stallAfter 永远够不着——注入就变成装饰字段）。分片拼接逐字等于完整正文。
  const PIECES = ['回复：网络慢', '但没坏，', '停顿之后', '正文还是', '一字不少。'];
  const llm = await new MockLlm([
    {
      id: 'slow-network',
      match: '网络测试',
      chunks: PIECES,
      latencyMs: 400,          // 首字节延迟
      chunkDelayMs: 40,        // 慢流
      chunkJitterMs: 60,       // 每帧抖动
      stallAfter: 2, stallMs: 700, // 第 2 帧后停顿一次（停在连接里，不关）
    },
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
  let api = null;
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '网络测试', provider: PROVIDER_ID, session: 'e2e-t52', timeoutMs: 90_000 });
    assert(r.code === 0, `网络拟真轮应正常完成（code=${r.code}, stderr: ${r.stderr.slice(0, 300)}）`);
    assert(
      r.stdout.includes(PIECES.join('')) && PIECES.every((p) => r.stdout.includes(p)),
      `用户应拿到完整回复且分片顺序正确（实际 stdout: ${JSON.stringify(r.stdout.slice(0, 300))}）`,
    );

    // 粗对表：延迟 + 停顿确实撑长了本轮耗时（注入真发生了，不是装饰字段）。
    // 下界 = latency 400 + stall 700 + 6 个事件的慢流 ≥ 240ms；取个稳妥的 1300ms。
    assert(
      r.ms >= 1300,
      `耗时应对得上注入量（latency 400 + stall 700 + 慢流 ≥240ms = 下界 1340ms，实际 ${r.ms}ms）`,
    );

    api = await startReadbackCli({
      homedir: hd.homedir, workdir: hd.workdir, session: 'e2e-t52', provider: PROVIDER_ID, gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t52');
    assertTranscriptInvariants(msgs, 'T52');
    const nonTerminal = (msgs ?? []).filter((m) => ['streaming', 'pending'].includes(m.status));
    assertEq(nonTerminal.length, 0, '不得有停在非终态的节点');
    const texts = (msgs ?? []).map((m) => (typeof m.content === 'string' ? m.content : '')).join('\n');
    assert(texts.includes(CONTENT), '完整回复应在历史里');
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});
