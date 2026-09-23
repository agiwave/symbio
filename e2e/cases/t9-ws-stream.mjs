// T9 gateway WS 实时面：前端同构边界上的「变更帧序 + 增量拼接」契约。
import './_selfrun.mjs';
//
// `WS /api/v1/ws` 是 Tauri 前端「会话实时面」的真实等价物：首帧发
// `PluginMessageWire`（与 route_v2 同线格式），此后这条连接只承载该频道的帧。
//
// ## 实时面**只有一条通道**（S27 / ADR-025）
//
// 订阅是**两步**，缺一不可（见 `helpers.subscribeSessionRealtime` 的文档）：
//   ① `event_bus/subscribe` → 总线广播口（收 `kind = "vdfs"` 的帧）；
//   ② `vdfs/watch`          → 把 sink 登记进 provider 的变更表（**谁来 publish**）。
//
// 变更信封**没有操作枚举**：形状恒为 `{ path, data? }`，语义全在 `data` 的字段上
// （`delta` 追加 / `content` 替换 / `status = removed` 移除）。
//
// | 断言 | 意图 |
// |---|---|
// | 变更非空 | 「两步订阅」任缺一步 ⇒ 零变更（不报错、不断连，最像"模型没产出"） |
// | 首帧全量 + 后续窄 delta | 增量不得落在没有基线的节点上 |
// | 首帧 content + Σdelta == 完整正文 | 增量不丢不重 |
// | 同一节点的 `seq` 逐帧相同 | `seq` 是**节点属性**（位置），不是投递序号 |
// | 全部变更 `path` 在会话作用域内 | 广播语义下消费端按地址归属 |
// | 离开 working 的会话帧**到达晚于**全部消息帧 | 「不忙 ⇒ 本轮已终态」的唯一依据 |
//
// 相对 cases/：上两层即仓库根（ws 依赖来自 tauri 前端的 node_modules）
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

const TERMINAL = new Set(['completed', 'failed', 'aborted', 'waiting_user_action']);

