import './_selfrun.mjs';
// T38 工具事实的投影消费：跨轮的工具结果进 `transcript` 投影 ⇒ **第二轮**请求的
// prompt 里带得着**第一轮**的工具结果。
//
// ## 本用例钉的是什么
//
// [10 §3 批 3](../../docs/plan/10-工具轮v2化实施方案.md) 的出口判据原文：
// 「两轮工具轮的会话，第二轮请求包里带第一轮的工具结果」。
//
// 为什么必须端到端：单测（`actors/mod.test.rs::transcript_includes_artifact_as_tool_line`）
// 已经证了「`artifact.added` 进投影 + 渲染成工具行」，但它证不了**这条事实真的
// 由一次真实工具调用产生、并真的走到下一轮的请求体里**——单测里的网格是手搭的。
//
// ## 为什么「跨轮」才测得出投影消费
//
// 工具轮**当轮**的交换由运行器就地追加（`render_tool_exchange`）——它不经过投影，
// 所以**轮 0 的第二次请求**带工具结果是「本来就对」的（T26 已证）。本用例要证的
// 是**跨轮**：轮 1 的 prompt 只由投影渲染（基线），若 `artifact.added` 不进投影，
// 轮 1 就看不见轮 0 的工具结果 ⇒ 模型会重复调用同一个工具。三处请求因此各有分工：
//
// | 请求 | 工具结果从哪来 | 证什么 |
// |---|---|---|
// | ① 轮 0 首轮 | ——（还没有） | 反向：此刻不该有工具结果 |
// | ② 轮 0 收尾轮 | 轮内交换（`render_tool_exchange`） | 当轮路径照常（与 T26 同形） |
// | ③ 轮 1（新用户发言） | **投影**（`transcript`） | **本批的主题**：跨轮带得着 |
//
// ## 为什么两轮用两次 `runCli`（而不是一轮里塞两次发言）
//
// 两次 `runCli` 是**两条独立用户发言**（各自开一轮），共享同一个会话目录 ⇒ 同一份
// 事实源。这正是「跨轮」的定义。同一轮内不会有第二个用户格（续写走 resume）。
//
// ## 为什么工具调用场景要 `once: true`
//
// v2 运行器把整轮 prompt **平铺成一条 user 消息**，而历史里**始终含**「帮我回显」
// 那句——不带 `once` 会让带 `match` 的工具场景反复命中 ⇒ 无限工具循环（与 T26 同因）。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  readMessagesJson,
  E2E_ROOT,
} from '../helpers.mjs';

/** 本用例的会话 id（两轮共享） */
const SID = 'e2e-t38';

/** mock-mcp 回显的内容标记：它必须从轮 0 的工具结果一路走到轮 1 的请求体 */
const ECHO_TEXT = '跨轮回显内容';

/** 被调工具名（MCP 前缀 + 服务名 + 工具名，与 T26 同一口径） */
const ECHO_TOOL = 'mcp__mockserv__echo';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 取请求体的**未转义 prompt 正文**（摊所有 message 的字符串 content）。 */
function promptOf(body) {
  return (body?.messages ?? [])
    .map((m) => (typeof m.content === 'string' ? m.content : ''))
    .join('\n');
}

