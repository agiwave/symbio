import './_selfrun.mjs';
// T91 压缩形态夹具：摘要落 `Message` 事实，省略事实跳区间（D-249）。
//
// ## 这条用例补的是什么
//
// T89 证的是「一个新能力包只经 plugin-api 就能交出工具 / 前置判断 / 领域事实 /
// 效果 / 上下文 / 视图 / 轮内行 / 模型往返」。本用例证的是**上下文改写**这一面：
// 一个能力在轮内把一段历史压成一句摘要，宿主按**同一条事实**把被取代的区间挡在
// 请求包之外——`ContextSection` 的成本/裁剪（D-243）管「放不下时裁谁」，
// 本用例管「能力自己声明这段已被取代」。
//
// ## 为什么读数取模型请求，而不是读 WAL / 内部投影
//
// 「摘要进了请求包」与「旧区间没进请求包」是同一件事的两面，而它们的唯一用户是
// **模型**。请求正文与用户看到的是同一条链路（prompt 由事实网格投影而来），
// 断言它才照得到「能力声明了省略、组装却没执行」这种静默分叉。
//
// 两条读数分别钉住 D-249 的两半：
//
// 1. 第二轮请求里有一条 **user** 消息，正文是 `便签摘要：<洞察>`——`message.added`
//    带 `elision` 时载荷节点是「取代旧区间的正文」（不带时是界面卡片，恒不进请求包）；
// 2. 同一条请求里**没有 `role: "tool"` 消息**——那条 `notes_add` 的工具结果事件
//    落在摘要声明的 `[from, to]` 区间内，转写照常收下（界面看得见），请求包里按
//    `Elisions::elides` 跳过。
//
// 第二轮的 `tool` 面是**零**而不是「少了一条」：本会话只调用过一次工具，而它正是
// 被取代的那一段。若省略没有生效，那一行会原样出现在请求里。
//
// ## 为什么摘要行不再调模型
//
// `notes.summarize` 从 `notes.insight` 的正文直接造摘要（`SUMMARY_HEAD` + 洞察），
// 不借模型往返。这不是为了省钱：请求次数是**可数的读数**（T89 的「三轮三次请求」
// 就钉着它），压缩形态若要再调一次模型，就把「上下文改写」与「往返预算」两件事
// 搅在一起，两侧的判据都会变钝。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

const NOTE_TEXT = '周五交报告';
/** 反思提示的开场白（`NoteReflect::run` 造的那条）——历史里不会出现这串字 */
const REFLECT_HEAD = '请反思它';
/** mock 对一次反思往返的回答：落成 `notes.insight` 的正文 */
const INSIGHT_TEXT = '这条便签提醒：报告要提前准备。';
/** 摘要行的开场白（`SUMMARY_HEAD`）——同样不在历史里，用它把摘要与别处正文分开 */
const SUMMARY_HEAD = '便签摘要：';

/**
 * mock 场景表：两轮共用一份。
 *
 * `times: 1` 不可省：第二轮请求里带着第一轮的召回原文（含「帮我记下」），
 * 名额耗尽后它才会落到 `note-ask`——否则同一幕会在第二轮再触发一次工具调用。
 * 本用例**不**触发压缩行自己的模型往返：摘要由 `notes.summarize` 按事实现算。
 */
function scenarios() {
  return [
    {
      id: 'note-tool',
      match: '帮我记下',
      times: 1,
      toolCalls: [
        { id: 'call_e1', name: 'notes_add', arguments: { text: NOTE_TEXT } },
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
  ];
}

const homedirFor = (port) =>
  makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
      session: { ...DIALOG_FACE_OFF },
      // 启用 notes：缺省即启用（`PluginDir::enabled` 的约定）。
    },
  });

/** 请求体的消息数组（openai_chat：`body.messages[]`）。 */
const messagesOf = (body) => body.messages ?? [];

/** 一条消息的正文（字符串或分片数组都摊平）。 */
const textOf = (m) =>
  typeof m.content === 'string'
    ? m.content
    : (m.content ?? []).map((p) => p.text ?? '').join('');

