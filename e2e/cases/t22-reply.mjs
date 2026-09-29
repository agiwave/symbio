import './_selfrun.mjs';
// T22 对话面措辞（S3）：模板产线零 LLM / 生成产线答话进请求包 / 首响被剔除 / 卸载与开关。
//
// ## 本用例钉的是什么
//
// S3 让 `reply` 真的开始说话，并把 `session` 接上它。判决是**枚举**（S2 已落位），
// 措辞是**文本**——本用例验的不是"插件返回了什么字符串"，而是**会话转写里多出了
// 什么、请求包里少了什么**：
//
// | 线 | 输入 | 判决 | 措辞产线 | 可观测后果 |
// |---|---|---|---|---|
// | A 模板路径 | 「谢谢」 | `Answered{thanks}` | 模板 | **零** LLM 请求；一条根级 `surface=reply` 节点 |
// | B 生成路径 | 「我们刚才聊了什么」 | `Answered{from_context}` | 生成 | 分类 1 次 + 生成 1 次静默请求；答话**进**后续请求包 |
// | B 首响剔除 | 再接「读一下 README」 | `Escalate{needs_work}` | 模板 | 首响节点**不进**请求包（同一批 worker 请求里查无此句） |
// | C 卸载平凡值 | 「谢谢」+ 不挂载 `reply` | `Answered{thanks}` | 无 | **降级进工具循环**；转写里没有对话面文本 |
// | D 开关平凡值 | 「谢谢」+ `reply_enabled=false` | `Answered{thanks}` | 无 | 同上（验的是"分支写对了"，C 验的是"插件边界真的存在"） |
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
// ## 为什么平凡值线要另起 homedir 与进程
//
// `reply_enabled` 与停用位都在**装配期**定型（前者读进 `SessionConfig` 快照、
// 后者决定挂不挂），运行期改配置不会生效。因此 C / D 只能另起进程——它们验的
// 正是"这条路径在关掉之后与今天逐字一致"。
//
// 端口来自 homedir 里的插件配置，而**同一个 homedir 的端口是固定的**：复用 `hd` 的
// 进程就得复用它的端口，而那个端口刚被 kill 掉，可能还在 TIME_WAIT 里。C 线因此
// 另起 homedir 并把停用位**预置**进 manifest（`ensure_manifest` 不覆盖已存在的
// 文件 ⇒ 第一次装配就不构造 `reply`）。
//
// 三组 homedir / 端口分配：
//   A·B 共享第一个 homedir 与端口（同进程，只换会话 id）
//   C 另起 homedir + 端口（要它"从一开始就没有 reply"）
//   D 另起 homedir + 端口（要 `reply_enabled=false`）
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

/** 本用例的会话 id（一条断言线一个：四条线各自从零开始，断言互不污染） */
const SIDS = {
  template: 'e2e-t22-template',
  generate: 'e2e-t22-generate',
  unmounted: 'e2e-t22-unmounted',
  off: 'e2e-t22-off',
};

