import './_selfrun.mjs';
// T42 **收束派生事实**：承诺 / 任务表也入格——全部经**用户可见面**读回。
//
// ## 本用例钉的是什么
//
// 轮次事实（用户格 / final 格 / 产物格）由运行器原生记账，`chat_loop` 因此
// **不再自己记一份**轮次账（`TurnState` 只用于读侧判「这一轮收束了没有」）。
// 但**承诺 / 任务表不是轮次事实**，而是本轮的派生副作用——数据来源都在工具
// 执行层（`Delegation` / `TaskDeclaration`），运行器一处都不写。这一半若也随轮次
// 事实一起被拦下，代际立约/ 任务表就**静默全丢**——与记忆/学习同一个
// 坑（t40 钉的是那一半）。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 任务表真的落成事实 | 生产出口 `session/stats` 的 `readyset` 列（就绪集从同一份事实算） |
// | 依赖关系真的穿到了读侧 | 就绪集 = 无依赖 / 依赖已满足的那两条（`[t1, t3]`，`t2` 被 `t1` 挡住） |
// | 代际立约真的发生并被了结 | 同一出口的 `reputation.own`（`offered` / `kept`） |
// | 两轮都真的收束 | 出口 `invariants` 为空 + 每轮 stdout 的收尾正文 |
//
// ## 熔断那一支为什么**不在这里**
//
// 早先版本还用「注入一条超预算的账 ⇒ 闸门 Break ⇒ `task.controlled` 入格」覆盖熔断。
// 但那条构造是**存储手术**（直写 `v2-events.wal` 的 JSONL）——[D-049] 读通道
// 禁令下，e2e 不该知道存储布局。熔断的**存储层判断**（闸门判 Break、`task.controlled`
// 的理由与锚）已在 Rust 单测面（`tool_executor.test.rs` 直接摆台账调
// `process_tool_calls_async`）覆盖；e2e 这一支**整体删除**，不为过渡形态背约束。
//
// ## 为什么对话面钉死（`DIALOG_FACE_OFF`）
//
// 对话面（`classify` 判决 / `compose` 措辞）出厂默认开启，会给每一轮多加一次静默
// 请求。本用例与对话面正交，故按约定把它钉死——请求数才可预期。
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  readSessionMessages,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t42';

/** 第 1 轮触发串：声明任务图（`todo_write`）。 */
const DECLARE = '登记任务';
/** 第 2 轮触发串：委托子智能体（`agent_run`）。 */
const DELEGATE = '委托评审';

/** 任务图（同 t31）：`t2` 依赖 `t1`；`t1`/`t3` 无依赖 ⇒ 就绪集 = `[t1, t3]`。 */
const TODOS = [
  { id: 't1', content: '分析架构', status: 'in_progress', priority: 'high' },
  { id: 't2', content: '落地实现', status: 'pending', priority: 'medium', depends_on: ['t1'] },
  { id: 't3', content: '写文档', status: 'pending', priority: 'low' },
];

/** 委托出去的那句话（= `agent_run` 的 `prompt`，也是子会话的触发串）。 */
const PROMPT = '请复查这段实现';

