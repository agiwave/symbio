import './_selfrun.mjs';
// T19 补充整合（G-A）：会话运行中连发的多条补充被**抽干并合并成一条**用户消息，
// 作为**同一轮**的输入被整体处理——而不是"一条消息 = 一轮"。
//
// ## 钉的是哪条不变量
//
// `chat_session/write.rs::prune_historical_tool_calls` 用 **`role = User` 的消息数**
// 算保留分水岭（`keep_turns = SessionConfig::context_messages`）。逐条追加会让 n 条补充
// 吃掉 n 份保留预算，把更早的轮次提前挤出裁剪窗口——用户只是多说了两句话，历史却
// 悄悄少了两轮。所以"整体处理"的可检验形式就是：**n 条补充在存储里仍是一轮**。
//
// （另一处窗口口径不同、**不受**影响：`context/window.rs::apply_layered_sliding_window`
// 按 **ToolCall** 计数。两处别混——这是本用例与 C-D2 断言的依据。）
//
// ## 三条断言线
//
// | 线 | 抽干点 | 形状 |
// |---|---|---|
// | **A 空闲抽干** | `inbox.rs::drain_inbox_once`（本轮收尾后，后台消费者再扫一趟） | 两条补充合成**新的一轮**输入 |
// | **B 轮边界折进** | `chat_loop.rs` 的轮边界（`gate_turn` 之后、`prepare_turn_inputs` 之前） | 两条补充折进**正在跑的同一轮**，出现在 tool 结果**之后** |
// | **C 平凡值** | `supplements_enabled: false` | 三条各自一轮，与改造前逐字一致 |
//
// A 与 B 的区别不在"合没合并"（都合并），而在**合并发生在哪**：
// A 无工具调用 ⇒ `turn.tool_rounds == 0` ⇒ 轮边界抽干被条件挡掉，只能由空闲抽干接手；
// B 有工具调用 ⇒ 轮边界抽干生效，合并消息出现在**同一条 LLM 请求链**里。
// 两条线各自钉一个抽干点，缺任一条就有整整一个抽干点没人守。
//
// ## 时序契约（为什么用 `chunkDelayMs`）
//
// A / B 都要求"补充在本轮还在跑的时候到达"。可用的手段只有一种：让 mock 的 SSE
// **慢吐**，在流还没结束时把条目写进收件箱（与 T15 拉长运行窗口同一手法）。
// 流越长窗口越宽——这里给到 ~2.4s，而两次 VDFS 写入只要毫秒级。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  readMessagesJson,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  textOf,
} from '../helpers.mjs';

/** gateway 入站配置（与其它用例同款：本机回环、无 token） */
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

/** 合并标记的三个键名（镜像 `transcript/supplements.rs` 的常量，唯一真源在那里） */
const META_SUPPLEMENT = 'supplement';
const META_SUPPLEMENT_IDS = 'supplement_ids';
const META_SUPPLEMENT_COUNT = 'supplement_count';

/** 补充之间的连接符（镜像 `supplements.rs::JOINER`——不引入用户没写过的字符） */
const JOINER = '\n\n';

const userTexts = (msgs) => msgs.filter((m) => m.role === 'user').map(textOf);

/**
 * 起一个长驻 CLI + 会话，并给出"往收件箱写一条"的入口。
 *
 * 会话必须先建出来：收件箱是**会话内部的集合**，会话不在就没有它可写
 * （与 T15 / T16 同一手法）。
 */
