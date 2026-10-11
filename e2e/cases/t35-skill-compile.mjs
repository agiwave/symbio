import './_selfrun.mjs';
// T35 技能编译与召回（`SkillCompiler` + 召回视图，S9 第 22 步，S11）：
// 一轮**成功**收束 ⇒ 编译出一条技能（`memory.encoded{tag:"skill"}`）；下一轮技能
// 进召回视图、注入段带上技能正文；换一个进程（B 会话）说同一句话 ⇒ 召回视图里
// 照样认得它（跨进程稳定）；校准列从出口读得出这次编译被用了两次。
//
// ## 本用例钉的是什么
//
// [11 批 2 ③](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的接线判据：
// 「每件接线一条 e2e（**接线前后行为可见地不同**，否则等于没接）」——这里钉的是
// `SkillCompiler`：写方挂在 `v2_facts::record_learning`（`v2_exec` 轮末）上，
// 读方挂在召回视图（注入段的原料）上。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 编译**真的发生了** | 下一轮请求包的注入段里带上技能正文（编译前那一轮没有） |
// | 技能**真的进得了召回视图** | 发给大模型的请求包里，注入段带上技能正文 |
// | 同一个 trigger 在**另一个进程**里也编得出来、也召得到 | B 会话（独立进程）第二轮的注入段里照样有技能正文 |
// | 校准列真的在读这份事实源 | gateway `session/stats` 的 `calibration.by_skill` |
//
// ## 为什么必须端到端（单测证不了的两件事）
//
// 单测（`v2_skills.test.rs`）直接调 `compile` / 召回，把两端的出口判据钉死了，
// 但它证不了这条链路**真的接在真实轮次上**：
//
// 1. **`actor` 归位是生产链路的生死线**：`SkillCompiler::compile` 造的信封 `actor`
//    是占位 `agent:main`，而召回投影按 `actor == viewer` 过滤（`thread_private`）。
//    写方不把它归位成属主（`user`），技能就被挡在召回视图外 ⇒ 注入段恒不带技能
//    （「编译了却从不生效」的静默失效）。这一条只有在**真实轮次**里跑得出来。
// 2. **注入段真的被改写**：摘技能 / 换段都发生在请求装配里，只有回读**发给大模型
//    的请求包**才看得见这一层。
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
  textOf,
  assertTranscriptInvariants,
  readSessionMessages,
} from '../helpers.mjs';

/** A / B 两个会话共用一个 homedir ⇒ 共用一个会话存储根（B 用来验技能跨进程稳定）。 */
const SID_A = 'e2e-t35-a';
const SID_B = 'e2e-t35-b';

/** 注入段抬头（与 `v2_memory.rs::RECALL_SECTION_HEAD` 同一常量口径） */
const HEAD = '【长期记忆】';

/** 第 1 轮：成功收束 ⇒ 编译技能。触发串即用户发言（`trigger = user_text`）。 */
const TRIGGER = '把周报整理成摘要';

/** 技能正文 = 第 1 轮的收束发言（`content = response`）。注入段里认得它。 */
const RESPONSE = '先列要点；再合并同类项；最后按重要度排序。';

/** 第 2 / 3 轮（A）与 B 的两轮：各跑一轮，让技能被召回（判据是「注入段里有它」）。 */
const R2 = '再帮我做一次';
const R3 = '继续处理一下';

/**
 * 请求包里带记忆抬头的那条（没有 ⇒ null）。
 *
 * ⚠️ 判据是「**含**抬头」而非「以抬头开头」——记忆段是 `chat_loop::request_view_prefix`
 * 拼进置顶前缀消息里的一段；`startsWith` 要求那条消息**整个**以抬头开头，前面多出
 * 任何东西就整条失配，症状是「技能没进召回视图」。理由与 t29 的同名函数同源。
 */
function recallMessage(req) {
  return (req?.body?.messages ?? []).find((m) => textOf(m).includes(HEAD)) ?? null;
}

