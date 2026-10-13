import './_selfrun.mjs';
// T51 不规范响应（黑盒）：真实 provider 偶尔吐脏帧——data 行坏 JSON / chunk JSON 半途截断 /
// 非 SSE 杂散行。用户视角判据：脏帧**不打断对话**，用户照常拿到**完整**回复（一个字都不少），
// 会话正常收敛；历史可读。
// 断言只走用户可见出口：stdout 正文 + 生产读出口的转写不变量。
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

const KINDS = [
  { kind: 'badJson', marker: '脏帧测试一', content: '回复一：坏 JSON 帧被跳过，正文完整。' },
  { kind: 'truncated', marker: '脏帧测试二', content: '回复二：截断帧被跳过，正文完整。' },
  { kind: 'garbage', marker: '脏帧测试三', content: '回复三：杂散行被跳过，正文完整。' },
];

export default defineCase('T51 不规范响应：三种脏帧混入流中，用户仍拿到完整回复', async () => {
  const llm = await new MockLlm(
    KINDS.map((k) => ({ id: `dirty-${k.kind}`, match: k.marker, malformed: k.kind, malformedAfter: 1, chunks: [k.content.split('，')[0] + '，', k.content.split('，')[1]] })),
  ).start();
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
    for (const k of KINDS) {
      const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: k.marker, provider: PROVIDER_ID, session: 'e2e-t51', timeoutMs: 90_000 });
      assert(r.code === 0, `${k.kind} 轮应正常完成（code=${r.code}, stderr: ${r.stderr.slice(0, 300)}）`);
      assert(
        r.stdout.includes(k.content),
        `${k.kind}：用户应拿到完整回复（实际 stdout: ${JSON.stringify(r.stdout.slice(0, 300))}）`,
      );
    }

    // 收口在用户出口：三个轮都在同一会话历史里，不变量成立。
    api = await startReadbackCli({
      homedir: hd.homedir, workdir: hd.workdir, session: 'e2e-t51', provider: PROVIDER_ID, gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t51');
    assertTranscriptInvariants(msgs, 'T51');
    const texts = (msgs ?? []).map((m) => (typeof m.content === 'string' ? m.content : '')).join('\n');
    for (const k of KINDS) assert(texts.includes(k.content), `${k.kind} 的完整回复应在历史里`);
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});
