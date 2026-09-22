// T9 gateway WebSocket 流式：前端同构边界上验证实时过程显示链路。
import './_selfrun.mjs';
//
// `WS /api/v1/ws` 是 Tauri 前端「会话/流式」的真实等价物：连接后第一帧发
// `PluginMessageWire`（与 route_v2 同线格式），后端返回会话通道后双向转发
// `PluginFrame`。本用例在**这条通道上**订阅 `session/stream`（NodeOp 帧流），
// 然后经 HTTP 边界发起对话，验证过程显示的核心契约：
// - 帧序呈现「Upsert(Start) → Append*（流式增量）→ Upsert(End)」的形态；
// - Append 只落在同一条消息上，按到达顺序拼接 == 最终正文（增量不丢不重）；
// - stream 帧带 session_id 归属（广播语义下前端可按会话过滤）。
// 相对 cases/：上两层即仓库根（ws 依赖来自 tauri 前端的 node_modules）
import WebSocket from '../../tauri/node_modules/ws/index.js';

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
} from '../helpers.mjs';

export default defineCase('T9 gateway WS 流式：session/stream NodeOp 帧序与增量拼接契约', async () => {
  const llm = await new MockLlm([
    {
      id: 'flow',
      match: '流式说',
      chunks: ['第一', '第二', '第三', '第四', '完成'],
      chunkDelayMs: 60,
    },
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
    session: 'unused-t9',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });
  try {
    await cli.waitGatewayReady();

    // ① 在前端同构边界上订阅消息实时面：首帧 = PluginMessageWire
    const ws = new WebSocket(`ws://127.0.0.1:${GATEWAY_PORT}/api/v1/ws`);
    await new Promise((res, rej) => {
      ws.once('open', res);
      ws.once('error', rej);
    });
    ws.send(JSON.stringify({ metadata: { path: 'session/stream' }, payload: {} }));

    // ② 收集 NodeOp 帧（发消息前订阅，不漏 Start）。
    // 信封：PluginFrame::Data({ type: 'transcript_event', data: NodeEvent })，
    // NodeEvent = { session_id, seq, ...NodeOp }（tag 字段 op，snake_case）
    /** @type {Array<{op:string, seq:number, session_id:string, [k:string]:any}>} */
    const ops = [];
    let wsClosed = false;
    ws.on('message', (data) => {
      try {
        const frame = JSON.parse(data.toString());
        const ev = frame?.Data?.type === 'transcript_event' ? frame.Data.data : null;
        if (ev?.op) ops.push(ev);
      } catch { /* 忽略非 JSON 帧 */ }
    });
    ws.on('close', () => { wsClosed = true; });

    // ③ 经 HTTP 边界发起对话（会话目标挂在 metadata.session_id）
    const send = await cli.invoke(
      'session/chat/send',
      {
        session_id: 'e2e-t9',
        message: { id: 'u-t9', role: 'user', type: 'text', content: '流式说' },
        mode: 'auto',
      },
      { session_id: 'e2e-t9', workdir: hd.workdir },
    );
    assert(send.status === 200, `chat/send 应受理（${send.status}: ${JSON.stringify(send.body)?.slice(0, 150)}）`);

    // ④ 等待流收敛：出现 assistant 正文完成帧
    await waitFor(
      () =>
        ops.some(
          (o) =>
            o.op === 'upsert' &&
            o.message?.role === 'assistant' &&
            o.message?.type === 'text' &&
            o.message?.status === 'completed',
        ),
      { what: '流式帧收敛（assistant completed）', timeoutMs: 20_000 },
    );
    ws.close();

    // ⑤ 帧序契约：同一 assistant 消息的 Start → Append* → End 形态
    const flowOps = ops.filter(
      (o) =>
        (o.op === 'upsert' && o.message?.role === 'assistant') || o.op === 'append',
    );
    // 正文子节点（type=text）承担流式内容；turn 节点是骨架
    const firstText = flowOps.find((o) => o.op === 'upsert' && o.message?.type === 'text');
    assert(!!firstText, '帧流应含正文的 Upsert（Start 快照）');
    const targetId = firstText.message.id;
    const appends = flowOps.filter((o) => o.op === 'append' && o.message_id === targetId);
    assert(appends.length >= 3, `应有流式 Append 增量（实际 ${appends.length}）`);

    // ⑥ 单调 seq：同一流的帧序号严格递增（缺口即 resync 的前提）
    const seqs = ops.map((o) => o.seq);
    for (let i = 1; i < seqs.length; i++) {
      assert(seqs[i] > seqs[i - 1], `seq 应严格递增（${seqs[i - 1]} -> ${seqs[i]}）`);
    }

    // ⑦ 归属：帧带 session_id（广播语义下前端按会话过滤）
    assert(ops.every((o) => o.session_id === 'e2e-t9'), '流帧应携带会话归属');

    // ⑧ 增量拼接 == 最终正文：不丢不重。
    // 首个分片随 Start 快照下发（content 非空），Append 只携带后续增量——
    // 最终正文 = Start 快照内容 + Appends 按序拼接
    const finalMsg = ops.find(
      (o) =>
        o.op === 'upsert' && o.message?.id === targetId && o.message?.status === 'completed',
    )?.message;
    assert(!!finalMsg, '流结束应有正文 completed 终态快照');
    const streamed = String(firstText.message.content ?? '') + appends.map((o) => o.delta).join('');
    assertEq(finalMsg.content, streamed, 'Start 内容 + Append 拼接应等于最终正文');

    assert(!wsClosed || ops.length > 0, 'WS 在会话期间不应被服务端提前关闭');
  } finally {
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
