#!/usr/bin/env node
/**
 * 真实会话有界压测（口径见 DE-016 `D-058`）。
 *
 * ## 为什么不在 `e2e/cases/`
 *
 * 这条入口会打**真实 provider**：它不进默认 CI、不进 `pre-commit`，也不该被
 * `e2e/cases/` 的目录发现式加载当成一条常规用例。放在 `e2e/` 根下，只能显式运行。
 *
 * ## 用法
 *
 * ```bash
 * SYMBIO_LOAD_API_BASE=http://127.0.0.1:1234/v1 \
 * SYMBIO_LOAD_API_KEY=... \
 * SYMBIO_LOAD_MODEL=... \
 * SYMBIO_LOAD_PROTOCOL=openai_chat \
 * node e2e/real-session-load.mjs --turns=6 --max-wall-ms=600000
 * ```
 *
 * 缺 `SYMBIO_LOAD_API_BASE` / `SYMBIO_LOAD_MODEL` 时**拒绝启动**（退出码 2）——
 * 这条入口存在的意义就是「真实 provider」，悄悄回退 MockLLM 会把判据偷换掉。
 *
 * ## 判据（全部读生产出口）
 *
 * - `session/stats` 的 `invariants` 为空；
 * - `checkpoint.event_count` 随轮数增长；
 * - `tiers.*.p99` 不超过对应档位的 `budget_ms`（DE-016 §8.2）；
 * - 墙钟 / 轮数 / 可选 token 上界；任一越界非零退出。
 *
 * 运行记录只打摘要到 stdout，不写密钥、不写完整响应。
 */
import { makeHomedir, dialogFaceOff, cleanupHomedir, nextPort, startRealtimeSession, settledAfterWorking, waitFor, assert } from './helpers.mjs';

/**
 * `sendAndSettle` 的整例预算是 e2e 的 120s；真实 provider 单轮就可能 60s+，
 * 六轮必然越过。这里只复用 `settledAfterWorking` 判据，预算由压测参数自己管。
 */
async function sendAndSettleLong(cli, sub, message, timeoutMs) {
  const mark = sub.sessionChanges().length;
  cli.send(message);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const frames = sub.sessionChanges().slice(mark);
    if (settledAfterWorking(frames)) {
      const stateFrames = frames.filter((f) => typeof f.data?.status === 'string');
      const last = stateFrames[stateFrames.length - 1]?.data?.status;
      if (last === 'failed') throw new Error(`本轮以 failed 收尾（${last}）`);
      return;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`本轮等待超时（${timeoutMs}ms）`);
}

const args = new Map(
  process.argv
    .slice(2)
    .filter((a) => a.startsWith('--'))
    .map((a) => {
      const [k, v] = a.replace(/^--/, '').split('=');
      return [k, v ?? 'true'];
    }),
);
const numArg = (k, d) => (args.has(k) ? Number(args.get(k)) : d);

const TURNS = Math.max(1, numArg('turns', 6));
const MAX_WALL_MS = numArg('max-wall-ms', 600_000);
const MAX_TOKENS = numArg('max-tokens', 0);
const PER_TURN_MS = numArg('per-turn-ms', 180_000);
const MAX_CONTEXT_TOKENS = numArg(
  'max-context-tokens',
  Number(process.env.SYMBIO_LOAD_MAX_CONTEXT_TOKENS ?? 32_768),
);
const AWAIT_COMPACTION_MS = numArg('await-compaction-ms', 0);
const API_BASE = process.env.SYMBIO_LOAD_API_BASE;
const API_KEY = process.env.SYMBIO_LOAD_API_KEY ?? '';
const MODEL = process.env.SYMBIO_LOAD_MODEL;
const PROTOCOL = process.env.SYMBIO_LOAD_PROTOCOL ?? 'openai_chat';

if (!API_BASE || !MODEL) {
  console.error(
    '拒绝启动：真实会话压测必须显式给出 provider。\n' +
      '  需要 SYMBIO_LOAD_API_BASE 与 SYMBIO_LOAD_MODEL（可选 SYMBIO_LOAD_API_KEY / SYMBIO_LOAD_PROTOCOL）。\n' +
      '  不提供时不会回退 MockLLM——那会把「真实会话」偷换成「跑过 e2e」。',
  );
  process.exit(2);
}