/**
 * 请求包里**记忆段那一截**的文本（按 `HEAD` 起到下一个空行为止）。
 *
 * ## 为什么必须切段，不能拿整条消息断言
 *
 * 发给模型的请求里，`chat_loop::request_view_prefix` 把请求视图层
 * 置顶的三段拼成**一条** `system` 消息（排在消息数组最前），其后是
 * `Reasoner::render_messages` 投影出的对话转写与本轮发言。于是同一批内容**在注入段
 * 与转写里各出现一次**。
 *
 * 而这几段的内容**本来就在对话转写里出现过**——技能正文是上一轮的**收束发言**、
 * trigger 是上一轮的**用户发言**，它们作为历史必然重现在 `<对话历史>` 里。所以
 * 「技能正文在不在注入段里」这类断言若拿整条消息去 `includes`，**恒真**，
 * 与接线是否生效无关。
 *
 * ## 切法：空行
 *
 * 三段之间由 `request_view_prefix` 用**空行**分隔（`parts.join("\n\n")`），而
 * `v2_memory::prompt_section` 产出的段内只有 `\n- ` 行、没有空行 ⇒ 首个空行
 * 就是本段的结束。比按「下一个抬头」切更稳：抬头是可改的展示文案，空行是分隔约定。
 */
function recallSectionText(req) {
  const all = textOf((req?.body?.messages ?? []).find((m) => textOf(m).includes(HEAD)));
  const at = all.indexOf(HEAD);
  if (at < 0) return '';
  const blank = all.indexOf('\n\n', at);
  return blank < 0 ? all.slice(at) : all.slice(at, blank);
}

/**
 * 校准列里**恰好被用了 `uses` 次**的那条技能 id（没有 ⇒ `null`）。
 *
 * ## 为什么按「次数」找而不按下标取
 *
 * `by_skill` 是 `BTreeMap` ⇒ 键是**字典序**（`skill_id` 是 FNV 哈希，与编译顺序、
 * 与 trigger 的语义都无关）。按下标取「第 0 条」等于赌哈希序，两条技能一换就红。
 * 而本用例要判的正是「TRIGGER 那条的账目」——它在生产读出口里**唯一可辨识**的
 * 特征就是「用了 2 次」：编它的那一轮还没有它（0 次），后两轮各召回一次（2 次）。
 */
function findSkillIdByUses(stats, uses) {
  return (
    Object.keys(stats.calibration.by_skill).find(
      (id) => stats.calibration.by_skill[id].uses === uses,
    ) ?? null
  );
}