export default defineCase(
  'T91 压缩形态夹具：摘要落 Message 事实并进下一轮 prompt，被取代的工具结果区间不再提交（D-249）',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port);
    try {
      // ── 第一轮：记便签 → 索引 → 反思往返 → 摘要，全部在同一轮内落下 ──────
      const first = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我记下：周五交报告',
        provider: PROVIDER_ID,
        session: 'e2e-t91',
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 400)}）`);
      assert(
        first.stdout.includes('便签已记下。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(first.stdout)}）`,
      );

      const afterFirst = await llm.requests();
      // 工具轮 → 能力行的一次往返 → 收尾轮：摘要行**不**再调模型（见文件头）。
      assertEq(
        afterFirst.length,
        3,
        `第一轮应是工具轮 + 能力往返 + 收尾轮三次请求，实际 ${afterFirst.length}`,
      );
      const reflectRequest = afterFirst.find((r) =>
        messagesOf(r.body).some((m) => textOf(m).includes(REFLECT_HEAD)),
      );
      assert(
        reflectRequest,
        `应有一次由能力行发起的模型往返（请求正文里出现 ${REFLECT_HEAD}）`,
      );

      // ── 第二轮：摘要与省略都应在这一轮的请求包里生效 ─────────────────────
      const second = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '我刚才说过什么？',
        provider: PROVIDER_ID,
        session: 'e2e-t91',
      });
      assertEq(second.code, 0, `第二轮 CLI 退出码（stderr: ${second.stderr.slice(0, 400)}）`);

      const afterSecond = await llm.requests();
      assertEq(
        afterSecond.length,
        afterFirst.length + 1,
        `第二轮应只有一次模型请求（实际新增 ${afterSecond.length - afterFirst.length} 次）`,
      );
      const msgs = messagesOf(afterSecond[afterSecond.length - 1].body);

      // 读数一：摘要作为 **user 正文**进了请求包（`message.added` 带 `elision` 的那一面）。
      const summaryMsg = msgs.find(
        (m) => m.role === 'user' && textOf(m).includes(SUMMARY_HEAD),
      );
      assert(
        summaryMsg,
        `第二轮 prompt 应含摘要行（实际角色/正文: ${JSON.stringify(
          msgs.map((m) => [m.role, textOf(m).slice(0, 60)]),
        )}）`,
      );
      assert(
        textOf(summaryMsg).includes(INSIGHT_TEXT),
        `摘要正文应来自那条答成了的洞察（实际: ${textOf(summaryMsg)}）`,
      );
      assertEq(
        textOf(summaryMsg),
        `${SUMMARY_HEAD}${INSIGHT_TEXT}`,
        '摘要正文 = 开场白 + 洞察正文：这一行的产物是纯值，可逐字断言',
      );

      // 读数二：被摘要取代的那段工具结果**不再**以 `role: "tool"` 出现在请求里。
      // 本会话只调用过一次工具（`notes_add`），它正是摘要声明的区间——若省略没生效，
      // 这里会看到那一条；而本用例没有别的工具调用能给 `tool` 面兜底。
      const toolMsgs = msgs.filter((m) => m.role === 'tool');
      assertEq(
        toolMsgs.length,
        0,
        `被省略区间的工具结果不应再进请求包（实际: ${JSON.stringify(
          toolMsgs.map((m) => textOf(m).slice(0, 120)),
        )}）`,
      );

      // 同一事实的另一面：历史本身没有被删——用户发言与助手收尾仍在，摘要排在它们之后。
      // 省略是**请求投影**的读法，不是把事件从网格里抹掉（那是 `message.removed`）。
      const bodies = msgs.map(textOf);
      assert(
        bodies.some((t) => t.includes('帮我记下')),
        `第二轮 prompt 仍应含第一轮的用户发言（实际: ${JSON.stringify(bodies)}）`,
      );
      assert(
        bodies.some((t) => t.includes('便签已记下。')),
        `第二轮 prompt 仍应含第一轮的助手收尾（实际: ${JSON.stringify(bodies)}）`,
      );
      assert(
        bodies.indexOf(textOf(summaryMsg)) > bodies.findIndex((t) => t.includes('帮我记下')),
        '摘要排在它所取代的那段历史之后：位置由事件顺序说，不由实现挑',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
