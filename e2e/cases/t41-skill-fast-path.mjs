import './_selfrun.mjs';
// T41 技能快路（S11）：命中已编译技能 ⇒ **不调模型**、以技能正文收束、走反射档。
//
// ## 本用例钉的是什么
//
// 与 [t35](../cases/t35-skill-compile.mjs) 的区别：t35 证「技能编得出、召得回」，
// 本用例证**快路开关**（`skill_fast_path`）打开后命中技能那一刻的行为差异：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 命中**跳过模型** | mock-llm 的请求计数：命中那一轮**不再增**（这是最硬的判据） |
// | 命中以**技能正文**收束 | 该轮 CLI stdout（用户看到的那段答话） |
// | 未命中**照常推理** | 换一句话 ⇒ 请求计数**照增**（把「命中才跳过」与「整档不调模型」分开） |
// | 请求级注入**不落存储** | 生产读出口 `vdfs/read` 读回的转写里没有注入段抬头 |
//
// ## 为什么第 3 轮是**必须**的
//
// 只有前两轮的话，「请求数没涨」还能被解释成"这个档位根本不调模型"。第三轮换一句话
// （不命中）⇒ 请求数照增，把"命中才跳过"与"整档不调模型"分开。
//
// ## 为什么不再直读 WAL（[ADR-049] 读通道禁令）
//
// 早先版本直读 `v2-events.wal` 判开轮格 `payload.tier == 'reflex'`、收束格
// `model == 'reflex'`、`cost_ms <= 80`、`produced_by`。这些是**存储白盒**，判据已
// 归 Rust 单测（`turn_runner.test.rs` 的档位 / 溯源用例、`turn_exec.test.rs` 的快路）。
// e2e 只保留「用户如何看到就如何读」：请求计数（LLM 请求包）、stdout（CLI）、
// 转写（生产读出口 `vdfs/read`）、出口读数（`session/stats`）。
import {
  MockLlm,
  makeHomedir,
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
  textOf,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 注入段抬头（与 `memory_events.rs::RECALL_SECTION_HEAD` 同一常量口径） */
const HEAD = '【长期记忆】';

/** 第 1 / 2 轮：同一句发言（第 2 轮即命中）。 */
const TRIGGER = '把周报整理成摘要';

/** 技能正文 = 第 1 轮的收束发言（`content = response`）。第 2 轮必须原样收束它。 */
const RESPONSE = '先列要点；再合并同类项；最后按重要度排序。';

/** 第 3 轮：另一句话（不命中）——用来把"命中才跳过"与"整档不调模型"分开。 */
const OTHER = '再帮我做一次';
const OTHER_REPLY = '好的，已按上次的方式处理。';

const SID = 'e2e-t41';

export default defineCase(
  'T41 技能快路：命中已编译技能 ⇒ 不调模型、以技能正文收束、走反射档',
  async () => {
    const llm = await new MockLlm([
      // 顺序即优先级：`match` 是对**最后一条 user 消息**的子串匹配，而 prompt 是
      // 整段转写（含召回注入段）——第 3 轮的 prompt 里**同时**含 `TRIGGER`
      // （注入段里那条记忆的正文）与 `OTHER`（本轮发言）。更具体的那个必须排在前面，
      // 否则第 3 轮永远命中 `TRIGGER` 的场景（表现为"未命中却答了技能正文"）。
      { id: 'other', match: OTHER, content: OTHER_REPLY },
      // 第 1 轮：成功收束（正文即被固化的技能内容）。
      { id: 'skill', match: TRIGGER, content: RESPONSE },
      { id: 'rest', content: '收到。' },
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
        // 轮次由 v2 运行器原生执行——"跳过模型"跳的正是它里面那一次调用。
        // 两个开关都**显式打开**：它们默认 off（S11 §4 的平凡值），本用例验的正是
        // 打开之后的行为，不能依赖默认值。
        session: {
          ...DIALOG_FACE_OFF,
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
      const r1 = turn(TRIGGER);
      let reqs = await llm.requests();
      assertEq(reqs.length, 1, '第 1 轮照常请求模型（还没有技能可用）');
      assert(
        r1.stdout.includes(RESPONSE),
        `第 1 轮收束正文来自模型（实际 stdout: ${JSON.stringify(r1.stdout)}）`,
      );

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

      // ── 第 3 轮：另一句话（不命中）⇒ 照常推理 ─────────────────────────────
      turn(OTHER);
      reqs = await llm.requests();
      assertEq(
        reqs.length,
        2,
        `未命中照常请求模型（否则"请求数没涨"会被读成"整档不调模型"；实际 ${reqs.length} 次）`,
      );

      // ── 请求级注入 ⇒ 会话存储里一个字都不该多（与 T29 / T35 同口径）────────
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady(20_000, 'T41 读回进程');
        const msgs = await readSessionMessages(api, SID);
        assert(msgs, '跑过三轮 ⇒ 生产读出口应读回转写');
        assert(
          !msgs.some((m) => textOf(m).includes(HEAD)),
          '注入段不得写进会话转写（请求级、与 nudge 同一机制）',
        );
        assertTranscriptInvariants(msgs, 'T41');

        // 出口读数：技能被用了（校准列从同一份事实源算出）。
        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats?.has_wal, '跑过三轮 ⇒ 事实源应在');
        const skills = Object.values(stats.calibration.by_skill);
        assertEq(skills.length, 1, `恰好编出一条技能（实际: ${JSON.stringify(stats.calibration)}）`);
        // `uses` 计的是**召回次数**（与 t35 同口径）：第 2 轮命中、第 3 轮注入段照样带
        // 技能正文 ⇒ 各召回一次 = 2。命中与否由请求计数分辨，不由本列分辨。
        assertEq(skills[0].uses, 2, '第 2 / 3 轮各召回一次');
        assertEq(skills[0].fallbacks, 0, '命中即用技能正文 ⇒ 不走回退');
      } finally {
        api.stop();
      }
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
