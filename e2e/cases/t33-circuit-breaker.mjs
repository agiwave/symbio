import './_selfrun.mjs';
// T33 外部执行闸门（熔断）：预算耗尽 ⇒ 工具**不执行**，且 `task.controlled` 落格。
//
// ## 本用例钉的是什么
//
// [11 批 2 ③](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的接线判据：
// 「每件接线一条 e2e（**接线前后行为可见地不同**，否则等于没接）」——
// 这里钉的是 `CircuitBreaker`（[roadmap/S09 §6](../../docs/plan/roadmap/S09-外部执行与熔断.md)
// 验收 2：预算耗尽必须**有事件**，不允许静默继续）。
//
// ## 为什么必须端到端（单测证不了的三件事）
//
// `tool_executor.test.rs::budget_exhausted_refuses_the_tool_and_reports_a_break` 已经
// 用假 sink 覆盖了「闸门判 Break ⇒ 结果节点写 `Refused:`」这一半，但它证不了：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 工具**真的没被执行** | mock-mcp 的 `--record`（进程外证据，不看后端自述） |
// | 熔断**真的入了格** | `<会话目录>/v2-events.wal` 的 `task.controlled`（写方在 `v2_bridge`） |
// | 闸门的判据**真的来自台账** | 注入一条已耗账 ⇒ 同形状的调用从「执行」翻成「不执行」 |
//
// 第三条是本用例的核心：**同一份编排、同一次工具调用**，只因台账里多了一条已耗，
// 判定就从 `Allow` 翻成 `Break`。这正是「接线前后行为可见地不同」的可执行形式——
// 常量判定拿不出这一对相反的结论。
//
// ## 怎么把 `Break` 做出来（`Refuse` 为什么不用）
//
// `CircuitBreaker::gate` 的三条出口里，本机生产只有 `Allow` 是**可达**的：
// - `Refuse` 要主体不持 `produce.artifact`，而 `authz` 的静态表给 `agent:*` 与
//   `agent:main` 同一套能力（身份分层 ≠ 权限分层），非 `agent:*` 主体又写不了收束格
//   ⇒ 生产里判不出 `Refuse`（它的单测在 `tool_executor.test.rs`，属单元面）；
// - `gate-timeout` 要闸门自身耗时 > 反射档 80ms，纯函数调用做不到；
// - `Break` 要 `spent_ms + requested_ms > budget_ms`：`requested_ms` = 深度档 60000ms、
//   `budget_ms` = 自主层 86400000ms，而 `spent_ms` 读的是 `cost_ledger` 台账——
//   **台账是事实源里的数据**，于是它可以被构造（本用例注入一条 `cost_ms` 超上限的账）。
//
// 注入用的是 T28 的同一条纪律：**照抄一条真实事件的行形状**，只换定位与载荷，
// 枚举拼写绝不手写（写错一个字母，replay 会把整行当撕裂尾行截断，测出来的是解析器
// 不是闸门）。被抄的是本轮真实落下的收束格——它的 `actor` 已经是 `agent:main`
// （`BreakerInputs::of` 要按同一主体取账），`ts` / `produced_by` 也都合法。
//
// ## 为什么是 `bridge` 档
//
// 本用例钉的是 `bridge` 档的熔断入格（`task.controlled` 的写方挂在 `v2_bridge::record`
// 上）；`full` 档的那一份由 `v2_exec` 轮末经 `SessionDispatchPort::take_derived`
// 交回的出参落格——两档同一个写方（`v2_bridge::record_derived`），`full` 档那一侧
// 另有 `t42` 覆盖。故本用例与 T28 读数口一样走 `bridge` 档。
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  readFileSyncSafe,
  textOf,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t33';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** mock-mcp 回显内容：只有**真执行了**才会在请求体里出现 */
const ECHO_TEXT = '闸门回显';

/**
 * 注入的已耗（毫秒）：`LatencyTier::Autonomic` = 86400000（会话累计上限），
 * 而 `requested_ms` = `LatencyTier::Deep` = 60000。取上限 +1 ⇒
 * `spent + requested > budget` 必然成立（S09 §6.4 的「只换预算就翻面」在生产判据上的形状）。
 */
const EXHAUST_MS = 86_400_001;

/** `CircuitBreaker::break_event` 复用 `control/opened` 格，kind 常量 = `task.controlled` */
const KIND_CONTROLLED = 'task.controlled';

/** 读文件里的全部事件。 */
function readEvents(walPath) {
  const raw = readFileSyncSafe(walPath);
  assert(raw.length > 0, `v2 事实源应存在且非空：${walPath}`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** mock-mcp 记录里真正被执行的 tools/call 条数（进程外证据）。 */
function mcpCallCount(homedir) {
  const raw = readFileSyncSafe(join(homedir, 'mcp-record.ndjson'));
  return raw
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l))
    .filter((e) => e.kind === 'call');
}

