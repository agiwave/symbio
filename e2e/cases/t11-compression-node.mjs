// T11 压缩节点协议：压缩是**消息节点**（不是会话级横幅），
// 且它在实时面上具备完整的「开始 → 结束」两态。
import './_selfrun.mjs';
//
// S20.4 把「正在压缩」从会话节点的 `attributes.phase` 搬成了消息节点
// （`msg_type = compression`）。本用例在**实时面**上钉住这条契约——实时面只有
// 一条：`event_bus` 的 `vdfs` 频道 + `vdfs/watch` 登记（见
// `helpers.subscribeSessionRealtime` 的「两步缺一不可」）。
//
// 变更信封：`{ path, data? }`（**没有操作枚举**），`data` 是一条 `ChatMessage`，
// 语义全在字段上（`delta` 追加 / `content` 替换 / `status = removed` 移除）。
// 先后由**到达顺序**（单一订阅 FIFO）给出，`seq` 是节点位置（同一节点逐帧恒定）。
//
// | 断言 | 意图 |
// |---|---|
// | 实时面出现 `compression` 节点 | 压缩不是会话级横幅，它有自己的位置（可回溯） |
// | 首帧 `streaming` + 非空正文 | 用户能看到「正在发生什么」，不是空白等待 |
// | 终态帧（completed / failed）**带正文** | 「压掉了多少 / 为什么失败」实时可见（不是只落库） |
// | 位置在「本轮 Turn 之前」 | 压缩是会话里真实发生的一步，有先后 |
// | 失败时带 `failure_kind` | 可诊断（S20.6） |
// | 重写后逐条 `removed` + 快照帧（`meta.compacted`） | 前端历史能被就地收敛，不必猜 |
//
// 触发参数沿用 T8（实测校准）：`max_context_tokens=12000` + 每轮超大回复，
// 第 3 轮起水位越过 70% 阈值。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  readMessagesJson,
  assertTranscriptInvariants,
  assertSeqAnchorIsNodeAttribute,
} from '../helpers.mjs';

