import './_selfrun.mjs';
// T74 淘汰回路（相应批次，S13 §6 断言 3）：
// 声誉低于阈值 ⇒ 一条**可审计**的 `verdict.asserted` 收回 `assign.work` ⇒ `agent_run`
// 被闸拒（同会话与**别的会话**都拒——部署级口径）。
//
// ## 本用例钉的是什么
//
// §2 的「淘汰」行在生产里的完整链路，四格各只有 e2e 能看见**接在一起**：
//
// | 格 | 单测钉在哪 | 这里钉什么 |
// |---|---|---|
// | 裁决落格（`verdict.asserted`） | `derived_facts.test.rs` / `actors/mod.test.rs` | 真实轮次真的把裁决写进了 WAL（actor = 仲裁者、载荷六字段） |
// | 闸拒 `agent_run` | `symbio-plugin-agent` 的 `host/behavior.test.rs`（`DelegationGate`）+ `loop` 的 `dispatch/gates.test.rs`（映射） | 工具结果真的是 `Refused: …` 且回灌进了模型下一发请求 |
// | 跨会话生效 | 同上（部署级扫描） | **另一个会话**的同一次 `agent_run` 同样被拒 |
// | 幂等 | `derived_facts.test.rs` | 再违约也不落第二条裁决（审计面不缺） |
//
// ## 场景（三个轮次）
//
// - **A 轮 1**（正对照）：`session/main` 委托一个**不存在**的智能体 ⇒ 工具失败 ⇒
//   立约 + 违约 ⇒ 声誉 −1 ⇒ 轮末落裁决。这一轮的 `agent_run` **放行**（闸只在
//   裁决之后才可能拒）——正对照与淘汰同批，缺了它「闸恒拒」也能全绿。
// - **A 轮 2**：再委托 ⇒ 闸拒（同会话，覆盖层里已有那条撤销）；轮末**不落第二条
//   裁决**（幂等），但如实再记一笔违约（约了没派成，S08 §5）。
// - **B 会话**：另一个会话、同一主体 ⇒ 同样被拒。裁决在 A 的 WAL 里，闸扫的是
//   **全部会话**——「换个会话绕开淘汰」这条静默面由这一格钉住。
//
// ## 为什么 B 的 WAL 里也会有一条裁决（不是缺陷）
//
// 淘汰**驱动**是会话级的（读本会话 WAL 算声誉、判既有裁决——与承诺 / 任务 / 记忆
// 同一条口径，派生事实都是会话级的），闸的**读侧**才是部署级的（扫全部 WAL 折
// 覆盖层）。于是 B 的事实源里「main 又违约一次且本地无既有裁决」⇒ B 也落一条。
// 同一主体两条裁决，last-wins 折叠出同一个答案——本用例把这件事断言出来，
// 而不是留给将来的读者纳闷。
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
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
} from '../helpers.mjs';
import { join } from 'node:path';

/** 会话 A：两轮（轮 1 违约 ⇒ 裁决；轮 2 被拒）。 */
const A_SID = 'e2e-t74-elim';
/** 会话 B：淘汰之后另起一个会话（部署级判据）。 */
const B_SID = 'e2e-t74-cross';

const A_Q1 = '第一问：把这件事委托给子智能体。';
const A_Q2 = '第二问：再委托一次。';
const B_Q = '换个会话再委托一次。';

/** v2 事实源文件名（与 `crates/symbio-plugin-session/src/paths.rs::V2_WAL_FILE` 同一常量口径）。 */
const V2_WAL = 'v2-events.wal';

