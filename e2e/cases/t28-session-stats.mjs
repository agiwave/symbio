import './_selfrun.mjs';
// T28 读数口 `session/stats`：跑一轮成功 + 一轮兜底，从**生产出口**读四列
// 与一份不变量清单，与同一份 `v2-events.wal` 逐列对账；再对事实源做手术，
// 验证列和清单真的会动。
//
// ## 编号为什么是 28
//
// `t27` 已被 [plan/10 批 2b](../../docs/plan/10-工具轮v2化实施方案.md) 预留给
// 「审批与恢复」e2e（要先补 CLI 的恢复入口才写得出），本用例顺延占 `t28`。
//
// ## 本用例钉的是什么
//
// [12 批 0 / 批 1](../../docs/plan/12-价值验收与基线埋点.md) 的出口判据（原文见该处）：
// 从出口读到的每一列必须与**读方复算**逐字相等；**反向用例三刀**——
// 删收束格 / 删开轮格 / **复制**开轮格 ⇒ 对应的列与不变量清单必须跟着变；
// 校准列另加**一注**（注入一条路由观测 ⇒ 该列认它）。三刀一注合起来证明
// 这些列真的在算，不是常数。
//
// 为什么必须端到端：复算的逐字相等在 Rust 单测里已钉（`stats.test.rs`），
// 但它证不了三件**只有这条链路能看见**的事：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 出口真的挂上了生产路由 | gateway `POST /api/v1/invoke` → 容器 → `session/stats` |
// | 读的是**这一轮真实写下**的事实 | 磁盘上的 `v2-events.wal`（由真实 LLM 流量产生） |
// | 读方真的每次重读文件 | 对文件动手术后**再调一次**，读数跟着变 |
// | `payload.principal` 真的进了读侧判定 | 线路层（`PluginMessageWire`）→ `ctx.payload` → 可见域闸 |
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
        session: { ...DIALOG_FACE_OFF, v2_mode: 'full' },
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

        // ── 证据 ①′：不变量列在**真实生产流量**下必须全绿（04 §3.1 批④）────
        // 这条断言同时钉两件事：wiring 真的接上了（不是恒空的摆设——下一刀会
        // 把它变红），以及首日不假红（在途轮 / 档位预算的宽限口径生效）。
        assertEq(
          stats.invariants.length,
          0,
          `两轮都收束、档位已声明且预算内 ⇒ 不变量清单为空（实际: ${JSON.stringify(stats.invariants)}）`,
        );

        // ── 证据 ②：逐列与**同一时刻的文件**对账（JS 只计数，不复刻统计口径）──
        const events = readEvents(walPath);
        const finals = events.filter((e) => e.kind === 'chat.assistant.final');
        const fallbacks = events.filter((e) => e.kind === 'chat.assistant.fallback');
        const opens = events.filter((e) => e.kind === 'user.message');
        const totalCost = events.reduce((n, e) => n + (e.cost_ms ?? 0), 0);
        // 手术靶子（轮 0 的开轮事件）的 event_id——稍后验不变量**锚在这条上**。
        const turn0OpenId = opens.find((e) => e.turn === 0)?.event_id;
        assert(turn0OpenId, `事实源里应有轮 0 的开轮事件（实际: ${JSON.stringify(opens)}）`);

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
        // 校准列（[12 批 1](../../docs/plan/12-价值验收与基线埋点.md)）：本会话没编过技能
        // （`skill_compile_enabled` 默认 off）⇒ **有据的空**，不是缺列。下一刀注入一条
        // 观测把它变非空——两刀合起来证明这一列真的在读文件，不是恒空也不是恒有。
        assertEq(
          stats.calibration.by_skill,
          {},
          `没编过技能 ⇒ 校准列为空（实际: ${JSON.stringify(stats.calibration)}）`,
        );

        // 一轮成功 + 一轮失败 ⇒ 期望值本身也钉死（防止两边一起算错）。
        assertEq(row.turns, 2, '两轮各开一格');
        assertEq(row.fallbacks, 1, '恰好一轮兜底');
        assertEq(row.samples, 1, '恰好一轮成功');
        assertEq(row.rate, 0.5, '兜底率 1/2');

        // ── 证据 ②′：读侧可见域（[04 §3.1 批⑥](../../docs/plan/04-工程落地.md)）──
        // 载荷声明**读方身份**才判可见域（`thread_private` 缺省，C10）；不声明 =
        // 本机默认，与上面 ①/② 的读数逐字相同。三条并排要证的是**同一条事实源、
        // 三种读法**——差异只来自闸，不来自文件（下面 ③ 才动文件）。
        const asOwner = (
          await api.invoke('session/stats', { principal: 'user' }, { session_id: SID })
        ).body.data;
        assertEq(
          asOwner.checkpoint.event_count,
          stats.checkpoint.event_count,
          '读方 = 属主 ⇒ 与本机默认读到同样多的事实',
        );
        assertEq(
          asOwner.cost.total_ms,
          stats.cost.total_ms,
          '读方 = 属主 ⇒ 成本列逐字相同（闸不是为了把属主挡在外面）',
        );
        assertEq(
          asOwner.tiers,
          stats.tiers,
          '读方 = 属主 ⇒ 档位行逐字相同',
        );

        // 矩阵内但**不是属主**的主体、以及矩阵外的未知主体 ⇒ 读数全空（fail-closed）。
        // `has_wal` 仍为真，于是「有源但不给你看」与「没有源」可分辨——不是被读成
        // 「这一格没有数」。
        for (const outsider of ['agent:main', 'agent:ghost']) {
          const denied = (
            await api.invoke('session/stats', { principal: outsider }, { session_id: SID })
          ).body.data;
          assertEq(denied.has_wal, true, `${outsider}: 事实源存在，只是不给你看`);
          assertEq(
            denied.checkpoint.event_count,
            0,
            `${outsider}: 断点列为空（越界读取 0）`,
          );
          assertEq(denied.cost.total_ms, 0, `${outsider}: 成本列为 0`);
          assertEq(denied.tiers.length, 0, `${outsider}: 四列为空 ⇒ 无档位行`);
          assertEq(
            denied.invariants.length,
            0,
            `${outsider}: 没有可见事实 ⇒ 无可报的违规`,
          );
        }

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

        // 复制刀：把命中的行**再写一遍**——注入「同一个 `u-{turn}` 落两回」。
        const duplicateMatching = (pred) => {
          const lines = readFileSyncSafe(walPath).split('\n').filter(Boolean);
          const out = [];
          let dup = 0;
          for (const line of lines) {
            out.push(line);
            if (pred(JSON.parse(line))) {
              out.push(line);
              dup += 1;
            }
          }
          assert(dup > 0, '复制刀必须真的复制到行，否则测不出变化');
          writeFileSync(walPath, `${out.join('\n')}\n`, 'utf8');
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
        // 断点列按**事实条数**计——绝对数会随记忆三段（[04 §3.1 批⑦](../../docs/plan/04-工程落地.md)
        // 往同一份事实源追加 `memory.*`）一起涨，故按「删几条少几条」判，不钉死数字。
        assertEq(
          afterDropFinal.checkpoint.event_count,
          events.length - 1,
          `删 1 条 ⇒ 断点 1（出口 ${afterDropFinal.checkpoint.event_count} vs 事实 ${events.length}）`,
        );
        // 同一刀必须也砍在不变量列上：轮 0 被轮 1 越过 ⇒ C4 报「未收束」，
        // 中间那行没了 ⇒ C1 报 seq 跳号。两刀一清单，证明这一列不是常数。
        assertEq(
          afterDropFinal.invariants.length,
          2,
          `手术后不变量必须红（实际: ${JSON.stringify(afterDropFinal.invariants)}）`,
        );
        assert(
          afterDropFinal.invariants.some((v) => v.why.includes('未收束')),
          `C4：删掉收束格 ⇒ 该轮被判未收束（实际: ${JSON.stringify(afterDropFinal.invariants)}）`,
        );
        assert(
          afterDropFinal.invariants.some((v) => v.event_id === turn0OpenId),
          `违规锚在开轮那条（缺口本身，id=${turn0OpenId}）（实际: ${JSON.stringify(afterDropFinal.invariants)}）`,
        );

        // 3b. 再删掉**第一轮**的开轮格 ⇒ 兜底率的**分母**必须变（2 → 1）
        dropMatching((e) => e.kind === 'user.message' && e.turn === 0);
        const afterDropOpen = (await api.invoke('session/stats', {}, { session_id: SID })).body.data;
        assertEq(afterDropOpen.tiers[0].turns, 1, '删掉一个开轮格 ⇒ 分母 2 → 1');
        assertEq(afterDropOpen.tiers[0].fallbacks, 1, '兜底格未动 ⇒ 分子不变');
        assertEq(afterDropOpen.tiers[0].rate, 1, '分母变了 ⇒ 比率必须跟着变（1/1）');
        assertEq(
          afterDropOpen.checkpoint.event_count,
          events.length - 2,
          '两刀之后 = 事实数 − 2（断点列真的是计数）',
        );
        // 开轮格也没了 ⇒ 没有可判的缺口：C4 必须跟着回落（否则它锚的是别的东西）。
        assert(
          !afterDropOpen.invariants.some((v) => v.why.includes('未收束')),
          `开轮格被删 ⇒ C4 无从判定（实际: ${JSON.stringify(afterDropOpen.invariants)}）`,
        );

        // 3c. 把**剩下那一格开轮事件复制成两条** ⇒ 同一个 `u-{turn}` 落两回。
        //     这一刀钉的是 [11 批 0-B](../../docs/plan/11-多执行器与多主体加固实施方案.md)
        //     的单写者观测面：写者令牌（0-A）在写入侧堵并发，本不变量在**事实源上**
        //     把「绕过唯一写入口」的痕迹报出来——令牌漏了，这一条必须红。
        duplicateMatching((e) => e.kind === 'user.message');
        const afterDupOpen = (await api.invoke('session/stats', {}, { session_id: SID })).body.data;
        assertEq(
          afterDupOpen.tiers[0].turns,
          2,
          '重复开轮 ⇒ 同一 turn 被数成两轮（兜底率分母虚增，这正是它的危害）',
        );
        assert(
          afterDupOpen.invariants.some((v) => v.why.includes('开轮')),
          `复制开轮格 ⇒「每 turn 至多 1 条开轮」必须红（实际: ${JSON.stringify(afterDupOpen.invariants)}）`,
        );

        // 3d. 往事实源**注入**一条路由观测（`memory.recalled{skill_id, fallback}`）⇒
        //     校准列必须认它。本会话跑不出真观测（技能编译默认关），所以用**构造**的
        //     那一条证「这一列真的在读文件」——读数与读方复算逐字相等（[12 批 1]）。
        const injectObservation = (skillId, fallback) => {
          const lines = readFileSyncSafe(walPath).split('\n').filter(Boolean);
          const parsed = lines.map((l) => JSON.parse(l));
          // 行形状**照抄一条真实事件**，只换定位与载荷：信封的 `entity` / `verb` 等
          // 枚举拼写不许手写——写错一个字母，`replay` 会把整行当撕裂尾行截断，
          // 测出来的是解析器不是校准。校准只读载荷的两把钥匙，别的一律不动。
          const ev = {
            ...parsed[0],
            event_id: `obs-${skillId}`,
            kind: 'memory.recalled',
            seq: Math.max(...parsed.map((e) => e.seq ?? -1)) + 1,
            payload: { skill_id: skillId, fallback },
          };
          writeFileSync(walPath, `${lines.join('\n')}\n${JSON.stringify(ev)}\n`, 'utf8');
        };

        injectObservation('sk-e2e', true);
        const afterObs = (await api.invoke('session/stats', {}, { session_id: SID })).body.data;
        // ⚠ 不要拿**手写对象字面量**直接比服务端对象：`serde_json::Value` 底层是
        // BTreeMap ⇒ 服务端 JSON 的键序是**字典序**，字面量的键序是书写序，
        // `JSON.stringify` 一比就假红。逐字段比（下面的复算同款）。
        const one = afterObs.calibration.by_skill['sk-e2e'];
        assert(one, `校准列应认下注入的路由观测（实际: ${JSON.stringify(afterObs.calibration)}）`);
        assertEq(one.uses, 1, '注入的那条算一次使用');
        assertEq(one.fallbacks, 1, '带 fallback 标记 ⇒ 回退一次');
        assertEq(one.skill_id, 'sk-e2e', '条目自带 skill_id（投影的口径，不是用例补的）');

        // 复算：列 == 读方按**同一把钥匙**归并事实源的结果（用例只计数、不复刻统计
        // 口径——口径属于 `calibration` 投影，出口与用例都只准调它）。
        const recomputed = new Map();
        for (const e of readEvents(walPath)) {
          const skillId = e.payload?.skill_id;
          if (!skillId || typeof e.payload.fallback !== 'boolean') continue;
          const s = recomputed.get(skillId) ?? { uses: 0, fallbacks: 0 };
          s.uses += 1;
          if (e.payload.fallback) s.fallbacks += 1;
          recomputed.set(skillId, s);
        }
        assertEq(
          Object.keys(afterObs.calibration.by_skill).sort(),
          [...recomputed.keys()].sort(),
          '校准列的技能集 == 读方复算（同一把钥匙）',
        );
        for (const [skillId, s] of recomputed) {
          assertEq(
            afterObs.calibration.by_skill[skillId].uses,
            s.uses,
            `${skillId}: uses == 复算`,
          );
          assertEq(
            afterObs.calibration.by_skill[skillId].fallbacks,
            s.fallbacks,
            `${skillId}: fallbacks == 复算`,
          );
        }
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