export default defineCase(
  'T38 工具事实的投影消费：跨轮工具结果进 transcript 投影 ⇒ 第二轮请求带第一轮的工具结果',
  async () => {
    const llm = await new MockLlm([
      // ① 轮 0 首轮：模型请求工具（带一段中途正文，逼出「定格并切节点」路径）。
      {
        id: 'echo',
        match: '帮我回显',
        once: true,
        content: '我先回显一下。',
        toolCalls: [{ id: 'call_1', name: ECHO_TOOL, arguments: { text: ECHO_TEXT } }],
      },
      // ② 轮 0 收尾轮（工具结果已回灌）：不再请求工具 ⇒ 本轮收束。
      { id: 'r0-final', once: true, content: '第一答：回显完成。' },
      // ③ 轮 1（新的用户发言）：收束。它的 prompt 只由**投影**渲染。
      { id: 'r1-final', content: '第二答。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 本用例的主题与对话面正交 ⇒ 把三个开关钉死（理由见 `DIALOG_FACE_OFF`），
        // 再叠加**本用例的主题**：full 档（v2 原生执行，prompt 从转写投影出）。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'full' },
      },
    });
    // MCP stdio 工具：`echo` 的返回是确定的（`JSON.stringify(参数)`），断言
    // 「标记串进了请求体」不依赖任何路径解析（与 T26 同因）。
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs')],
      enabled: true,
    });

    try {
      // ── 轮 0：工具轮（工具真的执行、结果真的回灌当轮请求）────────────────────
      const r0 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我回显一下',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r0.code, 0, `轮 0 CLI 退出码（stderr: ${r0.stderr.slice(0, 600)}）`);

      // ── 轮 1：新的用户发言（同一会话 ⇒ 同一份事实源）────────────────────────
      const r1 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '再问一句',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r1.code, 0, `轮 1 CLI 退出码（stderr: ${r1.stderr.slice(0, 600)}）`);

      // ── 证据 ①：三次请求（轮 0 × 2 + 轮 1 × 1）──────────────────────────────
      const reqs = await llm.requests();
      assertEq(reqs.length, 3, `mock-llm 应收到三次请求（实得 ${reqs.length}）`);

      const p0 = promptOf(reqs[0].body);
      const p1 = promptOf(reqs[1].body);
      const p2 = promptOf(reqs[2].body);

      // ⚠️ 判据从「文本里有 `工具结果(x): ` / `用户: ` / `助手: `」改成
      // 「**消息层上的角色与内容**」（ADR-048a）。
      //
      // 旧判据钉的是**拍平的产物**——那三种前缀都是 `to_prompt()` /
      // `render_tool_exchange` 拼出来的文本。结构化之后它们不再是前缀，而是
      // `role: user` / `role: assistant` / `role: tool` 三种消息。**继续钉前缀等于
      // 逼着代码保留拍平。**
      //
      // 新判据**更强**：旧判据只验「某个字符串出现过」，新判据验「这条内容在**哪个
      // 角色**的消息里」——所以「把用户发言标成 assistant」这类退化现在会红。
      // ⚠️ 形参不叫 `msgs`——本用例后面 `readMessagesJson` 的结果就叫 `msgs`。
      const msgsOf = (i) => reqs[i].body.messages ?? [];
      const textsOf = (i, role) =>
        msgsOf(i)
          .filter((m) => m.role === role)
          .map((m) => (typeof m.content === 'string' ? m.content : ''));
      const toolTexts = (i) => textsOf(i, 'tool');
      // 跨轮工具结果在请求里是 **assistant 消息**，不是 `role: tool`——
      // 见 `provider_adapter` 里的降级与理由（协议要求 tool 消息配对**本次**请求
      // 的 tool_calls，跨轮的调用在上一轮，于是那条消息会被清洗段当孤儿丢掉）。
      const crossTurnTexts = (i) => textsOf(i, 'assistant');
      const saw = (i, role, needle) => textsOf(i, role).some((t) => t.includes(needle));

      // 反向：轮 0 首轮请求里不该有工具结果（那时还没执行）。
      assert(
        toolTexts(0).length === 0,
        `轮 0 首轮请求不该出现 role=tool 消息（实际: ${JSON.stringify(msgsOf(0).map((m) => m.role))}）`,
      );

      // 当轮路径照常（与 T26 同形）：轮 0 收尾轮的工具结果来自**轮内交换**。
      assert(
        toolTexts(1).some((t) => t.includes(ECHO_TEXT)),
        `轮 0 收尾轮应带 role=tool 的结果（轮内交换）（实际: ${JSON.stringify(toolTexts(1)).slice(0, 400)}）`,
      );

      // ── 证据 ②：**本用例的主题**——轮 1 的请求带轮 0 的工具结果（投影消费）──
      // 轮 1 的消息由 `render_messages`（= `transcript` 投影 → `to_messages`）产出，
      // 轮内交换**不跨轮**。它带得着，只可能是因为 `artifact.added` 进了投影。
      assert(
        crossTurnTexts(2).some((t) => t.includes(ECHO_TEXT)),
        `轮 1 请求应带**第一轮**的工具结果（跨轮投影消费）（实际: ${JSON.stringify(crossTurnTexts(2)).slice(0, 400)}）`,
      );
      // 轮 1 的历史里也看得见轮 0 的问与答（投影的多轮形态没被工具消息挤掉），
      // 且**角色正确**——问是 user 说的、答是 assistant 说的。
      assert(saw(2, 'user', '帮我回显一下'), '轮 1 历史应含轮 0 的用户发言（role=user）');
      assert(
        saw(2, 'assistant', '第一答：回显完成。'),
        '轮 1 历史应含轮 0 的收束正文（role=assistant——拍平会把这条变成 user）',
      );
      assert(
        textsOf(2, 'user').at(-1)?.includes('再问一句') ?? false,
        `当前发言在历史之外且是最后一条 user（实际末条: ${JSON.stringify(textsOf(2, 'user').at(-1))}）`,
      );

      // ── 证据 ③：事实源——两轮各收束，工具格恰一格且带溯源 ──────────────────
      const walRaw = readFileSyncSafe(join(hd.homedir, 'session', SID, V2_WAL));
      assert(walRaw.length > 0, 'v2 事实源应存在且非空');
      const events = walRaw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
      const opens = events.filter((e) => e.kind === 'user.message');
      const finals = events.filter((e) => e.kind === 'chat.assistant.final');
      const artifacts = events.filter((e) => e.kind === 'artifact.added');
      assertEq(opens.length, 2, '两轮各一条用户格');
      assertEq(finals.length, 2, '两轮各一条收束格');
      assertEq(artifacts.length, 1, '轮 0 恰好一格工具产物');
      const artifact = artifacts[0];
      assertEq(artifact.entity, 'Artifact', '产物格实体坐标');
      assertEq(artifact.verb, 'Asserted', '产物格动词坐标');
      assertEq(artifact.payload.tool, ECHO_TOOL, '产物格记录被调工具名');
      assert(
        String(artifact.payload.text ?? '').includes(ECHO_TEXT),
        `产物格落的是结果正文（实际: ${JSON.stringify(artifact.payload)}）`,
      );
      // I2：产物格溯源指向**它那一轮**的用户格（轮 0）。
      assertEq(
        artifact.produced_by,
        opens.find((e) => e.turn === artifact.turn).seq,
        '产物格溯源应指向本轮用户格（caused_by 锚点）',
      );

      // ── 转写不变量 ─────────────────────────────────────────────────────────
      const msgs = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs, 'T38');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
