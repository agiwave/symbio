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
// | A | 摘要失败真的发生 | mock 收到摘要请求（最后一条 user 是转写形态）且返回 500 |
// | B | 失败不落压缩事实 | 事实源里没有 `session.consolidated` |
// | C | 失败有交代 | 至少一条 `failed` 事实，且落在 `ring:model.assist:`（`payload.class = non_retryable`、`reason` 非空） |
// | D | **失败合并不吞消息** | 每条用户 marker 在 live + 归档并集里**恰好出现一次** |
//
// ## 计量口径
//
// 触发与 t8 同一口径：mock 恒报 `prompt_tokens: 64`（见 `mock-llm.mjs`），上限取 80
// ⇒ 每轮越线、每轮都欠一次摘要。失败是 `NonRetryable`（provider 5xx 映射），
// 环不重试；`compact-write` 找不到摘要时返回成功、不落事实（「没压」不是领域事实）。
// `retryAfterSec: 0` 让客户端的 5xx 退避立即重试，避免空等指数退避。
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
    // 的对话线与主请求同形，只有摘要指令能把它与主请求区分开）。
    {
      id: 'snapshot',
      matchSystem: '你是上下文压缩器',
      status: 500,
      retryAfterSec: 0,
      error: 'mock 注入的摘要服务故障',
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

    // C：失败本身有交代——失败落在 `model.assist` 的环事实里（环的 failed 载荷
    //    形如 `{ class, reason, facts }`），且没有别的端口失败
    const failed = events.filter((e) => e.kind === 'failed');
    assert(failed.length >= 1, `失败应落 failed 事实（现有 kind: ${kinds}）`);
    assert(
      failed.every((e) => String(e.event_id ?? '').startsWith('ring:model.assist:')),
      `注入对象应只是 model.assist（实际 failed id: ${failed.map((e) => e.event_id).join(', ')}）`,
    );
    const f = failed[0];
    assertEq(f.payload?.class, 'non_retryable', 'HTTP 500 的失败类别');
    assert(String(f.payload?.reason ?? '').length > 0, 'failed 事实应带非空原因（reason）');

    // D：失败合并不吞消息——每条 marker 在 live + 归档并集里恰好一次
    for (const marker of MARKERS) {
      const count = events.filter(
        (e) => e.kind === 'user.message' && String(e.payload?.text ?? '') === marker,
      ).length;
      assertEq(count, 1, `用户消息「${marker}」在事实源（live + 归档）里应恰好出现一次（实际 ${count}）`);
    }

    // D 的读出口同源：前端 loadMessages 同一出口读回全部 6 条、无压缩节点
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
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});
