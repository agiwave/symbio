import './_selfrun.mjs';
// T28 读数口 `session/stats`：跑一轮成功 + 一轮兜底，从**生产出口**读各列；
// 另有**实例级聚合**（`fleet`）一列，一并对账。
//
// ## 本用例钉的是什么
//
// 读数口的出口判据：
// 从出口读到的每一列必须**真的在算**——按**已知动作**（跑两轮、成一败一）钉死期望值，
// 反向再用读侧闸验「读数随读方身份收放」。校准列另加**一注**（没编过技能 ⇒ 有据的空）。
//
// 为什么必须端到端：逐列口径在 Rust 单测里已钉（`stats.test.rs`），但它证不了
// 三件**只有这条链路能看见**的事：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 出口真的挂上了生产路由 | gateway `POST /api/v1/invoke` → 容器 → `session/stats` |
// | 读的是**这一轮真实写下**的事实 | 真实 CLI 轮次 + 真实 LLM 流量产生的事实源 |
// | `payload.principal` 真的进了读侧判定 | 线路层（`PluginMessageWire`）→ `ctx.payload` → 可见域闸 |
//
// ## 对账口径
//
// 本用例只做**取值与计数**（数轮次、求兜底率、取分位数下标），不复刻任何统计口径——
// 口径属于投影，出口与用例都只准调它。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t28';

export default defineCase(
  'T28 读数口 session/stats：出口各列按已知动作钉死期望值，读侧闸随读方收放',
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
        // 每轮收束把事实写进事实源——读数口的事实源由真实轮次产生。
        session: { ...DIALOG_FACE_OFF },
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

        // ── 证据 ①′：不变量列在**真实生产流量**下必须全绿 ────────────────
        // 这条断言同时钉两件事：wiring 真的接上了（不是恒空的摆设），以及首日不假红
        // （在途轮 / 档位预算的宽限口径生效）。
        assertEq(
          stats.invariants.length,
          0,
          `两轮都收束、档位已声明且预算内 ⇒ 不变量清单为空（实际: ${JSON.stringify(stats.invariants)}）`,
        );

        // ── 证据 ②：各列读数 = **已知动作**的期望值（不是常数）──────────────
        // 两轮分别成功 / 兜底，因此轮次、样本、比率全部可钉死；口径属于投影，
        // 用例只做取值与计数（会随派生事实浮动的绝对数只判「非零」，不钉死）。
        assert(
          stats.checkpoint.event_count > 0,
          `断点列应读到这一轮真实写下的事实（实际: ${stats.checkpoint.event_count}）`,
        );
        assertEq(stats.tiers.length, 1, `两轮同档 ⇒ 一档（实际: ${JSON.stringify(stats.tiers)})`);
        const row = stats.tiers[0];
        assertEq(row.tier, 'deep', '档位恒 deep');

        assertEq(row.turns, 2, '两轮各开一轮');
        assertEq(row.fallbacks, 1, '恰好一轮兜底');
        assertEq(row.samples, 1, '恰好一轮成功收束');
        assertEq(row.rate, 0.5, '兜底率 1/2');
        assert(
          Number(row.p95) > 0,
          `成功轮有真实耗时 ⇒ 分位数为正（实际: ${row.p95}）`,
        );
        assert(
          Number(stats.cost.total_ms) > 0,
          `成本台账应记下真实耗时（实际: ${stats.cost.total_ms}）`,
        );
        // 校准列：本会话没编过技能
        // （`skill_compile_enabled` 默认 off）⇒ **有据的空**，不是缺列。
        assertEq(
          stats.calibration.by_skill,
          {},
          `没编过技能 ⇒ 校准列为空（实际: ${JSON.stringify(stats.calibration)}）`,
        );

        // ── 证据 ②″：实例级聚合列（P2，2026-10-09）──────────────────
        // `fleet` = 本实例下**所有**会话的时延 / 兜底合并。本用例只跑了一个会话，
        // 于是聚合列必须与单会话列**逐字同数**（口径同一套，差别只肯能在范围上：
        // 一个会话的「全量」就是它自己）。这条钉的正是「范围」：若扫描进了 archive
        // 或别人的目录，两个数立刻分家。
        assertEq(
          stats.fleet.sessions,
          1,
          `本实例只有一个会话（实际: ${JSON.stringify(stats.fleet)}）`,
        );
        assertEq(stats.fleet.wals, 1, '读进来一个事实源');
        assertEq(
          stats.fleet.turns,
          row.turns,
          '聚合列的分母 = 单会话列的分母（全量 = 一个会话时逐字相同）',
        );
        assertEq(stats.fleet.fallbacks, row.fallbacks, '分子同理');
        assertEq(
          stats.fleet.by_tier.latency[0].samples,
          row.samples,
          '样本数同理（并集不是覆盖：一个会话时恰好等于自己）',
        );

        // ── 证据 ②′：读侧可见域（[DE-001 §7](../../docs/003-design/001-核心架构.md "DE-001-核心架构")）──
        // 载荷声明**读方身份**才判可见域（`thread_private` 缺省，C10）；不声明 =
        // 本机默认，与上面 ①/② 的读数逐字相同。三条并排要证的是**同一条事实源、
        // 三种读法**——差异只来自闸，不来自别的。
        const asOwner = (
          await api.invoke('session/stats', { principal: 'person/local' }, { session_id: SID })
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

        // 矩阵内主体（`session/main`）⇒ **读得到**本会话的共享内容（②：
        // 内容侧可见域产地——两轮的用户发言都编成了 `vis_scope = shared` 的经验
        // 记忆，读侧闸按内容侧最宽可见域判，不再恒传缺省值）。读数与属主逐字
        // 相同：闸是整列判定，放行即放行整个快照。负面对照（无共享内容的会话
        // ⇒ 同一读方照旧空切片）在 t73。
        const asMain = (
          await api.invoke('session/stats', { principal: 'session/main' }, { session_id: SID })
        ).body.data;
        assertEq(asMain.has_wal, true, 'session/main: 事实源存在，只是今天给你看');
        assertEq(
          asMain.checkpoint.event_count,
          stats.checkpoint.event_count,
          'session/main: 矩阵成员读得到他人共享内容 ⇒ 与属主读到同样多的事实',
        );
        assertEq(
          asMain.cost.total_ms,
          stats.cost.total_ms,
          'session/main: 成本列逐字相同',
        );
        assertEq(asMain.tiers, stats.tiers, 'session/main: 档位行逐字相同');

        // 矩阵外的未知主体 ⇒ 读数全空（fail-closed，不因 放宽）。
        // `has_wal` 仍为真，于是「有源但不给你看」与「没有源」可分辨——不是被读成
        // 「这一格没有数」。
        for (const outsider of ['agent:ghost']) {
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
