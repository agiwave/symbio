import './_selfrun.mjs';
// T41 技能快路的**执行半边**（S11 §2–§3、§6.2，`skill_fast_path`）：
// 同一句话说第二遍 ⇒ 命中已编译技能 ⇒ **一次模型调用都不发生**，以技能正文收束。
//
// ## 本用例钉的是什么
//
// T35 钉的是「编译 → 观测 → 校准 → 回退」这条**读侧**链路（低置信技能被摘出注入段）；
// 本用例钉的是同一条判定的**第二个后果**：判成快路的技能被拿去**执行**。
// 在此之前 `SkillRoute::SkillFastPath` 只作用在提示词上——判定被算出来，却从不产生
// 后果（`plan/04 §3.1` S9·215 行明写「快路的执行半边不在本批」）。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 命中 ⇒ **没调模型** | mock-llm 的请求记录（这一条别处都证不了） |
// | 命中 ⇒ 以**技能正文**收束 | 收束格的 `text`（不是模型这次会说的话） |
// | 命中 ⇒ 走**反射档** | 开轮格的 `tier = reflex` + 收束格 `cost_ms` 在 80ms 预算内 |
// | 未命中 ⇒ 照常推理 | 第三轮（另一句话）请求数照增、`tier = deep` |
//
// ## 为什么必须端到端
//
// 单测（`v2_exec.test.rs::skill_hit_takes_the_reflex_tier_without_any_model_call`）
// 已经把「命中集非空 ⇒ 不调模型」钉死了，但它证不了这条链路**真的接在真实轮次上**：
//
// 1. **可用集真的被填**：`skill_hits` 由 `prepare_turn_inputs` 的 `fast_armed` 分支
//    从 `route` 的出参取——开关、档位、`tool_rounds == 0` 三个条件任一写错，集合恒空，
//    而单测是手工把命中集塞进去的；
// 2. **命中判据与写方同源**：写方存的 `trigger` 是截断后的用户发言，读方拿原文比
//    就永远命不中（没有任何报错，表现为"技能编了却从不生效"）；
// 3. **技能真的进得了召回视图**：`actor` 归位、`route` 判得动——T35 证过读侧，
//    这里证的是它同时喂到了执行侧。
//
// ## 为什么第三轮是**必须**的
//
// 只有前两轮的话，「请求数没涨」还能被解释成"这个档位根本不调模型"。第三轮换一句话
// （不命中）⇒ 请求数照增，把"命中才跳过"与"整档不调模型"分开。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
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

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 技能标签（与 `v2_skills.rs::SKILL_TAG` 同一常量口径） */
const SKILL_TAG = 'skill';

/** 第 1 / 2 轮：同一句发言（第 2 轮即命中）。 */
const TRIGGER = '把周报整理成摘要';

/** 技能正文 = 第 1 轮的收束发言（`content = response`）。第 2 轮必须原样收束它。 */
const RESPONSE = '先列要点；再合并同类项；最后按重要度排序。';

/** 第 3 轮：另一句话（不命中）——用来把"命中才跳过"与"整档不调模型"分开。 */
const OTHER = '再帮我做一次';
const OTHER_REPLY = '好的，已按上次的方式处理。';

const SID = 'e2e-t41';

