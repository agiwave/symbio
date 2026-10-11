import './_selfrun.mjs';
// T4 多轮会话：第二轮请求携带第一轮历史（flatten 验证）+ 转写落盘对账。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
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
  textOf,
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T4 多轮会话：第二轮请求携带第一轮历史', async () => {
  const llm = await new MockLlm([
    { id: 'r1', match: '第一轮', content: '第一轮回复' },
    { id: 'r2', match: '第二轮', content: '第二轮回复' },
  ]).start();
  // 本用例的主题与对话面正交 ⇒ 把两个开关钉死（理由见 `DIALOG_FACE_OFF`）。
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
      session: { ...DIALOG_FACE_OFF },
    },
  });
  let api = null;
  try {
    // REPL 管道模式：两行输入 = 两轮对话
    const r = runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      provider: PROVIDER_ID,
      session: 'e2e-t4',
      stdinText: '第一轮问题\n第二轮问题\n',
    });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(r.stdout.includes('第一轮回复') && r.stdout.includes('第二轮回复'), 'stdout 应含两轮回复');

    const reqs = await llm.requests();
    assertEq(reqs.length, 2, 'mock-llm 应收到两次请求');
    assert(
      reqs[1].body.messages.some((m) => m.role === 'user' && textOf(m).includes('第一轮问题')),
      '第二轮请求应携带第一轮用户消息（历史回灌）',
    );

    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t4',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t4');
    assertTranscriptInvariants(msgs, 'T4');
    assertEq(msgs.filter((m) => m.role === 'user').length, 2, '落盘应有两条 user 消息');
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
