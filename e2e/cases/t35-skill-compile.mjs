import './_selfrun.mjs';
// T35 技能编译与路由（`SkillCompiler` + `SkillRouter`，S9 第 22 步，S11）：
// 一轮**成功**收束 ⇒ 编译出一条技能（`memory.encoded{tag:"skill"}`）；下一轮技能
// 进召回视图、按置信度闸判走**快路**；校准一掉 ⇒ 同一技能被**摘出**召回视图。
//
// ## 本用例钉的是什么
//
// [11 批 2 ③](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的接线判据：
// 「每件接线一条 e2e（**接线前后行为可见地不同**，否则等于没接）」——这里钉的是
// `SkillCompiler` / `SkillRouter`：写方挂在 `v2_facts::record` 的收束转写上，
// 读方挂在 `chat_loop::inputs::prepare_turn_inputs` 的召回之后，闸立在
// `calibration` 投影的置信度上。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 编译**真的发生在成功收束上** | 事实源里 `memory.encoded{tag:"skill"}`（+ 兜底轮不编） |
// | 技能**真的进得了召回视图** | 发给大模型的请求包里，注入段带上技能正文（`actor` 归位） |
// | 路由**真的在判**，且判据是校准 | 校准一变 ⇒ 同一技能从注入段里**消失** |
//
// ## 为什么必须端到端（单测证不了的三件事）
//
// 单测（`v2_skills.test.rs`）直接调 `compile` / `route`，把两端的出口判据钉死了，
// 但它证不了这条链路**真的接在真实轮次上**：
//
// 1. **`actor` 归位是生产链路的生死线**：`SkillCompiler::compile` 造的信封 `actor`
//    是占位 `agent:main`，而召回投影按 `actor == viewer` 过滤（`thread_private`）。
//    写方不把它归位成属主（`user`），技能就被挡在召回视图外 ⇒ `route` 恒快判返回、
//    观测永不落格、校准账零使用——整条链路**接好了却永远不跑**（S11 §5 静默失效）。
//    这一条只有在**真实轮次**里跑得出来：单测可以手工把 `actor` 设对，测不出写方有没有做。
// 2. **观测真的落格、校准真的累计**：`route` 的返回值要在**轮末收束**时落成
//    `memory.recalled{skill_id, fallback}`（`v2_bridge` 的步 22），下一轮的置信度
//    才判得动。这是「取视图那一刻」与「收束入格」两处的接线，单测各证一半。
// 3. **注入段真的被改写**：`route` 摘技能改的是 `turn.recall_view`，而注入段
//    （`prompt_section`）在它**之后**才排——顺序反了，摘了也白摘（模型照样看得见）。
//    只有回读**发给大模型的请求包**才看得见这一层。
//
// ## 判据分列（一个规则一个判定方）
//
// - **编译时机**：判定方是 `v2_facts::record` 的 `success_text`（只有 `Final` 收束有），
//   本用例只数条数（成功轮 ⇒ 一条，重复 trigger ⇒ 不再编）；
// - **置信度闸**：判定方是 core 的 `SkillRouter::route(confidence, 0.8)`，本用例只读
//   它**判完之后的落格**（`fallback` 标记）与**视图的增减**（注入段在不在）；
// - **校准归并**：判定方是 `calibration` 投影（按 `skill_id` 归并带 `fallback` 的事件），
//   本用例从出口 `session/stats` 读它，与事实源逐字对账。
//
// ## 为什么第三轮前要「对事实源动一刀」
//
// 置信度 = 1 − 回退率，而 `route` 只在**置信度低于阈值**时才判 `ReasonerFallback`——
// 这是个自指的环：首轮技能零使用 ⇒ 置信度 1.0 ⇒ 判快路 ⇒ 观测恒 `fallback:false`
// ⇒ 置信度永远是 1.0，回退**在生产里一次都不会自发发生**（除非别处先注入一条回退）。
// 所以第三轮前手工注入**一条** `fallback:true` 的观测（形状照抄第二轮真实落的那条，
// 只翻一个布尔）⇒ 置信度 0.5 < 0.8 ⇒ 第三轮 `route` 必须判回退、把技能摘出视图。
// 这一刀钉的是「闸真的在读校准」——注入前技能在段里、注入后技能不在段里，
// 唯一变量就是那条校准观测（不是常数，也不是别的东西）。
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
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** A / B 两个会话共用一个 homedir ⇒ 共用一个会话存储根（B 用来验 skill_id 跨进程稳定）。 */
const SID_A = 'e2e-t35-a';
const SID_B = 'e2e-t35-b';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 注入段抬头（与 `v2_memory.rs::RECALL_SECTION_HEAD` 同一常量口径） */
const HEAD = '【长期记忆】';

