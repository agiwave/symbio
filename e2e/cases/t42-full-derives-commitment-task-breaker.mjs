import './_selfrun.mjs';
// T42 `v2_mode = full` 档的**收束派生事实**：承诺 / 任务表 / 熔断也入格。
//
// ## 本用例钉的是什么
//
// `full` 档的轮次事实（用户格 / final 格 / 产物格）由 v2 运行器原生记账，
// `chat_loop` 因此以 `TurnState::v2_executed` 拦下整段 `v2_facts::record`。
// 但**承诺 / 任务表 / 熔断不是轮次事实**，而是本轮的派生副作用——数据来源都在工具
// 执行层（`Delegation` / `TaskDeclaration` / 熔断理由），运行器一处都不写。这一半若
// 也随轮次事实一起被拦下，`full` 档的代际立约（S08）/ 任务表（S7）/ 熔断（S9 §6
// 验收 2）就**静默全丢**——与记忆/学习同一个坑（t40 钉的是那一半）。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 任务表真的入格 | `v2-events.wal` 的 `task.opened`（写方 `v2_tasks::write`） |
// | 代际立约真的入格 | 同一份 WAL 的 `commitment.opened` → `commitment.released` |
// | 熔断真的入格 | 同一份 WAL 的 `task.controlled{reason}`（写方 `CircuitBreaker`） |
//
// ## 为什么必须端到端（单测证不了的那一件）
//
// 单测（`v2_exec.test.rs::full_turn_lands_derived_commitment_facts`）用假 provider
// 把「出参通道 + 写方」钉死了，但它的 `agent_run` 走的是「没有父插件」的诚实失败
// 路径——证不了**真工具真的跑起来、真成功**时出参也照样交回。任务表尤其如此：
// `note_tasks` 只记**成功**的 `todo_write`，失败路径根本进不了它。熔断更只能靠
// 台账手术构造（闸门的 `Break` 出口），单测碰不到。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 对话面（`classify` 判决 / `compose` 措辞）出厂默认开启，会给每一轮多加一次静默
// 请求。本用例与对话面正交，故按约定把它钉死——请求数才可预期。
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  makeAgentDir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t42';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 第 1 轮触发串：声明任务图（`todo_write`）。 */
const DECLARE = '登记任务';
/** 第 2 轮触发串：委托子智能体（`agent_run`）。 */
const DELEGATE = '委托评审';
/** 第 3 轮触发串：请求一个外部工具（会被熔断拦下）。 */
const RUN_TOOL = '执行工具';

/** 任务图（同 t31）：`t2` 依赖 `t1`；`t1`/`t3` 无依赖。 */
const TODOS = [
  { id: 't1', content: '分析架构', status: 'in_progress', priority: 'high' },
  { id: 't2', content: '落地实现', status: 'pending', priority: 'medium', depends_on: ['t1'] },
  { id: 't3', content: '写文档', status: 'pending', priority: 'low' },
];

/** 委托出去的那句话（= `agent_run` 的 `prompt`，也是子会话的触发串）。 */
const PROMPT = '请复查这段实现';

/** mock-mcp 回显内容：只有**真执行了**才会出现（熔断轮不该出现）。 */
const ECHO_TEXT = '闸门回显';

/**
 * 注入的已耗（毫秒）：`LatencyTier::Autonomic` = 86400000（会话累计上限），
 * 而 `requested_ms` = `LatencyTier::Deep` = 60000。取上限 +1 ⇒
 * `spent + requested > budget` 必然成立（与 t33 同一条构造）。
 */
const EXHAUST_MS = 86_400_001;

/** `CircuitBreaker::break_event` 复用 `control/opened` 格，kind 常量 = `task.controlled` */
const KIND_CONTROLLED = 'task.controlled';

