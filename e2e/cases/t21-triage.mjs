import './_selfrun.mjs';
// T21 轮首判决（S2）：规则短路 / 快速档分类 / 需要干活 / 开关与卸载的平凡值。
//
// ## 本用例钉的是什么
//
// S2 让 `triage` 真的开始判决，并把 `session` 接上它。判决是**枚举**，编排层据此
// **执行**——因此本用例验的不是"插件返回了什么"，而是**会话行为随判决改变**：
//
// | 线 | 输入 | 判决 | 会话行为 |
// |---|---|---|---|
// | A 规则短路 | 「谢谢」 | `Answered{thanks}` | **零** LLM 请求，不进工具循环 |
// | B 快速档 | 「我们刚才聊了什么」 | `Answered{from_context}` | 有分类请求；**无**工具节点 |
// | C 需要干活 | 「读一下 README」 | `Escalate{needs_work}` | 有工具节点（工具真跑） |
// | D 开关平凡值 | 「谢谢」+ `triage_enabled=false` | 不判决 | 与今天逐字一致（有 LLM 请求） |
// | E 卸载平凡值 | 「谢谢」+ 不挂载 `triage` | 不判决 | 与今天逐字一致（有 LLM 请求） |
//
// ## 为什么断言落在"请求数"与"节点"上，而不是"判决值"
//
// 判决值是**中间产物**：`triage` 的返回只在进程内活一次，落库的是它的**后果**。
// 拿判决值当断言，等于测一个用户看不到也存不下来的东西；而"零 LLM 请求"
// 与"没有工具节点"是同一件事的**可观测形式**——它们才是判据。
// 判决值本身（规则表命中什么、四选一怎么映射）由单测穷举，见
// `symbio/src/plugins/triage/rules.test.rs` 与 `classify.test.rs`。
//
// ## 为什么每一条线都要"先取请求基线"
//
// 一个 mock 实例服务全部五条线，`requests()` 是**累计**的。基线相减是唯一不会
// 随"再加一条线"而失效的写法（T19 已经踩过这个坑）。
//
// ## 为什么平凡值线要另起 homedir 与进程
//
// 两个开关都在**启动时**读进内存（`triage_enabled` 进 `SessionConfig`、停用进装配期），
// 运行期改配置不会生效。因此 D / E 两条平凡值线只能另起进程——它们验的正是
// "这条路径在关掉之后与今天逐字一致"。
//
// 端口来自 homedir 里的插件配置，而**同一个 homedir 的端口是固定的**：复用 `hd` 的
// 进程就得复用它的端口，而那个端口刚被 kill 掉，可能还在 TIME_WAIT 里。E 线因此
// 不复用 `hd`，而是另起一个 homedir 并把停用位**预置**进 manifest
// （`ensure_manifest` 不覆盖已存在的文件 ⇒ 第一次装配就不构造 `triage`）。
//
// 三组 homedir / 端口分配：
//   A·B·C 共享第一个 homedir 与端口（同进程，只换会话 id）
//   D 另起 homedir + 端口（要 `triage_enabled=false`）
//   E 另起 homedir + 端口（要它"从一开始就没有 triage"）
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

/** 本用例的会话 id（一条断言线一个：五条线各自从零开始，断言互不污染） */
const SIDS = {
  rule: 'e2e-t21-rule',
  classify: 'e2e-t21-classify',
  work: 'e2e-t21-work',
  off: 'e2e-t21-off',
  unmounted: 'e2e-t21-unmounted',
};

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