export default defineCase(
  'T33 外部执行闸门：预算耗尽 ⇒ 工具不执行（mock-mcp 无第二次调用）+ task.controlled 落格',
  async () => {
    const llm = await new MockLlm([
      // 第一轮：模型请求工具 ⇒ 台账还是空的 ⇒ 闸门 Allow ⇒ 工具执行。
      {
        id: 'call-1',
        match: '第一轮',
        toolCalls: [
          { id: 'c1', name: 'mcp__mockserv__echo', arguments: { text: ECHO_TEXT } },
        ],
      },
      // 工具结果轮（最后一条 role=tool）⇒ 必须用 `afterTool` 场景，否则带 match 的
      // 工具调用场景会反复命中（无限工具循环，见 mock-llm 文件头）。
      { id: 'after-1', afterTool: true, once: true, content: '第一轮收尾。' },
      // 第二轮：**同一形状**的工具请求，但台账已被注入一条已耗 ⇒ 闸门 Break。
      {
        id: 'call-2',
        match: '第二轮',
        toolCalls: [
          { id: 'c2', name: 'mcp__mockserv__echo', arguments: { text: ECHO_TEXT } },
        ],
      },
      { id: 'after-2', afterTool: true, once: true, content: '第二轮收尾。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 本用例的主题与对话面正交 ⇒ 钉死（理由见 `DIALOG_FACE_OFF`），再叠加主题：
        // `bridge` 档（熔断格的写方 `v2_bridge::record` 只在这一档跑）。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'bridge' },
      },
    });
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, 'mcp-record.ndjson')],
      enabled: true,
    });

    const walPath = join(hd.homedir, 'session', SID, V2_WAL);

    try {
      // ── 第一轮：闸门 Allow（台账零账）⇒ 工具真的执行 ───────────────────────
      const r1 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '第一轮：请执行工具',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r1.code, 0, `第一轮 CLI 退出码（stderr: ${r1.stderr.slice(0, 600)}）`);
      assert(
        r1.stdout.includes('第一轮收尾。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(r1.stdout)}）`,
      );

      const calls1 = mcpCallCount(hd.homedir);
      assertEq(calls1.length, 1, `第一轮应恰好执行一次工具（实际 ${calls1.length} 次）`);
      assertEq(calls1[0].name, 'echo', '被调工具名');
      assertEq(calls1[0].args, { text: ECHO_TEXT }, '工具参数');

      const events1 = readEvents(walPath);
      assert(
        !events1.some((e) => e.kind === KIND_CONTROLLED),
        `零账时不该有熔断格（实得 kind: ${events1.map((e) => e.kind).join(', ')}）`,
      );

      // ── 注入一条已耗超上限的账（台账是事实源里的数据 ⇒ 可构造）──────────────
      const lines = readFileSyncSafe(walPath).split('\n').filter(Boolean);
      const parsed = lines.map((l) => JSON.parse(l));
      const seed = parsed.find((e) => e.kind === 'chat.assistant.final');
      assert(seed, `应有收束格可作行形状模板（实得: ${parsed.map((e) => e.kind).join(', ')}）`);
      assertEq(seed.actor, 'agent:main', '模板的 actor 必须是闸门按之取账的那个主体');
      const inject = {
        ...seed,
        event_id: 'cb-budget-seed',
        // 换到一格没人用过的 turn：不制造第二条同轮收束（`final_unique_per_turn` 的判据
        // 是 turn 分桶），也不影响任何不变量——注入的是**账**，不是这一轮的发言。
        turn: 9_999,
        seq: Math.max(...parsed.map((e) => e.seq ?? -1)) + 1,
        cost_ms: EXHAUST_MS,
      };
      writeFileSync(walPath, `${lines.join('\n')}\n${JSON.stringify(inject)}\n`, 'utf8');

      // ── 第二轮：同一形状的工具请求，闸门 Break ⇒ 工具不执行 + 熔断格落格 ─────
      const r2 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '第二轮：请执行工具',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r2.code, 0, `第二轮 CLI 退出码（stderr: ${r2.stderr.slice(0, 600)}）`);
      assert(
        r2.stdout.includes('第二轮收尾。'),
        `被拒之后本轮仍应收束（实际 stdout: ${JSON.stringify(r2.stdout)}）`,
      );

      // 证据 ①：工具**没有**被执行——进程外证据，唯一能证"真的没跑"的地方。
      const calls2 = mcpCallCount(hd.homedir);
      assertEq(
        calls2.length,
        1,
        `预算耗尽后 mock-mcp 不该收到第二次调用（实际 ${calls2.length} 次：${JSON.stringify(calls2.map((c) => c.args))}）`,
      );

      // 证据 ②：熔断真的入了格（S09 §6 验收 2：不允许静默继续）。
      const events2 = readEvents(walPath);
      const controlled = events2.filter((e) => e.kind === KIND_CONTROLLED);
      assertEq(controlled.length, 1, `应恰好一格熔断（实得 ${controlled.length} 格）`);
      assertEq(controlled[0].entity, 'Control', '熔断格实体坐标');
      assertEq(controlled[0].verb, 'Opened', '熔断格动词坐标');
      assertEq(controlled[0].payload.reason, 'budget-exhausted', '熔断理由按理由区分（打断 vs 熔断）');
      assert(controlled[0].produced_by != null, '熔断格必须带溯源（I2）');

      // 证据 ③：被拒的调用在转写里**留了结果**（否则模型看到调用凭空消失）。
      const msgs = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs, 'T33');
      const refused = msgs.filter((m) => m.role === 'tool' && textOf(m).includes('Refused:'));
      assertEq(refused.length, 1, `应恰好一条被拒的工具结果（实得 ${refused.length} 条）`);
      assert(
        textOf(refused[0]).includes('熔断'),
        `被拒理由应写明是熔断（实际: ${textOf(refused[0]).slice(0, 200)}）`,
      );

      // ── 反向对照：两轮的编排同形、工具同名，差别只在台账 ─────────────────────
      const toolCalls = msgs.filter((m) => (m.type ?? m.msg_type) === 'tool_call');
      assertEq(toolCalls.length, 2, `两轮各请求过一次工具（实得 ${toolCalls.length} 次）`);
      assertEq(
        toolCalls.filter((m) => m.status === 'completed').length,
        2,
        '被拒的调用同样必须收口（否则父节点永远停在「运行中」）',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