/** DE-016 §8.2 的四层预算；真实模型请求通常落 `deep`。 */
const BUDGET_MS = {
  reflect: 80,
  fast: 300,
  deep: 60_000,
  autonomic: 86_400_000,
  unspecified: 60_000,
};

/** 每轮都是单行（REPL stdin 按行消费）；内容覆盖配置、日志、代码、压缩四类真实提问。 */
const PROMPTS = [
  '用三句话解释：为什么长会话不能靠裁剪历史来省上下文。',
  '请先调用 vdfs_list 工具列出工作目录，再用一句话说明看到了什么。',
  '下面这段日志里哪几条是退避重发？WARN retry 1/5 in 200ms / WARN retry 2/5 in 400ms / ERROR gave up after 5 attempts',
  '把一个 Rust 函数的命名从 process_data_v2_final 改成三个候选，并说明各自偏向。',
  '写一段 300 字左右的中文说明：压缩失败时为什么不能吞掉用户消息。',
  '解释 VDFS 挂载序为什么必须稳定，给一个能被用户观察到的后果。',
  '用一句话说明：同一次工具调用跨轮累积与每轮清空的区别。',
  '给出一份三步排查清单：会话收束后读不到最新助手正文。',
  '用 200 字总结：真实会话压测为什么要读生产出口而不是内部文件。',
];

const SID = `load-real-${Date.now()}`;
const GATEWAY_PORT = nextPort();
const hd = makeHomedir({
  providers: [
    {
      id: 'real',
      config: {
        id: 'real',
        name: 'LAN Provider',
        provider: 'openai',
        api_base: API_BASE,
        api_key: API_KEY,
        model: MODEL,
        api_protocol: PROTOCOL,
        temperature: 0.2,
        max_tokens: 8192,
        max_context_tokens: MAX_CONTEXT_TOKENS,
        // 推理模型会把 max_tokens 先花在 reasoning 上；压到 low，避免每轮被长度截断。
        // Anthropic 的 `reasoning` 是另一套形态（开 thinking + 预算），不在压测入口里
        // 冒充同一个字段；需要时由 provider 自己的配置决定。
        ...(PROTOCOL === 'openai_chat' ? { reasoning: { effort: 'low' } } : {}),
        enabled: true,
      },
    },
  ],
  pluginConfigs: {
    gateway: {
      inbound_enabled: true,
      inbound_protocol: 'http',
      inbound_bind: '127.0.0.1',
      inbound_port: GATEWAY_PORT,
      inbound_token: '',
      inbound_readonly: false,
    },
    ...dialogFaceOff(),
    // 真实模型会主动调 `web_search` 探索；压测要的是会话语义面，不是外网可用性。
    // 关掉 web 插件，工具面仍保留 VDFS / 本地能力，长轮里的工具调用照样能发生。
    web: { plugin_enabled: false },
  },
});

