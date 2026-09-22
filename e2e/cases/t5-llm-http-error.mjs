import './_selfrun.mjs';
// T5 LLM HTTP 500：失败路径收敛，存储中无停在 streaming/pending 的节点。
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
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T5 LLM HTTP 500：失败路径收敛，无「永远运行中」', async () => {
  const llm = await new MockLlm([
    { id: 'boom', match: '触发故障', status: 500, error: 'mock 注入的服务端错误' },
  ]).start();
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '触发故障', provider: PROVIDER_ID, session: 'e2e-t5', timeoutMs: 60_000 });
    assert(r.code !== 0, '失败轮 CLI 应以非零退出码结束');
    assert(
      r.stderr.includes('mock') || r.stderr.includes('失败') || r.stderr.includes('错误'),
      `stderr 应有错误信息（实际: ${r.stderr.slice(0, 400)}）`,
    );

    // 关键不变量：存储里不得有停在 streaming/pending 的节点（终态收敛）
    const msgs = readMessagesJson(hd.homedir, 'e2e-t5');
    assertTranscriptInvariants(msgs, 'T5');
    const nonTerminal = (msgs ?? []).filter((m) => ['streaming', 'pending'].includes(m.status));
    assertEq(
      nonTerminal.length, 0,
      `不得有停在非终态的节点（实际: ${JSON.stringify(nonTerminal.map((m) => ({ id: m.id, status: m.status })))}）`,
    );
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});
