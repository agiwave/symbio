import './_selfrun.mjs';
// T36 自主层接线（`AutonomousInitiator` + `IntentGate`，S9 第 21 步，S12）：
// 心跳调度器到点 ⇒ 触发事实落格 ⇒ 「欲」入格 ⇒ 闸门判 ⇒ 过了才开出长任务。
//
// ## 本用例钉的是什么
//
// [11 批 2 ③](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的接线判据：
// 「每件接线一条 e2e（**接线前后行为可见地不同**，否则等于没接）」，且该处明写
// **`IntentGate` 必须与 `AutonomousInitiator` 同批成对**——无门即写 = 自主层直写
// 对话/任务，破「自主写入对话 0」。所以本用例钉的是**这一对**：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 自主层**真的被定时器驱动**（不是只在测试里被调用） | 真进程里跑够一个 tick，事实源长出 `system.triggered` |
// | 三个 initiator 事实**成链**（触发 → 自检 → 欲） | 事实源里 `system.triggered` / `system.health` / `conation.expressed` 三条 + 溯源 |
// | 闸门**真的在链上**（不是摆设） | 同一条链路，闸门放行 ⇒ `task.opened`；闸门拒 ⇒ **没有** `task.opened` |
//
// ## 为什么必须端到端（单测证不了的三件事）
//
// 单测（`heartbeat/tests.rs`）直接调 `record_autonomous_trigger`，把三种「不触发」
// 与四个事实的形状钉死了，但它证不了：
//
// 1. **定时器真的接上了**：`run_heartbeat_loop` 是常驻后台循环（15s tick），
//    单测里没有 ticker、没有 `list_sessions`、没有 `updated_at` 空闲基线——
//    「到点真的会触发」只有真进程 + 真等一个 tick 才看得见；
// 2. **闸门真的在链上**：`IntentGate::approve` 的调用点在
//    `record_autonomous_trigger` 里，紧跟在「欲入格」之后——单测可以自己拿令牌
//    调 `approve`，但证不了**生产路径**有没有绕过它。本用例用「同一条链路、
//    只换闸门的判决」来钉：两个会话走完全相同的代码，只有提示词长度不同，
//    一个开出长任务、一个开不出。
// 3. **长任务的溯源真的指向被批准的欲**：`task.opened` 的 `produced_by` 必须是
//    `conation.expressed` 的 seq（`approved.from_seq`），而不是触发本身。
//
// ## 闸门的两个判决怎么在一条用例里都跑到
//
// `IntentGate::evaluate` 的三条（S12 §4）：
// - `!policy.enabled` ⇒ 拒（**关停开关**，`conation_enabled` 默认 false）；
// - `goal.len() > max_goal_len`（默认 **200 字节**）⇒ 拒「目标过宽」；
// - 否则 ⇒ 放行。
//
// 本用例把 `conation_enabled` 打开（走第三条路，验「能放行」），再用**两个会话**
// 分别喂一条短提示词与一条 >200 字节的提示词：前者放行、后者被「目标过宽」拒。
// 两个会话的心跳都到点（`HEARTBEAT_MAX_PER_TICK = 2`，一趟扫描正好各触发一次），
// 于是**同一个 tick、同一条链路**同时给出闸门的两侧判决——「行为可见地不同」的
// 唯一变量就是闸门，不是别的。
//
// ## 为什么把 `updated_at` 往回拨
//
// 调度器每 15s 扫一次，空闲基线取 `max(内存锚点, 磁盘侧 updated_at)`（新进程内存
// 锚点为空 ⇒ 就是 `updated_at`）。要触发得等会话空闲满 `interval + 抖动(0–30s)`——
// 空等墙上时钟会把这个用例拖到 45s。回拨 `updated_at` 是把这个**前置条件**
// （「这个会话已经空闲 3 分钟了」）直接摆好：被绕过的只是等待，调度器、触发链、
// 闸门一个都没少。
//
// 两处配套：`interval_seconds` 取 60（> 一个 tick 的 15s），使一趟触发之后**不会**
// 在同一个观测窗内再触发一次——否则「恰好一次触发」的判据会随 tick 数漂移；
// 长驻 CLI 用**中立会话 id**，因为 REPL 启动时会把自己那个会话落一次盘
// （`updated_at = now`），拿被测会话当 REPL 会话就会把回拨的空闲基线抹掉。
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  startLongLivedCli,
  waitFor,
  readFileSyncSafe,
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 两个会话：同一份代码、同一条链路，只有心跳提示词长度不同 ⇒ 闸门判决不同。 */
const SID_OK = 'e2e-t36-ok';
const SID_BROAD = 'e2e-t36-broad';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 第 1 轮的用户发言：只为把会话与事实源建出来（心跳的空闲基线要靠它之后的 updated_at）。 */
const MSG = '开始工作';