let rt = null;
let stats = null;
const startedAt = Date.now();
const turnMs = [];
try {
  rt = await startRealtimeSession({
    homedir: hd.homedir,
    workdir: hd.workdir,
    session: SID,
    provider: 'real',
    gatewayPort: GATEWAY_PORT,
  });
  const { cli, sub } = rt;
  await cli.waitGatewayReady(30_000, 'real-load');

  for (let i = 0; i < TURNS; i++) {
    const elapsed = Date.now() - startedAt;
    if (elapsed > MAX_WALL_MS) {
      throw new Error(`墙钟上界触发：已 ${elapsed}ms > ${MAX_WALL_MS}ms（第 ${i + 1} 轮前）`);
    }
    const t0 = Date.now();
    await sendAndSettleLong(cli, sub, PROMPTS[i % PROMPTS.length], PER_TURN_MS);
    turnMs.push(Date.now() - t0);
  }

  const readStats = async () => {
    const resp = await cli.invoke('session/stats', {}, { session_id: SID });
    assert(
      resp.body?.data,
      `session/stats 应带 data 载荷（status=${resp.status}, body=${JSON.stringify(resp.body)?.slice(0, 600)}）`,
    );
    return resp.body.data;
  };
  stats = await readStats();
  if (AWAIT_COMPACTION_MS > 0) {
    // 压缩由 turn 收束后的债处理触发，可能晚于「会话 settle」；等它落格再读数，
    // 否则会把「还在跑」误判成「没有压缩」。
    const deadline = Date.now() + AWAIT_COMPACTION_MS;
    while (
      Date.now() < deadline &&
      Number(stats.checkpoint?.kind_counts?.['session.consolidated'] ?? 0) === 0
    ) {
      await new Promise((r) => setTimeout(r, 500));
      stats = await readStats();
    }
  }
  assert(stats.has_wal, '跑过真实轮次 ⇒ 事实源应在');
  assert(
    Array.isArray(stats.invariants) && stats.invariants.length === 0,
    `不变量应全绿（实际: ${JSON.stringify(stats.invariants).slice(0, 600)}）`,
  );
  assert(
    Number(stats.checkpoint?.event_count ?? 0) > TURNS * 2,
    `事实总量应随轮数累积（实际: ${stats.checkpoint?.event_count}）`,
  );

  const tiers = Array.isArray(stats.tiers) ? stats.tiers : [];
  assert(tiers.length > 0, 'tiers 应至少有一档读数');
  for (const row of tiers) {
    const budget = BUDGET_MS[row.tier] ?? BUDGET_MS.unspecified;
    assert(
      Number(row.p99 ?? 0) <= budget,
      `档位 ${row.tier} 的 P99=${row.p99}ms 超过预算 ${budget}ms`,
    );
  }

  const usage = stats.usage ?? stats.reports?.usage ?? null;
  if (MAX_TOKENS > 0) {
    const total = Number(
      usage?.total_tokens ?? usage?.total ?? usage?.input_tokens + usage?.output_tokens ?? NaN,
    );
    assert(
      Number.isFinite(total) && total <= MAX_TOKENS,
      `token 上界触发：usage=${JSON.stringify(usage)}（上限 ${MAX_TOKENS}）`,
    );
  }

  const kinds = stats.checkpoint?.kind_counts ?? {};
  const wallMs = Date.now() - startedAt;
  assert(wallMs <= MAX_WALL_MS, `墙钟上界触发：${wallMs}ms > ${MAX_WALL_MS}ms`);

  if (process.env.SYMBIO_LLM_RAW_DEBUG) {
    console.error(rt.cli.out.stderr);
  } else if (Number(kinds.failed ?? 0) > 0) {
    console.error(
      `--- 本轮有 ${kinds.failed} 条 failed 事实（CLI stderr 尾部） ---\n${rt.cli.out.stderr.slice(-1600)}`,
    );
  }
  console.log(
    JSON.stringify(
      {
        ok: true,
        turns: TURNS,
        wall_ms: wallMs,
        turn_ms: turnMs,
        event_count: stats.checkpoint?.event_count,
        compactions: Number(kinds['session.consolidated'] ?? 0),
        context: stats.context,
        tiers: tiers.map((r) => ({ tier: r.tier, turns: r.turns, p99: r.p99, samples: r.samples })),
        usage,
        kind_counts: kinds,
      },
      null,
      2,
    ),
  );
} catch (e) {
  console.error(`压测失败：${e?.message ?? e}`);
  if (stats) {
    const kinds = stats.checkpoint?.kind_counts ?? {};
    console.error(
      `--- 失败轮读数 ---\n${JSON.stringify(
        {
          event_count: stats.checkpoint?.event_count,
          compactions: Number(kinds['session.consolidated'] ?? 0),
          invariants: stats.invariants,
          tiers: stats.tiers,
          usage: stats.usage,
          kind_counts: kinds,
        },
        null,
        2,
      )}`,
    );
  }
  if (rt) console.error(`--- CLI stderr 尾部 ---\n${rt.cli.out.stderr.slice(-2000)}`);
  process.exitCode = 1;
} finally {
  rt?.sub?.close?.();
  rt?.cli?.stop();
  cleanupHomedir(hd);
  try {
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: 'CLI 子进程退出',
      timeoutMs: 15_000,
    });
  } catch {
    /* 退出等待不覆盖原失败 */
  }
}