/** A 线期望的模板文本（与 `reply/templates.rs` 的 `REASON_THANKS` 行一致） */
const TEMPLATE_THANKS = '不客气。';
/** B 线期望的生成文本（由本用例的 mock 场景给出，逐字比对以证明它真的进了请求包） */
const ANSWER_FROM_CONTEXT = '我们刚才在聊 README 的事。';
/** B 线第二条消息的 `Escalate` 首响（与 `reply/templates.rs` 的 `REASON_NEEDS_WORK` 行一致） */
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
      // **生成**请求（B 线第 1 步）：`reply` 的措辞产线，同一句话、同一份对话线
      { id: 'gen-answer', match: '我们刚才聊了什么', content: ANSWER_FROM_CONTEXT },
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
        session: { triage_enabled: true, reply_enabled: true },
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

    /** 等该会话出现一条指定理由码的对话面节点 */
    const waitReplyNode = (homedir, sid, reason) =>
      waitFor(
        () =>
          (readMessagesJson(homedir, sid) ?? []).some(
            (m) => m.meta?.surface === 'reply' && m.meta?.reason === reason,
          ),
        { what: `${sid} 出现对话面节点（reason=${reason}）`, timeoutMs: 40_000 },
      );

    try {
      // ── A 模板路径：「谢谢」⇒ 规则短路 ⇒ 模板 ⇒ **零** LLM 请求 ──
      let base = await reqCount();
      await send(cli, SIDS.template, '谢谢', hd.workdir);
      await waitReplyNode(hd.homedir, SIDS.template, 'thanks');
      // 轮次收尾与"请求已发出"之间没有顺序保证，因此这里必须**等一个确定的量**：
      // 等到对话面节点落库之后，若模板路径真付了一次往返，它必然已经发出。
      await new Promise((r) => setTimeout(r, 800));
      assertEq(
        (await reqCount()) - base,
        0,
        '模板产线必须零 LLM 请求（致谢这类输入不该付任何往返）',
      );

      const tplMsgs = readMessagesJson(hd.homedir, SIDS.template);
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
      await waitReplyNode(hd.homedir, SIDS.generate, 'from_context');
      assertEq(
        (await reqCount()) - base,
        2,
        'B 线第 1 步应是"1 次分类 + 1 次生成"（都是无工具的内部请求）',
      );
      const genMsgs = readMessagesJson(hd.homedir, SIDS.generate);
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

      // ── B 首响剔除：再接一句「读一下 README」⇒ Escalate ⇒ 首响 + 工具循环 ──
      base = await reqCount();
      await send(cli, SIDS.generate, '读一下 README', hd.workdir);
      await waitReplyNode(hd.homedir, SIDS.generate, 'needs_work');
      await waitFor(
        () =>
          (readMessagesJson(hd.homedir, SIDS.generate) ?? []).some((m) => m.role === 'tool'),
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

      const escMsgs = readMessagesJson(hd.homedir, SIDS.generate);
      assertTranscriptInvariants(escMsgs, 'T22-B2');
      const escOpening = escMsgs.find((m) => m.meta?.reason === 'needs_work');
      assert(escOpening, 'B 线第 2 步应有首响节点');
      assertEq(escOpening.content, OPENING_NEEDS_WORK, '首响应拿到模板原文');
      assertEq(
        escOpening.meta?.exclude_from_context,
        true,
        'Escalate 的首响必须带剔除标记（界面开场白 ≠ 模型的对话内容）',
      );

      // ── C 卸载平凡值：不挂载 `reply` ⇒ 判决说能直接答却没人能说话 ⇒ 降级进工具循环 ──
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
          // 与 A/B 线**同一个完整值**：差别只有"挂不挂 reply"这一项，
          // 否则验的就不是"卸载"而是"两个变量一起变"。
          session: { triage_enabled: true, reply_enabled: true },
          reply: { plugin_enabled: false },
        },
      });
      const cli2 = startLongLivedCli({
        homedir: hdUn.homedir,
        workdir: hdUn.workdir,
        session: SIDS.unmounted,
        provider: PROVIDER_ID,
        gatewayPort: unmountedPort,
      });
      await cli2.waitGatewayReady(20_000, 'C 线进程（reply 未挂载）');
      try {
        // 停用后路由必须消失（卸载的判据是"路由 NotFound"，不是"读了 enabled 字段"）
        const gone = await cli2.invoke('reply/compose', {
          session_id: SIDS.unmounted,
          verdict: { verdict: 'answered', reason: 'thanks' },
          context: [],
        });
        assert(gone.status >= 400, `停用后 reply/compose 应不可达（实得 ${gone.status}）`);

        const unmBase = await reqCount();
        await send(cli2, SIDS.unmounted, '谢谢', hdUn.workdir);
        // 降级方向 = **进工具循环**（不是沉默）：因此必然出现 worker 请求与 Turn 节点。
        await waitFor(() => (readMessagesJson(hdUn.homedir, SIDS.unmounted) ?? []).some((m) => m.type === 'turn'), {
          what: 'C 线出现 Turn 节点（降级进工具循环）',
          timeoutMs: 40_000,
        });
        assert(
          (await reqCount()) - unmBase >= 1,
          'C 线应发一次 worker 请求——措辞拿不到时降级进工具循环，而不是沉默',
        );
        const unmMsgs = readMessagesJson(hdUn.homedir, SIDS.unmounted);
        assertTranscriptInvariants(unmMsgs, 'T22-C');
        assert(
          !unmMsgs.some((m) => m.meta?.surface === 'reply'),
          'C 线不该有任何对话面文本（没有 reply 插件，没人能说话）',
        );
      } finally {
        cli2.stop();
        cleanupHomedir(hdUn);
      }

      // ── D 开关平凡值：`reply_enabled = false` ⇒ 与 C 线同形（降级进工具循环）──
      //    C 与 D 断言相同、验证的却是两件不同的事：C 验"插件边界真的存在"（装配期
      //    不挂载），D 验"这个分支写对了"（插件挂着但调用方关掉了它）。缺任何一条，
      //    另一条都可能因为走错路径而**恰好**通过。
      const offPort = nextPort();
      const hdOff = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: {
          gateway: gatewayConfig(offPort),
          session: { triage_enabled: true, reply_enabled: false },
        },
      });
      const cli3 = startLongLivedCli({
        homedir: hdOff.homedir,
        workdir: hdOff.workdir,
        session: SIDS.off,
        provider: PROVIDER_ID,
        gatewayPort: offPort,
      });
      await cli3.waitGatewayReady(20_000, 'D 线进程（reply_enabled=false）');
      try {
        const offBase = await reqCount();
        await send(cli3, SIDS.off, '谢谢', hdOff.workdir);
        await waitFor(() => (readMessagesJson(hdOff.homedir, SIDS.off) ?? []).some((m) => m.type === 'turn'), {
          what: 'D 线出现 Turn 节点（降级进工具循环）',
          timeoutMs: 40_000,
        });
        assert(
          (await reqCount()) - offBase >= 1,
          'D 线应发一次 worker 请求——关掉措辞后判决仍会降级进工具循环',
        );
        const offMsgs = readMessagesJson(hdOff.homedir, SIDS.off);
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
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
