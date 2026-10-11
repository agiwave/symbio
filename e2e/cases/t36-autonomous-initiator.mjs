import './_selfrun.mjs';
// T36 自主层接线（`AutonomousInitiator` + `IntentGate`，S9 第 21 步，S12）：
// 会话空闲到点 ⇒ 心跳触发一轮 ⇒ 「欲」过闸门 ⇒ 过了才真的发出那一轮。
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
// | 心跳**真的被定时器驱动**（不是只在测试里被调用） | 真进程里跑够一个 tick，转写里长出心跳提示词那一轮 |
// | 闸门**真的在链上**（不是摆设） | 就绪集（`session/stats.readyset`）：过闸 ⇒ 那条长任务在其中；拒收 ⇒ 就绪集里没有它 |
//
// ## 为什么必须端到端（单测证不了的两件事）
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
//    一个真的跑出心跳轮、一个什么都没有。
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
// ## 心跳怎么装：经 **heartbeat 工具**（用户可见入口），不碰内部存储
//
// 装心跳的正路是模型在会话里调 `heartbeat` 工具（`action=set`，session 插件内聚的
// 设置入口）——它与触发、调度走同一条生产机制。`interval_seconds` 取下限 10s
// （调度器 15s 一 tick + 0–30s 抖动），触发轮以心跳提示词作为用户发言落进转写，
// 于是「有没有过闸」在**转写读出口**上直接可读（`vdfs/read` 会话叶子，前端加载
// 历史同一出口）。
//
// 长驻 CLI 用**中立会话 id**：REPL 启动时会把自己那个会话的 `updated_at` 推到 now，
// 拿被测会话当 REPL 会话就会把被测会话的空闲基线抹掉（SID_OK 于是比 SID_BROAD
// 晚触发）。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readSessionMessages,
} from '../helpers.mjs';

/** 两个会话：同一份代码、同一条链路，只有心跳提示词长度不同 ⇒ 闸门判决不同。 */
const SID_OK = 'e2e-t36-ok';
const SID_BROAD = 'e2e-t36-broad';

/** 装心跳那一轮的用户发言（两个会话同文，靠 `once` + 数组顺序各自消耗各自那条场景）。 */
const MSG = '开始工作';

/** 闸门**放行**的提示词：短（远低于 200 字节上界）⇒ 应开出心跳轮。 */
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

/** 心跳间隔取下限 10s（调度器 15s 一 tick + 0–30s 抖动）。 */
const INTERVAL = 10;