/** 读某个会话的事实源（JSONL）。 */
function readEvents(homedir, sid) {
  const raw = readFileSyncSafe(join(homedir, 'session', sid, V2_WAL));
  assert(raw.length > 0, `${sid} 的事实源应存在且非空`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** 某轮用户格的 seq（派生事实的溯源锚，I2）。 */
function userSeqOf(events, turn) {
  const e = events.find((x) => x.kind === 'user.message' && x.turn === turn);
  assert(e, `第 ${turn} 轮的用户格应入格`);
  return e.seq;
}

export default defineCase(
  'T42 full 档收束派生事实：承诺 / 任务表 / 熔断也入格（不再随轮次事实一起被拦下）',
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
      // ── 第 3 轮：外部工具请求（台账已注入已耗 ⇒ 闸门 Break）──────────────
      {
        id: 'r3-call',
        match: RUN_TOOL,
        once: true,
        content: '我试一下。',
        toolCalls: [
          { id: 'call_3', name: 'mcp__mockserv__echo', arguments: { text: ECHO_TEXT } },
        ],
      },
      { id: 'r3-final', once: true, content: '第三轮收尾。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 主题：full 档。对话面三项按 `DIALOG_FACE_OFF` 钉死。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'full' },
      },
    });
    // 子智能体目录：`agent_run` 的落点（t30 同款）。
    makeAgentDir(hd.homedir, {
      id: 'reviewer',
      persona: '你是评审者。',
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });
    // MCP stdio 工具：`--record` 是「工具真的执行过」的进程外证据。
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, 'mcp-record.ndjson')],
      enabled: true,
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const walPath = join(hd.homedir, 'session', SID, V2_WAL);

    try {
      // ── 第 1 轮：任务表 ─────────────────────────────────────────────────
      const r1 = runCli({ ...base, session: SID, message: `请${DECLARE}` });
      assertEq(r1.code, 0, `第 1 轮退出码（stderr: ${r1.stderr.slice(0, 600)}）`);
      assert(
        r1.stdout.includes('任务清单已登记'),
        `第 1 轮 stdout 应为收尾正文（实际: ${JSON.stringify(r1.stdout)}）`,
      );

      // ── 第 2 轮：承诺 ───────────────────────────────────────────────────
      const r2 = runCli({ ...base, session: SID, message: `请${DELEGATE}`, timeoutMs: 180_000 });
      assertEq(r2.code, 0, `第 2 轮退出码（stderr: ${r2.stderr.slice(0, 600)}）`);
      assert(
        r2.stdout.includes('已收到评审结论'),
        `第 2 轮 stdout 应为收尾正文（实际: ${JSON.stringify(r2.stdout)}）`,
      );

      // ── 注入一条已耗超上限的账（台账是事实源里的数据 ⇒ 可构造）──────────
      const lines = readFileSyncSafe(walPath).split('\n').filter(Boolean);
      const parsed = lines.map((l) => JSON.parse(l));
      const seed = parsed.find((e) => e.kind === 'chat.assistant.final');
      assert(seed, `应有收束格可作行形状模板（实得: ${parsed.map((e) => e.kind).join(', ')}）`);
      assertEq(seed.actor, 'agent:main', '模板的 actor 必须是闸门按之取账的那个主体');
      const inject = {
        ...seed,
        event_id: 'cb-budget-seed',
        turn: 9_999,
        seq: Math.max(...parsed.map((e) => e.seq ?? -1)) + 1,
        cost_ms: EXHAUST_MS,
      };
      writeFileSync(walPath, `${lines.join('\n')}\n${JSON.stringify(inject)}\n`, 'utf8');

      // ── 第 3 轮：熔断（同一形状的外部工具请求 ⇒ 闸门 Break）──────────────
      const r3 = runCli({ ...base, session: SID, message: `请${RUN_TOOL}` });
      assertEq(r3.code, 0, `第 3 轮退出码（stderr: ${r3.stderr.slice(0, 600)}）`);
      assert(
        r3.stdout.includes('第三轮收尾'),
        `被拒之后本轮仍应收束（实际 stdout: ${JSON.stringify(r3.stdout)}）`,
      );

      const events = readEvents(hd.homedir, SID);
      const kinds = events.map((e) => e.kind).join(', ');

      // ── ① 轮次事实：确实由 v2 原生记账（本用例的前提，不是主题）──────────
      assertEq(
        events.filter((e) => e.kind === 'user.message').length,
        3,
        `三轮各一格用户格：${kinds}`,
      );
      assert(
        events.some((e) => e.kind === 'chat.assistant.final'),
        `轮次事实应原生入格（full 档）：${kinds}`,
      );

      // ── ② 任务表：`todo_write` 的声明入格（修复前 full 档恒为 0）──────────
      const opened = events.filter((e) => e.kind === 'task.opened');
      assertEq(
        opened.length,
        3,
        `三条任务各开一格（修复前 full 档恒为 0——任务表整段被拦下）：${kinds}`,
      );
      assertEq(
        opened.find((e) => e.payload?.task_id === 't2')?.payload?.depends_on,
        ['t1'],
        `t2 的依赖必须原样落格（实际: ${JSON.stringify(opened.map((e) => e.payload))}）`,
      );
      for (const e of opened) {
        assertEq(
          e.produced_by,
          userSeqOf(events, 0),
          `任务事件溯源应锚在声明那一轮的用户格（${e.event_id} → ${e.produced_by}）`,
        );
        assert(
          /^v2t-t0-\d+$/.test(e.event_id),
          `full 档任务号词干是 t{turn}（调用编号跨轮会撞幂等键）：${e.event_id}`,
        );
      }

      // ── ③ 承诺：`agent_run` 的代际立约入格（修复前 full 档恒为 0）─────────
      const offers = events.filter((e) => e.kind === 'commitment.opened');
      const releases = events.filter((e) => e.kind === 'commitment.released');
      assertEq(
        offers.length,
        1,
        `第 2 轮委托 ⇒ 一格立约（修复前 full 档恒为 0——承诺整段被拦下）：${kinds}`,
      );
      assertEq(releases.length, 1, `委托成功 ⇒ 一格守约：${kinds}`);
      assertEq(
        offers[0].payload?.promise,
        PROMPT,
        `承诺内容 = 委托出去的那句话（实际: ${JSON.stringify(offers[0].payload)}）`,
      );
      assertEq(
        offers[0].produced_by,
        userSeqOf(events, 1),
        '立约锚在本轮开口（I2：承诺溯源 100%）',
      );
      assertEq(
        releases[0].payload?.id,
        offers[0].payload?.id,
        '了结按载荷 id 找回立约方（同锚成对）',
      );
      assert(
        /^c-offer-v2c-t1-/.test(offers[0].event_id),
        `full 档承诺号词干是 t{turn}：${offers[0].event_id}`,
      );

      // ── ④ 熔断：闸门判 Break ⇒ `task.controlled` 入格（修复前 full 档恒为 0）──
      const controlled = events.filter((e) => e.kind === KIND_CONTROLLED);
      assertEq(
        controlled.length,
        1,
        `应恰好一格熔断（修复前 full 档恒为 0——熔断整段被拦下）：${kinds}`,
      );
      assertEq(controlled[0].entity, 'Control', '熔断格实体坐标');
      assertEq(controlled[0].verb, 'Opened', '熔断格动词坐标');
      assertEq(
        controlled[0].payload.reason,
        'budget-exhausted',
        '熔断理由按理由区分（打断 vs 熔断）',
      );
      assertEq(
        controlled[0].produced_by,
        userSeqOf(events, 2),
        '熔断锚在本轮开口（I2）',
      );

      // ── ⑤ 熔断轮的进程外证据：工具**没有**被执行（唯一能证"真的没跑"的地方）──
      const calls = readFileSyncSafe(join(hd.homedir, 'mcp-record.ndjson'))
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l))
        .filter((e) => e.kind === 'call');
      assertEq(calls.length, 0, `预算耗尽后 mock-mcp 不该收到任何调用（实际 ${calls.length} 次）`);

      // ── 转写不变量 ─────────────────────────────────────────────────────────
      const msgs = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs, 'T42');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
