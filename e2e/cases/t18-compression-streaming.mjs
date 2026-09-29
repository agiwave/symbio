// T18 压缩实时流式预览 + 失败纪律 + 重试恢复。
import './_selfrun.mjs';
//
// T8 钉「压缩会发生、快照会落库」，T11 钉「压缩是一个消息节点、两态、位置、
// 重写收敛」。本用例钉 T8/T11 都够不着的六条契约：
//
// | # | 契约 | 判据 |
// |---|---|---|
// | A | **流式身份帧**：摘要以 `delta` 逐帧追加到 compression 节点（占位正文之后、终态之前），且不产生任何旁路节点 | compression 节点的流上出现 ≥1 帧纯窄增量；拼接结果 == mock 摘要去掉首个分片（首分片走「全量首帧」被吞）；除此之外没有任何节点收到过摘要增量 |
// | B | **结构化 meta**：compact_trigger / context_limit / before_tokens / after_tokens / dropped 按字段下发 | 成功节点终态帧的 meta 逐字段核对；失败节点**没有** after_tokens（没有可信的后水位，编 0 是谎报） |
// | C | **transcript 转存**：压缩前完整历史归档，可回溯 | `<会话目录>/transcripts/` 下出现存档 JSON，内容含被压掉的早期消息 |
// | D | **失败分支**：压缩 LLM 500 ⇒ failed 节点 + failure_kind=llm_error + 历史原封不动 | 节点 failed、meta.failure_kind；存储里早期消息仍在 |
// | E | **触发纪律**：连续失败触发**熔断**——开闸后的轮次直接跳过自动压缩，不再重发注定失败的摘要请求 | 连续 3 次失败后，再跑一轮水位超阈值的对话：摘要请求数不增、无新压缩节点、回复照常 |
// | F | **重试恢复**：`session/chat/send` + `resume.action = "retry_compaction"` ⇒ 删除旧失败节点（removed 帧）→ 绕过熔断重新压缩 → 成功 → 快照落库 → 后续轮恢复正常 | 旧 mid 收到 removed 帧；新节点 Completed；存储里失败节点消失、快照出现；之后水位低不再压缩 |
//
// 触发参数**刻意不同于** T8/T11（它们只需「压缩发生一次」；本用例要让连续 3 次
// 摘要请求都落在**同一个可发请求的窗口**里）。窗口宽度由下面三条决定：
//
// 1. 触发线 = 0.7 × `max_context_tokens`，超限线 = `max_context_tokens`
//    （`should_start_compression` 与 `compress_snapshot_inner` 的预判都含请求级
//    开销）⇒ 窗口只有 0.3 × L 宽；
// 2. 压缩失败**不改历史**，水位每轮被回复推高 g ⇒ 窗口只容得下约 0.3L / g 轮；
// 3. 第 3 次失败之后还有**熔断观察轮**与**一次重试**，而重试（force）同样要过
//    超限预判 ⇒ 窗口必须多留 2 轮余量，否则重试会变成 `input_over_limit`
//    （永久失败位，熔断从此不再半开），契约 F「重试恢复」无从谈起。
//
// 实测取 `max_context_tokens=12000` + `FILL.repeat(240)`：原始正文每轮涨约
// 3.9k 字符，但**水位按校准后的估算值推进**——mock 固定回报 usage
// （`prompt_tokens: 64`，见 `mock-llm.mjs`）⇒ `CalibratedTokenizer` 的比值压在
// 下限 0.2 ⇒ 估算每轮只涨约 0.5k tokens（实测 before_tokens 7346 → 7808 →
// 8323）。于是 0.3L ≈ 3.6k tokens 的窗口容得下 3 次失败 + 观察轮 + 重试：
// 失败落在第 8/9/10 轮，重试仍在线内。T8/T11 的 600 倍填充把增速放大 2.4 倍，
// 实测一次 llm_error 后紧接 input_over_limit，契约 F 无从谈起。
//
// **这份参数与 mock 的 usage 口径绑定**：若 mock 改成回报真实 usage，估算会贴回
// 原始字符数，触发轮与窗口一起前移，本用例需重新标定（轮次推进本身自适应，
// 先红的会是重试那步的超限预判）。轮次推进**不硬编码「第几轮触发」**：逐轮发送
// 直到观测到 3 次失败摘要请求（熔断阈值，见 `active.rs::COMPRESS_CIRCUIT_LIMIT`），
// 再进熔断观察轮。
//
// 「3 次失败」按**逻辑摘要请求**计，不是 HTTP 命中：LLM 层对 5xx 会退避重发
// （`model/http.rs` 的 `MAX_RETRIES`，重发复用同一份序列化字节），一次逻辑失败会
// 打到 mock 5 次。mock 因此按请求体指纹扣 `failTimes` 名额（见 `mock-llm.mjs`
// 文件头），本用例也按同一口径数摘要请求——否则第 1 次压缩就会把 3 个名额烧光，
// 「连续 3 次失败触发熔断」这条契约根本没有被执行到。
//
// mock 摘要场景用 `chunks` 分片：**首个分片**走 model 流循环的「全量首帧」（被
// 白名单吞掉），其余分片走窄增量——这正是 A 要验收的改道路径；若回归成「全帧
// 静默」，A 的增量断言会红；若回归成「全帧放行」，骨架节点会多出来，A 的
// 「无旁路节点」断言会红。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  readMessagesJson,
  assertTranscriptInvariants,
  sessionDir,
} from '../helpers.mjs';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const TERMINAL = new Set(['completed', 'failed', 'aborted', 'waiting_user_action']);
const CIRCUIT_LIMIT = 3; // 镜像 active.rs::COMPRESS_CIRCUIT_LIMIT

