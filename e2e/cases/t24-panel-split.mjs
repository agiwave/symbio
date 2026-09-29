import './_selfrun.mjs';
// T24 呈现分界（S5）：对话线与工作线是**同一份转写的两个视角**。
//
// ## 本用例钉的是什么
//
// S5 把「工具 / 推理」移出对话列、给「工作」单独一列。分界规则是**纯函数**
// （Rust `session/context/conversation_view.rs`，前端镜像
// `tauri/src/schemas/conversation_line.ts`），规则本身由两侧各自的单测穷举
// （`conversation_view.test.rs` / `conversation_line.spec.ts`）。
//
// 那些单测喂的是**人造**转写。本用例补的是另一半：**真实**转写的形状是否还
// 满足规则赖以成立的前提。规则写错了会有测试变红；**真实数据的形状变了**却
// 不会有任何错误信号——只表现为「助手答得不对」或「对话面板里没有回答」，
// 排查方向会被带偏到提示词上。这里钉的就是那个沉默的失效面。
//
// ## 四条形状锚（每条对应规则的一个前提）
//
// | 锚 | 断言 | 它守住什么 |
// |---|---|---|
// | ① | 末轮**回答**的 `parent_id` 非空，父节点是**根级** `turn` 容器 | 「位置不参与判定」是**必须**的——回答不在根级，按根级切会把它整个漏掉 |
// | ② | 至少一条**汇报**是根级 `assistant` 文本 | 助手说的话有**两种**位置：汇报在根级、回答在 `turn` 里，两条都得认 |
// | ③ | 存在 `role = tool` 且 `type = text` 的节点 | 「看角色」这一条不可省——工具结果就是文本节点 |
// | ④ | 存在 `role = assistant` 且 `type ∈ {turn, tool_call, reasoning}` 的节点 | 「看类型」这一条不可省——工作节点的角色与助手**相同** |
//
// ③④ 是一对：**只看类型会漏进工具结果，只看角色会漏进工具调用**。两条判据
// 各自必要，而不是"两条保险"。
//
// ## 为什么还验实时面
//
// 两个面板读的是**同一份** WS 帧流（`sessionTranscriptSync` → store），差别只在
// 过滤。工作面板要能看到东西，前提是工作节点**也进实时面**。若哪天后端只把
// 对话节点发上实时面，工作面板会空掉——同样没有错误信号（数据"没来"与"被滤掉"
// 长得一样）。因此这里断言同一份流里**既有**用户问句帧**也有** `tool_call` 帧。
//
// ## 为什么只有一条线
//
// 呈现分栏的开关（`dialog_panel_split`）是**纯前端**持久化偏好，不经后端、不落
// 转写——它在数据层无从观测，由 `stores/__tests__/appearance.spec.ts` 与
// `conversation_line.spec.ts` 覆盖。本用例要的是**后端事实**，一条线即可。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  readMessagesJson,
  nextPort,
  waitFor,
  subscribeSessionRealtime,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  textOf,
} from '../helpers.mjs';

const SID = 'e2e-t24-split';

/** 用户发言（mock 用它匹配第 1 轮） */
const ASK = '读一下 README 再总结';

/** 末轮正文——它是**回答**，落在 `turn` 之下（形状锚 ① 的哨兵） */
const ANSWER = '读完了，这里是总结。';

/**
 * 编排：2 轮工具 + 末轮正文。
 *
 * 两轮工具是为了让**汇报**（S4）有机会出现——`progress_min_rounds = 1` 时每个轮
 * 边界都够格，于是根级会多出一条 assistant 文本（形状锚 ② 的哨兵）。同时
 * `turn` 里留下 `reasoning` / `tool_call` / 工具结果三类工作节点（形状锚 ③④）。
 *
 * `round-2` 必须 `once`：`afterTool` 的池子里它不带 `match`，不消耗掉就会在每一轮
 * 工具结果之后再次命中 ⇒ 无限工具循环（见 `e2e/mock-llm.mjs` 的说明）。
 */