export default defineCase(
  'T42 收束派生事实：任务表 / 承诺从生产出口读得回（熔断下沉 Rust 单测）',
  async () => {
    const llm = await new MockLlm([
      // ── 第 1 轮：声明任务图（工具真的执行）──────────────────────────────
      {
        id: 'r1-todo',
        match: DECLARE,
        once: true,
        content: '我先把任务登记进清单。',
        toolCalls: [{ id: 'call_1', name: 'todo_write', arguments: { todos: TODOS } }],
      },
      { id: 'r1-final', once: true, content: '任务清单已登记。' },
      // ── 第 2 轮：委托子智能体（`agent_run` ⇒ 代际立约）────────────────────
      {
        id: 'r2-delegate',
        match: DELEGATE,
        once: true,
        content: '我来委托。',
        toolCalls: [
          { id: 'call_2', name: 'agent_run', arguments: { agent_id: 'reviewer', prompt: PROMPT } },
        ],
      },
      // 派生子会话自己的那一次请求（`prompt` 就是它的最后一条 user 消息）
      { id: 'r2-sub', match: PROMPT, once: true, content: '评审：通过。' },
      { id: 'r2-final', once: true, content: '已收到评审结论。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 对话面三项按 `DIALOG_FACE_OFF` 钉死；`task_cap: 3` 让两条无依赖任务
        // 同批进候选（平凡值 `1` 会截成一笔，就绪集就只剩第一笔）。
        session: { ...DIALOG_FACE_OFF, task_cap: 3 },
      },
    });
    // 子智能体目录：`agent_run` 的落点（t30 同款）。
    makeAgentDir(hd.homedir, {
      id: 'reviewer',
      persona: '你是评审者。',
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };

    try {
      // ── 第 1 轮：声明任务图 ─────────────────────────────────────────────
      const r1 = runCli({ ...base, session: SID, message: `请${DECLARE}` });
      assertEq(r1.code, 0, `第 1 轮退出码（stderr: ${r1.stderr.slice(0, 600)}）`);
      assert(
        r1.stdout.includes('任务清单已登记'),
        `第 1 轮 stdout 应为收尾正文（实际: ${JSON.stringify(r1.stdout)}）`,
      );

      // ── 第 2 轮：委托 ⇒ 代际立约 ────────────────────────────────────────
      const r2 = runCli({ ...base, session: SID, message: `请${DELEGATE}`, timeoutMs: 180_000 });
      assertEq(r2.code, 0, `第 2 轮退出码（stderr: ${r2.stderr.slice(0, 600)}）`);
      assert(
        r2.stdout.includes('已收到评审结论'),
        `第 2 轮 stdout 应为收尾正文（实际: ${JSON.stringify(r2.stdout)}）`,
      );

      // 读回 CLI：转写（读出口）与出口读数（session/stats）都经它——用户如何看到就如何读。
      const api = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await api.waitGatewayReady();

        // ── ① 轮次事实：两轮都收束（经生产读出口读回的转写）────────────────
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T42');
        const userTurns = msgs.filter((m) => msgRoleOf(m) === 'user');
        assert(userTurns.length >= 2, `两轮各有一格用户发言（实际 ${userTurns.length}）`);

        // ── ② 出口读数：派生事实从同一份事实源算出的列 ──────────────────────
        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);

        // 不变量：C2 每轮一条 final / C3 断言带溯源 / C4 未收束 / 依赖成环 …
        // 空 = 全绿。
        assertEq(
          stats.invariants.length,
          0,
          `两轮都收束 ⇒ 不变量清单为空（实际: ${JSON.stringify(stats.invariants)}）`,
        );

        // ── ③ 任务表：就绪集从声明入格后的事实算出（`[t1, t3]`）──────────────
        assertEq(
          stats.readyset.ready.map((t) => t.task_id),
          ['t1', 't3'],
          `出口读出的就绪集应与声明的任务图一致（实际: ${JSON.stringify(stats.readyset)}）`,
        );
        assertEq(
          stats.readyset.ready[1].depends_on,
          [],
          '就绪任务的依赖必然已全部终态（放进集合时就满足，不是断言时才校验）',
        );

        // ── ④ 承诺：委托成功 ⇒ 立约一次、了结一次（声誉列）──────────────────
        const own = stats.reputation.own;
        assertEq(own.principal, 'session/main', '声誉列 own 的主体 = 本会话主体');
        assertEq(own.offered, 1, '真实委托算出的立约 1 次（不是常数）');
        assertEq(own.kept, 1, '委托成功 ⇒ 了结为守约 kept 1');
        assertEq(own.broken, 0, '没有违约 ⇒ broken 0');
        assertEq(own.score, 1, '平凡打分 = 守约 − 违约');
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
