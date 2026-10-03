import './_selfrun.mjs';
// T28 读数口 `session/stats`：跑一轮成功 + 一轮兜底，从**生产出口**读四列，
// 与同一份 `v2-events.wal` 逐列对账；再对事实源做手术，验证列真的会动。
//
// ## 编号为什么是 28
//
// `t27` 已被 [plan/10 批 2b](../../docs/plan/10-工具轮v2化实施方案.md) 预留给
// 「审批与恢复」e2e（要先补 CLI 的恢复入口才写得出），本用例顺延占 `t28`。
//
// ## 本用例钉的是什么
//
// [12 批 0](../../docs/plan/12-价值验收与基线埋点.md) 的出口判据原文：
// 「跑一轮（含一次兜底）后，从出口读到的 P95 / 兜底率与**读方复算**逐字相等。
// **反向用例**：删掉一次收束事件 ⇒ 对应那一列必须变化（证明它真的在算，不是常数）」。
//
// 为什么必须端到端：复算的逐字相等在 Rust 单测里已钉（`stats.test.rs`），
// 但它证不了三件**只有这条链路能看见**的事：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 出口真的挂上了生产路由 | gateway `POST /api/v1/invoke` → 容器 → `session/stats` |
// | 读的是**这一轮真实写下**的事实 | 磁盘上的 `v2-events.wal`（由真实 LLM 流量产生） |
// | 读方真的每次重读文件 | 对文件动手术后**再调一次**，读数跟着变 |
//
// ## 对账口径
//
// 本用例只做**计数与取值**（数 `user.message` / `final` / `fallback` 的条数、
// 求 `cost_ms` 之和、取分位数下标），不复刻任何统计口径——口径属于投影，
// 出口与用例都只准调它。
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t28';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 读文件里的全部事件（出口读数的对账基准）。 */
function readEvents(walPath) {
  const raw = readFileSyncSafe(walPath);
  assert(raw.length > 0, `v2 事实源应存在且非空：${walPath}`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** 最近邻取法的 P95（与 `TierLatency::percentile` 同一取法：`ceil(p/100 × n) - 1`）。 */
function p95(samples) {
  const sorted = [...samples].sort((a, b) => a - b);
  if (!sorted.length) return 0;
  return sorted[Math.min(Math.ceil((95 * sorted.length) / 100), sorted.length) - 1];
}

export default defineCase(
  'T28 读数口 session/stats：出口读数与事实源逐列对账，且随事实源变动',
  async () => {
    const llm = await new MockLlm([
      // 成功轮：正常收束（final ⇒ 进时延样本）
      { id: 'ok', match: '普通提问', content: '这是正常回答。' },
      // 失败轮：HTTP 500 打穿重试 ⇒ turn 失败 ⇒ 兜底格（I3：失败也是一句话）
      { id: 'boom', match: '触发故障', status: 500, error: 'mock 注入的服务端错误' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 无此文件时网关回退默认配置（`inbound_enabled: false`）——监听不会被拉起，
        // 出口就打不到（与 T7 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // `bridge` 档：每轮收束转写进 `<会话目录>/v2-events.wal`——读数口的事实源。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'bridge' },
      },
    });

    try {
      // ── 跑一轮成功 + 一轮失败（两轮共用一个会话 ⇒ 同一个事实源）───────────
      const ok = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '普通提问',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(ok.code, 0, `成功轮 CLI 退出码（stderr: ${ok.stderr.slice(0, 400)}）`);

      const boom = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '触发故障',
        provider: PROVIDER_ID,
        session: SID,
        timeoutMs: 60_000,
      });
      assert(boom.code !== 0, '失败轮 CLI 应以非零退出码结束（与 T5 同口径）');

      const walPath = join(hd.homedir, 'session', SID, V2_WAL);

      // ── 证据 ①：从**生产路由**读出口（gateway HTTP 边界，与前端同构）────────
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
        assertEq(stats.session_id, SID, '出口应回带会话 id');
        assert(stats.has_wal, '跑过两轮 ⇒ 事实源应在');

        // ── 证据 ②：逐列与**同一时刻的文件**对账（JS 只计数，不复刻统计口径）──
        const events = readEvents(walPath);
        const finals = events.filter((e) => e.kind === 'chat.assistant.final');
        const fallbacks = events.filter((e) => e.kind === 'chat.assistant.fallback');
        const opens = events.filter((e) => e.kind === 'user.message');
        const totalCost = events.reduce((n, e) => n + (e.cost_ms ?? 0), 0);

        assertEq(
          stats.checkpoint.event_count,
          events.length,
          `断点列 = 事实条数（出口 ${stats.checkpoint.event_count} vs 文件 ${events.length}）`,
        );
        assertEq(stats.tiers.length, 1, `两轮同档 ⇒ 一档（实际: ${JSON.stringify(stats.tiers)})`);
        const row = stats.tiers[0];
        assertEq(row.tier, 'deep', 'v2 桥的档位恒 deep');

        assertEq(row.turns, opens.length, '兜底率分母 = 开轮事件数');
        assertEq(row.fallbacks, fallbacks.length, '兜底率分子 = 兜底事件数');
        assertEq(row.samples, finals.length, '时延样本 = 成功收束事件数（兜底不计入）');
        assertEq(
          row.rate,
          fallbacks.length / opens.length,
          '兜底率必须由两个计数相除得出（不是常数）',
        );
        assertEq(
          row.p95,
          p95(finals.map((e) => e.cost_ms)),
          'P95 必须等于文件里成功轮 cost_ms 的最近邻取法值',
        );
        assertEq(stats.cost.total_ms, totalCost, '成本台账 = 文件里 cost_ms 之和');

        // 一轮成功 + 一轮失败 ⇒ 期望值本身也钉死（防止两边一起算错）。
        assertEq(row.turns, 2, '两轮各开一格');
        assertEq(row.fallbacks, 1, '恰好一轮兜底');
        assertEq(row.samples, 1, '恰好一轮成功');
        assertEq(row.rate, 0.5, '兜底率 1/2');

        // ── 证据 ③：对事实源动手术 ⇒ 出口读数必须跟着变（不是常数）──────────
        // 每次手术都重读文件（出口也是每次重读），删掉命中的行。
        const dropMatching = (pred) => {
          const lines = readFileSyncSafe(walPath).split('\n').filter(Boolean);
          const kept = [];
          let dropped = 0;
          for (const line of lines) {
            if (pred(JSON.parse(line))) {
              dropped += 1;
              continue;
            }
            kept.push(line);
          }
          assert(dropped > 0, '手术刀必须真的删掉行，否则测不出变化');
          writeFileSync(walPath, `${kept.join('\n')}\n`, 'utf8');
        };

        // 3a. 删掉唯一的收束格 ⇒ 时延样本列归零
        dropMatching((e) => e.kind === 'chat.assistant.final');
        const afterDropFinal = (await api.invoke('session/stats', {}, { session_id: SID })).body.data;
        assertEq(
          afterDropFinal.tiers[0].samples,
          0,
          '删掉收束格 ⇒ 样本 1 → 0（出口真的在读文件）',
        );
        assertEq(afterDropFinal.tiers[0].p95, 0, '无样本 ⇒ 分位数 0');
        assertEq(afterDropFinal.tiers[0].turns, 2, '开轮格未动 ⇒ 分母不变');
        assertEq(afterDropFinal.checkpoint.event_count, 3, '断点是事实计数');

        // 3b. 再删掉**第一轮**的开轮格 ⇒ 兜底率的**分母**必须变（2 → 1）
        dropMatching((e) => e.kind === 'user.message' && e.turn === 0);
        const afterDropOpen = (await api.invoke('session/stats', {}, { session_id: SID })).body.data;
        assertEq(afterDropOpen.tiers[0].turns, 1, '删掉一个开轮格 ⇒ 分母 2 → 1');
        assertEq(afterDropOpen.tiers[0].fallbacks, 1, '兜底格未动 ⇒ 分子不变');
        assertEq(afterDropOpen.tiers[0].rate, 1, '分母变了 ⇒ 比率必须跟着变（1/1）');
        assertEq(afterDropOpen.checkpoint.event_count, 2);
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净：Windows 上带着活子进程 `process.exit()` 会撞
      // libuv 的 `uv_async_send` 断言（`!(handle->flags & UV_HANDLE_CLOSING)`），
      // 表现是**断言全过、退出码却是 0xC0000409** —— 判据过了却判失败。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
