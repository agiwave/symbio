import './_selfrun.mjs';
// T89 扩展夹具：`notes` 能力包只经 plugin-api 接入后，八个扩展面在真实链路上生效。
//
// ## 这条用例补的是什么
//
// T88 证的是「插件的工具随装配位进出」；本用例证的是 DE-025 的**扩展面本身**
// （EX-001）：一个新能力包不改 core / kernel / loop / session，只靠 plugin-api 的
// 提交点就能做到——工具、前置判断、领域事实、效果、上下文、**对外可读的视图**、
// **轮内行执行**、**模型往返**八面：
//
// | 面 | 提交点 | 本用例的读数 |
// |---|---|---|
// | 工具 | `traverse` + `CapabilityRow::tool` | 模型请求的 `tools` 里出现 `notes_add` |
// | 前置判断 | `submit_behavior!` + `ToolBehavior::before` | 重复便签被拒，工具结果里带回 `Refused: …` |
// | 领域事实 | `submit_declarations!`（`Domain × Opened`） | 工具轮结束后事件真的落格 |
// | 效果 | `submit_behavior!` + `CapabilityMeta.behaviors` | 下一轮上下文里出现那条便签 |
// | 上下文 | `submit_context_provider!` | 下一轮请求前缀里出现 `最近记下的便签` |
// | 视图 | `ViewDecl` + `PLUGIN.yml` 的 `shared_views` | `session/stats` 的 `capability_views['notes.recent']` 读到那条便签 |
// | 轮内行执行 | `submit_row!`（`by = plugin_row`，D-241） | `capability_views['notes.digest']` 读到行执行体落下的 `notes.indexed` |
// | 模型往返 | `RowStep::Ask` + `submit_row!`（D-242） | 行执行体借宿主一次往返；`capability_views['notes.insights']` 读到 `notes.insight` |
//
// 「事件真的落格」与「上下文真的贡献了」是**同一条读数的两面**：上下文 provider 只读
// 会话事实切片，它渲染得出来，就说明行为产出的 `notes.created` 已经入格。
//
// ## 为什么要多轮、为什么看**请求正文**
//
// 效果在轮末写口落格，上下文在**下一轮**轮首收集——一轮之内读不到，必须跑第二轮。
// 第二轮的判据取模型请求里的**原文**（与用户看到的是同一条链路：prompt 由事实网格
// 投影而来），而不是读 WAL 或内部投影——那两样是实现细节。
//
// `最近记下的便签` 是 provider 自己的开场白，历史里不存在；用它当判据，才能把
// 「notes provider 贡献了这一段」与「记忆召回恰好也带了上一轮原文」分开——后者
// 也会让正文里出现便签文本，但它不是本用例要验的那件事。
//
// 第三轮重发同一条便签：`before` 在**执行前**拒绝，模型拿到的工具结果是
// `Refused: 这条便签已经记过…`。这一面只由能力包自己的 `notes.created` 事实判定，
// loop 侧没有任何一条针对便签的判断——判据因此同时是「判断能住在 owner 里」的读数。
//
// ## 停用侧取什么
//
// 与 T88 同一口径：同一套夹具、同一类消息，只翻 `notes` 的装配位。**运行期停用**
// 拿掉的是插件实例与它注册的工具；声明 / 行为 / provider 是链接期贡献（见 DE-025
// 的 D-217），因此这里断言的是「工具面消失 + 会话照常跑完」，不把链接期贡献的
// 存在与否伪装成运行期读数。
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
} from '../helpers.mjs';

const NOTE_TEXT = '周五交报告';
/** provider 的开场白（`notes_context` 段的首行）——历史里不会出现这串字 */
const CONTEXT_HEAD = '最近记下的便签';
/** 反思提示的开场白（`NoteReflect::run` 造的那条）——同样不在历史里 */
const REFLECT_HEAD = '请反思它';
/** mock 对一次反思往返的回答：落成 `notes.insight` 的正文 */
const INSIGHT_TEXT = '这条便签提醒：报告要提前准备。';

/**
 * mock 场景表：三轮共用一份。
 *
 * `times: 1` 不可省：第二轮请求里可能带着第一轮的召回原文（含「帮我记下」），
 * 名额耗尽后它才会落到第三幕——否则同一幕会在第二轮再触发一次工具调用。
 */
