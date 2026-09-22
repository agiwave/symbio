// T11 压缩节点协议：压缩是**消息节点**（不是会话级横幅），
// 且它在实时流上具备完整的「开始 → 结束」两态。
import './_selfrun.mjs';
//
// S20.4 把「正在压缩」从会话节点的 `attributes.phase` 搬成了消息节点
// （`msg_type = compression`）。本用例在 `session/stream` 上钉住这条契约：
//
// 帧协议：**帧 = 一条 `ChatMessage`**（`{ session_id, seq, message }`），
// 语义全在字段上（`delta` 追加 / `content` 替换 / `status=removed` 移除）。
//
// | 断言 | 意图 |
// |---|---|
// | 流上出现 `compression` 节点 | 压缩不是会话级横幅，它有自己的位置（可回溯） |
// | 首帧 `streaming` + 非空正文 | 用户能看到「正在发生什么」，不是空白等待 |
// | 终态帧（completed / failed）**带正文** | 「压掉了多少 / 为什么失败」实时可见（不是只落库） |
// | 位置在「本轮用户消息之后、本轮 Turn 之前」 | 压缩是会话里真实发生的一步，有先后 |
// | 失败时带 `failure_kind` | 可诊断（S20.6） |
// | 重写后逐条 `removed` + 快照帧（`meta.compacted`） | 前端历史能被就地收敛，不必猜 |
//
// 触发参数沿用 T8（实测校准）：`max_context_tokens=12000` + 每轮超大回复，
// 第 3 轮起水位越过 70% 阈值。
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
  readMessagesJson,
  assertTranscriptInvariants,
} from '../helpers.mjs';

const FILL = '上下文填充内容用于推高历史水位。';
const REPLY = '填充：' + FILL.repeat(600);
const SNAPSHOT_XML = [
  '<state_snapshot>',
  '  <overall_goal>验证上下文压缩链路端到端可用。</overall_goal>',
  '  <key_knowledge>',
  '    - 用户偏好：mock 环境下回复应包含轮次编号。',
  '  </key_knowledge>',
  '  <completed_items>',
  '    - 前几轮对话已完成。',
  '  </completed_items>',
  '  <in_progress_items>',
  '    - 等待压缩后的下一轮对话验证记忆延续。',
  '  </in_progress_items>',
  '  <open_questions></open_questions>',
  '</state_snapshot>',
].join('\n');

