#!/usr/bin/env node
/**
 * 团队模式真实环境验证（DE-038：委派 / 对等 / 承诺 / 声誉）。
 *
 * ## 为什么不在 `e2e/cases/`
 *
 * 与 `real-session-load.mjs` 同一条约定：这条入口打**真实 provider**（本地 LMStudio
 * 服务），不进默认 CI、不进门禁，只能显式运行。悄悄回退 MockLLM 会把「真实环境的
 * 测试验证」偷换成「mock 又绿了一遍」——所以缺真实服务时**拒绝启动**（退出码 2）。
 *
 * ## 用法
 *
 * ```bash
 * node e2e/real-team-mode.mjs
 * ```
 *
 * provider 默认取仓内 `.symbio/model/LMStudio/provider.json`（指向 127.0.0.1:1234 的
 * 本地 LMStudio 服务）；可用环境变量覆盖：
 *
 * - `SYMBIO_TEAM_PROVIDER_JSON` —— 另一份 provider.json 的路径
 * - `SYMBIO_TEAM_API_BASE` / `SYMBIO_TEAM_MODEL` / `SYMBIO_TEAM_API_KEY` —— 逐项覆盖
 *
 * ## 场景与判定（全部读生产出口，不读内部状态）
 *
 * - **场景 A（树形委派团队）**：主会话（主体 `session/main`）经 `agent_run` 委派
 *   `researcher` 智能体完成实际任务。判定：子会话落盘且归档在父会话 `sessions/`
 *   子目录；事实源 `commitment.opened` + `commitment.released`（守约）；
 *   `session/stats` 声誉列 `offered=1 / kept=1 / broken=0`；`invariants` 为空；
 *   工具结果带回 `[subagent_session_id]`。
 * - **场景 B（对等协作轮）**：对等会话（主体 `session/peer-b`，登记窄行）经收件箱
 *   收一条带 `metadata.source_principal = 'session/main'` 的消息，对等体用真实模型
 *   跑一轮委托。判定：`artifact.added` 的作者 = 对等主体（产物收束）；
 *   事实源 `commitment.opened` + `commitment.released`；声誉列
 *   `offered=1 / kept=1 / broken=0 / score=1`；`invariants` 为空；子会话落盘。
 *
 * 「请求包不串主体」（t30 / t71 的探针判据）不在此重复：它需要 mock-llm 的
 * `/_requests` 回读，真实服务没有这个探测面——那半判据归 mock 用例，这里不偷。
 *
 * 真实模型一轮可达分钟级（含推理与嵌套子会话），等待上限放宽到 600s/场景。
 * 失败时保留现场 homedir 并打印路径（WAL / 转写都在里面，可事后取证）。
 */
import { mkdirSync, writeFileSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  nextPort,
  startLongLivedCli,
  createSession,
  sendInbox,
  readSessionMessages,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
} from './helpers.mjs';

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const DEFAULT_PROVIDER_JSON = join(REPO, '.symbio', 'model', 'LMStudio', 'provider.json');

/** 会话事实源文件名（与 `crates/symbio-plugin-session/src/paths.rs::V2_WAL_FILE` 同口径）。 */
const V2_WAL = 'v2-events.wal';

const MAIN_SID = 'real-team-main';
const PEER_SID = 'real-team-peer';

const MAIN_Q =
  '请务必通过 agent_run 工具，把任务「用不超过两句话说明冒泡排序的核心思想」委托给 researcher 智能体执行；等它完成后，把它的答复转告给我。';
const PEER_Q =
  '请务必通过 agent_run 工具，把任务「写出冒泡排序的时间复杂度，并用一句话解释」委托给 researcher 智能体执行。';

const PERSONA_WORKER = '你是严谨的研究员：直接给出结论，不做寒暄。';
const PERSONA_PEER = '你是协作对等体：只做工具调用，不做寒暄。';

