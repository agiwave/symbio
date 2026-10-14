import './_selfrun.mjs';
// T61 辅助请求形状契约（LC-007）：分类 / 措辞的静默请求各自是什么形状。
//
// ## 本用例钉的是什么
//
// 对话面（classify 判决 + compose 措辞）出厂默认开启，每轮多一至两次**静默**
// LLM 请求。t21/t22 判的是它们的**行为后果**（判决走向、措辞内容）；本用例判
// 的是**请求形状**——判据从模型侧取，因为「一次对话面调优静默改变了请求形状」
// （分类请求带上工具、指令段被人格段盖住）在行为侧可能毫无症状。
//
// 三个形状事实（三条辅助路径的共同实现形态：都经 `execute_turn(_, _, &[], …)`
// 发 ⇒ **不带工具面**，主线轮次恒带）：
// ① 分类请求：system 是分流器提示词、无工具面、末条 user 是本轮原话、
//    消息带对话线投影（第二轮的分类请求看得见第一轮历史）；
// ② 措辞请求：system 带生成指令段且**指令段排在注册段之后**（`wording.rs`
//    的顺序纪律：指令放后面才压得住人格段的措辞习惯）、无工具面；
// ③ 主线轮次：恒带工具面（与辅助请求可判别——这是上面两条判据的识别基础）。
//
// ## 请求怎么认
//
// 按**结构特征**认，不按顺序认：分类 = system 含分流器标记；措辞 = system 含
// 指令标记；主线 = 带工具面。顺序会随对话面调优变，结构特征不会——认错了
// 对象，后面全判错。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import {
  isSilentAuxRequest,
  assertClassifyRequestShape,
  assertComposeRequestShape,
  assertToolChoiceSync,
  systemTextOf,
  lastUserOf,
  textOf,
  envelopeOf,
} from '../llm-contract.mjs';

const PERSONA = '你是测试人格小Q，回答务必温和。';
const U_WORK = '帮我把 README 读一下';
const U_DIRECT = '我们刚才聊了什么';

/** 分类请求的识别标记（classify `decide.rs::SYSTEM_PROMPT` 首句） */
const CLASSIFY_MARK = '你是一个意图分流器';
/** 措辞请求的识别标记（compose `wording.rs::INSTRUCTION` 首句） */
const COMPOSE_MARK = '你是这个助手本人';

export default defineCase(
  'T61 辅助请求形状：分类/措辞静默请求无工具面、提示词各自正确、指令段置后（LC-007）',
  async () => {
    // 场景表：分类请求与主线请求的「最后一条 user 消息」是同一条（都是用户这句
    // 话），按数组序 + once 区分——分类先发生，用 once 的那条消耗掉
    // （与 t21 同一手法，那里有完整说明）。
    const llm = await new MockLlm([
      { id: 'cls-work', match: U_WORK, content: 'work', once: true },
      { id: 'cls-direct', match: U_DIRECT, content: 'direct', once: true },
      { id: 'main', content: '主线回答：README 已读完。' },
      { id: 'compose', content: '我们刚才在聊 README 的事。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port, { system_prompt: PERSONA }) }],
      // 对话面**显式全开**：本用例验的就是它们，不能依赖默认值（默认值翻转
      // 不该悄悄改变本用例验的是什么）。
      pluginConfigs: {
        session: { classify_enabled: true, compose_enabled: true, progress_enabled: true },
      },
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: 'e2e-t61',
        stdinText: `${U_WORK}\n${U_DIRECT}\n`,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      const classify = reqs.filter((q) => systemTextOf(q).includes(CLASSIFY_MARK));
      const compose = reqs.filter((q) => systemTextOf(q).includes(COMPOSE_MARK));
      const main = reqs.filter((q) => !isSilentAuxRequest(q));

      // ── ① 请求面构成：两轮各一次分类；work 轮多一条主线；direct 轮多一条措辞 ──
      assertEq(classify.length, 2, '两轮用户发言各一次分类请求（按结构特征认）');
      assertEq(main.length, 1, 'work 轮一条主线请求（direct 轮不走主线）');
      assertEq(compose.length, 1, 'direct 轮一条措辞请求');
      assertEq(reqs.length, 4, `总请求数应为 4（实际 ${reqs.length}: ${reqs.map((q) => (envelopeOf(q).hasTools ? '主线' : systemTextOf(q).includes(CLASSIFY_MARK) ? '分类' : '措辞')).join(',')}）`);

      // ── ② 分类请求形状（两条各自的末条 user 都是当轮原话）────────────────
      assertClassifyRequestShape(classify[0], { utterance: U_WORK }, 'T61② 第一轮分类');
      assertClassifyRequestShape(classify[1], { utterance: U_DIRECT }, 'T61② 第二轮分类');
      // 第二轮分类请求带对话线投影：看得见第一轮的用户发言与助手回答
      const cls2Body = JSON.stringify(classify[1].body);
      assert(
        cls2Body.includes(U_WORK),
        '第二轮分类请求应带对话线投影（含第一轮原话）——判决要能引用「刚才」',
      );
      // 无工具面 ⇒ tool_choice 不得出现（同步规则的另一半，t56 判主线侧）
      assertToolChoiceSync(classify[0], 'T61② 分类请求');

      // ── ③ 措辞请求形状 + 指令段置后 ─────────────────────────────────────
      assertComposeRequestShape(compose[0], { utterance: U_DIRECT }, 'T61③ 措辞请求');
      assertToolChoiceSync(compose[0], 'T61③ 措辞请求');
      const csys = systemTextOf(compose[0]);
      const personaAt = csys.indexOf(PERSONA);
      const instrAt = csys.indexOf(COMPOSE_MARK);
      assert(personaAt >= 0, `措辞请求的 system 应带注册段（人格「${PERSONA}」）`);
      assert(
        personaAt < instrAt,
        '指令段必须排在注册段之后（置前被人格段盖住 = 「这一句该怎么写」输给「我是谁」）',
      );
      // 措辞请求的消息 = 对话线投影（末条是本轮原话，前面是历史）
      const composeUsers = (compose[0].body.messages ?? []).filter((m) => m.role === 'user');
      assertEq(composeUsers.length, 2, '措辞请求应带两轮用户发言（对话线投影）');
      assertEq(textOf(lastUserOf(compose[0])), U_DIRECT, '措辞请求末条 user 应恰为本轮原话');

      // ── ④ 主线轮次恒带工具面（辅助/主线可判别的识别基础）────────────────
      assert(envelopeOf(main[0]).hasTools, '主线轮次必须带工具面（否则辅助请求的「无工具」判据失去识别基础）');
      assertEq(textOf(lastUserOf(main[0])), U_WORK, '主线轮次末条 user 应恰为本轮原话');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
