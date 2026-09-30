import './_selfrun.mjs';
// T23 中途汇报（S4）：长任务中途说一句 / 配额护栏 / 总开关平凡值。
//
// ## 本用例钉的是什么
//
// S4 让助手在**轮边界**主动说一句进度（`Verdict::Report` ⇒ `compose` 从运行现状
// 组织一句话 ⇒ 一条根级对话面节点）。本用例验的不是"某句文案长什么样"，而是
// **转写里多出了什么、多付了什么代价**：
//
// | 线 | 编排 | 期望 | 可观测后果 |
// |---|---|---|---|
// | A 长任务 | 3 轮工具 + 每边界都可报 | 说 | 恰 2 条根级 `surface=reply` / `reason=progress` 节点；**总请求数仍是 3**（汇报不付 LLM 往返） |
// | B 配额护栏 | 同上 + `progress_max_per_turn=1` | 只说一次 | 恰 1 条（同一份编排，差别只有配额） |
// | C 总开关 | 同上 + `progress_enabled=false` | 不说 | 0 条（工具回路照旧跑完） |
//
// ## 为什么 A 线要断言**请求总数**
//
// 汇报的产线是**填表**（事实随 `RunSnapshot` 带来，措辞固定）。这条断言的等价形式
// 就是"汇报没花任何往返"：一次 3 轮工具的任务本来发 3 次请求，若汇报去问模型，
// 就会变成 5 次。若哪天有人给汇报加一次"润色"，A 线会红——这正是它存在的意义。
//
// ## 为什么判决关掉（`classify_enabled=false`）
//
// 本用例的主题是**汇报**，不是判决（判决见 `t21`）。关掉判决有两个好处：
// ① 请求数只剩工具回路那几次，A 线的"零额外往返"才有判别力；② 顺带证明汇报
// **不依赖判决**——它的触发点在轮边界，与轮首的判决是两条独立的路径。
//
// ## 为什么三条线各起一个 homedir
//
// `progress_max_per_turn` / `progress_enabled` 都在**装配期**读进 `SessionConfig`
// 快照（`orchestrator/consume.rs`），运行期改配置不生效。三条线的差别正是这两个
// 取值，因此只能各起进程；端口来自各自 homedir 的插件配置（`nextPort()`），
// 互不干扰。
//
// ## 为什么阈值配成 0 / 1
//
// `progress_interval_ms = 0` 让"静默时长"这一条恒成立——本用例要验的是**触发点与
// 配额**，不是时长分档（那是 `compose/templates.test.rs` 的单测射程）。`min_rounds = 1`
// 同理：让第一个轮边界就有资格汇报，于是"边界数 = 汇报数"这条关系可以直接数出来。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  readMessagesJson,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 三条线各自的会话 id（各自从零开始，断言互不污染） */
const SIDS = {
  report: 'e2e-t23-report',
  capped: 'e2e-t23-capped',
  off: 'e2e-t23-off',
};

/** 长任务的用户发言（mock 用它匹配第 1 轮） */
const ASK = '跑个长任务';

/**
 * 长任务编排：**3 轮** LLM 请求，其中前两轮各带一次工具调用。
 *
 * 于是轮边界出现两次（第 1 轮工具跑完后、第 2 轮工具跑完后），第 3 轮无工具调用
 * ⇒ 收尾。边界数 = 2 是本用例全部计数的基准。
 *
 * `round-2` 必须 `once`：`afterTool` 的池子里它不带 `match`，不消耗掉就会在每一轮
 * 工具结果之后再次命中 ⇒ 无限工具循环（见 `e2e/mock-llm.mjs` 的说明）。
 */
const LONG_TASK = [
  {
    id: 'round-1',
    match: ASK,
    toolCalls: [{ id: 'c1', name: 'vdfs_write', arguments: { path: 'p1.md', text: '# r1' } }],
  },
  {
    id: 'round-2',
    afterTool: true,
    once: true,
    toolCalls: [{ id: 'c2', name: 'vdfs_write', arguments: { path: 'p2.md', text: '# r2' } }],
  },
  { id: 'round-3', afterTool: true, content: '两轮工具都跑完了。' },
];