export default defineCase(
  'T36 自主层接线：心跳到点成轮，闸门放行的会话真的跑出来 / 拒收的什么都没有',
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
      // 装心跳：两个会话各一次（数组顺序即消耗顺序——SID_OK 先跑，拿第一条）。
      // `once` 保证同文输入在两会话上各自落到各自那条场景上。
      {
        id: 'hb-ok',
        match: MSG,
        once: true,
        content: '我先把心跳设置好。',
        toolCalls: [
          {
            id: 'call_hb_ok',
            name: 'heartbeat',
            arguments: { action: 'set', interval_seconds: INTERVAL, prompt: PROMPT_OK },
          },
        ],
      },
      {
        id: 'hb-broad',
        match: MSG,
        once: true,
        content: '我先把心跳设置好。',
        toolCalls: [
          {
            id: 'call_hb_broad',
            name: 'heartbeat',
            arguments: { action: 'set', interval_seconds: INTERVAL, prompt: PROMPT_BROAD },
          },
        ],
      },
      // 工具结果轮（末条 role=tool）：两条线共用。
      { id: 'hb-after', afterTool: true, content: '心跳已设置。' },
      // 心跳触发轮：提示词没有专属场景 ⇒ 落末条兜底。
      { id: 'rest', content: '收到。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 读回走读出口（gateway `invoke` → `vdfs/read`），入站口必须开着
        // （与 T28/T31 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // `conation_enabled` **显式打开**：它默认 false（S12 §4 平凡值「关掉后系统
        // 退化为纯响应式」），本用例验的是打开之后闸门**能放行**的那条路。
        // 拒收那条路不靠这个开关，靠提示词长度——两个判决因此可以并存。
        session: { ...DIALOG_FACE_OFF, conation_enabled: true },
      },
    });

    // 长驻进程：调度器随插件树构建启动，第一个 tick 在 15s 后。用**中立会话 id**
    // （不是被测的两个）：REPL 启动时会把自己会话的 updated_at 推到 now，拿被测
    // 会话当 REPL 会话会把回拨的空闲基线抹掉。
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t36-live',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });

    /** 建会话并把一条消息写进收件箱（唯一输入入口：与 T16/T20 同一手法） */
    const send = async (sid, text) => {
      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${sid}`;
      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({ metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' } }),
      });
      assertEq(created.status, 200, `创建会话 ${sid}（${JSON.stringify(created.body)?.slice(0, 300)}）`);
      const w = await cli.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text });
      assertEq(w.status, 200, `写收件箱 ${sid}（${JSON.stringify(w.body)?.slice(0, 300)}）`);
    };

    /** 该会话转写里的用户发言正文（经读出口读回 = 前端加载历史同一出口） */
    const userTexts = async (sid) => {
      const msgs = (await readSessionMessages(cli, sid)) ?? [];
      return msgs
        .filter((m) => m.role === 'user')
        .map((m) => (typeof m.content === 'string' ? m.content : ''));
    };

    try {
      await cli.waitGatewayReady(20_000, 'T36 长驻进程');

      // ── 装心跳：SID_OK 先跑（拿 hb-ok 场景），SID_BROAD 后跑（拿 hb-broad）──
      // once + 数组顺序 = 两条同文输入各自消耗各自那条场景（见 mock-llm 的匹配规则）。
      await send(SID_OK, MSG);
      await send(SID_BROAD, MSG);

      // 两个装心跳的轮次都真的跑了（工具调用各一次，参数各带各的提示词）。
      await waitFor(async () => (await llm.requests()).length >= 4, {
        what: '两个装心跳轮次跑完（各一次工具调用 + 一次结果回灌）',
        timeoutMs: 60_000,
      });
      const calls = (await llm.requests())
        .flatMap((r) => (r.body.messages ?? []).flatMap((m) => m.tool_calls ?? []))
        .filter((c) => c.function?.name === 'heartbeat')
        .map((c) => c.function?.arguments ?? '');
      assert(
        calls.some((a) => a.includes(PROMPT_OK)),
        `SID_OK 应装上短提示词的心跳（实际: ${JSON.stringify(calls)}）`,
      );
      assert(
        calls.some((a) => a.includes(PROMPT_BROAD)),
        `SID_BROAD 应装上长提示词的心跳（实际: ${JSON.stringify(calls)}）`,
      );

      // ── 等一个 tick：真进程里的调度器到点触发 ───────────────────────────
      // 放行侧应长出心跳轮（提示词作为用户发言落进转写）；拒收侧什么都不该有。
      try {
        await waitFor(
          async () => (await userTexts(SID_OK)).some((t) => t.includes(PROMPT_OK)),
          {
            what: 'SID_OK 的心跳轮落进转写（闸门放行 ⇒ 那一轮真的发生了）',
            timeoutMs: 100_000,
          },
        );
      } catch (e) {
        // 超时必带诊断：调度器没触发的线索在 CLI stderr。
        throw new Error(`${e.message}\n--- 长驻 CLI stderr 尾部 ---\n${cli.out.stderr.slice(-1200) || '（空）'}`);
      }

      // ── 证据：闸门的两侧判决——同一条链路，只有闸门在分岔 ───────────────
      //
      // ## 判据落在**就绪集**上，不落在转写上
      //
      // 闸门管的是「欲**升格成任务**」这件事（`conation.expressed` → `task.opened`），
      // **不是**「心跳轮开不开」。这条边界由单测钉死并写在文档里
      // （`heartbeat/tests.rs::an_over_broad_goal_is_refused_even_with_the_gate_open`：
      // 「拒绝同样是**事实**——触发与欲照样入格，只是不长成任务；静默拒绝就等于
      // 没有闸门」）。所以两侧会话的**心跳轮都会跑**、提示词都进转写；拿转写判
      // 「拒收侧不发轮」是把闸门的职责读大了，判据本身错了。
      //
      // 闸门判决在用户可见面上的唯一形态是**就绪集**：过闸 ⇒ `task.opened` 落格
      // ⇒ 该任务进 `readyset.ready`；拒收 ⇒ 没有 `task.opened` ⇒ 就绪集里没有它。
      // 读口是生产路由 `session/stats`（与 T31 同一出口、同一列），不碰内部存储。
      const statsOf = async (sid) =>
        (await cli.invoke('session/stats', {}, { session_id: sid })).body?.data;

      // 放行侧：等到就绪集里长出那条心跳任务（调度器 15s 一 tick + 抖动）。
      try {
        await waitFor(
          async () => ((await statsOf(SID_OK))?.readyset?.ready ?? []).length >= 1,
          {
            what: 'SID_OK 的心跳任务进就绪集（闸门放行 ⇒ 真的升格成 task.opened）',
            timeoutMs: 100_000,
          },
        );
      } catch (e) {
        // 超时必带诊断：调度器没触发的线索在 CLI stderr。
        throw new Error(`${e.message}\n--- 长驻 CLI stderr 尾部 ---\n${cli.out.stderr.slice(-1200) || '（空）'}`);
      }

      const okReady = (await statsOf(SID_OK))?.readyset?.ready ?? [];
      assert(
        okReady.length >= 1,
        `短提示词过闸 ⇒ 恰好开出一条心跳任务（同一条链路，只有闸门在分岔；实际: ${JSON.stringify(okReady)}）`,
      );
      // 任务号按触发 seq 派生（`hb-{triggered_seq}`，见 `heartbeat::mod.rs`）——这是
      // 「这条就绪任务确实出自心跳触发」在就绪集（只有 `task_id`/`seq`/`depends_on`
      // 三列）里唯一可辨识的形态。
      assert(
        okReady.every((t) => /^hb-\d+$/.test(t.task_id)),
        `就绪的那条必须是心跳任务（task_id 形如 hb-{触发 seq}；实际: ${JSON.stringify(okReady)}）`,
      );

      // 拒收侧：同样等到心跳轮跑过一轮（它照样开轮），但就绪集必须**始终为空**。
      await waitFor(
        async () => (await userTexts(SID_BROAD)).some((t) => t.includes(PROMPT_BROAD)),
        {
          what: 'SID_BROAD 的心跳轮落进转写（拒绝的是升格，不是开轮——轮照样开）',
          timeoutMs: 100_000,
        },
      );
      const broadReady = (await statsOf(SID_BROAD))?.readyset?.ready ?? [];
      assertEq(
        broadReady.length,
        0,
        `超长提示词被闸门判「目标过宽」⇒ **不得**升格成任务（就绪集必须空；实际: ${JSON.stringify(broadReady)}）`,
      );
      // 反向对照：拒收侧**也有**心跳轮（上面那条 waitFor 已证）——所以「就绪集空」
      // 不是因为「什么都没跑」，而正是闸门那一处分岔。两句话缺一，判据都能被
      // 「调度器压根没触发」这种平凡原因满足。
      assert(
        !JSON.stringify(broadReady).includes(PROMPT_BROAD),
        `拒收的目标不得出现在就绪集里（实际: ${JSON.stringify(broadReady)}）`,
      );
    } finally {
      cli.stop();
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