const PLAN = [
  {
    id: 'round-1',
    match: ASK,
    // 思考通道：真实模型在正文之前输出，落成 `type = reasoning` 的工作节点
    reasoning: ['先看看文件……'],
    toolCalls: [{ id: 'c1', name: 'vdfs_write', arguments: { path: 'note.md', text: 'x' } }],
  },
  {
    id: 'round-2',
    afterTool: true,
    once: true,
    toolCalls: [{ id: 'c2', name: 'vdfs_write', arguments: { path: 'note2.md', text: 'y' } }],
  },
  { id: 'round-3', afterTool: true, content: ANSWER },
];

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

/** 汇报要真的发生：判决关掉（本用例不验判决），措辞与汇报打开、阈值配到恒成立 */
const SESSION = {
  triage_enabled: false,
  reply_enabled: true,
  progress_enabled: true,
  progress_interval_ms: 0,
  progress_min_rounds: 1,
  progress_max_per_turn: 5,
};

export default defineCase(
  'T24 呈现分界：对话线 / 工作线的真实数据形状 + 实时面同构',
  async () => {
    const llm = await new MockLlm(PLAN).start();
    const port = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { gateway: gatewayConfig(port), session: SESSION },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: SID,
      provider: PROVIDER_ID,
      gatewayPort: port,
    });
    let realtime = null;
    try {
      await cli.waitGatewayReady(20_000, 'T24 进程');

      // 先订阅实时面，再投消息——否则会漏掉开头的帧（订阅是登记 sink，不回放）
      realtime = await subscribeSessionRealtime(cli, { sessionId: SID, gatewayPort: port });

      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${SID}`;

      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({
          metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' },
        }),
      });
      assertEq(created.status, 200, `创建会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);
      const w = await cli.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text: ASK });
      assertEq(w.status, 200, `写收件箱（${JSON.stringify(w.body)?.slice(0, 300)}）`);

      await waitFor(
        () => (readMessagesJson(hd.homedir, SID) ?? []).some((m) => textOf(m) === ANSWER),
        { what: 'T24 末轮正文落库', timeoutMs: 60_000 },
      );

      const msgs = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs, 'T24');

      /** 落库的节点类型词（后端可能写 `type` 或 `msg_type`，两处都认） */
      const typeOf = (m) => m.type ?? m.msg_type;
      /** 根级 = 没有父节点 */
      const roots = msgs.filter((m) => m.parent_id == null);
      const byId = new Map(msgs.map((m) => [m.id, m]));

      // ── 形状锚 ①：回答在 `turn` 之下，不在根级 ──
      const answer = msgs.find((m) => textOf(m) === ANSWER);
      assert(answer, `末轮正文 ${ANSWER} 应落库`);
      assert(
        answer.parent_id != null,
        '回答（末轮正文）**不在根级**——若哪天它变成根级，前端镜像的「位置不参与判定」' +
          '就不再被真实数据覆盖，请同步复核 conversation_line.ts',
      );
      const turnOfAnswer = byId.get(answer.parent_id);
      assert(turnOfAnswer, `回答的父节点 ${answer.parent_id} 应在转写里`);
      assertEq(typeOf(turnOfAnswer), 'turn', '回答的父节点应是 turn 容器');
      assert(
        turnOfAnswer.parent_id == null,
        'turn 容器在根级——回答挂在它下面，所以「按根级切」会连容器带回答一起漏掉',
      );
      assertEq(answer.role, 'assistant', '回答是助手说的话');
      assertEq(typeOf(answer), 'text', '回答是文本节点');

      // ── 形状锚 ②：助手说的话有两种位置，汇报在**根级** ──
      const reports = msgs.filter((m) => m.meta?.surface === 'reply');
      assert(
        reports.length >= 1,
        `应至少有一条对话面文本（汇报）——两种位置的锚点（实得 ${reports.length} 条）`,
      );
      for (const r of reports) {
        assertEq(r.role, 'assistant', '汇报是助手说的话');
        assertEq(typeOf(r), 'text', '汇报是文本节点');
        assert(
          r.parent_id == null,
          '汇报必须挂根级——它与回答（turn 子节点）是「同一件事的两种位置」',
        );
      }

      // ── 形状锚 ③：「看角色」不可省——工具结果就是文本节点 ──
      const toolResults = msgs.filter((m) => m.role === 'tool');
      assert(toolResults.length >= 1, '工具回路应留下 role=tool 的结果节点');
      const toolTexts = toolResults.filter((m) => typeOf(m) === 'text');
      assert(
        toolTexts.length >= 1,
        '至少一个工具结果是 **text 类型**——只看类型挡不住它，必须看角色',
      );
      assert(
        !msgs.some((m) => m.role === 'tool' && m.parent_id == null),
        '工具结果挂在 tool_call 之下（不在根级）——位置同样挡不住它',
      );

      // ── 形状锚 ④：「看类型」不可省——工作节点的角色与助手相同 ──
      for (const t of ['turn', 'tool_call', 'reasoning']) {
        const same = msgs.filter((m) => typeOf(m) === t);
        assert(same.length >= 1, `真实转写里应出现 ${t} 节点`);
        for (const m of same) {
          assert(
            m.role === 'assistant',
            `${t} 节点的 role 也是 assistant（实得 ${m.role}）——只看角色挡不住它，必须看类型`,
          );
        }
      }

      // ── 根级同时含「对话线节点」与「工作节点」：分栏在真实数据上不是空动作 ──
      //
      // 若根级全是助手文本，分栏后工作列就是空的（白拆一列）；若根级全是 `turn`，
      // 对话列就只剩用户问句。两个方向都得有东西，这一列才值得存在。
      assert(
        roots.some((m) => (m.role === 'assistant' || m.role === 'user') && typeOf(m) === 'text'),
        '根级应有对话线节点（用户问句 / 根级汇报）',
      );
      assert(
        roots.some((m) => typeOf(m) === 'turn'),
        '根级应有工作节点（turn 容器）——否则分栏后的工作列是空的',
      );

      // ── 工具结果正文不该出现在任何对话面文本里 ──
      //
      // 哨兵**取自数据**而不是写死一段：工具结果的形状由工具自己决定，写死就会
      // 变成「改一个工具就红一次」的脆弱断言。取最长的一条（短结果如 `ok` 可能与
      // 对话正文巧合相同，长结果不会）。
      const payloads = toolResults.map(textOf).filter((s) => s.length > 0);
      assert(payloads.length >= 1, '工具结果应带正文（否则这条锚没有哨兵）');
      const sentinel = payloads.reduce((a, b) => (b.length > a.length ? b : a));
      assert(
        sentinel.length >= 8,
        `工具结果正文太短，不构成哨兵（实得 ${JSON.stringify(sentinel)}）`,
      );
      const dialogTexts = msgs
        .filter((m) => m.role === 'assistant' || m.role === 'user')
        .map((m) => textOf(m))
        .join('\n');
      assert(
        !dialogTexts.includes(sentinel),
        '工具结果正文不得混进对话面文本（那可能含几万 token 的源码或命令输出）',
      );

      // ── 实时面同构：两个面板读同一份流，工作节点也在这份流上 ──
      const frames = realtime.messages();
      assert(frames.length >= 3, `实时面应收到消息帧（实得 ${frames.length} 条）`);
      const frameTypes = new Set(
        frames.map((f) => (f.data ? (f.data.type ?? f.data.msg_type) : null)).filter(Boolean),
      );
      assert(
        frameTypes.has('tool_call'),
        `实时面应含 tool_call 帧（实得 ${[...frameTypes].join('/')}）——` +
          '工作面板读的就是这份流；缺了它工作面板会空掉且没有错误信号',
      );
      assert(
        frameTypes.has('turn'),
        `实时面应含 turn 帧（实得 ${[...frameTypes].join('/')}）——工作面板按它分组成"一轮"`,
      );
      const hasUserFrame = frames.some((f) => f.data?.role === 'user');
      assert(hasUserFrame, '实时面应含用户消息帧（对话面板的另一半）');
      assert(
        realtime.outOfScope.length === 0,
        `订阅作用域外的帧应为零（实得 ${realtime.outOfScope.slice(0, 3).join(', ')}）`,
      );

      // 顺带把「汇报不付 LLM 往返」再确认一次：3 轮编排 = 3 次请求（S4 的锚，
      // 这里只作交叉验证——它是 T23 的主题）
      assertEq((await llm.requests()).length, 3, '3 轮编排应恰好发 3 次 LLM 请求');
    } finally {
      if (realtime) realtime.close();
      cli.stop();
      cleanupHomedir(hd);
      llm.stop();
    }
  },
);