export default defineCase('T11 压缩节点协议：compression 节点的开始/终态与重写收敛', async () => {
  const llm = await new MockLlm([
    { id: 'snapshot', match: 'Distill the conversation above', content: SNAPSHOT_XML },
    { id: 'big', match: '轮问题', content: REPLY },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [
      { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 12000 }) },
    ],
    pluginConfigs: {
      session: { auto_compress: true, context_messages: 0 },
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
    session: 'unused-t11',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });
  try {
    await cli.waitGatewayReady();

    const ws = new WebSocket(`ws://127.0.0.1:${GATEWAY_PORT}/api/v1/ws`);
    await new Promise((res, rej) => {
      ws.once('open', res);
      ws.once('error', rej);
    });
    ws.send(JSON.stringify({ metadata: { path: 'session/stream' }, payload: {} }));

    /** @type {Array<{seq:number, session_id:string, message:any}>} */
    const ops = [];
    ws.on('message', (data) => {
      try {
        const frame = JSON.parse(data.toString());
        const ev = frame?.Data?.type === 'transcript_event' ? frame.Data.data : null;
        if (ev?.message) ops.push(ev);
      } catch { /* 忽略非 JSON 帧 */ }
    });

    const TERMINAL = new Set(['completed', 'failed', 'aborted', 'waiting_user_action']);
    const doneTurns = () =>
      ops.filter((o) => o.message?.type === 'turn' && TERMINAL.has(o.message?.status)).length;

    // 串行发 4 轮：每轮等「本轮新出现的 turn 终态」，避免并发发送把上一轮打成 aborted
    for (const [i, text] of ['第一轮问题', '第二轮问题', '第三轮问题', '第四轮问题'].entries()) {
      const before = doneTurns();
      const send = await cli.invoke(
        'session/chat/send',
        {
          session_id: 'e2e-t11',
          message: { id: `u-t11-${i}`, role: 'user', type: 'text', content: text },
          mode: 'auto',
        },
        { session_id: 'e2e-t11', workdir: hd.workdir },
      );
      assert(send.status === 200, `第 ${i + 1} 轮 chat/send 应受理（${send.status}）`);
      await waitFor(() => doneTurns() > before, { what: `第 ${i + 1} 轮 turn 终态`, timeoutMs: 60_000 });
    }
    await waitFor(
      () => ops.some((o) => o.message?.type === 'compression'),
      { what: '压缩节点出现在流上', timeoutMs: 10_000 },
    );
    ws.close();

    const timeline = ops.map(
      (o) =>
        `#${o.seq} [${o.message?.type ?? '-'}/${o.message?.role ?? '-'}/${o.message?.status ?? '-'}] ` +
        `${String(o.message?.id ?? '').slice(0, 8)}` +
        `${o.message?.content != null ? ` =${JSON.stringify(o.message.content).slice(0, 20)}` : ''}`,
    );

    // ① seq 严格递增
    for (let i = 1; i < ops.length; i++) {
      assert(ops[i].seq > ops[i - 1].seq, `seq 应严格递增（${ops[i - 1].seq} → ${ops[i].seq}）`);
    }

    // ② 压缩是一个**消息节点**：每一次压缩（可能多轮各一次）在流上是一组同 id 的帧
    const compFrames = ops.filter((o) => o.message?.type === 'compression');
    assert(compFrames.length >= 2, `流上应有 compression 节点的帧（实际 ${compFrames.length}）\n时间线:\n${timeline.join('\n')}`);
    /** @type {Map<string, Array<{seq:number, message:any}>>} */
    const episodes = new Map();
    for (const f of compFrames) {
      const list = episodes.get(f.message.id) ?? [];
      list.push(f);
      episodes.set(f.message.id, list);
    }
    assert(episodes.size >= 1, '至少应有一次压缩');

    for (const [id, frames] of episodes) {
      // ③ 开始帧：streaming + 非空正文（用户视角「正在发生什么」）
      assertEq(frames[0].message.status, 'streaming', `compression ${String(id).slice(0, 8)} 首帧应为 streaming`);
      assert(
        String(frames[0].message.content ?? '').length > 0,
        `compression ${String(id).slice(0, 8)} 开始帧应带非空正文（否则前端是一个没有内容的空壳节点）`,
      );
      // ④ 终态帧：completed 或 failed，且**带正文**（结果说明是压缩节点唯一的结果输出，
      //    从未经 delta 上线——必须以完整消息帧下发，而不是只发状态）
      const final = frames[frames.length - 1].message;
      assert(
        TERMINAL.has(final.status),
        `compression ${String(id).slice(0, 8)} 应以终态收场（实际 ${final.status}）\n时间线:\n${timeline.join('\n')}`,
      );
      assert(
        String(final.content ?? '').length > 0,
        `compression ${String(id).slice(0, 8)} 终态应带结果说明（成功写"压掉了多少"，失败写原因）；实时缺正文 = 用户要重开会话才看得到`,
      );
      // 失败必须可诊断：failure_kind 是机读码
      if (final.status === 'failed') {
        assert(
          final.meta?.failure_kind != null,
          `compression ${String(id).slice(0, 8)} 失败应带 meta.failure_kind（实际 meta=${JSON.stringify(final.meta)}）`,
        );
      }
    }

    // ⑤ 位置语义：压缩是**根级**节点（与 user / turn 平级），且早于它所属的那个 Turn
    for (const [id, frames] of episodes) {
      const beginSeq = frames[0].seq ?? 0;
      assert(
        frames.every((f) => f.message.parent_id == null),
        `compression ${String(id).slice(0, 8)} 应是根级节点（不是某个 Turn 的子节点）`,
      );
      const firstFrame = ops.find((o) => o.message?.id === id);
      assert(firstFrame.message.parent_id == null, `compression ${String(id).slice(0, 8)} 不得挂在 Turn 下`);
      // 它之后的第一个 Turn（在流上首次出现的）必须晚于压缩开始
      const nextTurn = ops.find(
        (o) => o.message?.type === 'turn' && o.seq > (frames[frames.length - 1].seq ?? 0),
      );
      assert(nextTurn, `compression ${String(id).slice(0, 8)} 之后应有一个 Turn`);
      const seenBefore = ops.some(
        (o) => o.message?.id === nextTurn.message.id && o.seq < beginSeq,
      );
      assert(
        !seenBefore,
        `compression ${String(id).slice(0, 8)} 应早于其所属 Turn（该 Turn 在压缩之前就已出现）\n时间线:\n${timeline.join('\n')}`,
      );
    }

    // ⑥ 重写收敛：压缩成功后流上应有被压掉历史的 removed 帧与快照帧（meta.compacted）
    const snapshotFrames = ops.filter((o) => o.message?.meta?.compacted === true);
    const snapshotFrame = snapshotFrames[snapshotFrames.length - 1];
    assert(
      snapshotFrame,
      `压缩成功后流上应出现 meta.compacted=true 的快照节点\n时间线:\n${timeline.join('\n')}`,
    );
    const removes = ops.filter((o) => o.message?.status === 'removed');
    assert(removes.length > 0, '压缩重写应向流下发被压掉消息的 removed 帧（前端据此就地收敛）');
    // removed 的目标必须是本连接上出现过的节点（删除未知 id = 前端既不知道该删谁，
    // 也不知道自己少了什么）
    for (const rm of removes) {
      const appeared = ops.some(
        (o) => o.message?.id === rm.message.id && o.seq < rm.seq,
      );
      assert(
        appeared,
        `removed 了流上从未出现过的节点 ${String(rm.message.id).slice(0, 8)}\n时间线:\n${timeline.join('\n')}`,
      );
    }

    // ⑦ 落盘同源：快照节点真的进了存储
    const msgs = readMessagesJson(hd.homedir, 'e2e-t11');
    assertTranscriptInvariants(msgs, 'T11');
    const snapshotIds = new Set(snapshotFrames.map((f) => f.message.id));
    const storedSnap = msgs.find((m) => m.meta?.compacted === true);
    assert(storedSnap, '存储里应有压缩快照消息');
    assert(
      snapshotIds.has(storedSnap.id),
      `存储里的快照（${String(storedSnap.id).slice(0, 8)}）应是流上下发过的那些之一（${[...snapshotIds].map((s) => String(s).slice(0, 8)).join(',')}）`,
    );
  } finally {
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