export default defineCase(
  'T21 轮首判决：规则短路零 LLM / 快速档不进工具循环 / 需要干活照旧 / 开关与卸载平凡值',
  async () => {
    // 场景表按**数组顺序**匹配，`match` 是对最后一条 user 消息的子串匹配。
    //
    // 分类请求与工具循环请求的"最后一条 user 消息"**是同一条**（都是用户这句话），
    // 因此区分两者的唯一手段是顺序 + `once`：分类请求先发生，用它消耗掉
    // `once` 的那一条；工具循环的请求于是落到下一条匹配上。
    const llm = await new MockLlm([
      // 分类请求（B 线）：模糊问题 ⇒ 判「能凭上下文直接答」
      { id: 'cls-direct', match: '我们刚才聊了什么', content: 'direct' },
      // 分类请求（C 线，once）⇒ 判「要干活」
      { id: 'cls-work', match: '读一下 README', content: 'work', once: true },
      // 工具循环（C 线）：读文件
      {
        id: 'read-readme',
        match: '读一下 README',
        toolCalls: [
          { id: 'call_r1', name: 'vdfs_read', arguments: { path: 'README.md' } },
        ],
      },
      { id: 'after-read', afterTool: true, content: 'README 已经读完了。' },
      // D / E 两条平凡值线：没有判决，输入直接进工具循环 ⇒ 必须有一次请求可回
      { id: 'plain', content: '（平凡值线：照旧进工具循环）' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        // 判决**出厂默认关闭**（见 `SessionConfig::triage_enabled` 的文档）：
        // 本用例把它显式打开，才有 A / B / C 三条线。
        session: { triage_enabled: true },
      },
    });

    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: SIDS.rule,
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    await cli.waitGatewayReady(20_000, 'A·B·C 线进程');

    /** mock 收到的请求数（**累计**——每条线开跑前取一次基线相减） */
    const reqCount = async () => (await llm.requests()).length;

    /** 建会话并把一条消息写进收件箱（**唯一输入入口**） */
    const send = async (sid, text) => {
      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${sid}`;

      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({ metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' } }),
      });
      assertEq(created.status, 200, `创建会话 ${sid}（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      const w = await cli.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text });
      assertEq(w.status, 200, `写收件箱 ${sid}（${JSON.stringify(w.body)?.slice(0, 300)}）`);
    };

    /** 等该会话的消息数达到 n（`readMessagesJson` 落盘判据） */
    const waitMessages = (sid, n, what) =>
      waitFor(() => (readMessagesJson(hd.homedir, sid) ?? []).length >= n, {
        what,
        timeoutMs: 40_000,
      });

    /**
     * 等一条**注定不产生任何 LLM 请求**的轮次收尾。
     *
     * 判据是"输入被消费"：收件箱条目被抽干后写入的 user 消息会落库
     * （`readMessagesJson` 能看到它），而本轮没有后续节点。等它出现再断言请求数，
     * 断言才有意义——否则可能在会话还没开始跑的时候就读到一个空请求表。
     */
    const waitTurnConsumed = (sid) => waitMessages(sid, 1, `${sid} 的输入被消费并落库`);

    try {
      // ── A 规则短路：「谢谢」⇒ 判决在规则表里出结果，零 LLM 请求 ──
      let base = await reqCount();
      await send(SIDS.rule, '谢谢');
      await waitTurnConsumed(SIDS.rule);
      // 轮次收尾与"请求已发出"之间没有顺序保证，因此这里必须**等一个确定的量**：
      // 等到该会话有输入落库之后，再给分类请求留出的窗口若真存在，它必然已经发出。
      await new Promise((r) => setTimeout(r, 800));
      assertEq(
        (await reqCount()) - base,
        0,
        '规则短路必须零 LLM 请求（问候 / 致谢这类输入不该付一次分类往返）',
      );
      const ruleMsgs = readMessagesJson(hd.homedir, SIDS.rule);
      assertTranscriptInvariants(ruleMsgs, 'T21-A');
      assert(
        !ruleMsgs.some((m) => m.type === 'tool_call'),
        'A 线不该出现工具调用节点（判决说不用干活）',
      );

      // ── B 快速档：「我们刚才聊了什么」⇒ 一次分类请求，但**不进工具循环** ──
      base = await reqCount();
      await send(SIDS.classify, '我们刚才聊了什么');
      await waitFor(async () => (await reqCount()) - base >= 1, {
        what: 'B 线出现分类请求',
        timeoutMs: 40_000,
      });
      await waitMessages(SIDS.classify, 1, 'B 线的输入落库');
      const clsMsgs = readMessagesJson(hd.homedir, SIDS.classify);
      assertTranscriptInvariants(clsMsgs, 'T21-B');
      assert(
        !clsMsgs.some((m) => m.type === 'tool_call'),
        'B 线判决是"能直接答" ⇒ 不该有工具调用节点',
      );
      assert(
        !clsMsgs.some((m) => m.type === 'turn'),
        'B 线不进工具循环 ⇒ 不该有 Turn 组合节点',
      );

      // 分类请求是**内部请求**：它必须静默（不产生任何可见节点）。
      // 反过来说，B 线落库的节点里除了那条 user 消息什么都不该有。
      assertEq(
        clsMsgs.filter((m) => m.role === 'assistant').length,
        0,
        '分类请求不得留下任何助手侧节点（它是静默的内部请求）',
      );

      // ── C 需要干活：「读一下 README」⇒ Escalate ⇒ 工具真的跑 ──
      base = await reqCount();
      await send(SIDS.work, '读一下 README');
      await waitFor(() => (readMessagesJson(hd.homedir, SIDS.work) ?? []).some((m) => m.type === 'tool_call'), {
        what: 'C 线出现工具调用节点',
        timeoutMs: 40_000,
      });
      await waitFor(() => (readMessagesJson(hd.homedir, SIDS.work) ?? []).some((m) => m.role === 'tool'), {
        what: 'C 线工具结果落库',
        timeoutMs: 40_000,
      });
      const workMsgs = readMessagesJson(hd.homedir, SIDS.work);
      assertTranscriptInvariants(workMsgs, 'T21-C');
      assert(
        (await reqCount()) - base >= 2,
        'C 线应有分类请求 + 工具循环请求（至少两次）',
      );
      assert(
        workMsgs.some((m) => m.role === 'assistant' && m.type === 'text' && (m.content ?? '').includes('README')),
        'C 线应有模型给出的正文（判决放行后工具循环照旧跑完）',
      );
      // 分类请求是**内部请求**，必须静默。但 C 线不能照抄 B 线那条"零助手节点"的
      // 判据：这里 session 循环本身就在发节点，而 Turn 根节点是**每轮一个**
      // （`prepare_turn_inputs` 内的 `emit_streaming_start`，`meta.turn` = 轮序）。
      // 能成立的是**数量对齐**：工具循环发了几次请求，就有几个 Turn 根节点——
      // 分类请求那一次不在其中。它若泄漏成可见轮次，这里会多一个。
      const cReqs = (await reqCount()) - base;
      const turnCount = workMsgs.filter((m) => m.type === 'turn').length;
      assertEq(
        turnCount,
        cReqs - 1,
        `C 线的 Turn 数应等于工具循环请求数（共 ${cReqs} 次请求，含 1 次静默分类）`,
      );

      // ── D 开关平凡值：`triage_enabled = false` ⇒ 行为与今天逐字一致 ──
      //    这个布尔在**启动时**读进 `SessionConfig`，运行期改它不生效；因此本线
      //    另起 homedir + 进程（只换会话 id 是没用的）。
      //    另注：本线写的是**显式** `false` 而不是靠缺省——缺省那条路由
      //    `session/config.test.rs` 的 `triage_is_off_by_default` 单测钉住，
      //    这里验的是"这个开关本身有效"。
      const offPort = nextPort();
      const hdOff = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: {
          gateway: gatewayConfig(offPort),
          session: { triage_enabled: false },
        },
      });
      const cliOff = startLongLivedCli({
        homedir: hdOff.homedir,
        workdir: hdOff.workdir,
        session: SIDS.off,
        provider: PROVIDER_ID,
        gatewayPort: offPort,
      });
      await cliOff.waitGatewayReady(20_000, 'D 线进程（另一个 homedir）');
      try {
        const offBase = await reqCount();
        const root = (await cliOff.invoke('vdfs/root', {})).body?.data?.path;
        const sessionAddr = `${root.replace(/\/+$/, '')}/session/${SIDS.off}`;
        await cliOff.invoke('vdfs/write', {
          path: sessionAddr,
          create: true,
          text: JSON.stringify({ metadata: { workdir: hdOff.workdir, mode: 'auto', risk_level: 'medium' } }),
        });
        await cliOff.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text: '谢谢' });
        // 关掉判决 ⇒ 「谢谢」照旧进工具循环 ⇒ 必然有一次 LLM 请求
        await waitFor(async () => (await reqCount()) - offBase >= 1, {
          what: 'D 线（判决关闭）出现 LLM 请求',
          timeoutMs: 40_000,
        });
      } finally {
        cliOff.stop();
        cleanupHomedir(hdOff);
      }

      // ── E 卸载平凡值：不挂载 `triage` ⇒ 行为与今天逐字一致 ──
      //    「没有这个插件也能正确运行」的可执行形式不是配置开关，而是**装配期**的
      //    不挂载：停用即"根本不构造"，路由随之 `NotFound`，`session` 按"缺插件"放行。
      //
      //    实现上是**预置**停用位（而不是像 T17/T20 那样先跑一遍再改 manifest）：
      //    `ensure_manifest` 对已存在的文件直接返回，`mount_all` 扫到 `plugin_enabled:
      //    false` 就跳过 ⇒ 第一次装配就不构造它。这样 E 线不必重启 `hd` 的进程，
      //    也就不必复用 `hd` 的网关端口（复用刚被 kill 的端口会撞上 TIME_WAIT）。
      const unmountedPort = nextPort();
      const hdUn = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: {
          gateway: gatewayConfig(unmountedPort),
          // 与 A/B/C 线**同一个完整值**：差别只有"挂不挂 triage"这一项，
          // 否则验的就不是"卸载"而是"两个变量一起变"。
          session: { triage_enabled: true },
          triage: { plugin_enabled: false },
        },
      });
      const cli2 = startLongLivedCli({
        homedir: hdUn.homedir,
        workdir: hdUn.workdir,
        session: SIDS.unmounted,
        provider: PROVIDER_ID,
        gatewayPort: unmountedPort,
      });
      await cli2.waitGatewayReady(20_000, 'E 线进程（triage 未挂载）');
      try {
        // 停用后路由必须消失（卸载的判据是"路由 NotFound"，不是"读了 enabled 字段"）
        const gone = await cli2.invoke('triage/decide', { session_id: SIDS.unmounted, utterance: '你好' });
        assert(gone.status >= 400, `停用后 triage/decide 应不可达（实得 ${gone.status}）`);

        const unmBase = await reqCount();
        const root = (await cli2.invoke('vdfs/root', {})).body?.data?.path;
        const sessionAddr = `${root.replace(/\/+$/, '')}/session/${SIDS.unmounted}`;
        await cli2.invoke('vdfs/write', {
          path: sessionAddr,
          create: true,
          text: JSON.stringify({ metadata: { workdir: hdUn.workdir, mode: 'auto', risk_level: 'medium' } }),
        });
        await cli2.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text: '谢谢' });
        await waitFor(async () => (await reqCount()) - unmBase >= 1, {
          what: 'E 线（卸载判决插件）出现 LLM 请求',
          timeoutMs: 40_000,
        });
        const unmMsgs = readMessagesJson(hdUn.homedir, SIDS.unmounted) ?? [];
        assertTranscriptInvariants(unmMsgs, 'T21-E');
      } finally {
        cli2.stop();
        cleanupHomedir(hdUn);
      }
    } finally {
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