function scenarios() {
  return [
    {
      id: 'note-tool',
      match: '帮我记下',
      times: 1,
      toolCalls: [
        { id: 'call_n1', name: 'notes_add', arguments: { text: NOTE_TEXT } },
      ],
    },
    // 能力行借宿主的那一次往返：请求正文是 `notes` 自己造的提示，不是用户发言。
    {
      id: 'note-reflect',
      match: REFLECT_HEAD,
      times: 1,
      content: INSIGHT_TEXT,
    },
    { id: 'note-final', afterTool: true, times: 1, content: '便签已记下。' },
    { id: 'note-ask', match: '我刚才说过', content: '（回顾）' },
    // 第三轮：同一条正文再记一次——`before` 应拒，模型收到的工具结果带拒因。
    {
      id: 'note-dup',
      match: '再记一次',
      times: 1,
      toolCalls: [
        { id: 'call_n2', name: 'notes_add', arguments: { text: NOTE_TEXT } },
      ],
    },
    { id: 'dup-final', afterTool: true, times: 1, content: '这条便签已经记过了。' },
  ];
}

function homedirFor(port, { notesEnabled, gatewayPort }) {
  return makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
      // 网关拉起才有 `session/stats` 这条**生产路由**可打（第六面的读数走它）。
      // 与 T28 同一份配置形状：无此段时入站监听不会被拉起。
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: gatewayPort,
        inbound_token: '',
        inbound_readonly: false,
      },
      session: { ...DIALOG_FACE_OFF },
      // 启用侧不写这一项：缺省即启用（`PluginDir::enabled` 的约定）。
      ...(notesEnabled ? {} : { notes: { plugin_enabled: false } }),
    },
  });
}

const toolNames = (body) =>
  (body.tools ?? []).map((t) => t.function?.name).filter(Boolean);

/**
 * 请求体全文（各条消息正文拼接）。
 *
 * **不只看最后一条 user**：注入段拼在**系统提示词**里（`request_view_prefix` 经
 * 基线系统消息送达），不是拼在用户发言上——只看 user 会把「已经注入了」读成
 * 「没有注入」，而那恰好是本用例要验的那件事。
 */
function requestText(body) {
  return (body.messages ?? []).map((m) => String(m.content ?? '')).join('\n');
}

