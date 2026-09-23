// T10 节点协议全景：Turn / Reasoning / Text / ToolCall 四类节点的
// 「开始 → 流增量 → 结束」三态在**实时面**上是否成立。
import './_selfrun.mjs';
//
// 与 T9 同在 gateway WS 边界上观察（实时面**只有一条**：`event_bus` 的 `vdfs`
// 频道 + `vdfs/watch` 登记，见 `helpers.subscribeSessionRealtime`），但断言
// 从「正文拼接」升级为「每个节点类型的状态机」。
//
// 变更信封：`{ path, data? }`（**没有操作枚举**），`data` 是一条 `ChatMessage`，
// 语义全在字段上（`delta` 追加 / `content` 替换 / `status = removed` 移除 /
// 其余字段合并）。
//
// 顺序的唯一来源是**到达顺序**（单一订阅 FIFO）——`seq` 是节点的**位置**（同一
// 节点逐帧恒定），不是投递序号。因此本用例用 `arrival`（收集下标）比较先后。
//
// | 断言 | 意图 |
// |---|---|
// | delta 必先有身份帧 | 增量不得落在未经身份建立的节点上（否则前端没有渲染语义） |
// | 首帧 content + Σdelta == 该节点正文 | 增量不丢不重，终态不含正文也不影响收敛 |
// | 每个非终态节点都到达终态 | 前端不会留下永远转圈的「运行中」 |
// | Turn 终态晚于全部子节点终态 | 组合节点终态跟随子树（§5.3.2） |
// | ToolCall 有 role=tool 结果子节点 | 「有请求必有响应」（前端响应段的唯一数据来源） |
//
// 本用例是**协议探针**：任何一条不成立都对应前端一个具体的显示缺口，
// 因此断言失败信息必须指出「哪个节点、哪一帧」。
import { join } from 'node:path';
import { readFileSync } from 'node:fs';

import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  SEG_MESSAGES,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  readMessagesJson,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 终态词（与后端 `is_terminal` 同口径） */
const TERMINAL = new Set(['completed', 'failed', 'aborted', 'waiting_user_action']);

