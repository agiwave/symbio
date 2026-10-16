import './_selfrun.mjs';
// T1 文本流式：SSE 分片 → stdout 拼接 + 转写落盘（turn/text 节点、seq 单调）。
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
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

export default defineCase('T1 文本流式：SSE 分片输出 + 转写落盘', async () => {
  const llm = await new MockLlm([
    { id: 'echo', match: '你好', chunks: ['你好', '，', 'mock', '世', '界'], chunkDelayMs: 10 },
  ]).start();
  // 本用例的主题与对话面正交 ⇒ 把两个开关钉死（理由见 `DIALOG_FACE_OFF`）。
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: {
      // 读转写要走生产读出口（gateway `invoke` → `vdfs/read`），入站口必须开着。
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
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '你好', provider: PROVIDER_ID, session: 'e2e-t1' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assertEq(r.stdout.trim(), '你好，mock世界', 'stdout 应为分片按序拼接的完整正文');

    // 请求记录：恰好一次调用，tools 里应含内置工具（能力收集链路）
    const reqs = await llm.requests();
    assertEq(reqs.length, 1, 'mock-llm 收到的请求数');
    const toolNames = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
    assert(toolNames.includes('vdfs_read'), `请求 tools 应含 vdfs_read（实际: ${toolNames.join(',')}）`);
    assertEq(reqs[0].body.stream, true, '请求应为流式');

    // 转写落盘：user + turn + assistant text(completed)——经生产读出口读
    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t1',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t1');
    assertTranscriptInvariants(msgs, 'T1');
    assert(msgs.some((m) => msgRoleOf(m) === 'user' && textOf(m).includes('你好')), '应有 user 消息');
    assert(msgs.some((m) => msgTypeOf(m) === 'turn'), '应有 turn 组合节点');
    const text = msgs.find((m) => msgRoleOf(m) === 'assistant' && msgTypeOf(m) === 'text');
    assert(text, '应有 assistant text 节点');
    assert(textOf(text).includes('你好，mock世界'), '落盘正文应为完整文本');
    assertEq(text.status, 'completed', 'assistant text 终态应为 completed');
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