/** 拒启动：真实 provider 入口不允许含糊其辞地继续跑。 */
function refuse(why) {
  console.error(`✖ 拒绝启动（真实 provider 入口不回退 mock）: ${why}`);
  process.exit(2);
}

/** 读真实 provider 配置：默认仓内 `.symbio/model/LMStudio/provider.json`，env 可逐项覆盖。 */
function loadRealProvider() {
  const path = process.env.SYMBIO_TEAM_PROVIDER_JSON || DEFAULT_PROVIDER_JSON;
  const raw = readFileSyncSafe(path);
  if (!raw.trim()) {
    refuse(`provider 配置不存在或为空: ${path}（也可用 SYMBIO_TEAM_PROVIDER_JSON 指定其它路径）`);
  }
  let cfg;
  try {
    cfg = JSON.parse(raw);
  } catch (e) {
    refuse(`provider 配置不是合法 JSON: ${path}（${e.message}）`);
  }
  if (!cfg.api_base || !cfg.model) {
    refuse(`provider 配置缺 api_base / model: ${path}`);
  }
  if (process.env.SYMBIO_TEAM_API_BASE) cfg.api_base = process.env.SYMBIO_TEAM_API_BASE;
  if (process.env.SYMBIO_TEAM_MODEL) cfg.model = process.env.SYMBIO_TEAM_MODEL;
  if (process.env.SYMBIO_TEAM_API_KEY) cfg.api_key = process.env.SYMBIO_TEAM_API_KEY;
  return cfg;
}

/** 真实服务可达性闸门：/models 必须列出目标模型，否则这条入口没有存在意义。 */
async function requireServer(cfg) {
  const base = cfg.api_base.replace(/\/+$/, '');
  const headers = cfg.api_key ? { Authorization: `Bearer ${cfg.api_key}` } : {};
  let ids;
  try {
    const resp = await fetch(`${base}/models`, { headers });
    const body = await resp.json();
    ids = (body?.data ?? []).map((m) => m?.id).filter(Boolean);
  } catch (e) {
    refuse(`真实服务不可达: ${base}（${e.message}）。请先启动本地 LMStudio 服务。`);
  }
  if (!ids.includes(cfg.model)) {
    refuse(`服务 ${base} 未加载模型 ${cfg.model}（可用: ${ids.join(', ') || '（空）'}）`);
  }
  console.error(`[real] provider ${cfg.id} → ${base} · 模型 ${cfg.model} ✓`);
}

/** 真实 provider 版的智能体目录（e2e 的 makeAgentDir 写的是 mock provider，不能复用）。 */
function makeRealAgentDir(homedir, { id, name, persona, providerCfg }) {
  const dir = join(homedir, 'agent', id);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, 'manifest.yaml'),
    `spec: "agent-dir/v2"\nid: "${id}"\nname: "${name}"\nversion: "1.0.0"\nrequires:\n  spec: "^2"\n`,
  );
  writeFileSync(join(dir, 'AGENTS.md'), persona);
  const pdir = join(dir, 'model', providerCfg.id);
  mkdirSync(pdir, { recursive: true });
  writeFileSync(join(pdir, 'provider.json'), JSON.stringify(providerCfg));
  return dir;
}

/** 读一份事实源 WAL 的行（末行未落全则丢弃——崩溃语义「已落盘的才算」）。 */
function walEvents(homedir, sid) {
  const raw = readFileSyncSafe(join(homedir, 'session', sid, V2_WAL));
  const lines = raw.split('\n').filter(Boolean);
  return lines
    .map((l, i) => {
      try {
        return JSON.parse(l);
      } catch {
        if (i === lines.length - 1) return null; // 尾行可能只写了一半
        throw new Error(`WAL ${sid} 第 ${i + 1} 行不是合法 JSON（盘上真坏了）`);
      }
    })
    .filter(Boolean);
}

