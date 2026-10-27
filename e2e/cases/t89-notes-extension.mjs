import './_selfrun.mjs';
// T89 扩展夹具：`notes` 能力包只经 plugin-api 接入后，五个扩展面在真实链路上生效。
//
// ## 这条用例补的是什么
//
// T88 证的是「插件的工具随装配位进出」；本用例证的是 DE-025 的**扩展面本身**
// （EX-001）：一个新能力包不改 core / kernel / loop / session，只靠 plugin-api 的
// 提交点就能做到——
//
// | 面 | 提交点 | 本用例的读数 |
// |---|---|---|
// | 工具 | `traverse` + `CapabilityRow::tool` | 模型请求的 `tools` 里出现 `notes_add` |
// | 前置判断 | `submit_behavior!` + `ToolBehavior::before` | 重复便签被拒，工具结果里带回 `Refused: …` |
// | 领域事实 | `submit_declarations!`（`Domain × Opened`） | 工具轮结束后事件真的落格 |
// | 效果 | `submit_behavior!` + `CapabilityMeta.behaviors` | 下一轮上下文里出现那条便签 |
// | 上下文 | `submit_context_provider!` | 下一轮请求前缀里出现 `最近记下的便签` |
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
  runCli,
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

function homedirFor(port, { notesEnabled }) {
  return makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
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
  'T89 扩展夹具：notes 能力包的工具 / 前置判断 / 领域事实 / 效果 / 上下文只经 plugin-api 接入即生效',
  async () => {
    // ── 启用侧：三轮，后两轮读到行为落下的那条事实与前置判断的拒因 ─────
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port, { notesEnabled: true });
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
      assertEq(afterFirst.length, 2, `第一轮应是工具轮 + 收尾轮两次请求，实际 ${afterFirst.length}`);
      const tools = toolNames(afterFirst[0].body);
      assert(
        tools.includes('notes_add'),
        `能力包注册的工具应进模型工具面（实际: ${tools.join(',')}）`,
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
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }

    // ── 停用侧：装配位翻掉后工具面没有它，会话照常跑完 ─────────────────
    const offLlm = await new MockLlm([{ id: 'final', content: '收到。' }]).start();
    const offHd = homedirFor(offLlm.port, { notesEnabled: false });
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