async function openSession(cli, { sid, workdir }) {
  const rootResp = await cli.invoke('vdfs/root', {});
  const root = rootResp.body?.data?.path;
  assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');

  const sessionAddr = `${root.replace(/\/+$/, '')}/session/${sid}`;
  const inboxAddr = `${sessionAddr}/inbox`;
  const created = await cli.invoke('vdfs/write', {
    path: sessionAddr,
    create: true,
    text: JSON.stringify({
      metadata: { workdir, mode: 'auto', risk_level: 'medium' },
    }),
  });
  assertEq(created.status, 200, `创建会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);

  /** 写一条补充进收件箱，返回它的**条目名**（= 消息 id：地址末段即身份） */
  const write = async (text) => {
    const r = await cli.invoke('vdfs/write', { path: inboxAddr, text });
    assertEq(r.status, 200, `写收件箱（${JSON.stringify(r.body)?.slice(0, 300)}）`);
    const iid = r.body?.data?.name;
    assert(typeof iid === 'string' && iid.length > 0, `回执应给出条目名（实际 ${JSON.stringify(iid)}）`);
    return iid;
  };

  return { sessionAddr, inboxAddr, write };
}

export default defineCase(
  'T19 补充整合（G-A）：运行中连发的补充被抽干合并成一条，作为同一轮输入',
  async () => {
    const llm = await new MockLlm([
      // ── A 空闲抽干：首轮慢吐，给"运行中再写两条"留窗口；合并后的正文以第二条开头 ──
      { id: 'a-first', match: 'A第一条', chunks: ['收到', 'A', '第一条'], chunkDelayMs: 600 },
      { id: 'a-merged', match: 'A第二条', content: 'A 的两条补充已一起处理。' },

      // ── B 轮边界：首轮先调工具（工具结果回来后才有第 2 次请求），同样慢吐 ──
      {
        id: 'b-tool',
        match: 'B第一条',
        toolCalls: [{ id: 'call_b1', name: 'vdfs_write', arguments: { path: 'b-note.md', text: '由 mock 写入' } }],
        chunkDelayMs: 600,
      },
      { id: 'b-after', afterTool: true, content: 'B：工具已完成，且已看到补充。' },

      // ── C 平凡值：三条各自成轮 ──
      { id: 'c-1', match: 'C第一条', chunks: ['C', '第一轮'], chunkDelayMs: 600 },
      { id: 'c-2', match: 'C第二条', content: 'C 第二轮回复。' },
      { id: 'c-3', match: 'C第三条', content: 'C 第三轮回复。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    // C 线是**另一个进程**，必须占另一个端口：共用端口时第二次绑定失败，
    // 而 `waitGatewayReady` 会由先绑上的那个进程答成"就绪"——请求于是被静默
    // 路由到错误的 homedir，表现为"C 线的消息永远不落库"。
    const GATEWAY_PORT_OFF = nextPort();
    // 默认配置（`supplements_enabled` 默认 true）：A / B 两线共用同一个 homedir，
    // 用不同的 sid 隔离——配置相同就没必要多起一个进程。
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT) },
    });
    // C 线要关掉开关，配置不同 ⇒ 必须另起一个 homedir / 进程。
    const hdOff = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT_OFF), session: { supplements_enabled: false } },
    });

    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'unused-t19',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const cliOff = startLongLivedCli({
      homedir: hdOff.homedir,
      workdir: hdOff.workdir,
      session: 'unused-t19c',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT_OFF,
    });

    try {
      await cli.waitGatewayReady();
      await cliOff.waitGatewayReady();

      // 三条线共用一个 mock，`requests()` 是**累计**的——每条线开跑前取一次基线，
      // 断言一律写成"基线 + n"。写死绝对条数会让"加一条线"变成"改三处断言"。
      const reqCount = async () => (await llm.requests()).length;

      // ═══════════════════════════════════════════════════════════════════
      // A 空闲抽干：无工具调用 ⇒ 轮边界抽干被 `tool_rounds > 0` 挡掉，
      //   两条补充只能等本轮收尾、由后台消费者抽干成**新的一轮**。
      // ═══════════════════════════════════════════════════════════════════
      const baseA = await reqCount();
      const sidA = 'e2e-t19a';
      const a = await openSession(cli, { sid: sidA, workdir: hd.workdir });
      const a1 = await a.write('A第一条：请开始。');

      // 等本轮**真的在跑**（mock 收到请求即流已开始），再写补充——
      // 写在开跑之前的话，三条会一起被抽干成一条，A 线就退化成 B 线。
      await waitFor(async () => (await reqCount()) >= baseA + 1, {
        what: 'A 线首轮 LLM 请求（证明消费已发生）',
        timeoutMs: 20_000,
      });
      const a2 = await a.write('A第二条：再补充一点。');
      const a3 = await a.write('A第三条：还有一点。');

      // 收敛：两条补充合并成**第二条**用户消息（不是第二、第三条）
      await waitFor(() => userTexts(readMessagesJson(hd.homedir, sidA) ?? []).length >= 2, {
        what: 'A 线合并后的用户消息落库',
        timeoutMs: 30_000,
      });
      // 再多等一拍，确认**没有**第三条冒出来（合并若失效，第三条会紧跟着出现）
      await waitFor(() => (readMessagesJson(hd.homedir, sidA) ?? []).length >= 4, {
        what: 'A 线两轮各自的助手正文落库',
        timeoutMs: 30_000,
      });

      const msgsA = readMessagesJson(hd.homedir, sidA);
      assertTranscriptInvariants(msgsA, 'T19-A');
      assertEq(
        userTexts(msgsA),
        ['A第一条：请开始。', `A第二条：再补充一点。${JOINER}A第三条：还有一点。`],
        'A 线：三条补充在存储里必须是**两条**用户消息（首条 + 合并成的一条），且正文是原文拼接',
      );

      const mergedA = msgsA.find((m) => m.role === 'user' && m.id === a2);
      assert(mergedA, `合并消息应沿用**第一条**补充的 id（${a2}）——前端按 id 合并权威帧`);
      // `parent_id` 为空在落盘里表现为**键缺失**（`None` 不序列化），故与 `null` 等价看待
      assertEq(mergedA.parent_id ?? null, null, '合并消息是顶层用户消息（与普通用户消息同形）');
      assertEq(mergedA.meta?.[META_SUPPLEMENT], true, '合并消息应带 supplement 标记');
      assertEq(mergedA.meta?.[META_SUPPLEMENT_COUNT], 2, '合并条数');
      assertEq(
        mergedA.meta?.[META_SUPPLEMENT_IDS],
        [a2, a3],
        '原始条目 id 必须按入队顺序留痕（合并后只此一处可回溯）',
      );
      assert(
        !msgsA.some((m) => m.id === a1 && m.meta?.[META_SUPPLEMENT]),
        '首条补充没有被合并（它是本轮的开端）',
      );

      // 两轮各打了一次模型：补充**真的进了下一轮请求**，而不只是"内存里多了一条"
      assertEq(await reqCount(), baseA + 2, 'A 线应恰好两次 LLM 请求（首轮 + 合并后的一轮）');
      const reqsA = await llm.requests();
      const lastA = [...reqsA[baseA + 1].body.messages].reverse().find((m) => m.role === 'user');
      assert(
        lastA.content.includes('A第二条') && lastA.content.includes('A第三条'),
        `A 线第 2 次请求的最后一条 user 消息应是合并结果（实际 ${JSON.stringify(lastA.content)}）`,
      );

      // ═══════════════════════════════════════════════════════════════════
      // B 轮边界折进：有工具调用 ⇒ 抽干发生在**同一条请求链**里，
      //   合并消息出现在 tool 结果**之后**（不可能插在 tool_calls 与结果之间）。
      // ═══════════════════════════════════════════════════════════════════
      const baseB = await reqCount();
      const sidB = 'e2e-t19b';
      const b = await openSession(cli, { sid: sidB, workdir: hd.workdir });
      await b.write('B第一条：先写个文件。');

      await waitFor(async () => (await reqCount()) >= baseB + 1, {
        what: 'B 线首轮 LLM 请求',
        timeoutMs: 20_000,
      });
      const b2 = await b.write('B第二条：另外再补充。');
      const b3 = await b.write('B第三条：还有。');

      await waitFor(() => userTexts(readMessagesJson(hd.homedir, sidB) ?? []).length >= 2, {
        what: 'B 线合并消息落库',
        timeoutMs: 30_000,
      });
      await waitFor(async () => (await reqCount()) >= baseB + 2, {
        what: 'B 线工具结果之后的第 2 次 LLM 请求',
        timeoutMs: 30_000,
      });

      const reqsB = await llm.requests();
      const reqB = reqsB[baseB + 1].body.messages;
      const toolIdx = reqB.findIndex((m) => m.role === 'tool');
      assert(toolIdx >= 0, 'B 线第 2 次请求应携带 role=tool 的工具结果');
      const mergedIdx = reqB.findIndex(
        (m) => m.role === 'user' && typeof m.content === 'string' && m.content.includes('B第二条'),
      );
      assert(mergedIdx >= 0, `B 线合并消息应进**同一轮**的第 2 次请求（实际消息序列：${reqB.map((m) => m.role).join(',')}）`);
      assert(
        mergedIdx > toolIdx,
        `合并消息必须出现在工具结果**之后**（tool@${toolIdx}，merged@${mergedIdx}）——` +
          '插在 assistant(tool_calls) 与 tool 结果之间会让部分协议直接 400',
      );
      assert(
        reqB[mergedIdx].content.includes('B第三条'),
        'B 线合并正文应含两条补充',
      );

      const msgsB = readMessagesJson(hd.homedir, sidB);
      assertTranscriptInvariants(msgsB, 'T19-B');
      assertEq(userTexts(msgsB).length, 2, 'B 线：两条补充在存储里仍算**一轮**（共两条用户消息）');
      const mergedB = msgsB.find((m) => m.id === b2);
      assertEq(mergedB?.meta?.[META_SUPPLEMENT_COUNT], 2, 'B 线合并条数');
      assertEq(mergedB?.meta?.[META_SUPPLEMENT_IDS], [b2, b3], 'B 线原始条目 id 留痕');

      // ═══════════════════════════════════════════════════════════════════
      // C 平凡值：`supplements_enabled = false` ⇒ 完全退回"一条消息 = 一轮"。
      //   平凡值的意义是"关掉它，行为与改造前**逐字**一致"——所以这里连
      //   `meta.supplement*` 一个字段都不该出现。
      // ═══════════════════════════════════════════════════════════════════
      const baseC = await reqCount();
      const sidC = 'e2e-t19c';
      const c = await openSession(cliOff, { sid: sidC, workdir: hdOff.workdir });
      await c.write('C第一条：请开始。');

      await waitFor(async () => (await reqCount()) >= baseC + 1, {
        what: 'C 线首轮 LLM 请求',
        timeoutMs: 20_000,
      });
      await c.write('C第二条：再补充一点。');
      await c.write('C第三条：还有一点。');

      await waitFor(() => userTexts(readMessagesJson(hdOff.homedir, sidC) ?? []).length >= 3, {
        what: 'C 线三条补充各自成轮',
        timeoutMs: 40_000,
      });
      // 用户消息**先落库**、该轮的 LLM 请求**后记录**——只等消息数就数请求，会在
      // 最后一轮还在飞的时候读到"少一次"。等三轮各自打完（每轮 1 用户 + 1 助手）。
      await waitFor(() => (readMessagesJson(hdOff.homedir, sidC) ?? []).length >= 6, {
        what: 'C 线三轮各自的助手正文落库',
        timeoutMs: 40_000,
      });

      const msgsC = readMessagesJson(hdOff.homedir, sidC);
      assertTranscriptInvariants(msgsC, 'T19-C');
      assertEq(
        userTexts(msgsC),
        ['C第一条：请开始。', 'C第二条：再补充一点。', 'C第三条：还有一点。'],
        'C 线（平凡值）：三条补充必须各自成轮，正文逐字不变',
      );
      assert(
        !msgsC.some((m) => m.meta?.[META_SUPPLEMENT]),
        'C 线（平凡值）不得出现任何合并标记——平凡值下行为与改造前逐字一致',
      );
      assertEq(await reqCount(), baseC + 3, 'C 线应恰好三次 LLM 请求（一条一轮）');
    } finally {
      cli.stop();
      cliOff.stop();
      llm.stop();
      cleanupHomedir(hd);
      cleanupHomedir(hdOff);
    }
  },
);
