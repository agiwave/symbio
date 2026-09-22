// T10 节点协议全景：Turn / Reasoning / Text / ToolCall 四类节点的
// 「开始 → 流增量 → 结束」三态在**实时流**上是否成立。
import './_selfrun.mjs';
//
// 与 T9 同在 gateway WS 边界上观察（`session/stream` 的 NodeOp 帧流），但断言
// 从「正文拼接」升级为「每个节点类型的状态机」：
//
// | 断言 | 意图 |
// |---|---|
// | append 必先有 upsert | 增量不得落在未知节点上（协议违例会在后端被丢弃） |
// | start + Σappend == 终态 content | 增量不丢不重，终态是增量的收敛而不是另一份数 |
// | 每个非终态节点都到达终态 | 前端不会留下永远转圈的「运行中」 |
// | Turn 终态晚于全部子节点终态 | 组合节点终态跟随子树（§5.3.2） |
// | ToolCall 有 role=tool 结果子节点 | 「有请求必有响应」（前端响应段的唯一数据来源） |
//
// 本用例是**协议探针**：任何一条不成立都对应前端一个具体的显示缺口，
// 因此断言失败信息必须指出「哪个节点、哪一帧」。
import WebSocket from '../../tauri/node_modules/ws/index.js';
import { join } from 'node:path';
import { readFileSync } from 'node:fs';