export default defineCase('T9 gateway WS 实时面：vdfs 变更帧序与增量拼接契约', async () => {
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
  let rt = null;
  try {
    await cli.waitGatewayReady();

    // ① 订阅实时面（**发消息之前**，不漏首帧）
    rt = await subscribeSessionRealtime(cli, { sessionId: 'e2e-t9', gatewayPort: GATEWAY_PORT });

    // ② 经 HTTP 边界发起对话（会话目标挂在 metadata.session_id）
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

    // ③ 等正文收敛（assistant text 到达终态）
    const isDone = () =>
      rt.messages().some((m) => m.data?.type === 'text' && m.data?.role === 'assistant' && TERMINAL.has(m.data?.status));
    await waitFor(isDone, { what: '实时面收敛（assistant text 终态）', timeoutMs: 20_000 });

    // ④ 再等**会话收尾那一帧**（离开 working）。它是本轮实时面的最后一帧，
    //    也是 ⑧ 那条结构性断言的锚点——不等它就关连接会偶发漏掉它。
    await waitFor(() => rt.sessionNode() != null && rt.sessionNode()?.status !== 'working', {
      what: '会话运行态收尾帧（离开 working）',
      timeoutMs: 20_000,
    });

    const changes = rt.changes;
    const msgs = rt.messages();
    const timeline = changes.map(
      (c, i) =>
        `#${i} ${c.path.slice(rt.scope.length + 1) || '<会话>'} ` +
        `[${c.data?.type ?? '-'}/${c.data?.role ?? '-'}/${c.data?.status ?? '-'}]` +
        `${c.data?.delta != null ? ` +${JSON.stringify(c.data.delta)}` : ''}` +
        `${c.data?.content != null ? ` =${JSON.stringify(c.data.content).slice(0, 20)}` : ''}`,
    );

    // ⑤ 「两步订阅」缺一不可：任缺一步 ⇒ 一条变更都收不到。
    //    这条断言的存在本身就是诊断——零变更时最像"模型没有产出内容"。
    assert(
      changes.length > 0,
      `实时面应收到变更（两步订阅：event_bus/subscribe + vdfs/watch）\n` +
        `作用域外的路径 ${rt.outOfScope.length} 条、时间线:\n${timeline.join('\n')}`,
    );

    // ⑥ 归属：变更落点即地址，消息是 `<会话>/message/<mid>`
    assert(
      msgs.length > 0,
      `应有消息节点的变更（落点 <会话>/${SEG_MESSAGES}/<mid>）\n时间线:\n${timeline.join('\n')}`,
    );
    assert(
      changes.every((c) => c.path === rt.scope || c.path.startsWith(`${rt.scope}/`)),
      '全部变更都应落在会话作用域内（广播语义下消费端按地址归属）',
    );

    // ⑦ 帧形态：同一正文节点的「首帧全量（身份 + 正文）→ 窄增量*」
    const firstText = msgs.find((m) => m.data?.type === 'text' && m.data?.role === 'assistant');
    assert(!!firstText, `应含正文节点的首帧（身份 + 首段正文）\n时间线:\n${timeline.join('\n')}`);
    const frames = rt.framesOf(firstText.mid);
    const head = frames[0].data;
    assertEq(head.status, 'streaming', '首帧状态应为 streaming');
    assertEq(head.type, 'text', '首帧应带节点身份（type）——增量不得落在没有身份的节点上');
    assert(head.content != null, '首帧应带正文（content，全量）');
    const deltas = frames.filter((f) => f.data?.delta != null);
    assert(deltas.length >= 3, `应有流式 delta 增量（实际 ${deltas.length}）`);
    assert(
      frames[frames.length - 1].data?.status === 'completed',
      `末帧应为终态（实际 ${frames[frames.length - 1].data?.status}）`,
    );

    // ⑧ `seq` 是**节点属性**（位置序号），不是投递序号：同一节点的每一帧都是同一个值。
    //    曾经 `seq` 是逐帧递增的流内序号，于是"顺序"成了投递属性——到达顺序一变
    //    （两条通道、乱序合并）就要靠补丁纠正。S27 后顺序由**单一订阅 FIFO** 给出。
    const seqsOfNode = frames.map((f) => f.data?.seq).filter((s) => s != null);
    assert(
      seqsOfNode.length > 0,
      `正文节点的帧应带位置序号 seq\n时间线:\n${timeline.join('\n')}`,
    );
    assert(
      new Set(seqsOfNode).size === 1,
      `同一节点的 seq 必须逐帧相同（位置不变，变的是正文）：实得 ${JSON.stringify(seqsOfNode)}`,
    );

    // ⑨ 增量拼接 == 完整正文：不丢不重。
    //    首个分片随首帧以 `content` 下发（全量），其余分片是 `delta`；终态帧只带状态。
    const rebuilt = frames.reduce(
      (acc, f) =>
        f.data?.content != null
          ? String(f.data.content)
          : f.data?.delta != null
            ? acc + String(f.data.delta)
            : acc,
      '',
    );
    assertEq(rebuilt, '第一第二第三第四完成', '首帧正文 + delta 拼接应等于模型产出的完整正文');

    // ⑩ **核心断言**：会话报"不忙"的那一帧必须**到达晚于**本轮全部消息帧。
    //
    // 后端在「清在途 → 复位 is_working」**之后**才发运行态帧，而两者走同一条订阅
    // （单一 FIFO），因此「读到 status != working」蕴含「本轮全部消息帧都已在其之前
    // 到达」。若哪天有人把运行态挪回另一条通道，或让消息绕开这条订阅，这里立刻变红。
    const lastMsgArrival = Math.max(...msgs.map((m) => m.arrival));
    // 取**最后一次**离开 working：订阅发生在发消息之前，因此第一帧是**空闲初始态**
    // （arrival 0）——它当然早于全部消息帧，不能用 `find` 取第一个。
    const idleArrival = changes
      .map((c, i) => ({ c, i }))
      .filter(({ c }) => c.path === rt.scope && c.data?.status !== 'working')
      .map(({ i }) => i)
      .pop();
    assert(idleArrival != null, '应观察到会话离开 working（收尾那一帧）');
    assert(
      idleArrival > lastMsgArrival,
      `会话报"不忙"的那一帧必须排在全部消息帧之后（idle@${idleArrival} > 末条消息@${lastMsgArrival}）` +
        '——否则「不忙 ⇒ 本轮已终态」推不出来',
    );

    // ⑪ 落盘同源：实时面与存储是同一份事实
    assertTranscriptInvariants(readMessagesJson(hd.homedir, 'e2e-t9'), 'T9');
  } finally {
    rt?.close();
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
