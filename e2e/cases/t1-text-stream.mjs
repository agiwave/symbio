import './_selfrun.mjs';
// T1 文本流式：SSE 分片 → stdout 拼接 + 转写落盘（turn/text 节点、seq 单调）。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readSessionJson,
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  textOf,
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T1 文本流式：SSE 分片输出 + 转写落盘', async () => {
  const llm = await new MockLlm([
    { id: 'echo', match: '你好', chunks: ['你好', '，', 'mock', '世', '界'], chunkDelayMs: 10 },
  ]).start();
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
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

    // 转写落盘：user + turn + assistant text(completed)
    const msgs = readMessagesJson(hd.homedir, 'e2e-t1');
    assertTranscriptInvariants(msgs, 'T1');
    assert(msgs.some((m) => m.role === 'user' && textOf(m).includes('你好')), '应有 user 消息');
    assert(msgs.some((m) => m.type === 'turn'), '应有 turn 组合节点');
    const text = msgs.find((m) => m.role === 'assistant' && m.type === 'text');
    assert(text, '应有 assistant text 节点');
    assert(textOf(text).includes('你好，mock世界'), '落盘正文应为完整文本');
    assertEq(text.status, 'completed', 'assistant text 终态应为 completed');

    const sess = readSessionJson(hd.homedir, 'e2e-t1');
    assert(sess, 'session.json 应存在');
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});
