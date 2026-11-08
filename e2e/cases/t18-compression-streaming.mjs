// T18 压缩失败纪律：`model.assist`（摘要）失败 ⇒ 不落 `session.consolidated`、
// 不吞任何用户消息；失败本身有交代（`failed` 事实）。
import './_selfrun.mjs';
//
// ## 本用例钉的是什么
//
// 压缩现在是两行债（D-336）：`compact-summarize`（端口 `model.assist`）取摘要，
// `compact-write`（端口 `compact`）落 `session.consolidated`。故障注入点因此必须是
// **`model.assist` 那次问答**——注在「压缩请求」上是注空（新形态没有独立的压缩请求，
// 摘要走 `model.assist`，与主请求同形，按 system 里的压缩指令识别）。
//
// 失败路径的硬不变式：
//
// | # | 契约 | 判据 |
// |---|---|---|
// | A | 摘要失败真的发生 | mock 收到摘要请求（最后一条 user 是转写形态）且返回 400（超窗） |
// | B | 失败不落压缩事实 | 事实源里没有 `session.consolidated` |
// | C | 失败有交代 | `model.assist` 的 `failed`（`class = non_retryable`、`reason` 非空）+ `compact` 的 `failed`（`stamp.failure = compression`、`failure_class = non_retryable_context_length`） |
// | D | **失败合并不吞消息** | 每条用户 marker 在 live + 归档并集里**恰好出现一次** |
// | E | 跨轮抑制 | 6 轮里摘要请求**只有 1 次**（同一前缀失败过 ⇒ 不再每轮重付）；仍无 `session.consolidated` |
// | F | 失败产物 + 重试出路 | 读出口有一个 `Failed` 压缩节点（`actions` 含 `retry_compaction`）；对它发 `retry_compaction` ⇒ 摘要请求 +1 |
//
// ## 计量口径
//
// 触发与 t8 同一口径：mock 恒报 `prompt_tokens: 64`（见 `mock-llm.mjs`），上限取 80
// ⇒ 第一轮越线、欠一次摘要。失败按**真实 provider 的超窗错误**注入：HTTP 400 +
// `maximum context length`——它不是可重试状态，因此**一次派发只对应一次请求**（500
// 会被 HTTP 层退避重试 4 次，用它计量会把一次派发数成 5 次，抑制就测不出来）。
// `compact-write` 拿不到摘要 ⇒ **失败是产物**（D-402）：落带原因分类的 `failed` 事实
// （超窗归 `non_retryable_context_length`），读面折出 `Failed` 压缩叶，重试入口可达。
// 两行债的 id 按**前缀指纹**派生（最旧未压事实 + 上限 + 代），同一前缀失败过就收束在
// 同一把键上——其后轮次不再重付。
//
// 第三步的读数同用这一格：失败应答带 150ms `latencyMs`（模拟 provider 首字节），mock
// 另记 `doneAt`；用例按主请求分轮数「额外请求数 / 额外往返 ms」，判据是同一前缀判死后，
// 其后每轮这两项都为 0——即跨轮抑制省下的正是这份重复往返。
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  nextPort,
  startReadbackCli,
  readSessionMessages,
  readFileSyncSafe,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
  textOf,
  resumeSession,
} from '../helpers.mjs';

const ROUNDS = 6;
const MARKERS = Array.from({ length: ROUNDS }, (_, i) => `第${i + 1}轮问题`);

const FILL = '上下文填充内容用于推高历史水位。';
const REPLY = '填充：' + FILL.repeat(240);

/** 会话事实源：live 段 + 归档段（`transcripts/wal-archive`）的并集。 */
const walEvents = (homedir, sid) => {
  const dir = join(homedir, 'session', sid);
  const archiveDir = join(dir, 'transcripts', 'wal-archive');
  const texts = [join(dir, 'v2-events.wal')];
  if (existsSync(archiveDir)) {
    for (const name of readdirSync(archiveDir)) texts.push(join(archiveDir, name));
  }
  return texts
    .map(readFileSyncSafe)
    .join('')
    .split('\n')
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line));
};

/** 一条请求的 system 正文（摘要请求的特征 = compact 插件的摘要指令）。 */
const systemText = (q) => {
  const sys = (q.body?.messages ?? []).find((m) => m.role === 'system');
  return String(sys?.content ?? '');
};