const FILL = '上下文填充内容用于推高历史水位。';
// 每轮约 3.9k 字符原文，校准后水位约 +0.5k tokens/轮：与 12000 的上限配套
// （0.3L 窗口 ≈ 7 轮，见文件头的实测标定）
const REPLY = '填充：' + FILL.repeat(240);
// 摘要正文分片：首个分片会成为「全量首帧」被吞，其余逐帧以 delta 改道到压缩节点
const SNAPSHOT_XML = [
  '<state_snapshot>',
  '  <overall_goal>验证压缩摘要的实时流式预览与失败恢复。</overall_goal>',
  '  <key_knowledge>',
  '    - 用户偏好：mock 环境下回复应包含轮次编号。',
  '  </key_knowledge>',
  '  <completed_items>',
  '    - 前几轮对话已完成。',
  '  </completed_items>',
  '  <in_progress_items>',
  '    - 等待压缩后的下一轮对话验证记忆延续。',
  '  </in_progress_items>',
  '  <open_questions></open_questions>',
  '</state_snapshot>',
];
// 分片 = 每行一段（末行不带换行）：拼接严格等于 `SNAPSHOT`，于是 A 的期望值可以
// 从常量推导，而不是手抄第二份摘要文本（两份必然漂移，且漂移只在断言失败时暴露）。
const SNAPSHOT = SNAPSHOT_XML.join('\n');
const SNAPSHOT_CHUNKS = SNAPSHOT_XML.map((line, i) =>
  i === SNAPSHOT_XML.length - 1 ? line : `${line}\n`,
);