import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
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
  try {
    await cli.waitGatewayReady();

    // ① 订阅消息实时面
    const ws = new WebSocket(`ws://127.0.0.1:${GATEWAY_PORT}/api/v1/ws`);
    await new Promise((res, rej) => {
      ws.once('open', res);
      ws.once('error', rej);
    });
    ws.send(JSON.stringify({ metadata: { path: 'session/stream' }, payload: {} }));

    /** @type {Array<{op:string, seq:number, session_id:string, [k:string]:any}>} */
    const ops = [];
    ws.on('message', (data) => {
      try {
        const frame = JSON.parse(data.toString());
        const ev = frame?.Data?.type === 'transcript_event' ? frame.Data.data : null;
        if (ev?.op) ops.push(ev);
      } catch { /* 忽略非 JSON 帧 */ }
    });

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

    // ③ 等整轮收敛：根 Turn 到达终态
    const turnDone = () =>
      ops.some((o) => o.op === 'upsert' && o.message?.type === 'turn' && TERMINAL.has(o.message?.status));
    await waitFor(turnDone, { what: '根 Turn 到达终态', timeoutMs: 30_000 });
    // 给尾部帧（工具结果子节点 / 会话运行态）一点落地时间
    await waitFor(
      () => ops.some((o) => o.op === 'upsert' && o.message?.role === 'tool'),
      { what: '工具结果子节点出现在流上', timeoutMs: 8_000 },
    ).catch(() => { /* 由下方断言给出结论 */ });
    ws.close();

    // ── 时间线（失败时打印，定位到帧） ──
    const timeline = ops.map(
      (o) =>
        `#${o.seq} ${o.op}${o.message ? ` [${o.message.type ?? '-'}/${o.message.role ?? '-'}/${o.message.status ?? '-'}] ${String(o.message.id ?? '').slice(0, 8)}` : ''}${
          o.delta != null ? ` +${JSON.stringify(o.delta).slice(0, 20)}` : ''
        }${o.message_id ? ` ->${String(o.message_id).slice(0, 8)}` : ''}`,
    );

    // ④ seq 严格递增（单调 seq 是「丢帧可检测」的唯一前提）
    for (let i = 1; i < ops.length; i++) {
      assert(ops[i].seq > ops[i - 1].seq, `seq 应严格递增（${ops[i - 1].seq} → ${ops[i].seq}）`);
    }
    assert(ops.every((o) => o.session_id === 'e2e-t10'), '流帧应携带会话归属');

    // ⑤ append 必先有 upsert（对未知 id 追加是协议违例，后端会丢弃该帧 → 丢字）
    const seen = new Set();
    for (const o of ops) {
      if (o.op === 'upsert' && o.message?.id) seen.add(o.message.id);
      if (o.op === 'append') {
        assert(
          seen.has(o.message_id),
          `append 落在尚未 upsert 的节点 ${o.message_id}（协议违例）\n时间线:\n${timeline.join('\n')}`,
        );
      }
      if (o.op === 'remove') seen.delete(o.message_id);
    }

    // ⑥ 逐节点重建：start 快照 + Σappend == 终态 content
    /** @type {Map<string, any>} */
    const nodes = new Map();
    for (const o of ops) {
      if (o.op === 'upsert' && o.message?.id) nodes.set(o.message.id, o.message);
    }
    const rebuild = (id) => {
      const frames = ops.filter((o) => (o.op === 'upsert' && o.message?.id === id) || (o.op === 'append' && o.message_id === id));
      const start = frames.find((o) => o.op === 'upsert');
      if (!start) return null;
      let text = String(start.message.content ?? '');
      for (const f of frames) if (f.op === 'append') text += f.delta;
      return { start, final: frames[frames.length - 1], text };
    };

    // ⑦ Reasoning 节点
    const reasonNode = [...nodes.values()].find((m) => m.type === 'reasoning');
    assert(reasonNode, `流上应有 reasoning 节点\n时间线:\n${timeline.join('\n')}`);
    const rb = rebuild(reasonNode.id);
    assertEq(rb.start.message.status, 'streaming', `reasoning 首帧应为 streaming（节点 ${rb.start.message.id}）`);
    assert(
      ops.some((o) => o.op === 'append' && o.message_id === rb.start.message.id),
      'reasoning 应有流式增量帧',
    );
    assertEq(rb.final.message.status, 'completed', 'reasoning 终态应为 completed');
    assertEq(rb.text, '先分析一下', 'reasoning 增量拼接应等于终态内容');

    // ⑧ Text 节点（正文，role=assistant）
    const textNode = [...nodes.values()].find((m) => m.type === 'text' && m.role === 'assistant');
    assert(textNode, `流上应有 assistant text 节点\n时间线:\n${timeline.join('\n')}`);
    const tb = rebuild(textNode.id);
    assertEq(tb.start.message.status, 'streaming', `text 首帧应为 streaming（节点 ${tb.start.message.id}）`);
    assert(ops.filter((o) => o.op === 'append' && o.message_id === tb.start.message.id).length >= 2, 'text 应有多片流式增量');
    assertEq(tb.final.message.status, 'completed', 'text 终态应为 completed');

    // ⑨ ToolCall 节点：参数流式 → 执行窗口（started_at）→ 终态
    const tcNode = [...nodes.values()].find((m) => m.type === 'tool_call');
    assert(tcNode, `流上应有 tool_call 节点\n时间线:\n${timeline.join('\n')}`);
    const cb = rebuild(tcNode.id);
    assertEq(cb.start.message.status, 'streaming', 'tool_call 首帧应为 streaming');
    assert(cb.start.message.name === 'vdfs_write', `tool_call 应带工具名（实际 ${cb.start.message.name}）`);
    // 参数增量：首帧之后应有 append（参数分两片）
    assert(
      ops.some((o) => o.op === 'append' && o.message_id === cb.start.message.id),
      'tool_call 参数应有窄增量（不得每片全量重发）',
    );
    // 执行窗口：终态之前应有一帧带 meta.started_at（否则前端在长工具上无任何「运行中」判据）
    const runningFrame = ops.find(
      (o) => o.op === 'upsert' && o.message?.id === cb.start.message.id && o.message?.meta?.started_at != null,
    );
    assert(
      runningFrame,
      `tool_call 应有一帧带 meta.started_at（执行窗口的运行中信号）\n时间线:\n${timeline.join('\n')}`,
    );
    assertEq(cb.final.message.status, 'completed', 'tool_call 终态应为 completed');

    // ⑩ 组合节点终态跟随子树：Turn 终态不得早于任一子节点终态
    const turnNode = [...nodes.values()].find((m) => m.type === 'turn');
    assert(turnNode, '流上应有 turn 组合节点');
    const turnTerminalSeq = Math.max(
      ...ops
        .filter((o) => o.op === 'upsert' && o.message?.id === turnNode.id && TERMINAL.has(o.message?.status))
        .map((o) => o.seq),
    );
    for (const id of [reasonNode.id, textNode.id, tcNode.id]) {
      const childTerminalSeq = Math.max(
        ...ops
          .filter((o) => o.op === 'upsert' && o.message?.id === id && TERMINAL.has(o.message?.status))
          .map((o) => o.seq),
      );
      assert(
        childTerminalSeq < turnTerminalSeq,
        `子节点 ${String(id).slice(0, 8)} 终态(seq=${childTerminalSeq}) 应早于 Turn 终态(seq=${turnTerminalSeq})`,
      );
    }

    // ⑪ 有请求必有响应：ToolCall 必须有 role=tool 的结果子节点，且它要在**实时流**上
    const resultNode = [...nodes.values()].find((m) => m.role === 'tool' && m.parent_id === cb.start.message.id);
    assert(
      resultNode,
      `tool_call ${String(cb.start.message.id).slice(0, 8)} 应有 role=tool 的结果子节点，且该子节点必须出现在实时流上（前端响应段的唯一数据来源）\n时间线:\n${timeline.join('\n')}`,
    );

    // ⑫ 凡是进入过非终态的节点，都必须以终态收场（不留永远转圈的「运行中」）。
    //
    // 判据是「进入过非终态」而不是「全部节点都必须有终态」：用户消息不参与状态机
    // （后端 `emit_persisted_message` 发的是存储副本，不带 status），这是**已知
    // 的协议空洞**（见评审记录），不在此把它钉成不变量。
    for (const [id] of nodes) {
      const frames = ops.filter((o) => o.op === 'upsert' && o.message?.id === id);
      // 从不带 status 的节点（用户消息）不参与状态机，跳过
      if (frames.every((f) => f.message?.status == null)) continue;
      if (!frames.some((f) => !TERMINAL.has(f.message?.status))) continue;
      const last = frames[frames.length - 1].message?.status;
      assert(
        TERMINAL.has(last),
        `节点 ${String(id).slice(0, 8)}（${frames[0].message?.type}）进入过非终态却收在 ${last}\n时间线:\n${timeline.join('\n')}`,
      );
    }

    // ⑬ 落盘侧同源：实时流与存储是同一份事实
    const msgs = readMessagesJson(hd.homedir, 'e2e-t10');
    assertTranscriptInvariants(msgs, 'T10');
    for (const m of [reasonNode, textNode, tcNode]) {
      const stored = msgs.find((x) => x.id === m.id);
      assert(stored, `存储里应有流上出现过的节点 ${String(m.id).slice(0, 8)}（${m.type}）`);
      assertEq(stored.status, m.status, `节点 ${m.type} 存储状态应与流上终态一致`);
    }
    // 工具真的执行了（不是只在协议层打转）
    assert(
      readFileSync(join(hd.workdir, 't10.md'), 'utf8').includes('# 写入内容'),
      'vdfs_write 应真实写入工作目录',
    );
  } finally {
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
