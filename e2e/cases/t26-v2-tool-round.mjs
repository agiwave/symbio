import './_selfrun.mjs';
// T26 工具轮 v2 化：`v2_mode = full` 下工具调用真的执行、结果真的回灌下一次请求。
//
// ## 本用例钉的是什么
//
// [10 §3 批 1](../../docs/plan/10-工具轮v2化实施方案.md) 的出口判据原文：
// 「mock-llm `toolCalls` → 工具执行 → **`/_requests` 回读**：工具结果进了下一次请求；
// 网格有 `artifact.added` 且带溯源」。
//
// 这条判据之所以必须端到端，是因为它要证的三件事**各自只能在一处被看见**：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 工具**真的执行**了 | mock-mcp 的 `--record`（进程外证据，不看后端自述） |
// | 结果**真的回了模型** | mock-llm 的 `/_requests` 回读**第二次请求体** |
// | 事实**真的入了格** | `<会话目录>/v2-events.wal` 的 `artifact.added` + 溯源 |
//
// 单测（`v2_exec.test.rs::tool_round_lands_artifact_and_feeds_next_request`）用假 provider
// 覆盖了同一形状，但它证不了"真实 provider + 真实 MCP 工具 + 真实落盘"这条链——
// 单测里的 provider 是假件，工具分发走的是「没有父插件」的诚实失败路径。
//
// ## 为什么用 MCP 工具而不是内置工具
//
// 内置 `vdfs_read` 的路径要经「虚拟根 → 物理工作目录」两层解析（相对路径落在
// `<workdir>` 下），断言结果正文就得先知道这两层怎么拼——那是**另一个**主题。
// MCP `echo` 的返回是确定的（`JSON.stringify(参数)`），参数里放一个标记串即可，
// 断言「标记串出现在第二次请求里」不依赖任何路径解析。
//
// ## 为什么工具调用场景要 `once: true`（与 v1 的 T2 不同）
//
// T2（v1）里工具结果轮的**最后一条消息 role = tool**，mock 靠 `afterTool` 场景区分。
// 但 v2 运行器把整轮 prompt **平铺成一条 user 消息**（`render_prompt` + 工具交换
// 追加，见 `symbio_core/actors/mod.rs::render_tool_exchange`）——第二次请求的最后一条
// 仍是 user，`afterTool` 分支**永不命中**。因此这里改用 `once: true`：
// 带 `match` 的工具场景只烧一次，第二次请求落到下一条无 `match` 场景上。
// 这不是绕路，而是 v2 的 prompt 形态与 v1 不同这一事实的**直接后果**。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 对话面（`classify` 判决 / `compose` 措辞）出厂默认开启，会给每一轮多加一次静默
// 请求。本用例的断言按**精确请求数**（2 次）写，且验的是工具链路——与对话面正交，
// 故按 `DIALOG_FACE_OFF` 的约定把它钉死。
import { join } from 'node:path';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  turnFacts,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t26';

/** mock-mcp 回显的内容标记：它必须从工具结果一路走到第二次请求体 */
const ECHO_TEXT = 'mock 回显内容';