export default defineCase('T18 压缩实时流式预览 + 失败纪律 + 重试恢复', async () => {
  const llm = await new MockLlm([
    // failTimes=3：前 3 次摘要请求 500（连续失败 → 熔断开闸），第 4 次（用户重试）成功
    { id: 'snapshot', match: 'Distill the conversation above', failTimes: 3, chunks: SNAPSHOT_CHUNKS },
    { id: 'big', match: '轮问题', content: REPLY },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [
      { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 12000 }) },
    ],
    pluginConfigs: {
      // 本用例的主题是压缩，与对话面正交 ⇒ 把两个开关钉死（理由见 `DIALOG_FACE_OFF`）。
      session: { auto_compress: true, context_messages: 0, ...DIALOG_FACE_OFF },
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
  const cli = startLongLivedCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    session: 'unused-t18',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });
  let rt = null;
  /**
   * **逻辑**摘要请求数：同一请求体的 5xx 退避重发只算一次（`model/http.rs` 的
   * 重发复用同一份序列化字节）。熔断按**逻辑失败**计数（`active.rs`），判据必须
   * 同口径——按 HTTP 命中数，一次压缩失败会被数成 5 次，熔断看起来「提前开闸」。
   */
  const snapshotReqCount = async () => {
    const hits = (await llm.requests()).filter((q) =>
      (q.body?.messages ?? []).some(
        (m) => m.role === 'user' && String(m.content ?? '').includes('Distill the conversation above'),
      ),
    );
    return new Set(hits.map((q) => JSON.stringify(q.body))).size;
  };

  try {
    await cli.waitGatewayReady();
    rt = await subscribeSessionRealtime(cli, { sessionId: 'e2e-t18', gatewayPort: GATEWAY_PORT });

    const doneTurns = () => {
      const last = new Map();
      for (const { mid, data } of rt.messages()) {
        if (data?.type === 'turn') last.set(mid, data.status);
      }
      return [...last.values()].filter((s) => TERMINAL.has(s)).length;
    };
    const sendRound = async (i, text) => {
      const before = doneTurns();
      const send = await cli.invoke(
        'session/chat/send',
        { session_id: 'e2e-t18', message: { id: `u-t18-${i}`, role: 'user', type: 'text', content: text }, mode: 'auto' },
        { session_id: 'e2e-t18', workdir: hd.workdir },
      );
      assert(send.status === 200, `第 ${i + 1} 轮 chat/send 应受理（${send.status}）`);
      await waitFor(() => doneTurns() > before, { what: `第 ${i + 1} 轮 turn 终态`, timeoutMs: 60_000 });
    };

    // ── 阶段 1：自适应逐轮推进，直到观测到 CIRCUIT_LIMIT 次失败的摘要请求 ──
    let rounds = 0;
    let attempts = 0;
    while (attempts < CIRCUIT_LIMIT && rounds < 22) {
      const before = await snapshotReqCount();
      await sendRound(rounds, `第${rounds + 1}轮问题`);
      rounds += 1;
      attempts = await snapshotReqCount();
      assert(attempts >= before, `摘要请求数不得回退（${before} → ${attempts}）`);
      // 每轮的失败压缩都必须原样保留历史（失败的硬不变式，存储侧核对在阶段 4）
      const msgs = readMessagesJson(hd.homedir, 'e2e-t18');
      assertTranscriptInvariants(msgs, `T18 第 ${rounds} 轮后`);
    }
    assertEq(
      attempts,
      CIRCUIT_LIMIT,
      `应观测到 ${CIRCUIT_LIMIT} 次摘要请求（全部失败——水位超阈值而摘要 500）`,
    );
    assert(rounds >= 3, `阈值触发应发生在第 3 轮或之后（实际第 ${rounds} 轮前已达）`);

    // ── 阶段 2：失败分支（D）+ 结构化 meta 的失败形态（B）────────────────
    const compFrames = rt.messages().filter(({ data }) => data?.type === 'compression');
    /** @type {Map<string, Array<{mid:string, data:any, arrival:number}>>} */
    const episodes = new Map();
    for (const f of compFrames) {
      const list = episodes.get(f.mid) ?? [];
      list.push(f);
      episodes.set(f.mid, list);
    }
    const failedEpisodes = [...episodes.entries()].filter(([, fs]) => fs[fs.length - 1].data.status === 'failed');
    assertEq(failedEpisodes.length, CIRCUIT_LIMIT, `每次失败的摘要请求都应留下一个 failed 压缩节点`);
    let failId = null;
    for (const [id, frames] of failedEpisodes) {
      failId = id;
      const final = frames[frames.length - 1].data;
      assertEq(final.status, 'failed', `压缩节点 ${id.slice(0, 8)} 应以 failed 收场`);
      assert(String(final.content ?? '').includes('已保留完整历史'), `失败说明应明示历史未丢（实际：${final.content}）`);
      assertEq(final.meta?.failure_kind, 'llm_error', 'HTTP 500 的失败码必须是 llm_error（机读）');
      assertEq(final.meta?.compact_trigger, 'threshold', '自动路径的触发来源必须是 threshold');
      assertEq(final.meta?.context_limit, 12000, 'meta.context_limit 应为配置的上限');
      assert(
        Number(final.meta?.before_tokens) > 0,
        `失败时也应有压缩前水位（实际 ${JSON.stringify(final.meta)}）`,
      );
      assert(
        final.meta?.after_tokens == null,
        `失败节点不得携带 after_tokens（没有可信的后水位，编 0 是谎报）`,
      );
      assertSeqShapeOk(frames);
    }

    // ── 阶段 3：熔断纪律（E）——开闸后的轮次不再重发注定失败的摘要请求 ──
    const beforeSkip = await snapshotReqCount();
    await sendRound(rounds, `第${rounds + 1}轮问题`);
    rounds += 1;
    assertEq(await snapshotReqCount(), beforeSkip, '熔断开闸后的自动压缩必须被跳过（不得重发摘要请求）');
    // 熔断跳过路径**不产生任何新压缩节点**——这正是被验的语义；
    // 之前的 failed 节点原样留在原地（等待用户重试，而不是被静默清理）。
    const afterSkipMsgs = readMessagesJson(hd.homedir, 'e2e-t18');
    assert(
      afterSkipMsgs.some((m) => m.role === 'user' && String(m.content ?? '').includes(`第${rounds}轮问题`)),
      '熔断轮的历史必须原样保留（失败/跳过都不改历史）',
    );
    assert(
      doneTurns() > 0,
      '熔断轮的对话本身必须照常完成',
    );

    // ── 阶段 4：transcript 转存（C）——失败的尝试同样先归档再请求 ──
    const trDir = join(sessionDir(hd.homedir, 'e2e-t18'), 'transcripts');
    let archives = [];
    try {
      archives = readdirSync(trDir).filter((f) => /^transcript_\d+_\d+\.json$/.test(f));
    } catch { /* 目录尚不存在 */ }
    assert(archives.length >= 1, `transcripts/ 下应有压缩前历史存档（实际 ${archives.length}）`);
    const archived = JSON.parse(readFileSync(join(trDir, archives[0]), 'utf8'));
    assert(
      Array.isArray(archived) && archived.some((m) => String(m.content ?? '').includes('第1轮问题')),
      '存档应包含被压缩的早期消息（可回溯）',
    );

    // ── 阶段 5：重试恢复（F）——绕过熔断、删除-重建、成功 ──
    const retryAnchor = rt.changes.length;
    const retry = await cli.invoke(
      'session/chat/send',
      { session_id: 'e2e-t18', resume: { target_id: failId, action: 'retry_compaction' } },
      { session_id: 'e2e-t18', workdir: hd.workdir },
    );
    assert(retry.status === 200, `retry_compaction 应受理（${retry.status}: ${JSON.stringify(retry.body)?.slice(0, 200)}）`);

    // 旧失败节点被 removed（删除-重建的实时面交代）
    await waitFor(
      () => rt.framesOf(failId).some((f) => f.data?.status === 'removed' && f.arrival >= retryAnchor),
      { what: '旧失败压缩节点的 removed 帧', timeoutMs: 30_000 },
    );
    // 新压缩节点出现并终态 completed（第 4 次命中 = failTimes 之外 = 成功）
    await waitFor(
      () => [...episodesSnapshot().entries()].some(([, fs]) => fs[fs.length - 1].data.status === 'completed'),
      { what: '重试产生的 completed 压缩节点', timeoutMs: 30_000 },
    );

    const allEpisodes = episodesSnapshot();
    const okEntry = [...allEpisodes.entries()].find(([, fs]) => fs[fs.length - 1].data.status === 'completed');
    assert(okEntry, '应有 completed 压缩节点');
    const [okId, okFrames] = okEntry;
    assert(okId !== failId, '重试必须产生**新**节点（删除-重建，不是原地复活旧 id）');
    const final = okFrames[okFrames.length - 1].data;

    // A：流式身份帧——占位之后、终态之前，摘要以纯窄增量逐帧改道到压缩节点
    const deltaFrames = okFrames.filter(
      (f) => f.arrival > okFrames[0].arrival && f.arrival < okFrames[okFrames.length - 1].arrival && f.data?.delta != null,
    );
    assert(deltaFrames.length >= 1, '压缩节点上应有摘要的流式增量帧（用户看得到摘要在长出来）');
    const streamed = deltaFrames.map((f) => f.data.delta).join('');
    assertEq(
      streamed,
      SNAPSHOT.slice(SNAPSHOT_CHUNKS[0].length),
      '增量拼接应等于摘要去掉首个分片（首分片走全量首帧被吞，其余走窄增量改道）',
    );
    for (const f of deltaFrames) {
      assert(f.data.content == null && f.data.status == null, '改道帧必须是纯窄增量（不构成节点身份）');
      assertEq(f.data.type, undefined, '增量帧不携带身份字段');
    }
    // A（反面）：没有任何**旁路节点**收到过摘要增量
    const bypass = rt
      .messages()
      .filter(({ mid, data }) => mid !== okId && String(data?.content ?? '').includes('state_snapshot') && data?.type !== 'compression');
    // 快照 head 消息（meta.compacted）合法持有渲染后的快照正文；其余任何节点不得出现原始 XML
    assert(
      bypass.every(({ data }) => data?.meta?.compacted === true),
      `摘要增量不得以旁路节点身份上流（可疑节点：${bypass.map((b) => String(b.mid).slice(0, 8)).join(',')}）`,
    );

    // B：成功节点的结构化 meta
    assertEq(final.meta?.compact_trigger, 'retry', '重试路径的触发来源必须是 retry');
    assertEq(final.meta?.context_limit, 12000, 'meta.context_limit');
    assert(Number(final.meta?.before_tokens) > 0, 'meta.before_tokens');
    assert(
      Number(final.meta?.after_tokens) > 0 && Number(final.meta?.after_tokens) < Number(final.meta?.before_tokens),
      `后水位应存在且低于前水位（${final.meta?.before_tokens} → ${final.meta?.after_tokens}）`,
    );
    assert(Number(final.meta?.dropped) > 0, 'meta.dropped 应记录这次压掉的条数');

    // 重写收敛：被压掉历史逐条 removed + 快照帧（meta.compacted）
    const removes = rt.messages().filter(({ data, arrival }) => data?.status === 'removed' && arrival >= retryAnchor);
    assert(removes.length > 0, '重试成功后应下发被压掉消息的 removed 帧');
    const snapshotFrame = rt
      .messages()
      .filter(({ data, arrival }) => data?.meta?.compacted === true && arrival >= retryAnchor)
      .pop();
    assert(snapshotFrame, '重试成功后实时面应出现 meta.compacted=true 的快照帧');

    // 落盘同源：失败节点已删、快照在、唯一 compression 节点是重试产物
    await waitFor(() => {
      const msgs = readMessagesJson(hd.homedir, 'e2e-t18') ?? [];
      return msgs.some((m) => (m.type ?? m.msg_type) === 'compression' && m.meta?.compact_trigger === 'retry');
    }, { what: '重试的压缩节点落库', timeoutMs: 30_000 });
    const msgs = readMessagesJson(hd.homedir, 'e2e-t18');
    assertTranscriptInvariants(msgs, 'T18 终态');
    // 重试只删除**被点击的那一个**失败节点（failId）；更早的两个失败节点原样
    // 留在存储里（它们各自都有交代，删除它们不是重试的职责）。
    const compNodes = msgs.filter((m) => (m.type ?? m.msg_type) === 'compression');
    const completedComp = compNodes.filter((m) => m.status === 'completed');
    assertEq(completedComp.length, 1, `存储里应恰有一个 completed 压缩节点（实际 ${completedComp.length}）`);
    assertEq(completedComp[0].id, okId, '存储里的 completed 压缩节点应是实时面上的那个新节点');
    assert(!completedComp[0].meta?.failure_kind, '成功的压缩节点不带失败码');
    assert(
      !msgs.some((m) => m.id === failId),
      '旧失败节点应已从存储删除（不是留给下次会话当尸体）',
    );
    const storedSnap = msgs.find((m) => m.meta?.compacted === true);
    assert(storedSnap, '存储里应有压缩快照');
    assertEq(storedSnap.role, 'user', '快照必须是 user 角色');
    assert(
      !msgs.some((m) => m.role === 'user' && String(m.content ?? '').includes('第1轮问题')),
      '第一轮用户消息应已被重试压缩出存储',
    );

    // ── 阶段 6：恢复后的正常轮——水位已低，不再触发压缩 ──
    const beforeCalm = await snapshotReqCount();
    await sendRound(rounds, `第${rounds + 1}轮问题`);
    rounds += 1;
    assertEq(await snapshotReqCount(), beforeCalm, '恢复后的正常轮不得再触发压缩（水位已降）');
    const calmMsgs = readMessagesJson(hd.homedir, 'e2e-t18');
    assertTranscriptInvariants(calmMsgs, 'T18 恢复后');
  } finally {
    rt?.close();
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }

  // ── 局部工具 ──
  /** 当前实时面上 compression 节点的分幕（身份帧 + 夹在中间的增量帧） */
  function episodesSnapshot() {
    const map = new Map();
    for (const f of rt.messages()) {
      if (f.data?.type === 'compression' || (f.data?.delta != null && rt.framesOf(f.mid).some((x) => x.data?.type === 'compression'))) {
        const list = map.get(f.mid) ?? [];
        list.push(f);
        map.set(f.mid, list);
      }
    }
    return map;
  }
  /** 一个分幕内：除 removed/终态外，位置序号必须是节点属性（复用 T11 的形状判据） */
  function assertSeqShapeOk(frames) {
    const seqs = frames.map((f) => f.data?.seq).filter((s) => s != null);
    const authority = seqs.filter((s) => s < (1 << 50));
    assert(new Set(authority).size <= 1, `压缩节点权威 seq 至多一个（实得 ${JSON.stringify(seqs)}）`);
  }
});
