import './_selfrun.mjs';

// D-401 预算切片：摘要输入永远装得下。
//
// ## 这根判据在说什么
//
// 旧实现把**整份**转写拼进摘要请求：转写越接近窗口上限，摘要请求越先撞同一个上限，
// 而压缩恰恰只在那个时刻需要它。本用例喂一份比切片预算大一个数量级的转写，钉住：
//
// 1. 每个摘要请求（system = 「你是上下文压缩器」）的输入 token ≤ k × limit（k = 0.5）；
// 2. 连续两轮压缩之后，主请求水位真实下降（而不是只降在 mock 的固定读数里）；
// 3. 夹具的转写规模 ≥ 10 × 切片预算——否则「装得下」是白检的。
//
// 水位口径：mock 打开 `usageFromRequest`，把请求体按 CJK 1 token/字、其余 0.25/字符
// 粗估（与真实 provider 同一量级）；主请求的实测值随 `model` 收束载荷的 `context`
// 落进 WAL，读它就是「provider 报的水位」而不是测试自算的。
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readFileSyncSafe,
  waitFor,
  assert,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

const LIMIT = 16_384;
const K = 0.5;
const SLICE_BUDGET = Math.floor(LIMIT * K);
const ROUNDS = 24;
const USER_FILL = '用户填充内容用于拉长历史。'.repeat(100);
const REPLY = `填充：${'上下文填充内容用于推高历史水位。'.repeat(156)}`;

const SNAPSHOT = '<state_snapshot><overall_goal>切片摘要可用。</overall_goal></state_snapshot>';

/** 与 `mock-llm.mjs::estimateTokens` 同口径（CJK 1 / 其余 0.25）。 */
function estimateTokens(text) {
  let weight = 0;
  for (const ch of text) {
    const cp = ch.codePointAt(0);
    const cjk =
      (cp >= 0x3000 && cp <= 0x9fff) ||
      (cp >= 0xf900 && cp <= 0xfaff) ||
      (cp >= 0xff00 && cp <= 0xffef);
    weight += cjk ? 1 : 0.25;
  }
  return Math.max(1, Math.ceil(weight));
}

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

const isSummaryRequest = (req) =>
  (req.body?.messages ?? []).some(
    (m) => m.role === 'system' && String(m.content ?? '').includes('你是上下文压缩器'),
  );

export default defineCase('压缩摘要按预算切片：摘要输入 ≤ k×limit，水位两降', async () => {
  const llm = await new MockLlm(
    [
      { id: 'snapshot', matchSystem: '你是上下文压缩器', content: SNAPSHOT },
      { id: 'filler', match: '轮问题', content: REPLY },
    ],
    { usageFromRequest: true },
  ).start();
  const hd = makeHomedir({
    providers: [
      { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: LIMIT }) },
    ],
    pluginConfigs: { session: { context_messages: 0 } },
  });
  try {
    const stdinText = Array.from(
      { length: ROUNDS },
      (_, i) => `第${i + 1}轮问题：${USER_FILL}`,
    ).join('\n');
    const r = await runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      provider: PROVIDER_ID,
      session: 'e2e-t106',
      stdinText: `${stdinText}\n`,
      timeoutMs: 180_000,
    });
    assert(r.code === 0, `CLI 退出码（stderr 尾部: ${r.stderr.slice(-600)}）`);

    const events = walEvents(hd.homedir, 'e2e-t106');

    // 夹具规模：转写（user/final 正文）必须比切片预算大一个数量级。
    const transcriptTokens = events
      .filter((e) => e.kind === 'user.message' || e.kind === 'chat.assistant.final')
      .reduce(
        (sum, e) =>
          sum + estimateTokens(String(e.payload?.text ?? e.payload?.answer ?? '')),
        0,
      );
    assert(
      transcriptTokens >= SLICE_BUDGET * 10,
      `夹具的转写规模 ${transcriptTokens} 应 ≥ 10 × 切片预算 ${SLICE_BUDGET * 10}`,
    );

    // ① 摘要请求的输入 token ≤ k × limit（改前：整份转写 ⇒ 这里红）。
    const summaryRequests = (await llm.requests()).filter(isSummaryRequest);
    assert(
      summaryRequests.length >= 2,
      `至少应有两次摘要往返（实得 ${summaryRequests.length}）`,
    );
    const summarySizes = summaryRequests.map((req) =>
      estimateTokens(JSON.stringify(req.body)),
    );
    const oversized = summarySizes.filter((size) => size > SLICE_BUDGET);
    assert(
      oversized.length === 0,
      `摘要请求输入必须 ≤ k×limit=${SLICE_BUDGET}；超限读数: ${oversized.join(', ')}（全部: ${summarySizes.join(', ')}）`,
    );

    // ② 水位两降：主请求的实测 token 序列里至少出现两次下降。
    const watermarks = events
      .filter((e) => e.kind === 'response' && e.payload?.context?.request_tokens != null)
      .map((e) => Number(e.payload.context.request_tokens));
    const drops = [];
    for (let i = 1; i < watermarks.length; i += 1) {
      if (watermarks[i] < watermarks[i - 1]) {
        drops.push({ at: i, from: watermarks[i - 1], to: watermarks[i] });
      }
    }
    assert(
      drops.length >= 2,
      `连续两轮压缩后水位应真实下降两次；现有水位序列: ${watermarks.join(', ')}`,
    );
    assert(
      drops.every((d) => d.from - d.to > 100),
      `水位下降应是压缩掉的真实体积（现有: ${JSON.stringify(drops)}）`,
    );

    // 实测读数留在输出里：报号与日后回归取数不必另写脚本。
    console.log(
      `  [读数] 摘要输入 token: ${summarySizes.join(', ')}｜转写 ${transcriptTokens}` +
        `｜水位 ${watermarks.join('→')}`,
    );
  } finally {
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: 'mock-llm 子进程退出',
    });
  }
});