const TERMINAL = new Set(['completed', 'failed', 'aborted', 'waiting_user_action']);

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
  let rt = null;
  try {
    await cli.waitGatewayReady();

    // ① 订阅实时面（发消息之前，不漏首帧）
    rt = await subscribeSessionRealtime(cli, { sessionId: 'e2e-t11', gatewayPort: GATEWAY_PORT });

    // 每个 turn 节点**最后一次**状态到达终态的个数（按节点去重，不按帧计数）
    const doneTurns = () => {
      const last = new Map();
      for (const { mid, data } of rt.messages()) {
        if (data?.type === 'turn') last.set(mid, data.status);
      }
      return [...last.values()].filter((s) => TERMINAL.has(s)).length;
    };

    // ② 串行发 4 轮：每轮等「本轮新出现的 turn 终态」，避免并发发送把上一轮打成 aborted
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
      () => rt.messages().some(({ data }) => data?.type === 'compression'),
      { what: '压缩节点出现在实时面上', timeoutMs: 10_000 },
    );

    const ops = rt.messages();
    const timeline = ops.map(
      ({ mid, data, arrival }) =>
        `#${arrival} [${data?.type ?? '-'}/${data?.role ?? '-'}/${data?.status ?? '-'}] ` +
        `${String(mid).slice(0, 8)}` +
        `${data?.content != null ? ` =${JSON.stringify(data.content).slice(0, 20)}` : ''}`,
    );

    // ③ 订阅必须真的接到变更（两步订阅缺一不可）
    assert(
      ops.length > 0,
      `实时面应有消息变更（event_bus/subscribe + vdfs/watch）\n时间线:\n${timeline.join('\n')}`,
    );

    // ④ 压缩是一个**消息节点**：每一次压缩（可能多轮各一次）在实时面上是一组同 id 的帧
    const compFrames = ops.filter(({ data }) => data?.type === 'compression');
    assert(
      compFrames.length >= 2,
      `实时面上应有 compression 节点的帧（实际 ${compFrames.length}）\n时间线:\n${timeline.join('\n')}`,
    );
    /** @type {Map<string, Array<{mid:string, data:any, arrival:number}>>} */
    const episodes = new Map();
    for (const f of compFrames) {
      const list = episodes.get(f.mid) ?? [];
      list.push(f);
      episodes.set(f.mid, list);
    }
    assert(episodes.size >= 1, '至少应有一次压缩');

    for (const [id, frames] of episodes) {
      // ⑤ 开始帧：streaming + 非空正文（用户视角「正在发生什么」）
      assertEq(frames[0].data.status, 'streaming', `compression ${String(id).slice(0, 8)} 首帧应为 streaming`);
      assert(
        String(frames[0].data.content ?? '').length > 0,
        `compression ${String(id).slice(0, 8)} 开始帧应带非空正文（否则前端是一个没有内容的空壳节点）`,
      );
      // ⑥ 终态帧：completed 或 failed，且**带正文**（结果说明是压缩节点唯一的结果输出，
      //    从未经 delta 上线——必须以完整消息帧下发，而不是只发状态）
      const final = frames[frames.length - 1].data;
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
      // 位置序号是节点属性：允许「在途号… → 权威号（其后全部帧）」一次换号——
      // 压缩节点的号是 `begin` 时转写发的在途号，落库后由回包帧交回权威号
      // （§3.4 唯一的换号时机）。断言"逐帧完全相同"会把"从未落库"当成正确。
      assertSeqAnchorIsNodeAttribute(frames, `compression ${String(id).slice(0, 8)}`);
    }

    // ⑦ 位置语义：压缩是**根级**节点（与 user / turn 平级），且**早于**它所属的那个 Turn
    //    （先后由到达序给出——单一订阅 FIFO）
    const firstArrivalOfTurn = (mid) =>
      Math.min(...rt.framesOf(mid).map((f) => f.arrival));
    for (const [id, frames] of episodes) {
      assert(
        frames.every((f) => f.data.parent_id == null),
        `compression ${String(id).slice(0, 8)} 应是根级节点（不是某个 Turn 的子节点）`,
      );
      const beginArrival = frames[0].arrival;
      const endArrival = frames[frames.length - 1].arrival;
      // 它之后的第一个 Turn（在实时面上首次出现的）必须晚于压缩开始
      const turnMids = [...new Set(ops.filter(({ data }) => data?.type === 'turn').map(({ mid }) => mid))];
      const nextTurnMid = turnMids
        .map((mid) => ({ mid, at: firstArrivalOfTurn(mid) }))
        .filter((t) => t.at > endArrival)
        .sort((a, b) => a.at - b.at)[0];
      assert(nextTurnMid, `compression ${String(id).slice(0, 8)} 之后应有一个 Turn`);
      assert(
        nextTurnMid.at > beginArrival,
        `compression ${String(id).slice(0, 8)} 应早于其所属 Turn（该 Turn 在压缩之前就已出现）\n时间线:\n${timeline.join('\n')}`,
      );
    }

    // ⑧ 重写收敛：压缩成功后实时面上应有被压掉历史的 removed 帧与快照帧（meta.compacted）
    const snapshotFrames = ops.filter(({ data }) => data?.meta?.compacted === true);
    const snapshotFrame = snapshotFrames[snapshotFrames.length - 1];
    assert(
      snapshotFrame,
      `压缩成功后实时面上应出现 meta.compacted=true 的快照节点\n时间线:\n${timeline.join('\n')}`,
    );
    const removes = ops.filter(({ data }) => data?.status === 'removed');
    assert(removes.length > 0, '压缩重写应下发被压掉消息的 removed 帧（前端据此就地收敛）');
    // removed 的目标必须是本连接上出现过的节点（删除未知 id = 前端既不知道该删谁，
    // 也不知道自己少了什么）
    for (const rm of removes) {
      const appeared = ops.some((o) => o.mid === rm.mid && o.arrival < rm.arrival);
      assert(
        appeared,
        `removed 了实时面上从未出现过的节点 ${String(rm.mid).slice(0, 8)}\n时间线:\n${timeline.join('\n')}`,
      );
    }

    // ⑨ 落盘同源：快照节点真的进了存储
    const msgs = readMessagesJson(hd.homedir, 'e2e-t11');
    assertTranscriptInvariants(msgs, 'T11');
    const snapshotIds = new Set(snapshotFrames.map((f) => f.mid));
    const storedSnap = msgs.find((m) => m.meta?.compacted === true);
    assert(storedSnap, '存储里应有压缩快照消息');
    assert(
      snapshotIds.has(storedSnap.id),
      `存储里的快照（${String(storedSnap.id).slice(0, 8)}）应是实时面上下发过的那些之一（${[...snapshotIds].map((s) => String(s).slice(0, 8)).join(',')}）`,
    );
  } finally {
    rt?.close();
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
