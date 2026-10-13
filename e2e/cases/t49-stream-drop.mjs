import './_selfrun.mjs';
// T49 流中断（黑盒）：LLM 在一次回答完成前把连接断掉（真实服务挂掉 / 网络中断的形态）。
// 用户视角判据：这一轮要么得到完整回复、要么得到**可见的失败**并当场收敛——
// 不允许「会话悬挂在永远运行中」，也不允许把半截回复静默当成完整答案。
// 随后的正常轮必须照常工作（真实用户被断流坑过后要能继续用）。
// 断言只走用户可见出口：CLI 成败 / stderr 错误信息 / 生产读出口的转写不变量。
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

export default defineCase('T49 流中断：断流轮用户可见地失败并收敛，随后恢复轮照常', async () => {
  const llm = await new MockLlm([
    // 断流场景：吐两个分片后半途 destroy（无终止帧、无 [DONE]）。不限次数——
    // 客户端无论重试与否，每次命中同一条场景都断，行为才确定。
    { id: 'drop', match: '断流测试', dropAfter: 2, chunks: ['回答的前半段', '回答的后半段（模型本来想说的）'] },
    { id: 'answer', match: '正常回答', content: '回复：恢复轮正常完成。' },
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
    // ① 断流轮：用户拿到的是「可见的失败」，而不是悬挂，也不是静默的半截答案。
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '断流测试', provider: PROVIDER_ID, session: 'e2e-t49', timeoutMs: 90_000 });
    assert(
      r.code !== 0 || r.stdout === '',
      `断流轮要么非零退出、要么拿不到一轮完整回复（实际 code=${r.code}, stdout=${JSON.stringify(r.stdout.slice(0, 200))}）`,
    );
    if (r.code !== 0) {
      assert(
        /错误|失败|error|中断|断|超时/i.test(r.stderr),
        `失败应可见（stderr 带错误信息，实际 stderr 头部: ${r.stderr.slice(0, 300)}）`,
      );
    }

    // ② 用户视角的收敛：转写里没有停在 streaming/pending 的节点（不悬挂）。
    api = await startReadbackCli({
      homedir: hd.homedir, workdir: hd.workdir, session: 'e2e-t49', provider: PROVIDER_ID, gatewayPort: GATEWAY_PORT,
    });
    const msgsAfterDrop = await readSessionMessages(api, 'e2e-t49');
    assertTranscriptInvariants(msgsAfterDrop, 'T49 断流轮');
    const nonTerminal = (msgsAfterDrop ?? []).filter((m) => ['streaming', 'pending'].includes(m.status));
    assert(
      nonTerminal.length === 0,
      `断流后不得有停在非终态的节点（实际: ${JSON.stringify(nonTerminal.map((m) => ({ id: m.id, status: m.status })))}）`,
    );

    // ③ 恢复轮：同一会话照常工作（用户被坑过后能继续用）。
    const r2 = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '正常回答', provider: PROVIDER_ID, session: 'e2e-t49', timeoutMs: 90_000 });
    assert(r2.code === 0, `恢复轮应正常完成（code=${r2.code}, stderr: ${r2.stderr.slice(0, 300)}）`);

    // ④ 两个轮都进了同一个转写，历史一致可读。
    const msgs = await readSessionMessages(api, 'e2e-t49');
    assertTranscriptInvariants(msgs, 'T49 全程');
    const texts = (msgs ?? []).map((m) => (typeof m.content === 'string' ? m.content : '')).join('\n');
    assert(texts.includes('恢复轮正常完成'), '恢复轮回复应在转写里');
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});