export default defineCase(
  'T35 技能编译与召回：成功收束编译技能 → 注入段带上技能正文 → 跨进程召得到 → 校准列计数',
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
        // 技能编译挂在这条路上（`v2_facts::record_learning` 的步 22）。
        // `skill_compile_enabled` **显式打开**：它默认 off（S11 §4 平凡值
        // 「不编译，只检索」），本用例验的正是打开之后的行为，不能依赖默认值。
        session: { ...DIALOG_FACE_OFF, skill_compile_enabled: true },
      },
    });

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
      // ── A 会话：第 1 轮编译，第 2 / 3 轮各召回一次 ───────────────────────
      turn(TRIGGER, SID_A);
      turn(R2, SID_A);
      turn(R3, SID_A);

      // ── B 会话（**另一个进程**）：同一句话 ⇒ 自己也编一条、也召得到 ────────
      turn(TRIGGER, SID_B);
      turn(R2, SID_B);

      const reqs = await llm.requests();
      assertEq(reqs.length, 5, 'A 三轮 + B 两轮，各一次请求（对话面已关，注入不加请求）');

      // ── 证据 ①：编译真的发生了——第 1 轮没有注入段，第 2 年起就有了技能正文 ──
      assert(
        recallMessage(reqs[0]) === null,
        '第 1 轮没有可召回的记忆 ⇒ 整段省略（不印空壳标题）',
      );
      const section2 = recallSectionText(reqs[1]);
      assert(
        recallMessage(reqs[1]) !== null,
        `第 2 轮请求包应有注入段（技能进了召回视图，段就不该是空）`,
      );
      assert(
        section2.includes(RESPONSE),
        `技能正文应出现在注入段里（ actor 归位后技能才进得了召回视图）：\n${section2}`,
      );
      // 第 3 轮照旧（校准没掉 ⇒ 技能还在视图里）。
      assert(
        recallSectionText(reqs[2]).includes(RESPONSE),
        `第 3 轮注入段应仍带技能正文：\n${recallSectionText(reqs[2])}`,
      );

      // ── 证据 ②：跨进程稳定——B 会话（独立进程）也说同一句话 ⇒ 注入段照样有 ──
      // `skill_id_of` 若用进程内的 `DefaultHasher`，两个进程各算各的 ⇒ 技能永远
      // 「零使用」、注入段恒不带技能正文。B 的第 2 轮注入段带着它，就是跨进程稳定
      // 的用户可见形态。
      const sectionB = recallSectionText(reqs[4]);
      assert(
        sectionB.includes(RESPONSE),
        `B 会话（另一个进程）第 2 轮的注入段应带技能正文（同一 trigger 编出的技能跨进程认得出）：\n${sectionB}`,
      );

      // ── 证据 ③：出口 `session/stats` 的校准列认这个技能（读侧出口）────────
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID_A,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady(20_000, 'T35 读回进程');
        const inv = await api.invoke('session/stats', {}, { session_id: SID_A });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats?.has_wal, '跑过三轮 ⇒ 事实源应在');

        // ## 校准列里有几条技能？（判据是「TRIGGER 那条的账目」，不是「总共几条」）
        //
        // `compile` 的去重判据是**同一 trigger 至多一条**（`v2_skills::compile` 的文
        // 档），**不是**「一个会话至多一条」——每轮成功收束都按本轮用户发言编一条是
        // 设计（技能集按轨迹生长）。所以这里数「编了几条」是**测错的东西**：A 跑了
        // TRIGGER / R2 / R3 三轮成功轮，`memory.encoded{tag:"skill"}` 就有三条
        // （trigger 分别是这三句），只是只有被**召回判过**的那些才在校准列里有账。
        //
        // 本用例真正要钉的是「同一 trigger 去重」——它的可见形态是：TRIGGER 那条技能
        // 的账目恰好是「第 2 / 3 轮各用一次」，**不多不少**。多一条就是重复编译
        // （去重失效），少一条就是召回漏判。用 `uses` 而不是「条数」判，判据才落在
        // 这条技能自己身上。
        const skillId = findSkillIdByUses(stats, 2);
        assert(
          skillId,
          `应有一条恰好用 2 次的技能（TRIGGER 那条：第 2 / 3 轮各召回一次；同一 trigger 若被重复编译，它的账目会被拆成两条各 1 次 ⇒ 这里就找不到。实际: ${JSON.stringify(stats.calibration)}）`,
        );
        const one = stats.calibration.by_skill[skillId];
        // 逐字段比（`serde_json::Value` 的键序是字典序，字面量直接比会假红）。
        assertEq(one.skill_id, skillId, '条目自带 skill_id（投影的口径）');
        assertEq(one.uses, 2, '第 2 / 3 轮各用一次（编译那一轮还没有技能）');
        assertEq(one.fallbacks, 0, '置信度恒 1.0 ⇒ 不走回退');

        // ── 请求级注入 ⇒ 会话存储里一个字都不该多（与 T29 同口径）────────────
        const msgsA = await readSessionMessages(api, SID_A);
        assert(
          !msgsA.some((m) => textOf(m).includes(HEAD)),
          '注入段不得写进会话转写（请求级、与 nudge 同一机制）',
        );
        assertTranscriptInvariants(msgsA, 'T35(A)');
        assertTranscriptInvariants(await readSessionMessages(api, SID_B), 'T35(B)');
      } finally {
        api.stop();
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
