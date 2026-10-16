import './_selfrun.mjs';
// T22 对话面措辞（S3）：模板产线零 LLM / 生成产线答话进请求包 / 首响被剔除 / 卸载与开关。
//
// ## 本用例钉的是什么
//
// S3 让 `compose` 真的开始说话，并把 `session` 接上它。判决是**枚举**（S2 已落位），
// 措辞是**文本**——本用例验的不是"插件返回了什么字符串"，而是**会话转写里多出了
// 什么、请求包里少了什么**：
//
// | 线 | 输入 | 判决 | 措辞产线 | 可观测后果 |
// |---|---|---|---|---|
// | A 模板路径 | 「谢谢」 | `Answered{thanks}` | 模板 | **零** LLM 请求；一条根级 `surface=reply` 节点 |
// | B 生成路径 | 「我们刚才聊了什么」 | `Answered{from_context}` | 生成 | 分类 1 次 + 生成 1 次静默请求；答话**进**后续请求包 |
// | B 首响剔除 | 再接「读一下 README」 | `Escalate{needs_work}` | 模板 | 首响节点**不进**请求包（同一批 worker 请求里查无此句） |
// | B 首响到达时序 | 同上（同一次投递） | 同上 | 同上 | 首响帧在实时面上**早于**第一条工具结果帧，且只占一个节点 id |
// | C 卸载平凡值 | 「谢谢」+ 不挂载 `compose` | `Answered{thanks}` | 无 | **降级进工具循环**；转写里没有对话面文本 |
// | D 开关平凡值 | 「谢谢」+ `compose_enabled=false` | `Answered{thanks}` | 无 | 同上（验的是"分支写对了"，C 验的是"插件边界真的存在"） |
//
// ## 为什么 A 线必须是**零** LLM 请求
//
// 模板产线的全部价值就在这个零上：「你好」「谢谢」这类输入不该付任何往返。若
// 某天有人给模板路径加了一次"润色"请求，A 线会红——这正是它存在的意义。
//
// ## 为什么 B 线要接第二条消息
//
// 「答话进请求包」是一个**关于后续请求**的断言：不接第二条消息，本轮根本不发
// worker 请求，也就无从观察。而第二条消息（`Escalate`）顺带把**首响剔除**也验了
// ——两条断言落在同一批 worker 请求上，互为对照：
//
// - 答话（`Answered`，**不设** `exclude_from_context`）必须**在**包里；
// - 首响（`Escalate`，**设** `exclude_from_context`）必须**不在**包里。
//
// 两者若一起在或一起不在，说明两个标记被合并成了一个——那是本用例要拦的错。
//
// ## 为什么 B 线第 2 步还要看**到达时序**
//
// 上面那些断言数的都是「转写里有什么」——**落格**那一半。下发是另一件事，两者可以
// 各自成立：`chat.assistant.opening` 一直按时序在 WAL 里，实时面却可以等轮末才见到它
// （这段断言补进本用例之前正是如此），而用户看到的就是「一个跑了几分钟的轮次里
// 助手一个字都没有」。所以第 2 步在投递**之前**订阅会话实时面，断言首响帧早于**第一条
// 工具结果帧**——工具帧一直是当场下发的（`tools/tool_executor.rs`），它因此是一条不靠
// sleep、不靠墙钟的时序基准；方向反了即红。
//
// 为什么基准不取末轮正文（`t23` 那一半取的是它）：首响若攒到轮末，它会与末轮正文
// **同批**到达，两者谁先谁后取决于落库镜像的遍历顺序，不是这条缺陷的判据；工具结果
// 帧在第 1 个工具轮就到达，间隔是整个轮次，判别力更强。
//
// 反向自检实跑过：把写侧 `turn_runner.rs` 步骤 1a 那行 `sink.on_tool_frame` 注掉、
// 重建被测二进制复跑 ⇒ 本用例红在「首响帧的到达时刻要早于第一条工具结果帧」。
//
// ## 为什么平凡值线要另起 homedir 与进程
//
// `compose_enabled` 与停用位都在**装配期**定型（前者读进 `SessionConfig` 快照、
// 后者决定挂不挂），运行期改配置不会生效。因此 C / D 只能另起进程——它们验的
// 正是"这条路径在关掉之后与今天逐字一致"。
//
// 端口来自 homedir 里的插件配置，而**同一个 homedir 的端口是固定的**：复用 `hd` 的
// 进程就得复用它的端口，而那个端口刚被 kill 掉，可能还在 TIME_WAIT 里。C 线因此
// 另起 homedir 并把停用位**预置**进 manifest（`ensure_manifest` 不覆盖已存在的
// 文件 ⇒ 第一次装配就不构造 `compose`）。
//
// 三组 homedir / 端口分配：
//   A·B 共享第一个 homedir 与端口（同进程，只换会话 id）
//   C 另起 homedir + 端口（要它"从一开始就没有 compose"）
//   D 另起 homedir + 端口（要 `compose_enabled=false`）
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  readSessionMessages,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id（一条断言线一个：四条线各自从零开始，断言互不污染） */
const SIDS = {
  template: 'e2e-t22-template',
  generate: 'e2e-t22-generate',
  unmounted: 'e2e-t22-unmounted',
  off: 'e2e-t22-off',
};

