import './_selfrun.mjs';
// T40 `v2_mode = full` 档的**收束派生事实**：记忆三段 + 技能观测/编译也入格。
//
// ## 本用例钉的是什么
//
// `full` 档的轮次事实（用户格 / final 格 / 产物格）由 v2 运行器原生记账，
// `chat_loop` 因此以 `TurnState::v2_executed` 拦下整段 `v2_bridge::record`
// ——「同一轮两份记账是假象」。
//
// 但**记忆与学习不是轮次事实**，而是本轮的派生副作用：运行器一处都不写。
// 这一半若也随轮次事实一起被拦下，`full` 档下「编码 / 检索锚 / 巩固 / 技能观测 /
// 技能编译」会**静默全丢**——S06 的长期记忆与 S11 的自我改进在这个档位整体失效，
// 而档位名还自称「整体切换」。（`v2_bridge.rs` 曾写「记忆写方同此档位……见
// `v2_exec` 侧的同批注记」，而那一侧既无注记也无实现。）
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 步 11 编码真的发生 | 事实源里 `memory.encoded{tag:"经验"}`，**逐轮**一条 |
// | 步 12 检索锚真的落 | `memory.recalled`（无 `skill_id`）带本轮用户格溯源 |
// | 步 22 观测真的落 | `memory.recalled{skill_id, fallback}`——缺它 `calibration` 恒读「零使用」 |
// | 步 22 编译真的发生 | `memory.encoded{tag:"skill"}`，**逐轮**一条（去重按触发串），正文 = 该轮收束发言 |
//
// ## 为什么必须端到端（单测证不了的那一件）
//
// 单测（`v2_exec.test.rs::full_turn_lands_memory_and_learning_facts`）把
// `execute_turn` 这一层钉死了，但 `recalled` / `skill_obs` 是**从 `chat_loop` 传进来**
// 的（`TurnState::recall_view` / `skill_route`）——单测自己填这两个入参，
// 证不了生产侧真的把它们递下来了。若 `chat_loop` 递的是空值，单测照样全绿而
// `memory.recalled` 一条都不会有（观测是回退的唯一数据源，S11 §5 静默失效第 2 行）。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 对话面（`classify` 判决 / `compose` 措辞）出厂默认开启，会给每一轮多加一次静默
// 请求。本用例与对话面正交，故按约定把它钉死——请求数才可预期。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t40';

/** v2 事实源文件名（与 `plugins/session/v2_exec.rs` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 第 1 轮发言：成功收束 ⇒ 编译技能（触发串 = 它）。 */
const ASK = '把周报整理成摘要';

/**
 * 第 2 轮发言：**故意换一句**。
 *
 * 两个理由：① 步 11 的编码按**内容**同文去重（`RecallView::contains_content`）——
 * 两轮说同一句只会编出一条，那就证不了「编码逐轮都跑」；② 技能路由判的是
 * **置信度**不是「本轮输入是否匹配触发串」（S11 §7：匹配算法不在架构内），
 * 所以换一句照样进召回视图、照样判一次。换成不同的发言，两条都验得更实。
 */
const ASK2 = '再帮我看看还有什么要补充的';

/** 第 1 轮的收束发言 = 编译出的技能正文（`content = response`）。 */
const RESPONSE = '先列要点；再合并同类项；最后按重要度排序。';