export default defineCase(
  'T89 扩展夹具：notes 能力包的工具 / 前置判断 / 领域事实 / 效果 / 上下文 / 视图 / 轮内行执行只经 plugin-api 接入即生效',
  async () => {
    // ── 启用侧：三轮，后两轮读到行为落下的那条事实与前置判断的拒因 ─────
    const llm = await new MockLlm(scenarios()).start();
    const GW = nextPort();
    const hd = homedirFor(llm.port, { notesEnabled: true, gatewayPort: GW });
    try {
      const first = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我记下：周五交报告',
        provider: PROVIDER_ID,
        session: 'e2e-t89',
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 400)}）`);
      assert(
        first.stdout.includes('便签已记下。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(first.stdout)}）`,
      );

      const afterFirst = await llm.requests();
      // 工具轮 → 能力行的一次往返 → 收尾轮：能力往返是**真的模型调用**，也计数。
      assertEq(
        afterFirst.length,
        3,
        `第一轮应是工具轮 + 能力往返 + 收尾轮三次请求，实际 ${afterFirst.length}`,
      );
      const tools = toolNames(afterFirst[0].body);
      assert(
        tools.includes('notes_add'),
        `能力包注册的工具应进模型工具面（实际: ${tools.join(',')}）`,
      );
      // 中间那次请求的正文由 `NoteReflect` 自己造：带反思提示与索引到的便签正文。
      const reflectRequest = afterFirst.find((r) => requestText(r.body).includes(REFLECT_HEAD));
      assert(
        reflectRequest,
        `应有一次由能力行发起的模型往返（请求正文里出现 ${REFLECT_HEAD}）`,
      );
      assert(
        requestText(reflectRequest.body).includes(NOTE_TEXT),
        `能力往返的提示应带上索引到的便签正文（实际: ${requestText(reflectRequest.body)}）`,
      );

      const second = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '我刚才说过什么？',
        provider: PROVIDER_ID,
        session: 'e2e-t89',
      });
      assertEq(second.code, 0, `第二轮 CLI 退出码（stderr: ${second.stderr.slice(0, 400)}）`);

      const afterSecond = await llm.requests();
      assertEq(
        afterSecond.length,
        afterFirst.length + 1,
        `第二轮应只有一次模型请求（实际新增 ${afterSecond.length - afterFirst.length} 次）`,
      );
      const prompt = requestText(afterSecond[afterSecond.length - 1].body);
      assert(
        prompt.includes(CONTEXT_HEAD),
        `第二轮 prompt 应含 notes 上下文段（实际: ${prompt.slice(0, 600)}）`,
      );
      assert(
        prompt.includes(NOTE_TEXT),
        `第二轮 prompt 应含行为落下的便签正文（实际: ${prompt.slice(0, 600)}）`,
      );

      // ── 第三轮：同一条正文再记一次 ⇒ before 拒，工具结果带回拒因 ─────
      const third = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '再记一次：周五交报告',
        provider: PROVIDER_ID,
        session: 'e2e-t89',
      });
      assertEq(third.code, 0, `第三轮 CLI 退出码（stderr: ${third.stderr.slice(0, 400)}）`);
      assert(
        third.stdout.includes('这条便签已经记过了。'),
        `第三轮 stdout 应为收尾正文（实际: ${JSON.stringify(third.stdout)}）`,
      );

      const afterThird = await llm.requests();
      assertEq(
        afterThird.length,
        afterSecond.length + 2,
        `第三轮应是工具轮 + 收尾轮两次请求（实际新增 ${afterThird.length - afterSecond.length} 次）`,
      );
      const judged = requestText(afterThird[afterThird.length - 1].body);
      assert(
        judged.includes('Refused: 这条便签已经记过'),
        `重复便签应在执行前被拒并回灌模型（实际: ${judged.slice(-800)}）`,
      );

      // ── 第六面：owner 声明的视图被**另一个插件**的读数口按名字读到 ──────
      // 判据只有一句：`notes.recent` 这一格出现在 `session/stats` 里，值就是 notes
      // 自己的折叠算出来的那条便签。session 没为便签写过一行——它只遍历清单生成的
      // 公开表（DE-025 D-221）；走生产路由（gateway → 容器 → 路由）才证得到「装配
      // 真的把 owner 的 ViewDecl 冻结进了同一张表」。
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'e2e-t89',
        provider: PROVIDER_ID,
        gatewayPort: GW,
      });
      try {
        await api.waitGatewayReady();
        const inv = await api.invoke('session/stats', {}, { session_id: 'e2e-t89' });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const view = inv.body?.data?.capability_views?.['notes.recent'];
        assert(view, `能力视图列应有 notes.recent 这一格（实际: ${JSON.stringify(inv.body?.data?.capability_views)}）`);
        assertEq(view.status, 'ok', `跨插件读应成功，不是缺席态：${JSON.stringify(view)}`);
        assertEq(view.owner, 'notes', 'owner 来自清单生成物');
        assertEq(view.ver, 1, '版本随 owner 的 ViewDecl 走');
        assertEq(
          view.value?.count,
          1,
          `三轮里只有一条便签落格（第三条被 before 拒）：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.items?.[0]?.text,
          NOTE_TEXT,
          '读到的正文就是行为落下的那条事实（折叠在 owner 手里）',
        );

        // ── 第七面：轮内行执行体落下的派生事实被同一读口读到 ─────────────
        // `notes.index` 这一行把 `notes_add` 的成功产物记成 `notes.indexed`；行本身
        // 归 `plugin_row`（宿主唯一执行体），实现由能力包经 `submit_row!` 交出。
        // 这条读数同时证两件事：宿主没有按工具名或行名写分支，能力包的草稿经唯一
        // 写口真的入了格。第三轮那次调用被 `before` 拒（`success = false`），
        // 行前件的 `eq` 不收它——所以计数仍是 1。
        const digest = inv.body?.data?.capability_views?.['notes.digest'];
        assert(
          digest,
          `能力视图列应有 notes.digest 这一格（实际: ${JSON.stringify(inv.body?.data?.capability_views)}）`,
        );
        assertEq(digest.status, 'ok', `notes.digest 跨插件读应成功：${JSON.stringify(digest)}`);
        assertEq(digest.owner, 'notes', 'owner 来自清单生成物');
        assertEq(digest.ver, 1, '版本随 owner 的 ViewDecl 走');
        assertEq(
          digest.value?.count,
          1,
          `只有一次成功的 notes_add 会开行债（重复那条被 before 拒）：${JSON.stringify(digest.value)}`,
        );
        // 回执里的 `tool_call_id` 是**执行层交回**的权威 id，不是模型给的那个：
        // 端到端只断言它存在（行按它开账与关账），正文才是这条通路的内容判据。
        const indexedId = digest.value?.items?.[0]?.tool_call_id;
        assert(
          typeof indexedId === 'string' && indexedId.length > 0,
          `行执行体应把工具调用的结果按 tool_call_id 收进索引：${JSON.stringify(digest.value)}`,
        );
        assertEq(
          digest.value?.items?.[0]?.text,
          NOTE_TEXT,
          '行执行体从 artifact.added 的定稿参数里取出的正文应与原调用一致',
        );

        // ── 第八面：能力行借宿主一次模型往返，落下的洞察被同一读口读到 ─────
        // `notes.reflect` 读 `notes.indexed`、交一条提示；宿主调模型、落四格协议事实，
        // 回执交回 `parse`，落一条 `notes.insight`。`status = ok` 说明这次往返答成了；
        // `tool_call_id` 与索引行同键——两笔事实说的是同一条便签。
        const insights = inv.body?.data?.capability_views?.['notes.insights'];
        assert(
          insights,
          `能力视图列应有 notes.insights 这一格（实际: ${JSON.stringify(inv.body?.data?.capability_views)}）`,
        );
        assertEq(insights.status, 'ok', `notes.insights 跨插件读应成功：${JSON.stringify(insights)}`);
        assertEq(insights.owner, 'notes', 'owner 来自清单生成物');
        assertEq(insights.ver, 1, '版本随 owner 的 ViewDecl 走');
        assertEq(
          insights.value?.count,
          1,
          `只有一条成功索引的便签会开反思行：${JSON.stringify(insights.value)}`,
        );
        assertEq(
          insights.value?.items?.[0]?.status,
          'ok',
          `这次往返应答成：${JSON.stringify(insights.value)}`,
        );
        assertEq(
          insights.value?.items?.[0]?.text,
          INSIGHT_TEXT,
          '洞察正文就是模型回执，由能力的解析函数原样落格',
        );
        assertEq(
          insights.value?.items?.[0]?.tool_call_id,
          indexedId,
          '反思与索引说的是同一条工具调用',
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }

    // ── 停用侧：装配位翻掉后工具面没有它，会话照常跑完 ─────────────────
    const offLlm = await new MockLlm([{ id: 'final', content: '收到。' }]).start();
    const offHd = homedirFor(offLlm.port, {
      notesEnabled: false,
      gatewayPort: nextPort(),
    });
    try {
      const r = runCli({
        homedir: offHd.homedir,
        workdir: offHd.workdir,
        message: '你好',
        provider: PROVIDER_ID,
        session: 'e2e-t89-off',
      });
      assertEq(r.code, 0, `停用 notes 后 CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      assert(
        r.stdout.includes('收到。'),
        `停用 notes 不该让会话装载失败（实际: ${JSON.stringify(r.stdout)}）`,
      );

      const reqs = await offLlm.requests();
      assertEq(reqs.length, 1, `一轮对话应只有一次模型请求，实际 ${reqs.length}`);
      const tools = toolNames(reqs[0].body);
      assert(
        !tools.includes('notes_add'),
        `停用 notes 后工具面不该含 notes_add（实际: ${tools.join(',')}）`,
      );
      assert(tools.length > 0, '停用 notes 只该拿掉它的工具，其余插件的能力应仍在');
    } finally {
      offLlm.stop();
      cleanupHomedir(offHd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