/** 生产出口读数：`session/stats`（读不到按 null 处理，等待逻辑自己分辨）。 */
async function statsOf(api, sid) {
  try {
    const r = await api.invoke('session/stats', {}, { session_id: sid });
    return r.status === 200 ? (r.body?.data ?? null) : null;
  } catch {
    return null;
  }
}

/** 等一轮真实收尾：声誉列把立约与结算都记到（轮离开 working 之后才会齐）。 */
async function waitRoundSettled(api, sid, principal, what) {
  return waitFor(
    async () => {
      const rep = (await statsOf(api, sid))?.reputation?.by_principal?.[principal];
      return rep && rep.offered >= 1 && (rep.kept >= 1 || rep.broken >= 1);
    },
    { what, timeoutMs: 600_000 },
  );
}

/** 父会话 `sessions/` 子目录下的子会话 id 列表（存储归档位置，磁盘判据）。 */
function childSessionDirs(homedir, sid) {
  const base = join(homedir, 'session', sid, 'sessions');
  try {
    return readdirSync(base, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => d.name);
  } catch {
    return [];
  }
}

function repOf(stats, principal) {
  return stats?.reputation?.by_principal?.[principal] ?? null;
}

// ─────────────────────────────────────────────────────────────────────────────

const cfg = loadRealProvider();
await requireServer(cfg);