/** 读一份 WAL 文件的行（逐行 JSON）——与 t71 / t73 同一条直读口径。 */
function walLines(path) {
  return readFileSyncSafe(path)
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

/** 请求体里最后一条 user 消息的正文（mock 匹配同口径）。 */
function lastUserText(r) {
  const msgs = r.body?.messages ?? [];
  const last = [...msgs].reverse().find((m) => msgRoleOf(m) === 'user');
  if (typeof last?.content === 'string') return last.content;
  if (Array.isArray(last?.content)) return last.content.map((p) => p.text ?? '').join('');
  return '';
}

export default defineCase(
  'T74 淘汰回路：违约 ⇒ verdict.asserted 收回 assign.work ⇒ agent_run 被拒（同会话与跨会话）',
  async () => {
    const llm = await new MockLlm([
      // A 轮 1：真跑一次 agent_run，委托一个不存在的智能体 ⇒ 工具失败 ⇒ 违约
      {
        id: 'a1-tool',
        match: A_Q1,
        once: true,
        toolCalls: [
          {
            id: 'call_a1',
            name: 'agent_run',
            arguments: { agent_id: 'ghost-agent', prompt: '把第一件事办了' },
          },
        ],
      },
      { id: 'a1-final', once: true, content: '第一轮的答复。' },
      // A 轮 2：再委托 ⇒ 闸拒（工具结果回灌后照常收束）
      {
        id: 'a2-tool',
        match: A_Q2,
        once: true,
        toolCalls: [
          {
            id: 'call_a2',
            name: 'agent_run',
            arguments: { agent_id: 'ghost-agent', prompt: '再委托一次' },
          },
        ],
      },
      { id: 'a2-final', once: true, content: '第二轮的答复。' },
      // B 会话：另一个会话、同一主体 ⇒ 同样被拒
      {
        id: 'b-tool',
        match: B_Q,
        once: true,
        toolCalls: [
          {
            id: 'call_b',
            name: 'agent_run',
            arguments: { agent_id: 'ghost-agent', prompt: '把另一件事办了' },
          },
        ],
      },
      { id: 'b-final', once: true, content: 'B 会话的答复。' },
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
        session: { ...DIALOG_FACE_OFF },
      },
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const aWal = join(hd.homedir, 'session', A_SID, V2_WAL);
    const bWal = join(hd.homedir, 'session', B_SID, V2_WAL);

    try {
      // ── ① A 轮 1：委托失败 ⇒ 违约 ⇒ 轮末落裁决 ──────────────────────────
      const a1 = runCli({ ...base, session: A_SID, message: A_Q1 });
      assertEq(a1.code, 0, `A 轮 1 退出码（stderr: ${a1.stderr.slice(0, 400)}）`);

      const a1Events = walLines(aWal);
      const a1Kinds = a1Events.map((e) => e.kind);
      const offered = a1Events.find((e) => e.kind === 'commitment.opened');
      assert(offered, `轮 1 应先立约（实际 kinds: ${JSON.stringify(a1Kinds)}）`);
      assertEq(offered.payload?.from, 'session/main', '承诺方 = 会话主体（声誉记在它头上）');
      assert(
        a1Kinds.includes('commitment.broken'),
        `委托失败 ⇒ 违约成格（实际 kinds: ${JSON.stringify(a1Kinds)}）`,
      );

      // 裁决：恰好一条，六字段齐全，作者是仲裁者。
      const verdicts = a1Events.filter((e) => e.kind === 'verdict.asserted');
      assertEq(verdicts.length, 1, `轮 1 应恰好落一条裁决（实际 ${verdicts.length}）`);
      const v = verdicts[0];
      assertEq(v.actor, 'session/arbiter', '裁决者是仲裁者，不是被淘汰主体');
      assertEq(v.payload?.subject, 'session/main');
      assertEq(v.payload?.capability, 'assign.work', '收回的是派活能力（S13 §2 字面机制）');
      assertEq(v.payload?.change, 'revoke');
      assertEq(v.payload?.score, -1, '判据值随事件落盘（守约 − 违约）');
      assertEq(v.payload?.threshold, 0);
      assert(typeof v.payload?.why === 'string' && v.payload.why.length > 0, '人话理由');
      assert(typeof v.produced_by === 'number', 'I2：断言类事件必须带溯源');

      // ── ② A 轮 2：再委托 ⇒ 闸拒；不落第二条裁决 ─────────────────────────
      const a2 = runCli({ ...base, session: A_SID, message: A_Q2 });
      assertEq(a2.code, 0, `A 轮 2 退出码（stderr: ${a2.stderr.slice(0, 400)}）`);

      const a2Events = walLines(aWal);
      assertEq(
        a2Events.filter((e) => e.kind === 'verdict.asserted').length,
        1,
        '幂等：再违约也不落第二条裁决（无既有 verdict 的主体才写）',
      );
      assertEq(
        a2Events.filter((e) => e.kind === 'commitment.broken').length,
        2,
        '但违约如实再记一笔（约了没派成，S08 §5——被闸拒同样是没履约）',
      );

      // ── ③ B 会话：部署级口径——别处的裁决在这里照样拒 ────────────────────
      const b = runCli({ ...base, session: B_SID, message: B_Q });
      assertEq(b.code, 0, `B 会话退出码（stderr: ${b.stderr.slice(0, 400)}）`);

      // B 的事实源里也有自己的一条裁决（会话级驱动，见文件头说明）。
      const bVerdicts = walLines(bWal).filter((e) => e.kind === 'verdict.asserted');
      assertEq(
        bVerdicts.length,
        1,
        'B 的事实源各自完整：本地违约 + 本地无既有裁决 ⇒ 本地也落一条（last-wins 折叠同一答案）',
      );
      assertEq(bVerdicts[0].payload?.subject, 'session/main');

      // ── ④ 请求留痕：拒因真的回灌进了模型（用户与模型都看得见）────────────
      const reqs = await llm.requests();
      /** 某一轮用户问题的**第二发**请求（工具结果回灌那发）里的工具结果正文。 */
      const toolResultOf = (q) => {
        const rs = reqs.filter((r) => lastUserText(r) === q);
        assertEq(rs.length, 2, `${q} 应恰好两发请求（工具轮 + 回灌；实际 ${rs.length}）`);
        const tools = (rs[1].body?.messages ?? []).filter((m) => m.role === 'tool');
        assert(tools.length > 0, `第二发应带回灌的工具结果（实际 roles: ${JSON.stringify((rs[1].body?.messages ?? []).map((m) => m.role))}）`);
        return JSON.stringify(tools);
      };

      // 正对照：轮 1 的 agent_run **放行**过——失败来自「子智能体不存在」，不是闸。
      const a1Tool = toolResultOf(A_Q1);
      assert(!a1Tool.includes('Refused'), `轮 1 的委派应真的执行过（实际: ${a1Tool.slice(0, 300)}）`);
      // 轮 2 与 B：闸拒，且拒因点名能力与原因。
      const a2Tool = toolResultOf(A_Q2);
      assert(a2Tool.includes('Refused'), `轮 2 应被闸拒（实际: ${a2Tool.slice(0, 300)}）`);
      assert(a2Tool.includes('assign.work'), `拒因应点名被收回的能力（实际: ${a2Tool.slice(0, 300)}）`);
      assert(a2Tool.includes('verdict.asserted'), '拒因应点名裁决事件（可审计）');
      const bTool = toolResultOf(B_Q);
      assert(bTool.includes('Refused'), `B 会话应被闸拒（实际: ${bTool.slice(0, 300)}）`);
      assert(bTool.includes('assign.work'), `B 的拒因同样点名能力（实际: ${bTool.slice(0, 300)}）`);

      // ── ⑤ 生产读出口：转写不变量 + 读数 ─────────────────────────────────
      const api = startLongLivedCli({
        ...base,
        session: A_SID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        const aMsgs = await readSessionMessages(api, A_SID);
        assertTranscriptInvariants(aMsgs, 'T74 会话 A');
        const aToolNodes = aMsgs.filter((m) => msgRoleOf(m) === 'tool');
        assertEq(aToolNodes.length, 2, '会话 A 两个工具结果节点（轮 1 真跑 + 轮 2 被拒）');
        assert(
          JSON.stringify(aToolNodes[1]).includes('Refused'),
          `轮 2 的工具节点应带拒因（实际: ${JSON.stringify(aToolNodes[1]).slice(0, 300)}）`,
        );

        const bMsgs = await readSessionMessages(api, B_SID);
        assertTranscriptInvariants(bMsgs, 'T74 会话 B');
        assert(
          bMsgs.some((m) => msgRoleOf(m) === 'tool' && JSON.stringify(m).includes('Refused')),
          'B 会话转写里应有一条被拒的工具结果',
        );

        // 读数：A 的声誉（−2：两笔违约）与不变量为空。
        const stats = await api.invoke('session/stats', {}, { session_id: A_SID });
        assertEq(stats.status, 200, `session/stats 应成功（实际: ${JSON.stringify(stats.body)}）`);
        const data = stats.body?.data;
        assert(data, `响应应带 data 载荷（实际: ${JSON.stringify(stats.body)}）`);
        assertEq(
          data.reputation.by_principal['session/main']?.score,
          -2,
          '两轮各一笔违约 ⇒ 声誉 −2（闸拒的那笔也如实记账）',
        );
        assertEq(
          data.reputation.by_principal['session/main']?.broken,
          2,
          '违约两笔',
        );
        assertEq(
          data.invariants.length,
          0,
          `裁决也是普通事件，不变量清单应为空（实际: ${JSON.stringify(data.invariants)}）`,
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净：Windows 上带着活子进程 `process.exit()` 会撞
      // libuv 的 `uv_async_send` 断言，表现是**断言全过、退出码却是 0xC0000409**
      // ——判据过了却判失败。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
