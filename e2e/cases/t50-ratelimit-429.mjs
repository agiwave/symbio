import './_selfrun.mjs';
// T50 限流（黑盒）：真 provider 对高频请求返回 429（带 retry-after / x-ratelimit-* 头），
// 窗口冷却后自动恢复。用户视角判据：被限的轮**自己缓过来**、完整拿到回复——
// 用户不用重开会话、不用重启进程；历史里两轮对话都完整可读。
// 断言只走用户可见出口：实时面（用户在看的窗口）+ 生产读出口的转写 + mock 的请求账本。
import {
  DIALOG_FACE_OFF,
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  nextPort,
  startRealtimeSession,
  readSessionMessages,
  sendAndSettle,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
} from '../helpers.mjs';

export default defineCase('T50 限流：429 配额超额被限后冷却自动恢复，两轮均完整收敛', async () => {
  // 配额极小（1 次 / 1.2s）：第一轮占满配额，第二轮被限 → 冷却后恢复。
  const llm = await new MockLlm(
    [{ id: 'answer', content: '回复：正常回答内容。' }],
    { name: 't50', rateLimit: { limit: 1, windowMs: 1200 } },
  ).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    // 本用例主题与对话面正交 ⇒ 把对话面三个开关钉死：配额语义只跟主对话轮对表，
    // 不然每轮多出的静默分类/措辞请求也让 429 的节奏难对上（DIALOG_FACE_OFF 的理由见 helpers）。
    session: { ...DIALOG_FACE_OFF },
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
  let cli = null;
  try {
    const rt = await startRealtimeSession({
      homedir: hd.homedir, workdir: hd.workdir, session: 'e2e-t50', provider: PROVIDER_ID, gatewayPort: GATEWAY_PORT,
    });
    cli = rt.cli;
    const sub = rt.sub;

    // 把限流事实从「时序竞猜」里拿出来：先发两次直探请求（不经过被测系统）——
    // P1 占掉配额（200），P2 被拒（429，mock 语义自检）。此后不管轮 1 的首请求
    // 什么时候落地：落在窗口内就 429 后重试，落在窗口外则直接 200——两种都算
    // 「限流存在且未阻挡用户继续用」；请求账本里必有 429（P2）。
    const probe = (body) => fetch(`http://127.0.0.1:${llm.port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    const p1 = await probe({ model: 'mock-model', messages: [{ role: 'user', content: '探探配额一' }] });
    assertEq(p1.status, 200, 'P1 应被获准（配额为空，占位语义自检）');
    const p2 = await probe({ model: 'mock-model', messages: [{ role: 'user', content: '探探配额二' }] });
    assertEq(p2.status, 429, 'P2 应被限流（配额已占用，限流语义自检）');

    // 两轮连续发出：第一轮的首次请求落在被探针占满的配额窗口内（必然 429），
    // 第二轮在冷却后发出（配额已重开）。「被限又自己缓过来」两轮都验到。
    await sendAndSettle(cli, sub, '限流测试第一问', { timeoutMs: 90_000 });
    await sendAndSettle(cli, sub, '限流测试第二问', { timeoutMs: 90_000 });

    // ① 用户视角：两轮都完整收敛，历史里都有完整回复。
    // 「实时面离开 working」≠「那一轮已经落完库」（夹具陷阱 4）：落库有自己的时序，
    // 读历史要**等到事实出现**再断言，不能拿到句柄就比对。
    let msgs = null;
    await waitFor(
      () => {
        msgs = readSessionMessages(cli, 'e2e-t50');
        return msgs
          .then((m) => {
            const t = (m ?? []).map((x) => (typeof x.content === 'string' ? x.content : '')).join('\n');
            const answered = (m ?? []).filter(
              (x) => msgRoleOf(x) === 'assistant' && (typeof x.content === 'string' ? x.content : '').includes('正常回答内容'),
            ).length;
            // 等「两问 + 两答」都落历史：turn 2 的 429 重试有冷却延迟（可到数秒），
            // 只等用户发言会在它仍在途时提前放行（夹具陷阱 4 的镜像面）。
            return t.includes('第一问') && t.includes('第二问') && answered >= 2;
          })
          .catch(() => false);
      },
      { what: '两轮问答都进历史', timeoutMs: 90_000 },
    );
    msgs = await msgs;
    assertTranscriptInvariants(msgs, 'T50');
    const texts = (msgs ?? []).map((m) => (typeof m.content === 'string' ? m.content : '')).join('\n');
    for (const q of ['第一问', '第二问']) {
      assert(texts.includes(q), `第 ${q} 的用户发言应在转写里`);
    }
    const assistantCount = (msgs ?? [])
      .filter((m) => msgRoleOf(m) === 'assistant' && (typeof m.content === 'string' ? m.content : '').includes('正常回答内容'))
      .length;
    assert(
      assistantCount >= 2,
      `两轮都应拿到完整回复（含「正常回答内容」的助手消息 ≥ 2，实际 ${assistantCount}；转写正文:\n${texts.slice(0, 900)}）`,
    );

    // ② 限流真的发生过（请求账本里有 429），且最终以 200 收尾（冷却后恢复）。
    const reqs = await llm.requests();
    const statuses = reqs.map((rq) => rq.status).filter((s) => s != null);
    assert(
      statuses.includes(429),
      `请求账本里应有 429（实际状态序列: ${JSON.stringify(statuses)}）`,
    );
    assert(
      statuses.filter((s) => s === 200).length > 0,
      '被限之后应有 200 收尾（冷却后恢复）',
    );
    const limited = reqs.find((rq) => rq.status === 429);
    assertEq(limited?.body?.messages != null, true, '429 响应对应的请求应是正常 chat 请求');
  } finally {
    cli?.stop();
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: 'REPL CLI / mock-llm 子进程退出',
    });
  }
});