const GATEWAY_PORT = nextPort();
const hd = makeHomedir({
  providers: [{ id: cfg.id, config: cfg }],
  pluginConfigs: {
    ...dialogFaceOff(),
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
for (const a of [
  { id: 'researcher', name: '研究员', persona: PERSONA_WORKER },
  { id: 'peer-b', name: '对等体', persona: PERSONA_PEER },
]) {
  makeRealAgentDir(hd.homedir, { ...a, providerCfg: cfg });
}

let api = null;
let exitCode = 0;
try {
  api = startLongLivedCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    session: MAIN_SID,
    gatewayPort: GATEWAY_PORT,
  });
  await api.waitGatewayReady(20_000, ' real-team');

  // ── 场景 A：树形委派团队（主会话 → agent_run → researcher 子会话）────────────
  console.error('[real] 场景 A：主会话经 agent_run 委派 researcher …');
  await createSession(api, MAIN_SID, { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' });
  const sentA = await sendInbox(api, MAIN_SID, MAIN_Q);
  assertEq(sentA.status, 200, `主会话收件箱写入应受理（${JSON.stringify(sentA.body)?.slice(0, 200)}）`);
  await waitRoundSettled(api, MAIN_SID, 'session/main', '场景 A 主会话委托轮收尾');

  const statsA = await statsOf(api, MAIN_SID);
  const repA = repOf(statsA, 'session/main');
  assert(repA, `声誉列应有 session/main 条目（实际: ${JSON.stringify(statsA?.reputation)}）`);
  assertEq(repA.offered, 1, '真实委托恰好立约一次');
  assertEq(repA.kept, 1, '子会话成功 ⇒ 守约（kept）');
  assertEq(repA.broken, 0, '无违约');
  assertEq(statsA.invariants.length, 0, `不变量清单为空（实际: ${JSON.stringify(statsA.invariants)}）`);

  const mainWal = walEvents(hd.homedir, MAIN_SID);
  assert(mainWal.some((e) => e.kind === 'commitment.opened'), '事实源应有 commitment.opened');
  assert(mainWal.some((e) => e.kind === 'commitment.released'), '事实源应有 commitment.released');

  const kidsA = childSessionDirs(hd.homedir, MAIN_SID);
  assertEq(kidsA.length, 1, `恰好一个子会话归档在父会话 sessions/ 下（实际: ${JSON.stringify(kidsA)}）`);
  const childMsgs = await readSessionMessages(api, kidsA[0]);
  assert(childMsgs.length >= 2, `子会话转写应有委托消息与产出（实际 ${childMsgs.length} 条）`);
  const mainMsgs = await readSessionMessages(api, MAIN_SID);
  const mainJoined = JSON.stringify(mainMsgs);
  assert(mainJoined.includes('subagent_session_id'), '主会话工具结果应带回 [subagent_session_id]');
  console.error(`[real] 场景 A ✓（子会话 ${kidsA[0]}，声誉 kept=${repA.kept}）`);

  // ── 场景 B：对等协作轮（peer-b 窄行主体，收件箱 + 来源主体）──────────────────
  console.error('[real] 场景 B：对等主体 peer-b 经收件箱跑一轮真实委托 …');
  await createSession(api, PEER_SID, {
    agent_id: 'peer-b',
    workdir: hd.workdir,
    mode: 'auto',
    risk_level: 'medium',
  });
  const sentB = await sendInbox(api, PEER_SID, PEER_Q, { source_principal: 'session/main' });
  assertEq(sentB.status, 200, `对等收件箱写入应受理（${JSON.stringify(sentB.body)?.slice(0, 200)}）`);
  await waitRoundSettled(api, PEER_SID, 'session/peer-b', '场景 B 对等轮收尾');

  const statsB = await statsOf(api, PEER_SID);
  const repB = repOf(statsB, 'session/peer-b');
  assert(repB, `声誉列应有 session/peer-b 条目（实际: ${JSON.stringify(statsB?.reputation)}）`);
  assertEq(repB.principal, 'session/peer-b');
  assertEq(repB.offered, 1, '对等体真实委托恰好立约一次');
  assertEq(repB.kept, 1, '真实委托成功 ⇒ 守约（kept）');
  assertEq(repB.broken, 0, '无违约（对照 t71 的 mock 违约路径）');
  assertEq(repB.score, 1, '平凡打分 = 守约 − 违约');
  assertEq(statsB.invariants.length, 0, `产物收束 ⇒ 不变量清单为空（C4 认产物，实际: ${JSON.stringify(statsB.invariants)}）`);

  const peerWal = walEvents(hd.homedir, PEER_SID);
  const artifact = peerWal.find((e) => e.kind === 'artifact.added');
  assert(artifact, `对等事实源应有产物格（实际 kinds: ${JSON.stringify(peerWal.map((e) => e.kind))}）`);
  assertEq(artifact.actor, 'session/peer-b', '产物格作者 = 对等主体');
  assert(peerWal.some((e) => e.kind === 'commitment.opened'), '事实源应有 commitment.opened');
  assert(peerWal.some((e) => e.kind === 'commitment.released'), '事实源应有 commitment.released');

  const kidsB = childSessionDirs(hd.homedir, PEER_SID);
  assert(kidsB.length >= 1, `对等体委托的子会话应落盘（实际: ${JSON.stringify(kidsB)}）`);
  console.error(`[real] 场景 B ✓（声誉 kept=${repB.kept} broken=${repB.broken} score=${repB.score}）`);

  console.log('\n══ 团队模式真实环境验证（LMStudio）══');
  console.log(`  provider: ${cfg.id} → ${cfg.api_base} · 模型 ${cfg.model}`);
  console.log(`  场景 A 树形委派: ✓ 主会话立约/守约，子会话 ${kidsA[0]} 落盘`);
  console.log(`  场景 B 对等协作: ✓ 对等主体产物收束，声誉 offered=1 kept=1 broken=0`);
  console.log('  判定出口: 退出码 / session/stats（声誉列 + invariants）/ 事实源 WAL / 转写');
} catch (e) {
  exitCode = 1;
  console.error(`\n✖ 失败: ${e.message}`);
  console.error(`  （现场保留: ${hd.homedir} —— WAL 与转写都在里面，可事后取证）`);
} finally {
  if (api) api.stop();
  if (hd && exitCode === 0) cleanupHomedir(hd);
  try {
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '长驻 CLI 子进程退出',
      timeoutMs: 15_000,
    });
  } catch {
    /* 收尾等待超时不改变测试结论 */
  }
}
process.exit(exitCode);