/** 读该会话的全部事实。 */
function readEvents(homedir, sid) {
  const raw = readFileSyncSafe(join(homedir, 'session', sid, V2_WAL));
  assert(raw.length > 0, `${sid} 的事实源应存在且非空`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** 第 `turn` 轮的开轮格 / 收束格。 */
function cells(events, turn) {
  return {
    open: events.find((e) => e.kind === 'user.message' && e.turn === turn),
    close: events.find(
      (e) => (e.kind === 'chat.assistant.final' || e.kind === 'chat.assistant.fallback') && e.turn === turn,
    ),
  };
}

export default defineCase(
  'T41 技能快路：命中已编译技能 ⇒ 不调模型、以技能正文收束、走反射档',
  async () => {
    const llm = await new MockLlm([
      // 顺序即优先级：`match` 是对**最后一条 user 消息**的子串匹配，而 `full` 档的
      // prompt 是整段转写（含召回注入段）——第 3 轮的 prompt 里**同时**含 `TRIGGER`
      // （注入段里那条记忆的正文）与 `OTHER`（本轮发言）。更具体的那个必须排在前面，
      // 否则第 3 轮永远命中 `TRIGGER` 的场景（表现为"未命中却答了技能正文"）。
      { id: 'other', match: OTHER, content: OTHER_REPLY },
      // 第 1 轮：成功收束（正文即被固化的技能内容）。
      { id: 'skill', match: TRIGGER, content: RESPONSE },
      { id: 'rest', content: '收到。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // `full` 档：轮次由 v2 运行器原生执行——"跳过模型"跳的正是它里面那一次调用。
        // 两个开关都**显式打开**：它们默认 off（S11 §4 的平凡值），本用例验的正是
        // 打开之后的行为，不能依赖默认值。
        session: {
          ...DIALOG_FACE_OFF,
          v2_mode: 'full',
          skill_compile_enabled: true,
          skill_fast_path: true,
        },
      },
    });

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

    try {
      // ── 第 1 轮：成功收束 ⇒ 编译技能，本轮照常调模型 ─────────────────────
      turn(TRIGGER);
      let reqs = await llm.requests();
      assertEq(reqs.length, 1, '第 1 轮照常请求模型（还没有技能可用）');

      let events = readEvents(hd.homedir, SID);
      const skills = events.filter((e) => e.kind === 'memory.encoded' && e.payload?.tag === SKILL_TAG);
      assertEq(skills.length, 1, `成功轮 ⇒ 恰好一条技能（实际 kinds: ${events.map((e) => e.kind).join(', ')}）`);
      const skillId = skills[0].payload.skill_id;
      assertEq(skills[0].payload.trigger, TRIGGER, 'trigger = 本轮用户发言（命中判据比的就是它）');
      assertEq(skills[0].payload.content, RESPONSE, 'content = 本轮收束正文（命中后直接收束的那段）');

      const t1 = cells(events, 0);
      assert(t1.open && t1.close, '第 1 轮的开轮格与收束格都应在');
      assertEq(t1.open.payload.tier, 'deep', '第 1 轮未命中 ⇒ 深度档');
      assertEq(t1.close.payload.text, RESPONSE, '第 1 轮的收束正文来自模型');

      // ── 第 2 轮：同一句话 ⇒ 命中技能 ⇒ **一次模型调用都不发生** ─────────────
      const r2 = turn(TRIGGER);
      reqs = await llm.requests();
      assertEq(
        reqs.length,
        1,
        `命中技能 ⇒ 请求数不增（这是"跳过模型"最硬的判据；实际 ${reqs.length} 次）`,
      );
      assert(
        r2.stdout.includes(RESPONSE),
        `命中即以技能正文收束（stdout 应含技能正文）：${r2.stdout.slice(0, 300)}`,
      );

      events = readEvents(hd.homedir, SID);
      const t2 = cells(events, 1);
      assert(t2.open && t2.close, '第 2 轮的开轮格与收束格都应在');
      assertEq(
        t2.open.payload.tier,
        'reflex',
        '命中 ⇒ 开轮格声明反射档（档位由令牌类型给出，不是调用方填的字符串）',
      );
      assertEq(t2.close.payload.text, RESPONSE, '收束正文 = 技能正文（不是模型这次会说的话）');
      assertEq(t2.close.payload.model, 'reflex', '载荷的 `model` 记产者：反射档没有模型');
      assert(
        t2.close.cost_ms <= 80,
        `命中后耗时必须在反射档预算（80ms）内——S11 §6.2「命中显著低于未命中」：${t2.close.cost_ms}`,
      );
      assertEq(
        t2.close.produced_by,
        t2.open.seq,
        '收束溯源指向本轮开轮格（I2；与真实路径同一份纪律）',
      );
      // 命中轮仍走同一段收束派生事实（记忆与学习）——两条路只是"生成那一格"不同。
      const obs2 = events.filter(
        (e) => e.kind === 'memory.recalled' && e.payload?.skill_id === skillId,
      );
      assertEq(obs2.length, 1, `命中轮照常落路由观测（实际 ${obs2.length} 条）`);
      assertEq(obs2[0].payload.fallback, false, '零使用 ⇒ 判快路');

      // ── 第 3 轮：另一句话（不命中）⇒ 照常推理 ─────────────────────────────
      turn(OTHER);
      reqs = await llm.requests();
      assertEq(
        reqs.length,
        2,
        `未命中照常请求模型（否则"请求数没涨"会被读成"整档不调模型"；实际 ${reqs.length} 次）`,
      );
      events = readEvents(hd.homedir, SID);
      const t3 = cells(events, 2);
      assert(t3 && t3.open && t3.close, '第 3 轮的开轮格与收束格都应在');
      assertEq(t3.open.payload.tier, 'deep', '未命中 ⇒ 深度档');
      assertEq(t3.close.payload.text, OTHER_REPLY, '未命中的正文来自模型');
      assertEq(t3.close.payload.model, PROVIDER_ID, '未命中的产者是模型服务');

      // ── 请求级注入 ⇒ 会话存储里一个字都不该多（与 T29 / T35 同口径）────────
      const msgs = readMessagesJson(hd.homedir, SID);
      assert(
        !msgs.some((m) => {
          const t = typeof m.content === 'string' ? m.content : '';
          return t.includes('【长期记忆】');
        }),
        '注入段不得写进会话存储（请求级、与 nudge 同一机制）',
      );
      assertTranscriptInvariants(msgs, 'T41');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28/t34 同一约定：Windows 上带活子进程退出会
      // 撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