/** 闸门**放行**的提示词：短（远低于 200 字节上界）⇒ 应开出长任务。 */
const PROMPT_OK = '推进待办';

/**
 * 闸门**拒收**的提示词：> 200 字节 ⇒ `goal too broad`。
 *
 * 上界是**字节**不是字符（`ConationCandidate.goal.len()` = `String::len()`），
 * 中文一个字 3 字节 ⇒ 约 66 字之后就被拒（`config.rs::conation_enabled` 的注记）。
 */
const PROMPT_BROAD =
  '把当前所有尚未完成的工作项、它们各自的上下文、可能影响这些工作项推进的外部因素、' +
  '以及此前每一次尝试留下的结论都仔细检查一遍，然后持续不断地推进直到全部完成';

/** 调度器扫描周期 = 15s（`heartbeat/mod.rs::HEARTBEAT_TICK_SECS`）——注释用，不参与判定。 */

/** 读一个会话的全部事实。 */
function readEvents(hd, sid) {
  const raw = readFileSyncSafe(join(hd.homedir, 'session', sid, V2_WAL));
  assert(raw.length > 0, `${sid} 的事实源应存在且非空`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** 某会话某类事实。 */
function eventsOf(hd, sid, kind) {
  return readEvents(hd, sid).filter((e) => e.kind === kind);
}

/**
 * 给会话装上心跳（直接写 `session.json` 的 `metadata.heartbeat`，与
 * `HeartbeatConfig::from_metadata` 的读侧同构），并把 `updated_at` 回拨 3 分钟。
 *
 * `interval_seconds` 取 60（> 一个 tick 的 15s）：一趟触发之后不会在同一观测窗内
 * 再触发一次，「恰好一次触发」的判据因此不随 tick 数漂移。
 */
function armHeartbeat(hd, sid, prompt) {
  const p = join(hd.homedir, 'session', sid, 'session.json');
  const raw = readFileSyncSafe(p);
  assert(raw.length > 0, `${sid} 的 session.json 应已存在（先跑一轮把会话建出来）`);
  const s = JSON.parse(raw);
  assertEq(s.id, sid, '会话 id 原样落盘');
  s.metadata = {
    ...(s.metadata ?? {}),
    heartbeat: { enabled: true, interval_seconds: 60, prompt, include_history: true },
  };
  s.updated_at = Date.now() - 180_000;
  writeFileSync(p, JSON.stringify(s), 'utf8');
}

export default defineCase(
  'T36 自主层接线：心跳触发成链，闸门放行开长任务 / 拒收则不开',
  async () => {
    // 上界是字节：先把这个前置条件本身钉住（否则「拒收」可能只是提示词太短没触发）。
    assert(
      Buffer.byteLength(PROMPT_BROAD, 'utf8') > 200,
      `拒收侧提示词必须超过闸门的 200 字节上界（实际 ${Buffer.byteLength(PROMPT_BROAD, 'utf8')} 字节）`,
    );
    assert(
      Buffer.byteLength(PROMPT_OK, 'utf8') <= 200,
      '放行侧提示词必须在上界之内',
    );

    const llm = await new MockLlm([
      { id: 'turn1', match: MSG, content: '好的，开始。' },
      { id: 'rest', content: '收到。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // `conation_enabled` **显式打开**：它默认 false（S12 §4 平凡值「关掉后系统
        // 退化为纯响应式」），本用例验的是打开之后闸门**能放行**的那条路。
        // 拒收那条路不靠这个开关，靠提示词长度——两个判决因此可以并存。
        //
        // 刻意**不配 gateway**：本用例只关心事实源，不需要入站口；而 gateway 配置是
        // homedir 级的——配了它，下面两个一次性 CLI 会跟长驻 CLI 抢同一个端口，
        // 「网关就绪」于是时对时错（长驻进程没抢到端口时，轮询打到的是别人的端口）。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'bridge', conation_enabled: true },
      },
    });

    try {
      // ── 前置：两个会话各跑一轮，把会话与事实源建出来 ──────────────────────
      for (const sid of [SID_OK, SID_BROAD]) {
        const r = runCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          message: MSG,
          provider: PROVIDER_ID,
          session: sid,
        });
        assertEq(r.code, 0, `${sid} 建会话轮退出码（stderr: ${r.stderr.slice(0, 400)}）`);
        assert(
          !eventsOf(hd, sid, 'system.triggered').length,
          `${sid}: 还没装心跳，不该有任何触发事实（触发是**到点判出来的**，不是声明出来的）`,
        );
      }

      // ── 装心跳（短提示词 ⇒ 放行；长提示词 ⇒ 拒收）────────────────────────
      armHeartbeat(hd, SID_OK, PROMPT_OK);
      armHeartbeat(hd, SID_BROAD, PROMPT_BROAD);

      // ── 起长驻进程：调度器随插件树构建启动，第一个 tick 在 15s 后 ──────────
      // 用**中立会话 id**（不是被测的两个）：REPL 会在启动时把自己的会话落一次盘
      // （`updated_at = now`），若直接拿被测会话当 REPL 会话，回拨的空闲基线会被
      // 这次启动写盘抹掉——SID_OK 于是比 SID_BROAD 晚触发，等「两个都触发」就得多等
      // 一整个 tick，还把「恰好一次触发」的判据变成「三次」。
      const cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'e2e-t36-live',
        provider: PROVIDER_ID,
      });
      try {
        // ── 等一个 tick：真进程里的调度器到点触发 ───────────────────────────
        try {
          await waitFor(
            () =>
              eventsOf(hd, SID_OK, 'system.triggered').length >= 1 &&
              eventsOf(hd, SID_BROAD, 'system.triggered').length >= 1,
            {
              what: '两个会话的心跳到点（`system.triggered` 落格；调度器 15s 一 tick）',
              timeoutMs: 70_000,
            },
          );
        } catch (e) {
          // 超时必带诊断：调度器没触发的线索在 CLI stderr。
          throw new Error(`${e.message}\n--- 长驻 CLI stderr 尾部 ---\n${cli.out.stderr.slice(-1200) || '（空）'}`);
        }

        // ── 证据 ①：三个 initiator 事实成链，且溯源指得对 ──────────────────────
        for (const sid of [SID_OK, SID_BROAD]) {
          const triggered = eventsOf(hd, sid, 'system.triggered');
          assertEq(triggered.length, 1, `${sid}: 恰好一次触发（实际 ${triggered.length}）`);
          assertEq(triggered[0].entity, 'System', `${sid}: 触发格实体坐标`);
          assertEq(triggered[0].verb, 'Opened', `${sid}: 触发格动词坐标（system × opened）`);
          assertEq(triggered[0].payload.kind, 'scheduled', `${sid}: 触发种类是"定时"`);
          assertEq(
            triggered[0].actor,
            'agent:autonomous',
            `${sid}: 自主行为记在自主主体头上（不冒充用户对话）`,
          );
          assert(triggered[0].produced_by != null, `${sid}: 触发必带溯源（I2：指向上一格）`);

          const health = eventsOf(hd, sid, 'system.health');
          assertEq(health.length, 1, `${sid}: 触发即自检一格（实际 ${health.length}）`);
          assertEq(health[0].entity, 'System', `${sid}: 自检格实体坐标`);
          assertEq(health[0].verb, 'Progressed', `${sid}: 自检格动词坐标（system × progressed）`);
          assert(
            health[0].payload.idle_ms > 0,
            `${sid}: 自检记下触发时测得的空闲时长（实际 ${health[0].payload.idle_ms}）`,
          );
          assertEq(
            health[0].produced_by,
            triggered[0].seq,
            `${sid}: 自检是**这一次触发**的观测附录（锚在触发格上）`,
          );

          const conation = eventsOf(hd, sid, 'conation.expressed');
          assertEq(conation.length, 1, `${sid}: 恰好一条「欲」（实际 ${conation.length}）`);
          assertEq(conation[0].entity, 'Conation', `${sid}: 欲格实体坐标`);
          assertEq(conation[0].verb, 'Opened', `${sid}: 欲格动词坐标（conation × opened）`);
          assertEq(
            conation[0].payload.goal,
            sid === SID_OK ? PROMPT_OK : PROMPT_BROAD,
            `${sid}: 欲的目标就是心跳提示词本身`,
          );
          assertEq(
            conation[0].produced_by,
            triggered[0].seq,
            `${sid}: 欲溯源到触发（E1：欲是数据、必带溯源，否则过不了闸门）`,
          );
        }

        // ── 证据 ②：闸门的**两侧判决**——同一条链路，只有闸门在分岔 ─────────────
        const openedOk = eventsOf(hd, SID_OK, 'task.opened');
        assertEq(
          openedOk.length,
          1,
          `短提示词过闸 ⇒ 恰好开出一个长任务（实际 ${openedOk.length}）`,
        );
        const okTriggered = eventsOf(hd, SID_OK, 'system.triggered')[0];
        const okConation = eventsOf(hd, SID_OK, 'conation.expressed')[0];
        assertEq(openedOk[0].entity, 'Task', '长任务实体坐标');
        assertEq(openedOk[0].verb, 'Opened', '长任务动词坐标（task × opened）');
        assertEq(openedOk[0].payload.goal, PROMPT_OK, '长目标就是被批准的那条欲');
        assertEq(
          openedOk[0].payload.task_id,
          `hb-${okTriggered.seq}`,
          '长任务号按触发 seq 派生（同一次触发只开一个）',
        );
        assertEq(
          openedOk[0].payload.budget_ms,
          86_400_000,
          '长任务取自主档预算（`LatencyTier::Autonomic` = 24h）',
        );
        assertEq(
          openedOk[0].produced_by,
          okConation.seq,
          '★ 长任务溯源到**被批准的欲**（`approved.from_seq`），不是触发本身——闸门真的在链上',
        );
        assertEq(
          openedOk[0].actor,
          'agent:autonomous',
          '长任务由自主主体开出（不冒充用户）',
        );

        assertEq(
          eventsOf(hd, SID_BROAD, 'task.opened').length,
          0,
          '超长提示词被闸门判「目标过宽」⇒ **不得**开出长任务（同一链路，只有闸门在分岔）',
        );

        // ── 证据 ③：心跳那一轮真的跑了（触发不是只落事实、不发轮）────────────
        // 触发事实**先于**这一轮落格（`record_autonomous_trigger` 在 `handle_chat_send_oneoff`
        // 之前跑，正是"事实没落成就不发这一轮"）⇒ 等到触发不等于等到这一轮收束，
        // 必须再等收束转写把心跳轮的用户发言写进事实源。
        await waitFor(
          () =>
            eventsOf(hd, SID_OK, 'user.message').some((e) =>
              String(e.payload?.text ?? '').includes(PROMPT_OK),
            ),
          { what: '心跳轮收束转写落格（提示词作为一条用户发言进事实源）', timeoutMs: 40_000 },
        );
        assert(
          eventsOf(hd, SID_OK, 'chat.assistant.final').length >= 2,
          `心跳轮应真的走完（建会话轮 + 心跳轮各一条收束，实际 ${eventsOf(hd, SID_OK, 'chat.assistant.final').length}）`,
        );

        // ── 证据 ④：转写不变量在两份事实源上都成立 ────────────────────────────
        assertTranscriptInvariants(readMessagesJson(hd.homedir, SID_OK), 'T36(ok)');
        assertTranscriptInvariants(readMessagesJson(hd.homedir, SID_BROAD), 'T36(broad)');
      } finally {
        cli.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28/t34 同一约定：Windows 上带活子进程退出会
      // 撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