/** A 线期望的模板文本（与 `compose/templates.rs` 的 `REASON_THANKS` 行一致） */
const TEMPLATE_THANKS = '不客气。';
/** B 线期望的生成文本（由本用例的 mock 场景给出，逐字比对以证明它真的进了请求包） */
const ANSWER_FROM_CONTEXT = '我们刚才在聊 README 的事。';
/** B 线第二条消息的 `Escalate` 首响（与 `compose/templates.rs` 的 `REASON_NEEDS_WORK` 行一致） */
const OPENING_NEEDS_WORK = '好，我来处理。';

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

/** 把一个请求包里所有消息的正文拍平成一串（用于断言"某句话在不在包里"）。 */
function packText(request) {
  return (request.body?.messages ?? [])
    .map((m) => (typeof m.content === 'string' ? m.content : JSON.stringify(m.content ?? '')))
    .join('\n');
}

/** 带工具的那些请求 = worker 的请求（分类 / 生成都是无工具的内部请求）。 */
function workerRequests(requests) {
  return requests.filter((r) => (r.body?.tools ?? []).length > 0);
}

export default defineCase(
  'T22 对话面措辞：模板零 LLM / 生成答话进请求包 / Escalate 首响被剔除 / 卸载与开关平凡值',
  async () => {
    // 场景表按**数组顺序**匹配，`match` 是对最后一条 user 消息的子串匹配。
    //
    // B 线第 1 步会发**两次**请求，且两次的"最后一条 user 消息"是**同一句**
    // （分类看的是这句话，生成看的是同一条对话线）：区分手段只有顺序 + `once`
    // ——分类先发生，用 `once` 把它消耗掉，生成于是落到下一条匹配上。
    const llm = await new MockLlm([
      // 分类请求（B 线第 1 步）⇒ 判「能凭上下文直接答」⇒ 走生成产线
      { id: 'cls-direct', match: '我们刚才聊了什么', content: 'direct', once: true },
      // **生成**请求（B 线第 1 步）：`compose` 的措辞产线，同一句话、同一份对话线。
      // 带 `reasoning` ⇒ 生成产线的思考事实要随答话一路入格（Reason 节点）。
      {
        id: 'gen-answer',
        match: '我们刚才聊了什么',
        content: ANSWER_FROM_CONTEXT,
        reasoning: '想一想：用户在问对话内容，直接从上下文总结即可。',
      },
      // 分类请求（B 线第 2 步，once）⇒ 判「要干活」⇒ 走模板产线（首响）
      { id: 'cls-work', match: '读一下 README', content: 'work', once: true },
      // 工具循环（B 线第 2 步）：读文件
      {
        id: 'read-readme',
        match: '读一下 README',
        toolCalls: [{ id: 'call_r1', name: 'vdfs_read', arguments: { path: 'README.md' } }],
      },
      { id: 'after-read', afterTool: true, content: 'README 已经读完了。' },
      // C / D 两条平凡值线：走到这里说明"判决说能直接答、却没人能说话" ⇒ 降级进工具循环
      { id: 'degraded', match: '谢谢', content: '（平凡值线：照旧进工具循环）' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        // 两个开关出厂默认都已是 `true`（S3 起）。这里仍**显式**写出，是让本用例的
        // 前提自证——不依赖默认值，翻转默认值不会悄悄改变本用例验的是什么。
        session: { classify_enabled: true, compose_enabled: true },
      },
    });

    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: SIDS.template,
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    await cli.waitGatewayReady(20_000, 'A·B 线进程');

    /** mock 收到的请求数（**累计**——每条线开跑前取一次基线相减） */
    const reqCount = async () => (await llm.requests()).length;

    /** 建会话并把一条消息写进收件箱（**唯一输入入口**） */
    const send = async (c, sid, text, workdir) => {
      const root = (await c.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${sid}`;

      const created = await c.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({
          metadata: { workdir, mode: 'auto', risk_level: 'medium' },
        }),
      });
      assertEq(created.status, 200, `创建会话 ${sid}（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      const w = await c.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text });
      assertEq(w.status, 200, `写收件箱 ${sid}（${JSON.stringify(w.body)?.slice(0, 300)}）`);
    };

    /** 等该会话出现一条指定理由码的对话面节点（读出口判据） */
    const waitReplyNode = (c, sid, reason) =>
      waitFor(
        async () =>
          ((await readSessionMessages(c, sid)) ?? []).some(
            (m) => m.meta?.surface === 'reply' && m.meta?.reason === reason,
          ),
        { what: `${sid} 出现对话面节点（reason=${reason}）`, timeoutMs: 40_000 },
      );

    /** B 线第 2 步的实时面订阅（在投递之前建立，见 §为什么还要看到达时序） */
    let subB2 = null;

    try {
      // ── A 模板路径：「谢谢」⇒ 规则短路 ⇒ 模板 ⇒ **零** LLM 请求 ──
      let base = await reqCount();
      await send(cli, SIDS.template, '谢谢', hd.workdir);
      await waitReplyNode(cli, SIDS.template, 'thanks');
      // 轮次收尾与"请求已发出"之间没有顺序保证，因此这里必须**等一个确定的量**：
      // 等到对话面节点落库之后，若模板路径真付了一次往返，它必然已经发出。
      await new Promise((r) => setTimeout(r, 800));
      assertEq(
        (await reqCount()) - base,
        0,
        '模板产线必须零 LLM 请求（致谢这类输入不该付任何往返）',
      );

      const tplMsgs = await readSessionMessages(cli, SIDS.template);
      assertTranscriptInvariants(tplMsgs, 'T22-A');
      const tplReply = tplMsgs.filter((m) => m.meta?.surface === 'reply');
      assertEq(tplReply.length, 1, 'A 线应恰有一条对话面节点');
      assertEq(tplReply[0].role, 'assistant', '对话面节点是助手说的话');
      assertEq(tplReply[0].type, 'text', '对话面节点是文本节点');
      assertEq(tplReply[0].content, TEMPLATE_THANKS, 'A 线应拿到致谢模板的原文');
      assert(
        tplReply[0].parent_id == null,
        '答话必须挂在根级（对话线），不是某个 Turn 的子节点',
      );
      assertEq(
        tplReply[0].meta?.exclude_from_context,
        undefined,
        'Answered 的答话就是这一轮的答复 ⇒ 必须进请求包（不得带剔除标记）',
      );
      assert(
        !tplMsgs.some((m) => m.type === 'tool_call'),
        'A 线判决是"能直接答" ⇒ 不该有工具调用节点',
      );
      assert(
        !tplMsgs.some((m) => m.type === 'turn'),
        'A 线不进工具循环 ⇒ 不该有 Turn 组合节点',
      );

      // ── B 生成路径：「我们刚才聊了什么」⇒ 分类 + 生成（各一次静默请求）──
      base = await reqCount();
      await send(cli, SIDS.generate, '我们刚才聊了什么', hd.workdir);
      await waitReplyNode(cli, SIDS.generate, 'from_context');
      assertEq(
        (await reqCount()) - base,
        2,
        'B 线第 1 步应是"1 次分类 + 1 次生成"（都是无工具的内部请求）',
      );
      const genMsgs = await readSessionMessages(cli, SIDS.generate);
      assertTranscriptInvariants(genMsgs, 'T22-B1');
      const genReply = genMsgs.filter((m) => m.meta?.surface === 'reply');
      assertEq(genReply.length, 1, 'B 线应恰有一条对话面节点');
      assertEq(genReply[0].content, ANSWER_FROM_CONTEXT, '答话应是生成产线的产物（不是模板）');
      assert(
        genReply[0].parent_id == null,
        '答话必须挂在根级（对话线），不是某个 Turn 的子节点',
      );
      assert(
        !genMsgs.some((m) => m.type === 'turn'),
        'B 线第 1 步不进工具循环 ⇒ 不该有 Turn 组合节点（生成请求也不得泄漏成可见轮次）',
      );
      // 思考事实入格：生成产线是这条轮次里唯一一次真实模型推理，mock 已带
      // `reasoning` ⇒ 收束格载荷携带 reasoning/reasoning_id ⇒ 重建复刻 Reason 节点
      // （挂在答话节点之下）。此前它在措辞插件出口就被丢弃，Reason 节点从未存在。
      const genReason = genMsgs.find(
        (m) =>
          m.type === 'reasoning' &&
          m.parent_id === genReply[0].id,
      );
      assert(genReason, '答话节点下应有 Reason 子节点（思考事实入格）');
      assertEq(genReason.content, '想一想：用户在问对话内容，直接从上下文总结即可。', 'Reason 节点正文 = 生成时的思考内容');
      assertEq(genReason.status, 'completed', 'Reason 节点落库即终态');

      // ── B 首响剔除：再接一句「读一下 README」⇒ Escalate ⇒ 首响 + 工具循环 ──
      base = await reqCount();
      // 实时面在**投递之前**订阅：本步的时序判据看的是到达序，晚一步订阅等于把要验的
      // 那一段截掉（与 Tauri 前端同构：订阅一次覆盖整个会话目录）。
      subB2 = await subscribeSessionRealtime(cli, {
        sessionId: SIDS.generate,
        gatewayPort: GATEWAY_PORT,
      });
      await send(cli, SIDS.generate, '读一下 README', hd.workdir);
      await waitReplyNode(cli, SIDS.generate, 'needs_work');
      await waitFor(
        async () => ((await readSessionMessages(cli, SIDS.generate)) ?? []).some((m) => m.role === 'tool'),
        { what: 'B 线第 2 步工具结果落库', timeoutMs: 40_000 },
      );

      const allReqs = await llm.requests();
      const worker = workerRequests(allReqs);
      assert(worker.length >= 1, 'B 线第 2 步应至少发一次 worker 请求（工具循环真的跑了）');
      for (const r of worker) {
        const text = packText(r);
        assert(
          text.includes(ANSWER_FROM_CONTEXT),
          '上一轮 Answered 的答话必须进请求包（它是对话内容，不是界面文本）',
        );
        assert(
          !text.includes(OPENING_NEEDS_WORK),
          'Escalate 的首响必须被剔除出请求包（它是界面开场白；进了会让线上出现连续两条 assistant）',
        );
      }

      const escMsgs = await readSessionMessages(cli, SIDS.generate);
      assertTranscriptInvariants(escMsgs, 'T22-B2');
      const escOpening = escMsgs.find((m) => m.meta?.reason === 'needs_work');
      assert(escOpening, 'B 线第 2 步应有首响节点');
      assertEq(escOpening.content, OPENING_NEEDS_WORK, '首响应拿到模板原文');
      assertEq(
        escOpening.meta?.exclude_from_context,
        true,
        'Escalate 的首响必须带剔除标记（界面开场白 ≠ 模型的对话内容）',
      );

      // ── 到达时序：首响帧早于第一条工具结果帧（T-136 开场白那一半的常驻判据）──
      //
      // 上面那段验的是**转写里有什么**（落格），这一段验的是**实时面什么时候给的**
      // （下发）。基准取第一条工具结果帧：工具帧一直是当场下发的，而首响若只随轮末的
      // 落库镜像下发，就会排在整个工具循环之后。
      const liveB2 = subB2.messages();
      const arrivedAt = (pred) => {
        const hit = liveB2.find((m) => pred(m.data));
        return hit ? hit.arrival : -1;
      };
      const openingAt = arrivedAt((d) => d?.meta?.reason === 'needs_work');
      const toolResultAt = arrivedAt((d) => d?.role === 'tool');
      assert(
        openingAt >= 0,
        `首响必须出现在实时面上（只在转写里看得见 = 只落格不下发）：帧数 ${liveB2.length}`,
      );
      assert(
        toolResultAt >= 0,
        `第一条工具结果帧也在实时面上（时序基准）：帧数 ${liveB2.length}`,
      );
      assert(
        openingAt < toolResultAt,
        `首响帧的到达时刻要早于第一条工具结果帧（实得 opening=${openingAt} tool=${toolResultAt}）：` +
          `那句「我来处理」是说给正在等的用户听的，压到轮末等于抵消它的设计目的`,
      );
      // 身份只有一份：轮首那一句在实时面上就是一个节点。落格载荷与下发若用两个 id，
      // 轮末镜像会再多出一张卡。
      assertEq(
        new Set(
          liveB2.filter((m) => m.data?.meta?.reason === 'needs_work').map((m) => m.mid),
        ).size,
        1,
        '一个轮首在实时面上就是一个节点（不多成两张卡）',
      );

      // ── C 卸载平凡值：不挂载 `compose` ⇒ 判决说能直接答却没人能说话 ⇒ 降级进工具循环 ──
      //    「没有这个插件也能正确运行」的可执行形式不是配置开关，而是**装配期**的
      //    不挂载：停用即"根本不构造"，路由随之 `NotFound`，`session` 按"缺插件"降级。
      //
      //    实现上是**预置**停用位（而不是先跑一遍再改 manifest）：`ensure_manifest`
      //    对已存在的文件直接返回，`mount_all` 扫到 `plugin_enabled: false` 就跳过
      //    ⇒ 第一次装配就不构造它。这样 C 线不必重启 `hd` 的进程，也就不必复用
      //    `hd` 的网关端口（复用刚被 kill 的端口会撞上 TIME_WAIT）。
      const unmountedPort = nextPort();
      const hdUn = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: {
          gateway: gatewayConfig(unmountedPort),
          // 与 A/B 线**同一个完整值**：差别只有"挂不挂 compose"这一项，
          // 否则验的就不是"卸载"而是"两个变量一起变"。
          session: { classify_enabled: true, compose_enabled: true },
          compose: { plugin_enabled: false },
        },
      });
      const cli2 = startLongLivedCli({
        homedir: hdUn.homedir,
        workdir: hdUn.workdir,
        session: SIDS.unmounted,
        provider: PROVIDER_ID,
        gatewayPort: unmountedPort,
      });
      await cli2.waitGatewayReady(20_000, 'C 线进程（compose 未挂载）');
      try {
        // 停用后路由必须消失（卸载的判据是"路由 NotFound"，不是"读了 enabled 字段"）
        const gone = await cli2.invoke('compose/compose', {
          session_id: SIDS.unmounted,
          verdict: { verdict: 'answered', reason: 'thanks' },
          context: [],
        });
        assert(gone.status >= 400, `停用后 compose/compose 应不可达（实得 ${gone.status}）`);

        const unmBase = await reqCount();
        await send(cli2, SIDS.unmounted, '谢谢', hdUn.workdir);
        // 降级方向 = **进工具循环**（不是沉默）：因此必然出现 worker 请求与 Turn 节点。
        // 两个判据**在同一个轮询里一起等**，而不是「先等 Turn 再数请求」——后者有竞态：
        // Turn 节点与 worker 请求几乎同时可见，`waitFor` 一返回就立刻 `reqCount()`
        // 可能抢在请求被 mock 记账之前（实测偶发 C 线「应发一次 worker 请求」失败，
        // 单独重跑必过）。把两者合成一个条件，「等到」与「判据」就是同一个瞬间。
        await waitFor(
          async () =>
            (await reqCount()) - unmBase >= 1 &&
            ((await readSessionMessages(cli2, SIDS.unmounted)) ?? []).some((m) => m.type === 'turn'),
          { what: 'C 线降级进工具循环（worker 请求 + Turn 节点）', timeoutMs: 40_000 },
        );
        const unmMsgs = await readSessionMessages(cli2, SIDS.unmounted);
        assertTranscriptInvariants(unmMsgs, 'T22-C');
        assert(
          !unmMsgs.some((m) => m.meta?.surface === 'reply'),
          'C 线不该有任何对话面文本（没有 compose 插件，没人能说话）',
        );
      } finally {
        cli2.stop();
        cleanupHomedir(hdUn);
      }

      // ── D 开关平凡值：`compose_enabled = false` ⇒ 与 C 线同形（降级进工具循环）──
      //    C 与 D 断言相同、验证的却是两件不同的事：C 验"插件边界真的存在"（装配期
      //    不挂载），D 验"这个分支写对了"（插件挂着但调用方关掉了它）。缺任何一条，
      //    另一条都可能因为走错路径而**恰好**通过。
      const offPort = nextPort();
      const hdOff = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: {
          gateway: gatewayConfig(offPort),
          session: { classify_enabled: true, compose_enabled: false },
        },
      });
      const cli3 = startLongLivedCli({
        homedir: hdOff.homedir,
        workdir: hdOff.workdir,
        session: SIDS.off,
        provider: PROVIDER_ID,
        gatewayPort: offPort,
      });
      await cli3.waitGatewayReady(20_000, 'D 线进程（compose_enabled=false）');
      try {
        const offBase = await reqCount();
        await send(cli3, SIDS.off, '谢谢', hdOff.workdir);
        // 同 C 线：两个判据合成一个轮询（理由见 C 线的注释）。
        await waitFor(
          async () =>
            (await reqCount()) - offBase >= 1 &&
            ((await readSessionMessages(cli3, SIDS.off)) ?? []).some((m) => m.type === 'turn'),
          { what: 'D 线降级进工具循环（worker 请求 + Turn 节点）', timeoutMs: 40_000 },
        );
        const offMsgs = await readSessionMessages(cli3, SIDS.off);
        assertTranscriptInvariants(offMsgs, 'T22-D');
        assert(
          !offMsgs.some((m) => m.meta?.surface === 'reply'),
          'D 线不该有任何对话面文本（开关关掉了措辞）',
        );
      } finally {
        cli3.stop();
        cleanupHomedir(hdOff);
      }
    } finally {
      if (subB2) subB2.close();
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