/** 技能标签（与 `v2_skills.rs::SKILL_TAG` / core `SkillCompiler` 写死的字面量同口径） */
const SKILL_TAG = 'skill';

/** 第 1 轮：成功收束 ⇒ 编译技能。触发串即用户发言（`trigger = user_text`）。 */
const TRIGGER = '把周报整理成摘要';

/** 技能正文 = 第 1 轮的收束发言（`content = response`）。注入段里认得它。 */
const RESPONSE = '先列要点；再合并同类项；最后按重要度排序。';

/** 第 2 / 3 轮：各跑一轮，让技能被召回两次（判据分别是「用」与「回退」）。 */
const R2 = '再帮我做一次';
const R3 = '继续处理一下';

/** 读一个会话的全部事实（出口读数的对账基准）。 */
function readEvents(homedir, sid) {
  const raw = readFileSyncSafe(join(homedir, 'session', sid, V2_WAL));
  assert(raw.length > 0, `${sid} 的事实源应存在且非空`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** 请求包里一条消息的正文（string / 分片数组两种形态都认）。 */
function textOf(m) {
  if (typeof m?.content === 'string') return m.content;
  if (Array.isArray(m?.content)) return m.content.map((p) => p?.text ?? '').join('');
  return '';
}

/**
 * 请求包里带记忆抬头的那条（没有 ⇒ null）。
 *
 * ⚠️ 判据是「**含**抬头」而非「以抬头开头」——`full` 档（出厂）下整段请求被渲染成
 * **一条** user 消息（`ProviderLlmAdapter::generate_turn`），记忆段拼在它里面；`startsWith`
 * 要求那条消息**整个**以抬头开头，前面多出任何东西就整条失配，症状是「技能没进召回
 * 视图」。理由与 t29 的同名函数同源。
 */
function recallMessage(req) {
  return (req?.body?.messages ?? []).find((m) => textOf(m).includes(HEAD)) ?? null;
}

/**
 * 请求包里**记忆段那一截**的文本（按 `HEAD` 起到下一个空行为止）。
 *
 * ## 为什么必须切段，不能拿整条消息断言
 *
 * 出厂档位（`full`）下发给模型的是**一条** user 消息：`chat_loop::request_view_prefix`
 * 把请求视图层置顶的三段拼成前缀，`Reasoner::render_prompt` 的对话转写接在它后面。
 * 于是同一条消息里同时有「记忆段 / 能力目录 / 对话历史 / 本轮发言」。
 *
 * 而这三段的内容**本来就在对话转写里出现过**——技能正文是上一轮的**收束发言**、
 * trigger 是上一轮的**用户发言**，它们作为历史必然重现在 `<对话历史>` 里。所以
 * 「技能正文被摘出注入段」这类断言若拿整条消息去 `includes`，**恒真**，
 * 与接线是否生效无关。
 *
 * ## 切法：空行
 *
 * 三段之间由 `request_view_prefix` 用**空行**分隔（`parts.join("\n\n")`），
 * 而 `v2_memory::prompt_section` 产出的段内只有 `\n- ` 行、没有空行 ⇒ 首个空行
 * 就是本段的结束。比按「下一个抬头」切更稳：抬头是可改的展示文案，空行是分隔约定。
 */
function recallSectionText(req) {
  const all = textOf((req?.body?.messages ?? []).find((m) => textOf(m).includes(HEAD)));
  const at = all.indexOf(HEAD);
  if (at < 0) return '';
  const blank = all.indexOf('\n\n', at);
  return blank < 0 ? all.slice(at) : all.slice(at, blank);
}

/** 事实源里带 `tag:"skill"` 的记忆事件（技能）。 */
function skillEvents(homedir, sid) {
  return readEvents(homedir, sid).filter(
    (e) => e.kind === 'memory.encoded' && e.payload?.tag === SKILL_TAG,
  );
}

export default defineCase(
  'T35 技能编译与路由：成功收束编译技能 → 召回按校准判快路 → 校准掉则摘出视图',
  async () => {
    const llm = await new MockLlm([
      // 第 1 轮：成功收束（正文即被固化的技能内容）。
      { id: 'skill', match: TRIGGER, content: RESPONSE },
      { id: 'r2', match: R2, content: '好的，已按上次的方式处理。' },
      { id: 'r3', match: R3, content: '没问题，继续处理中。' },
      { id: 'rest', content: '收到。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 无此文件时网关回退默认配置（`inbound_enabled: false`）——出口就打不到
        // （本用例末尾从生产路由读 `session/stats`，与 T28 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // `bridge` 档：每轮收束转写进 `<会话目录>/v2-events.wal`——技能编译挂在
        // 这条路上。`skill_compile_enabled` **显式打开**：它默认 off（S11 §4 平凡值
        // 「不编译，只检索」），本用例验的正是打开之后的行为，不能依赖默认值。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'full', skill_compile_enabled: true },
      },
    });

    const walA = join(hd.homedir, 'session', SID_A, V2_WAL);
    const turn = (message, session) => {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message,
        provider: PROVIDER_ID,
        session,
      });
      assertEq(r.code, 0, `${session}「${message}」退出码（stderr: ${r.stderr.slice(0, 500)}）`);
    };

    try {
      // ── 第 1 轮：成功收束 ⇒ 编译出一条技能 ────────────────────────────────
      turn(TRIGGER, SID_A);
      const a1 = readEvents(hd.homedir, SID_A);
      const skills1 = skillEvents(hd.homedir, SID_A);
      assertEq(
        skills1.length,
        1,
        `成功轮 ⇒ 恰好一条技能（实际 kinds: ${a1.map((e) => `${e.kind}/${e.payload?.tag ?? ''}`).join(', ')}）`,
      );
      const skill = skills1[0];
      assertEq(skill.entity, 'Memory', '技能格实体坐标（技能是记忆的一种，不是特殊类型）');
      assertEq(skill.verb, 'Opened', '技能格动词坐标（memory × opened）');
      assertEq(skill.payload.trigger, TRIGGER, 'trigger = 本轮用户发言（编译的输入）');
      assertEq(skill.payload.content, RESPONSE, 'content = 本轮收束正文（被固化的套路）');
      assert(
        typeof skill.payload.skill_id === 'string' && skill.payload.skill_id.length > 0,
        `技能必带 skill_id（校准账的归并键）：${JSON.stringify(skill.payload)}`,
      );
      // ★ 本用例最重要的一条：`actor` 必须归位成**属主**，不是 core 造的占位 `agent:main`。
      // 召回投影按 `actor == viewer` 过滤（`thread_private`）⇒ 归位没做，技能进不了
      // 召回视图，下面第三轮「技能出现在注入段里」会当场失败——这正是要钉的接线。
      assertEq(
        skill.actor,
        'user',
        '技能事件的 actor = 属主（memory × * 的 actor 是属主不是作者；占位不归位 = 链路静默失效）',
      );
      const u1 = a1.find((e) => e.kind === 'user.message' && e.turn === 0);
      assert(u1, '第 1 轮用户格应在');
      assertEq(skill.produced_by, u1.seq, '技能溯源指向源轨迹（I2：技能溯源 100%）');
      assertEq(skill.turn, 0, '技能记在第 1 轮上（写方落格前归位轮号）');
      assert(skill.ts > 0, '技能事件填真实编码时刻');

      // 同轮还落了一条**普通**记忆（`tag:"经验"`）——两者同格子、不同标签，别混为一谈。
      const exp1 = a1.filter((e) => e.kind === 'memory.encoded' && e.payload?.tag === '经验');
      assertEq(exp1.length, 1, '同一轮另有一条情景记忆（技能不顶替它）');
      assert(
        exp1[0].seq !== skill.seq,
        '技能与情景记忆是两条独立事实（技能不是"改标签"改出来的）',
      );

      // ── 第 2 轮：技能进召回视图 ⇒ 判快路 ⇒ 注入段带上技能正文 ──────────────
      turn(R2, SID_A);
      const a2 = readEvents(hd.homedir, SID_A);
      const skillId = skill.payload.skill_id;
      const obs2 = a2.filter(
        (e) => e.kind === 'memory.recalled' && e.payload?.skill_id === skillId,
      );
      assertEq(
        obs2.length,
        1,
        `零使用的技能按置信度 1.0 判快路 ⇒ 恰好一条路由观测（实际: ${JSON.stringify(
          a2.filter((e) => e.kind === 'memory.recalled').map((e) => e.payload),
        )}）`,
      );
      assertEq(obs2[0].payload.fallback, false, '置信度 1.0 ≥ 0.8 ⇒ 走技能快路（不判回退）');
      assertEq(
        obs2[0].produced_by,
        a2.find((e) => e.kind === 'user.message' && e.turn === 1)?.seq,
        '观测溯源锚 = 触发本次判定的那格用户发言（I2）',
      );

      let reqs = await llm.requests();
      assertEq(reqs.length, 2, '前两轮各一次请求（对话面已关，注入不加请求）');
      const section2 = recallMessage(reqs[1]);
      assert(
        section2,
        `第 2 轮请求包应有注入段（技能进了召回视图，段就不该是空）`,
      );
      assert(
        recallSectionText(reqs[1]).includes(RESPONSE),
        `技能正文应出现在注入段里（actor 归位后技能才进得了召回视图）：\n${recallSectionText(reqs[1])}`,
      );
      assert(
        recallMessage(reqs[0]) === null,
        '第 1 轮没有可召回的记忆 ⇒ 整段省略（不印空壳标题）',
      );

      // ── 动一刀：注入一条**回退**观测（形状照抄第 2 轮真实落的那条）──────────
      // 置信度 1.0 → 0.5（1 用 1 回退）⇒ 低于阈值 0.8 ⇒ 第 3 轮必须判回退。
      const lines = readFileSyncSafe(walA).split('\n').filter(Boolean);
      const parsed = lines.map((l) => JSON.parse(l));
      const realObs = parsed.find(
        (e) => e.kind === 'memory.recalled' && e.payload?.skill_id === skillId,
      );
      assert(realObs, '注入要以真实观测为蓝本（行形状照抄，只翻一个布尔）');
      const injected = {
        ...realObs,
        event_id: `obs-injected-${skillId}`,
        seq: Math.max(...parsed.map((e) => e.seq ?? -1)) + 1,
        payload: { ...realObs.payload, fallback: true },
      };
      writeFileSync(walA, `${lines.join('\n')}\n${JSON.stringify(injected)}\n`, 'utf8');

      // ── 第 3 轮：校准掉下来 ⇒ 同一技能被**摘出**召回视图 ────────────────────
      turn(R3, SID_A);
      const a3 = readEvents(hd.homedir, SID_A);
      const obs3 = a3.filter(
        (e) => e.kind === 'memory.recalled' && e.payload?.skill_id === skillId,
      );
      // 三条：注入的那条 + 第 2 轮真实那条 + 第 3 轮刚落的这条。
      assertEq(obs3.length, 3, `注入 1 + 第 2 轮 1 + 第 3 轮 1 = 3 条观测（实际 ${obs3.length}）`);
      const last3 = obs3[obs3.length - 1];
      assertEq(
        last3.payload.fallback,
        true,
        '置信度 0.5 < 0.8 ⇒ 第 3 轮必须判**回退**（不是恒快路）',
      );
      assertEq(
        last3.produced_by,
        a3.find((e) => e.kind === 'user.message' && e.turn === 2)?.seq,
        '第 3 轮的观测锚在本轮用户发言上',
      );

      reqs = await llm.requests();
      const section3 = recallMessage(reqs[2]);
      assert(section3, '第 3 轮请求包仍应有注入段（还有别的记忆可召回）');
      // ⚠️ 两条都只看**记忆段那一截**：`RESPONSE` / `TRIGGER` 是上一轮的收束与
      // 发言，作为对话历史必然重现在同一条消息里——拿整条消息断言恒真，与接线无关。
      assert(
        !recallSectionText(reqs[2]).includes(RESPONSE),
        `技能被摘出视图 ⇒ 注入段不得再带技能正文（这正是"接线前后行为可见地不同"）：\n${recallSectionText(reqs[2])}`,
      );
      assert(
        recallSectionText(reqs[2]).includes(TRIGGER),
        `摘的是**技能**那一条，别的记忆原样在段里（不是把整段清空）：\n${recallSectionText(reqs[2])}`,
      );

      // ── 跨进程稳定：B 会话（**另一个进程**）编同一个 trigger ⇒ 同一个 skill_id ──
      // `skill_id_of` 若用进程内的 `DefaultHasher`，两个进程各算各的 ⇒ 校准账按
      // skill_id 归并就永远归不到一起（技能永远"零使用"，阈值永远不触发）。
      turn(TRIGGER, SID_B);
      const skillsB = skillEvents(hd.homedir, SID_B);
      assertEq(skillsB.length, 1, 'B 会话自己的事实源里也编出了一条技能（同文去重只在本会话内）');
      assertEq(
        skillsB[0].payload.skill_id,
        skillId,
        '同一个 trigger 在两个进程里编出同一个 skill_id（FNV-1a 跨进程稳定，不是进程内哈希）',
      );

      reqs = await llm.requests();
      assertEq(reqs.length, 4, '四个 runCli 轮次各一次请求');

      // ── 出口：`session/stats` 的校准列必须认这个 skill_id（读侧出口对账）─────
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID_A,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();
        const inv = await api.invoke('session/stats', {}, { session_id: SID_A });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats?.has_wal, '跑过三轮 ⇒ 事实源应在');

        const one = stats.calibration.by_skill[skillId];
        assert(
          one,
          `校准列应认下这个技能（实际: ${JSON.stringify(stats.calibration)}）`,
        );
        // 逐字段比（`serde_json::Value` 的键序是字典序，字面量直接比会假红）。
        assertEq(one.skill_id, skillId, '条目自带 skill_id（投影的口径）');
        assertEq(one.uses, 3, '3 次使用：第 2 轮快路 1 + 注入回退 1 + 第 3 轮回退 1');
        assertEq(one.fallbacks, 2, '2 次回退：注入 1 + 第 3 轮 1');
      } finally {
        api.stop();
      }

      // ── 请求级注入 ⇒ 会话存储里一个字都不该多（与 T29 同口径）──────────────
      const msgsA = readMessagesJson(hd.homedir, SID_A);
      assert(
        !msgsA.some((m) => textOf(m).includes(HEAD)),
        '注入段不得写进会话存储（请求级、与 nudge 同一机制）',
      );
      assertTranscriptInvariants(msgsA, 'T35(A)');
      assertTranscriptInvariants(readMessagesJson(hd.homedir, SID_B), 'T35(B)');
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