/** v2 事实源文件名（与 `plugins/session/v2_exec.rs` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

export default defineCase(
  'T26 工具轮 v2 化：full 档工具调用执行 → 结果回灌下一次请求 → artifact.added 带溯源',
  async () => {
    const llm = await new MockLlm([
      // 第一次请求：模型请求一个工具（带一段中途正文，逼出「定格并切节点」路径）。
      // `once: true` —— 第二次请求的最后一条仍是 user（v2 平铺 prompt），
      // 不带 `once` 会让本场景反复命中 ⇒ 无限工具循环（见文件头）。
      {
        id: 'call-echo',
        match: '帮我回显',
        once: true,
        content: '我先回显一下。',
        toolCalls: [
          { id: 'call_1', name: 'mcp__mockserv__echo', arguments: { text: ECHO_TEXT } },
        ],
      },
      // 第二次请求（工具结果已回灌）：不再请求工具 ⇒ 本轮收束。
      { id: 'after-echo', content: '工具调用完成，回显成功。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 本用例的主题与对话面正交 ⇒ 把三个开关钉死（理由见 `DIALOG_FACE_OFF`），
        // 再叠加**本用例的主题**：full 档。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'full' },
      },
    });
    // MCP stdio 工具：`--record` 是「工具真的执行过」的进程外证据。
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, 'mcp-record.ndjson')],
      enabled: true,
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我回显一下',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);
      assert(
        r.stdout.includes('工具调用完成，回显成功'),
        `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`,
      );

      // ── 证据 ①：工具真的执行了（进程外，不看后端自述）────────────────────
      const record = readFileSyncSafe(join(hd.homedir, 'mcp-record.ndjson'));
      const calls = record
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l))
        .filter((e) => e.kind === 'call');
      assertEq(calls.length, 1, `mock-mcp 应恰好收到一次 tools/call（实际 ${calls.length} 次）`);
      assertEq(calls[0].name, 'echo', '被调工具名');
      assertEq(calls[0].args, { text: ECHO_TEXT }, '工具参数');

      // ── 证据 ②：工具结果进了**下一次请求**（只能靠回读请求体证明）─────────
      const reqs = await llm.requests();
      assertEq(reqs.length, 2, 'mock-llm 应收到两次请求（工具轮 + 收尾轮）');

      // 工具清单确实下行（批 1 的 §2.1 工具通道）：第一次请求里看得见 MCP 工具。
      const tools0 = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      assert(
        tools0.includes('mcp__mockserv__echo'),
        `MCP 工具应注册进第一次请求（实际: ${tools0.join(',')}）`,
      );

      // 第二次请求：模型请求过的工具名 + 工具结果正文都在 prompt 里。
      // 判据取 `render_tool_exchange` 的**两种前缀**，因为它们是这条回灌链路的
      // 唯一可见形态（网格与 UI 帧都证明不了"结果进了 prompt"）。
      const second = JSON.stringify(reqs[1].body);
      assert(
        second.includes('助手请求工具: mcp__mockserv__echo'),
        `第二次请求应带上模型请求过的工具名（实际请求体片段: ${second.slice(0, 800)}）`,
      );
      assert(
        second.includes('工具结果(mcp__mockserv__echo):'),
        `第二次请求应带上工具结果行（实际请求体片段: ${second.slice(0, 800)}）`,
      );
      assert(
        second.includes(ECHO_TEXT),
        '工具结果正文（回显内容）应原样回灌给模型',
      );
      // 反向：第一次请求里不该有工具结果（那时还没执行）。
      assert(
        !JSON.stringify(reqs[0].body).includes('工具结果(mcp__mockserv__echo):'),
        '第一次请求不该出现工具结果（结果尚未产生）',
      );

      // ── 证据 ③：事实入了格（v2 WAL 的 artifact.added + 溯源）──────────────
      const walRaw = readFileSyncSafe(join(hd.homedir, 'session', SID, V2_WAL));
      assert(walRaw.length > 0, `v2 事实源应存在且非空：${join(hd.homedir, 'session', SID, V2_WAL)}`);
      const events = walRaw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
      const kinds = events.map((e) => e.kind).join(', ');
      // 只数**轮次事实**：网格里同时住着派生副作用（`memory.encoded`，S5 步 11），
      // 按总数断言会把两类混在一起（见 `helpers.mjs::turnFacts`）。
      const facts = turnFacts(events);
      assertEq(
        facts.length,
        3,
        `full 档一轮应恰好 3 格轮次事实（用户 + 产物 + final），实得 ${facts.length}: ${kinds}`,
      );

      const user = events.find((e) => e.kind === 'user.message');
      const artifact = events.find((e) => e.kind === 'artifact.added');
      const final = events.find((e) => e.kind === 'chat.assistant.final');
      assert(user, `网格缺用户格（实得: ${kinds}）`);
      assert(artifact, `网格缺产物格 artifact.added（实得: ${kinds}）`);
      assert(final, `网格缺收束格 chat.assistant.final（实得: ${kinds}）`);

      // 产物格的坐标（`artifact × asserted`）与载荷形状。
      assertEq(artifact.entity, 'Artifact', '产物格实体坐标');
      assertEq(artifact.verb, 'Asserted', '产物格动词坐标');
      assertEq(artifact.payload.tool, 'mcp__mockserv__echo', '产物格记录被调工具名');
      assert(
        String(artifact.payload.text ?? '').includes(ECHO_TEXT),
        `产物格落的是结果正文（实际: ${JSON.stringify(artifact.payload)}）`,
      );
      // I2：断言类事件必带溯源；产物格溯源指向**本轮用户格**。
      assert(artifact.produced_by != null, '产物格必须带溯源（I2）');
      assertEq(artifact.produced_by, user.seq, '产物格溯源应指向本轮用户格（caused_by 锚点）');
      assert(final.produced_by != null, '收束格必须带溯源（I2）');
      assertEq(final.payload.text, '工具调用完成，回显成功。', 'final 落的是收尾轮正文');

      // ── 转写不变量：tool_call 与结果成对（v2 侧补建的两类节点）────────────
      const msgs = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs, 'T26');
      const tc = msgs.find((m) => m.type === 'tool_call');
      assert(tc, `应有 tool_call 节点（实得类型: ${msgs.map((m) => m.type).join(',')}）`);
      assert(
        msgs.some((m) => m.role === 'tool' && m.parent_id === tc.id),
        '工具结果必须挂在 tool_call 节点之下',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