export default defineCase('T18 压缩失败纪律：model.assist 失败不落事实、不吞消息', async () => {
  const llm = await new MockLlm([
    // 静态注入：这段时间里**每一次**摘要问答都失败。命中标记是 system（`model.assist`
    // 的对话线与主请求同形，只有摘要指令能把它与主请求区分开）。400 是不可重试状态，
    // 一次派发只发一次请求（见文件头计量口径）。
    {
      id: 'snapshot',
      matchSystem: '你是上下文压缩器',
      status: 400,
      error: 'maximum context length exceeded',
      // 失败应答同样要等一次首字节：真实 provider 的超窗 400 不是瞬时的。
      // 没有这段延迟，第三步的「每轮额外往返」会恒为 0，读数失去意义。
      latencyMs: 150,
    },
    { id: 'big', match: '轮问题', content: REPLY },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [
      { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 80 }) },
    ],
    pluginConfigs: {
      // 本用例的主题是压缩，与对话面正交 ⇒ 把两个开关钉死（理由见 `DIALOG_FACE_OFF`）。
      session: { context_messages: 0, ...DIALOG_FACE_OFF },
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
    const r = await runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      provider: PROVIDER_ID,
      session: 'e2e-t18',
      stdinText: MARKERS.join('\n') + '\n',
      timeoutMs: 120_000,
    });
    assertEq(r.code, 0, `CLI 退出码（stderr 尾部: ${r.stderr.slice(-600)}）`);

    // A：注入命中对象真的是 `model.assist`（摘要请求的 system 是压缩指令）
    const reqs = await llm.requests();
    const summaryReqs = reqs.filter((q) => systemText(q).includes('你是上下文压缩器'));
    assert(
      summaryReqs.length >= 1,
      `mock 应收到摘要问答（model.assist 的 system 是压缩指令）；实际请求 ${reqs.length} 条，末条 system: ${JSON.stringify(systemText(reqs[reqs.length - 1] ?? {}).slice(0, 40))}`,
    );

    // B：摘要失败 ⇒ 一条压缩事实都不落
    const events = walEvents(hd.homedir, 'e2e-t18');
    const kinds = [...new Set(events.map((e) => e.kind))].join(',');
    assert(
      !events.some((e) => e.kind === 'session.consolidated'),
      `摘要失败时不得落 session.consolidated（现有 kind: ${kinds}）`,
    );

    // C：失败本身有交代——上游 `model.assist` 的 failed（类 + 原因）与
    //    `compact` 的 failed（压缩失败产物：`stamp.failure = compression` + 原因分类）
    const failed = events.filter((e) => e.kind === 'failed');
    assert(failed.length >= 1, `失败应落 failed 事实（现有 kind: ${kinds}）`);
    const assistantFailed = failed.filter((e) =>
      String(e.event_id ?? '').startsWith('ring:model.assist:'),
    );
    const compactFailed = failed.filter(
      (e) =>
        String(e.event_id ?? '').startsWith('ring:compact:') &&
        e.payload?.stamp?.failure === 'compression',
    );
    assert(assistantFailed.length >= 1, `注入对象应只是 model.assist（实际 failed id: ${failed.map((e) => e.event_id).join(', ')}）`);
    const f = assistantFailed[0];
    assertEq(f.payload?.class, 'non_retryable', 'HTTP 400 超窗的失败类别');
    assert(String(f.payload?.reason ?? '').length > 0, 'failed 事实应带非空原因（reason）');
    assert(compactFailed.length >= 1, `压缩失败要有产物（D-402）；实际 failed id: ${failed.map((e) => e.event_id).join(', ')}`);
    const c = compactFailed[0];
    assertEq(
      c.payload?.failure_class,
      'non_retryable_context_length',
      '真实超窗错误码应归类为 non_retryable_context_length',
    );
    assert(String(c.payload?.detail ?? '').length > 0, '失败产物应带原因正文（detail）');
    assert(String(c.payload?.compression_id ?? '').length > 0, '失败产物应带压缩节点身份（compression_id）');

    // E：跨轮抑制——同一前缀只欠一次。6 轮里摘要请求只有 1 次，判死也只落一条；
    //    水位仍越线，但债键（前缀指纹）已收束，不再每轮重付注定失败的往返。
    assertEq(
      summaryReqs.length,
      1,
      `6 轮里摘要请求只应有 1 次（同一前缀失败过 ⇒ 跨轮抑制）；实际 ${summaryReqs.length} 次；failed: ${failed.map((e) => e.event_id).join(' | ')}`,
    );
    assertEq(compactFailed.length, 1, '同一前缀只判死一次，不随轮次重复落格');

    // 读数（第三步，先度量不优化）：持续失败时，每轮为压缩失败**多付了多少**。
    // 按「带用户 marker 的主请求」分轮，摘要请求归到它之前的那一轮；耗时取 mock 的
    // `doneAt - at`（失败应答的首字节 150ms + 服务端处理），不含主请求自身的时间。
    const isSummaryReq = (q) => systemText(q).includes('你是上下文压缩器');
    const lastUserText = (q) => {
      const msgs = Array.isArray(q.body?.messages) ? q.body.messages : [];
      const m = [...msgs].reverse().find((x) => x.role === 'user');
      return typeof m?.content === 'string' ? m.content : '';
    };
    const perRound = Array.from({ length: ROUNDS }, () => ({ requests: 0, ms: 0 }));
    let round = -1;
    for (const q of reqs) {
      if (!isSummaryReq(q)) {
        if (MARKERS.includes(lastUserText(q))) round += 1;
        continue;
      }
      if (round < 0 || round >= ROUNDS) continue;
      perRound[round].requests += 1;
      perRound[round].ms += Math.max(0, (q.doneAt ?? q.at) - q.at);
    }
    const extraRequests = perRound.map((r) => r.requests);
    const extraMs = perRound.map((r) => r.ms);
    const totalExtra = extraRequests.reduce((a, b) => a + b, 0);
    console.log(
      `  [读数] 持续失败 ${ROUNDS} 轮：每轮额外请求 ${JSON.stringify(extraRequests)}` +
        `｜每轮额外往返 ${JSON.stringify(extraMs)} ms` +
        `｜合计 ${totalExtra} 次 / ${extraMs.reduce((a, b) => a + b, 0)} ms（失败应答按 150ms 首字节计）`,
    );
    assertEq(totalExtra, summaryReqs.length, '分轮读数应覆盖全部摘要请求（归属不漏）');
    assertEq(
      extraRequests.slice(1).reduce((a, b) => a + b, 0),
      0,
      '同一前缀判死后，其后每轮不得再付压缩失败的额外往返',
    );

    // D：失败合并不吞消息——每条 marker 在 live + 归档并集里恰好一次
    for (const marker of MARKERS) {
      const count = events.filter(
        (e) => e.kind === 'user.message' && String(e.payload?.text ?? '') === marker,
      ).length;
      assertEq(count, 1, `用户消息「${marker}」在事实源（live + 归档）里应恰好出现一次（实际 ${count}）`);
    }

    // D 的读出口同源：前端 loadMessages 同一出口读回全部 6 条、无压缩快照正文
    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t18',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t18');
    assertTranscriptInvariants(msgs, 'T18');
    for (const marker of MARKERS) {
      const count = msgs.filter((m) => msgRoleOf(m) === 'user' && textOf(m) === marker).length;
      assertEq(count, 1, `读出口里用户消息「${marker}」应恰好一条（实际 ${count}）`);
    }
    assert(
      !msgs.some((m) => String(m.content ?? '').includes('state_snapshot')),
      '摘要失败时读出口不应出现压缩快照正文',
    );

    // F：失败产物在**读出口**上是一个 `Failed` 压缩节点，且 `actions` 带重试出路；
    //    对它发一次 `retry_compaction` ⇒ 只看生产出口：摘要请求数 +1。
    const failedNodes = msgs.filter(
      (m) => m.status === 'failed' && Array.isArray(m.actions) && m.actions.includes('retry_compaction'),
    );
    assertEq(
      failedNodes.length,
      1,
      `读出口应恰有一个可重试的失败压缩节点（实际 ${failedNodes.length}；节点: ${JSON.stringify(msgs.map((m) => [m.id, m.status, m.actions]))}）`,
    );
    const target = failedNodes[0].id;
    const before = summaryReqs.length;
    const retry = await resumeSession(api, 'e2e-t18', target, 'retry_compaction', {});
    assert(
      retry.status === 200 && retry.body?.data?.ok === true,
      `retry_compaction 应被受理（实际: ${retry.status} ${JSON.stringify(retry.body)}）`,
    );
    await waitFor(
      async () => {
        const now = (await llm.requests()).filter((q) =>
          systemText(q).includes('你是上下文压缩器'),
        );
        return now.length >= before + 1;
      },
      { what: '重试重新发出一次摘要请求', timeoutMs: 30_000 },
    );
    // 重试仍失败 ⇒ 出现**新一代**失败产物（身份按前缀指纹换纪元，旧节点被移除事实隐藏），
    // 读出口再次出现可重试的 `Failed` 压缩节点、不再有 `session.consolidated`。
    await waitFor(
      async () => {
        const again = await readSessionMessages(api, 'e2e-t18');
        return again.some(
          (m) =>
            m.id !== target &&
            m.status === 'failed' &&
            Array.isArray(m.actions) &&
            m.actions.includes('retry_compaction'),
        );
      },
      { what: '失败产物在重试后重建', timeoutMs: 30_000 },
    );
    assert(
      !walEvents(hd.homedir, 'e2e-t18').some((e) => e.kind === 'session.consolidated'),
      '重试仍然失败 ⇒ 仍不得落 session.consolidated',
    );
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});