/** 汇报句里**不随运行快慢变化**的部分（时长那一段见下） */
const ROUND_1_PREFIX = '已经完成 1 轮工具调用';
const ROUND_2_PREFIX = '已经完成 2 轮工具调用';
/** 汇报句的收尾（模板固定） */
const REPORT_SUFFIX = '还在继续。';

function gatewayConfig(port) {
  return {
    inbound_enabled: true,
    inbound_protocol: 'http',
    inbound_bind: '127.0.0.1',
    inbound_port: port,
    inbound_token: '',
    inbound_readonly: false,
  };
}

/** 长任务那一轮的 session 配置：判决关掉、措辞与汇报打开，阈值配到恒成立 */
function longTaskSession(overrides) {
  return {
    classify_enabled: false,
    compose_enabled: true,
    progress_enabled: true,
    // 0 / 1：让"静默时长"与"最少轮次"两条恒成立，本用例只数**边界**。
    progress_interval_ms: 0,
    progress_min_rounds: 1,
    progress_max_per_turn: 5,
    ...overrides,
  };
}

export default defineCase(
  'T23 中途汇报：长任务中途说一句 / 配额护栏 / 总开关平凡值',
  async () => {
    const llm = await new MockLlm(LONG_TASK).start();

    /**
     * 起一条线：自己的 homedir + 自己的端口 + 自己的 session 配置。
     *
     * 返回 `{ cli, hd, run }`：`run()` 建会话、投一条长任务、等这一轮收尾，
     * 并把该会话的落库节点读回来。
     */
    const startLine = async (sid, sessionCfg, label) => {
      const port = nextPort();
      const hd = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: { gateway: gatewayConfig(port), session: sessionCfg },
      });
      const cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: sid,
        provider: PROVIDER_ID,
        gatewayPort: port,
      });
      await cli.waitGatewayReady(20_000, label);
      return { cli, hd, sid };
    };

    /** 建会话 + 投一条消息（**唯一输入入口**：写收件箱） */
    const send = async ({ cli, hd, sid }) => {
      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${sid}`;

      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({
          metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' },
        }),
      });
      assertEq(created.status, 200, `创建会话 ${sid}（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      const w = await cli.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text: ASK });
      assertEq(w.status, 200, `写收件箱 ${sid}（${JSON.stringify(w.body)?.slice(0, 300)}）`);
    };

    /** 等这一轮真正收尾：末轮正文落库（它是本轮最后一条消息） */
    const waitDone = ({ hd, sid }) =>
      waitFor(
        () => (readMessagesJson(hd.homedir, sid) ?? []).some((m) => m.content === '两轮工具都跑完了。'),
        { what: `${sid} 末轮正文落库`, timeoutMs: 60_000 },
      );

    /** 该会话的汇报节点（按落点标记筛，不按文案筛） */
    const reportsOf = (homedir, sid) =>
      (readMessagesJson(homedir, sid) ?? []).filter(
        (m) => m.meta?.surface === 'reply' && m.meta?.reason === 'progress',
      );

    const lines = [];
    try {
      // ── A 长任务：两个轮边界 ⇒ 两句汇报，且**不付任何 LLM 往返** ──
      const a = await startLine(SIDS.report, longTaskSession(), 'A 线进程');
      lines.push(a);
      await send(a);
      await waitDone(a);

      const aMsgs = readMessagesJson(a.hd.homedir, SIDS.report);
      assertTranscriptInvariants(aMsgs, 'T23-A');
      const aReports = reportsOf(a.hd.homedir, SIDS.report);
      assertEq(aReports.length, 2, `A 线两个轮边界应各汇报一次（实得 ${aReports.length} 条）`);
      for (const r of aReports) {
        assertEq(r.role, 'assistant', '汇报是助手说的话');
        assertEq(r.type, 'text', '汇报是文本节点');
        assert(r.parent_id == null, '汇报必须挂根级——挂在 Turn 之下会让前端「对话」面板看不到它');
        assertEq(
          r.meta?.exclude_from_context,
          true,
          '汇报是给用户看的界面文本，必须被请求包剔除',
        );
        assert(r.content.endsWith(REPORT_SUFFIX), `汇报应说清"还在继续"（实得：${r.content}）`);
      }
      assert(
        aReports[0].content.includes(ROUND_1_PREFIX),
        `第一句应报第 1 轮（实得：${aReports[0].content}）`,
      );
      assert(
        aReports[1].content.includes(ROUND_2_PREFIX),
        `第二句应报第 2 轮（实得：${aReports[1].content}）`,
      );

      // 汇报的产线是**填表** ⇒ 3 轮工具的任务恰好发 3 次请求。若汇报去问模型，
      // 这里会是 5 次——那正是本断言要拦的。
      const aReqs = await llm.requests();
      assertEq(
        aReqs.length,
        3,
        `3 轮工具应恰好发 3 次请求（汇报若付一次往返就会是 5 次）`,
      );

      // ── B 配额护栏：同一份编排，`progress_max_per_turn = 1` ⇒ 只说一次 ──
      await llm.reset();
      const b = await startLine(
        SIDS.capped,
        longTaskSession({ progress_max_per_turn: 1 }),
        'B 线进程（配额 1）',
      );
      lines.push(b);
      await send(b);
      await waitDone(b);

      const bMsgs = readMessagesJson(b.hd.homedir, SIDS.capped);
      assertTranscriptInvariants(bMsgs, 'T23-B');
      const bReports = reportsOf(b.hd.homedir, SIDS.capped);
      assertEq(
        bReports.length,
        1,
        `配额 1 ⇒ 两个轮边界也只说一次（实得 ${bReports.length} 条）`,
      );
      assert(
        bReports[0].content.includes(ROUND_1_PREFIX),
        '被保留下来的应是**最早**那一次（护栏只封顶，不推迟第一次）',
      );
      // 护栏只封顶，不改变工具回路本身：两轮工具照旧跑完
      assertEq(
        bMsgs.filter((m) => m.type === 'tool_call').length,
        2,
        '配额不影响工具回路（两轮工具调用照旧）',
      );

      // ── C 总开关平凡值：`progress_enabled = false` ⇒ 一句都不说 ──
      await llm.reset();
      const c = await startLine(
        SIDS.off,
        longTaskSession({ progress_enabled: false }),
        'C 线进程（progress_enabled=false）',
      );
      lines.push(c);
      await send(c);
      await waitDone(c);

      const cMsgs = readMessagesJson(c.hd.homedir, SIDS.off);
      assertTranscriptInvariants(cMsgs, 'T23-C');
      assertEq(
        reportsOf(c.hd.homedir, SIDS.off).length,
        0,
        '关掉总开关后一句都不说——行为与引入汇报之前逐字一致',
      );
      // 平凡值线必须**仍然完整可用**：工具照跑、正文照出，只是没有汇报
      assertEq(
        cMsgs.filter((m) => m.type === 'tool_call').length,
        2,
        'C 线工具回路照旧（关掉汇报不影响干活）',
      );
      assert(
        cMsgs.some((m) => m.content === '两轮工具都跑完了。'),
        'C 线末轮正文照旧落库',
      );
      assert(
        !cMsgs.some((m) => m.meta?.surface === 'reply'),
        'C 线不该有任何对话面文本（判决与汇报都关掉了）',
      );
    } finally {
      for (const l of lines) {
        l.cli.stop();
        cleanupHomedir(l.hd);
      }
      llm.stop();
    }
  },
);
