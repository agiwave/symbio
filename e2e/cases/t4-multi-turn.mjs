import './_selfrun.mjs';
// T4 多轮会话：第二轮请求携带第一轮历史（flatten 验证）。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
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
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
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

    const msgs = readMessagesJson(hd.homedir, 'e2e-t4');
    assertTranscriptInvariants(msgs, 'T4');
    assertEq(msgs.filter((m) => m.role === 'user').length, 2, '落盘应有两条 user 消息');
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});