/** 读某个会话的事实源（JSONL）。 */
function readEvents(homedir, sid) {
  const raw = readFileSyncSafe(join(homedir, 'session', sid, V2_WAL));
  assert(raw.length > 0, `${sid} 的事实源应存在且非空`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** 某轮用户格的 seq（记忆类事件的溯源锚，I2）。 */
function userSeqOf(events, turn) {
  const e = events.find((x) => x.kind === 'user.message' && x.turn === turn);
  assert(e, `第 ${turn} 轮的用户格应入格`);
  return e.seq;
}

export default defineCase(
  'T40 full 档收束派生事实：记忆三段 + 技能观测/编译也入格（不再随轮次事实一起被拦下）',
  async () => {
    const llm = await new MockLlm([
      // 第 1 轮：成功收束 ⇒ 编译技能。
      { id: 'turn-1', match: ASK, once: true, content: RESPONSE },
      // 第 2 轮：换一句发言 ⇒ 技能照样进召回视图、路由判定落格。
      { id: 'turn-2', content: '（第二轮）已按摘要模板处理。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 主题：full 档 + 技能编译开。对话面三项按 `DIALOG_FACE_OFF` 钉死。
        session: {
          ...DIALOG_FACE_OFF,
          v2_mode: 'full',
          skill_compile_enabled: true,
        },
      },
    });

    try {
      const turn = (message) => {
        const r = runCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          message,
          provider: PROVIDER_ID,
          session: SID,
        });
        assertEq(r.code, 0, `「${message}」退出码（stderr: ${r.stderr.slice(0, 500)}）`);
        return r;
      };

      const r1 = turn(ASK);
      assert(
        r1.stdout.includes(RESPONSE),
        `第 1 轮 stdout 应为收束发言（实际: ${JSON.stringify(r1.stdout)}）`,
      );
      turn(ASK2);

      const events = readEvents(hd.homedir, SID);
      const kinds = events.map((e) => e.kind);

      // ── ① 轮次事实：确实由 v2 原生记账（本用例的前提，不是主题）──────────
      assertEq(
        events.filter((e) => e.kind === 'user.message').length,
        2,
        `两轮各一格用户格：${JSON.stringify(kinds)}`,
      );
      assert(
        events.some((e) => e.kind === 'chat.assistant.final'),
        `轮次事实应原生入格（full 档）：${JSON.stringify(kinds)}`,
      );

      // ── ② 步 11 编码：**逐轮**一条情景记忆，锚 = 本轮用户格（I2）──────────
      const encoded = events.filter(
        (e) => e.kind === 'memory.encoded' && e.payload?.tag === '经验',
      );
      assertEq(
        encoded.length,
        2,
        `两轮各编码一条情景记忆（修复前这里恒为 0——记忆整段被拦下）：${JSON.stringify(kinds)}`,
      );
      for (const e of encoded) {
        assertEq(
          e.produced_by,
          userSeqOf(events, e.turn),
          `编码的溯源锚必须是本轮开口（turn=${e.turn}）`,
        );
      }

      // ── ③ 步 12 检索锚：第 2 轮的召回视图非空 ⇒ 一条 `memory.recalled` ────
      const anchors = events.filter(
        (e) => e.kind === 'memory.recalled' && !e.payload?.skill_id,
      );
      assert(
        anchors.length >= 1,
        `第 2 轮召回到了第 1 轮编的记忆 ⇒ 检索锚必须入格：${JSON.stringify(
          events.filter((e) => e.kind === 'memory.recalled').map((e) => e.payload),
        )}`,
      );
      assertEq(
        anchors[0].produced_by,
        userSeqOf(events, 1),
        '检索锚的溯源指向第 2 轮用户发言',
      );

      // ── ④ 步 22 观测：第 2 轮判了一次技能，`fallback` 落格 ────────────────
      const obs = events.filter(
        (e) => e.kind === 'memory.recalled' && e.payload?.skill_id,
      );
      assertEq(
        obs.length,
        1,
        `第 2 轮恰好一条技能路由观测（修复前恒为 0 ⇒ calibration 恒读「零使用」）：${JSON.stringify(
          kinds,
        )}`,
      );
      assertEq(obs[0].payload.fallback, false, '零使用的技能按置信度 1.0 判快路（不判回退）');
      assertEq(
        obs[0].produced_by,
        userSeqOf(events, 1),
        '观测的溯源锚 = 触发本次判定的那格用户发言（I2）',
      );

      // ── ⑤ 步 22 编译：**逐轮**各一条（触发串不同 ⇒ 各编一条，去重按 trigger）──
      const skills = events.filter(
        (e) => e.kind === 'memory.encoded' && e.payload?.tag === 'skill',
      );
      assertEq(
        skills.length,
        2,
        `两轮各编一条技能（修复前恒为 0——下一轮因此无技能可用）：${JSON.stringify(kinds)}`,
      );
      const s1 = skills.find((e) => e.payload.trigger === ASK);
      const s2 = skills.find((e) => e.payload.trigger === ASK2);
      assert(s1 && s2, '两条技能各以**自己那轮**的发言为触发串');
      assertEq(s1.payload.content, RESPONSE, '技能正文 = 该轮收束发言');
      assertEq(
        s1.produced_by,
        userSeqOf(events, 0),
        '第 1 轮的技能锚在第 1 轮开口（I2：技能溯源 100%）',
      );
      assertEq(
        s2.produced_by,
        userSeqOf(events, 1),
        '第 2 轮的技能锚在第 2 轮开口',
      );
      assert(
        s1.payload.skill_id && s2.payload.skill_id,
        '技能必须带 skill_id（校准账按它归并，缺了回退永远不会发生）',
      );
      assertEq(
        obs[0].payload.skill_id,
        s1.payload.skill_id,
        '第 2 轮判的是**第 1 轮**编的那条技能（本轮的编译发生在判定之后）',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