export default defineCase('T10 节点协议全景：Turn/Reason/Text/ToolCall 三态与增量收敛', async () => {
  const llm = await new MockLlm([
    {
      id: 'panorama',
      match: '全景',
      reasoning: ['先', '分析', '一下'],
      toolCalls: [
        { id: 'call_t10', name: 'vdfs_write', arguments: { path: 't10.md', text: '# 写入内容' } },
      ],
      chunks: ['正文甲', '正文乙', '正文丙'],
      chunkDelayMs: 25,
    },
    { id: 'after-tool', afterTool: true, chunks: ['工具已完成'] },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: {
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: GATEWAY_PORT,
        inbound_token: '',
        inbound_readonly: false,
      },
    },
  });
  const cli = startLongLivedCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    session: 'unused-t10',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });
  let rt = null;
  try {
    await cli.waitGatewayReady();

    // ① 订阅实时面（发消息之前，不漏首帧）
    rt = await subscribeSessionRealtime(cli, { sessionId: 'e2e-t10', gatewayPort: GATEWAY_PORT });

    // ② 发起对话
    const send = await cli.invoke(
      'session/chat/send',
      {
        session_id: 'e2e-t10',
        message: { id: 'u-t10', role: 'user', type: 'text', content: '全景' },
        mode: 'auto',
      },
      { session_id: 'e2e-t10', workdir: hd.workdir },
    );
    assert(send.status === 200, `chat/send 应受理（${send.status}）`);

    // ③ 等实时面**静默收敛**：没有任何节点停在非终态，且连续 `QUIET_MS` 没有新变更。
    //
    // ⚠️ 判据不能是「任一 Turn 到达终态」：本用例是**两轮**对话（第一轮调工具、
    // 结果回灌后再来一轮），第一轮的 Turn 到终态时第二轮才刚开始——按那个条件
    // `close()` 会砍掉第二轮的尾巴，于是 ⑫「进入过非终态的节点都收在终态」
    // 随机变红（实测 8 次跑 4 次失败，因为 `waitFor` 是 100ms 轮询：
    // 轮询间隔内到达的第二轮帧进了收集器，而它的终态帧没进）。
    //
    // 静默窗口同时解决「瞬时空闲」：两轮之间的空隙里 `unsettled` 也可能为空，
    // 要求「连续无新帧」才不会在那里提前收网。
    const QUIET_MS = 600;
    /** 已出现过、但最后一帧状态不是终态的节点 id */
    function unsettledIds() {
      const entered = new Set();
      const last = new Map();
      for (const { mid, data } of rt.messages()) {
        if (!mid || data?.status == null) continue;
        if (!TERMINAL.has(data.status)) entered.add(mid);
        last.set(mid, data.status);
      }
      return [...entered].filter((id) => !TERMINAL.has(last.get(id)));
    }
    let lastLen = -1;
    let quietSince = Date.now();
    await waitFor(
      () => {
        if (rt.messages().length !== lastLen) {
          lastLen = rt.messages().length;
          quietSince = Date.now();
        }
        return unsettledIds().length === 0 && Date.now() - quietSince >= QUIET_MS;
      },
      { what: '实时面静默收敛（无未终态节点 + 连续无新帧）', timeoutMs: 30_000 },
    );

    const ops = rt.messages();
    // ── 时间线（失败时打印，定位到帧） ──
    const timeline = ops.map(
      ({ mid, data, arrival }) =>
        `#${arrival} [${data?.type ?? '-'}/${data?.role ?? '-'}/${data?.status ?? '-'}] ` +
        `${String(mid ?? '').slice(0, 8)}` +
        `${data?.delta != null ? ` +${JSON.stringify(data.delta).slice(0, 20)}` : ''}` +
        `${data?.content != null ? ` =${JSON.stringify(data.content).slice(0, 20)}` : ''}`,
    );

    // ④ 订阅必须真的接到变更（两步订阅缺一不可，见 helper 文档）
    assert(
      ops.length > 0,
      `实时面应有消息变更（event_bus/subscribe + vdfs/watch）\n时间线:\n${timeline.join('\n')}`,
    );
    assert(
      ops.every(({ mid }) => mid.length > 0),
      `消息变更的落点应是 <会话>/${SEG_MESSAGES}/<mid>`,
    );

    // ④' `seq` 是节点位置：同一节点的每一帧都是同一个值（逐帧递增的是"投递序号"，
    //     那个概念在 S27 已随 `session/stream` 一起退役）
    for (const { mid } of ops) {
      const seqs = rt
        .framesOf(mid)
        .map((f) => f.data?.seq)
        .filter((s) => s != null);
      assert(
        new Set(seqs).size <= 1,
        `节点 ${String(mid).slice(0, 8)} 的 seq 必须逐帧相同（位置不变）：实得 ${JSON.stringify(seqs)}`,
      );
    }

    // ⑤ delta 必先有身份帧（对未经身份建立的节点追加 = 前端拿不到渲染语义）
    const established = new Set();
    for (const { mid, data } of ops) {
      if (!mid) continue;
      if (data?.type != null) established.add(mid);
      if (data?.delta != null) {
        assert(
          established.has(mid),
          `delta 落在尚未建立身份的节点 ${String(mid).slice(0, 8)}（协议违例）\n时间线:\n${timeline.join('\n')}`,
        );
      }
      if (data?.status === 'removed') established.delete(mid);
    }

    // ⑥ 逐节点重建：末帧 = 该节点在流上的最后一帧；正文 = 首帧 content + Σdelta
    //（`content` 是整条替换，`delta` 是尾部追加，二者互斥——与前端落地同源）
    /** @type {Map<string, any>} */
    const nodes = new Map();
    for (const { mid, data } of ops) nodes.set(mid, data);
    const framesFor = (mid) => rt.framesOf(mid);
    const rebuild = (mid) => {
      const frames = framesFor(mid);
      const start = frames.find((f) => f.data?.content != null);
      if (!start) return null;
      let text = '';
      for (const f of frames) {
        if (f.data?.content != null) text = String(f.data.content);
        else if (f.data?.delta != null) text += String(f.data.delta);
      }
      return { start, final: frames[frames.length - 1], text };
    };

    // ⑦ Reasoning 节点
    const reasonNode = [...nodes.values()].find((m) => m.type === 'reasoning');
    assert(reasonNode, `实时面应有 reasoning 节点\n时间线:\n${timeline.join('\n')}`);
    const rb = rebuild(reasonNode.id);
    assertEq(rb.start.data.status, 'streaming', `reasoning 首帧应为 streaming（节点 ${reasonNode.id}）`);
    assert(
      framesFor(reasonNode.id).some((f) => f.data?.delta != null),
      'reasoning 应有流式增量帧',
    );
    assertEq(rb.final.data.status, 'completed', 'reasoning 终态应为 completed');
    assertEq(rb.text, '先分析一下', 'reasoning 增量拼接应等于完整内容');

    // ⑧ Text 节点（正文，role=assistant）
    const textNode = [...nodes.values()].find((m) => m.type === 'text' && m.role === 'assistant');
    assert(textNode, `实时面应有 assistant text 节点\n时间线:\n${timeline.join('\n')}`);
    const tb = rebuild(textNode.id);
    assertEq(tb.start.data.status, 'streaming', `text 首帧应为 streaming（节点 ${textNode.id}）`);
    assert(
      // 不相对于**源分片数**断言：投递会把相邻的同节点纯增量合成一帧
      // （`DELIVER_WINDOW_MS`），帧数因此可能少于分片数。这里要守的是
      // 「增量是窄追加、不是每片全量重发」——有 `delta` 且拼接结果正确即已覆盖。
      framesFor(textNode.id).some((f) => f.data?.delta != null),
      'text 应有流式增量帧（窄追加，而非每片全量重发）',
    );
    assertEq(tb.final.data.status, 'completed', 'text 终态应为 completed');
    assertEq(tb.text, '正文甲正文乙正文丙', 'text 增量拼接应等于完整正文');

    // ⑨ ToolCall 节点：参数流式 → 执行窗口（started_at）→ 终态
    const tcNode = [...nodes.values()].find((m) => m.type === 'tool_call');
    assert(tcNode, `实时面应有 tool_call 节点\n时间线:\n${timeline.join('\n')}`);
    const cb = rebuild(tcNode.id);
    assertEq(cb.start.data.status, 'streaming', 'tool_call 首帧应为 streaming');
    assert(
      cb.start.data.name === 'vdfs_write',
      `tool_call 应带工具名（实际 ${cb.start.data.name}）`,
    );
    // 参数增量：首帧之后应有 delta（参数分片）
    assert(
      framesFor(tcNode.id).some((f) => f.data?.delta != null),
      'tool_call 参数应有窄增量（不得每片全量重发）',
    );
    assert(
      cb.text.includes('t10.md'),
      `tool_call 参数增量应拼出完整 JSON（实得 ${JSON.stringify(cb.text)}）`,
    );
    // 执行窗口：终态之前应有一帧带 meta.started_at（否则前端在长工具上无任何「运行中」判据）
    const runningFrame = ops.find(
      ({ mid, data }) => mid === tcNode.id && data?.meta?.started_at != null,
    );
    assert(
      runningFrame,
      `tool_call 应有一帧带 meta.started_at（执行窗口的运行中信号）\n时间线:\n${timeline.join('\n')}`,
    );
    assertEq(cb.final.data.status, 'completed', 'tool_call 终态应为 completed');

    // ⑩ 组合节点终态跟随子树：Turn 终态不得早于任一子节点终态。
    //    顺序用**到达序**（单一 FIFO）——`seq` 是节点位置，不承担投递顺序。
    const turnNode = [...nodes.values()].find((m) => m.type === 'turn');
    assert(turnNode, '实时面应有 turn 组合节点');
    const terminalArrival = (mid) =>
      Math.max(
        ...rt
          .framesOf(mid)
          .filter((f) => TERMINAL.has(f.data?.status))
          .map((f) => f.arrival),
      );
    const turnTerminalArrival = terminalArrival(turnNode.id);
    for (const id of [reasonNode.id, textNode.id, tcNode.id]) {
      const childTerminalArrival = terminalArrival(id);
      assert(
        childTerminalArrival < turnTerminalArrival,
        `子节点 ${String(id).slice(0, 8)} 终态(arrival=${childTerminalArrival}) 应早于 Turn 终态(arrival=${turnTerminalArrival})`,
      );
    }

    // ⑪ 有请求必有响应：ToolCall 必须有 role=tool 的结果子节点，且它要在**实时面**上
    const resultNode = [...nodes.values()].find(
      (m) => m.role === 'tool' && m.parent_id === tcNode.id,
    );
    assert(
      resultNode,
      `tool_call ${String(tcNode.id).slice(0, 8)} 应有 role=tool 的结果子节点，且该子节点必须出现在实时面上（前端响应段的唯一数据来源）\n时间线:\n${timeline.join('\n')}`,
    );

    // ⑫ 凡是进入过非终态的节点，都必须以终态收场（不留永远转圈的「运行中」）。
    //
    // 判据是「进入过非终态」而不是「全部节点都必须有终态」：用户消息不参与状态机
    // （后端 `emit_persisted_message` 发的是存储副本，落点即终态），这是**已知
    // 的协议空洞**（见评审记录），不在此把它钉成不变量。
    for (const [id] of nodes) {
      const frames = framesFor(id);
      // 从不带 status 的帧组成（纯 delta / content 帧）不参与状态机，跳过
      if (frames.every((f) => f.data?.status == null)) continue;
      if (!frames.some((f) => f.data?.status != null && !TERMINAL.has(f.data.status))) continue;
      const last = frames.filter((f) => f.data?.status != null).pop()?.data?.status;
      assert(
        TERMINAL.has(last),
        `节点 ${String(id).slice(0, 8)}（${frames[0].data?.type}）进入过非终态却收在 ${last}\n时间线:\n${timeline.join('\n')}`,
      );
    }

    // ⑬ 落盘侧同源：实时面与存储是同一份事实
    const stored = readMessagesJson(hd.homedir, 'e2e-t10');
    assertTranscriptInvariants(stored, 'T10');
    for (const m of [reasonNode, textNode, tcNode]) {
      const row = stored.find((x) => x.id === m.id);
      assert(row, `存储里应有实时面上出现过的节点 ${String(m.id).slice(0, 8)}（${m.type}）`);
      assertEq(row.status, m.status, `节点 ${m.type} 存储状态应与实时面终态一致`);
    }
    // 工具真的执行了（不是只在协议层打转）
    assert(
      readFileSync(join(hd.workdir, 't10.md'), 'utf8').includes('# 写入内容'),
      'vdfs_write 应真实写入工作目录',
    );
  } finally {
    rt?.close();
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
