import './_selfrun.mjs';
// T15 子智能体空间自驱动会话：**往空间写收件箱**即驱动一轮，不需要调用方连接。
//
// 钉的是 ADR-026 的四条：
// 1. 用户消息 = 对空间的一次 `vdfs/write(<agentdir>/session/<sid>/inbox)`；
// 2. 运行由空间自己消费（收件箱条目被取走、消息落进**子智能体自己的**存储）；
// 3. 机制统一（顶层与子空间同一套 inbox 语义，没有子智能体专用分支）；
// 4. FIFO、忙则排队：一轮在跑时新消息只入队，跑完再取；排队中的条目可删除取消。
//
// 与 `agent_run` 的区分：这条路径全程没有「派生子会话」——子会话是**那个空间
// 自己的**（`<homedir>/agent/<id>/session/<sid>`，与父会话不同的 store）。
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  cleanupHomedir,
  startLongLivedCli,
  readAgentMessagesJson,
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

const AGENT_ID = 'reviewer';
const PERSONA = '你是评审子智能体：回答必须简短，并称自己为「评审」。';
const SUB_SESSION = 'sub-1';

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

export default defineCase('T15 子智能体空间自驱动：写 inbox → 空间自己跑完整会话（FIFO / 忙则排队 / 可取消）', async () => {
  const llm = await new MockLlm([
    // 第一轮：慢吐（拉长运行窗口，给「忙则排队 + 取消」留出时序空间）
    { id: 'slow', match: '第一步', chunks: ['收到', '第一', '步', '了'], chunkDelayMs: 500 },
    { id: 'second', match: '第二步', content: '收到第二步了。' },
    { id: 'third', match: '第三步', content: '收到第三步了。' },
  ]).start();

  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT) },
  });
  // 子智能体目录：自带 manifest / 自身指令层 / **自己的模型服务**
  makeAgentDir(hd.homedir, {
    id: AGENT_ID,
    name: '评审',
    persona: PERSONA,
    providerId: PROVIDER_ID,
    providerPort: llm.port,
  });

  const cli = startLongLivedCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    session: 'e2e-t15',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });

  const subSession = `agent/${AGENT_ID}/session/${SUB_SESSION}`;
  const subMessages = () => readAgentMessagesJson(hd.homedir, AGENT_ID, SUB_SESSION) ?? [];

  /** 空间内地址：`<根>/agent/<id>/session/<sid>/inbox[/<iid>]` */
  const inboxAddr = (root, iid) =>
    `${root.replace(/\/+$/, '')}/${subSession}/inbox${iid ? `/${iid}` : ''}`;

  try {
    await cli.waitGatewayReady();

    const rootResp = await cli.invoke('vdfs/root', {});
    const root = rootResp.body?.data?.path;
    assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');

    // ── ① 空间里的会话：经 VDFS 创建（元数据带 workdir，与 CLI `ensure_session` 同一手法）
    const created = await cli.invoke('vdfs/write', {
      path: `${root.replace(/\/+$/, '')}/${subSession}`,
      create: true,
      text: JSON.stringify({
        metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' },
      }),
    });
    assertEq(created.status, 200, `创建子智能体空间里的会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);

    // ── ② 收件箱是会话内部的**一个普通集合**（与会话转写并列），地址可 stat
    const inboxStat = await cli.invoke('vdfs/stat', { path: inboxAddr(root) });
    assertEq(inboxStat.body?.data?.kind, 'inbox', `收件箱的 kind 是稳定协议词（${JSON.stringify(inboxStat.body)?.slice(0, 300)}）`);

    // ── ③ 写入收件箱 = 发消息（无 create 位，与 `vdfs_write` 工具同一形状）
    const first = await cli.invoke('vdfs/write', {
      path: inboxAddr(root),
      text: '第一步：报告。',
    });
    assertEq(first.status, 200, `写收件箱应成功（${JSON.stringify(first.body)?.slice(0, 300)}）`);
    const firstAddr = first.body?.data?.path;
    assert(
      typeof firstAddr === 'string' && firstAddr.endsWith('/inbox/' + firstAddr.split('/').pop()),
      `回执应给出条目自身地址（实际 ${JSON.stringify(firstAddr)}）`,
    );

    // ⑧ 忙则排队：第一轮（慢吐）已在跑时再入队两条 + 取消其中一条
    await waitFor(async () => (await llm.requests()).length >= 1, {
      what: '子空间的首轮 LLM 请求（证明消费确实发生了）',
      timeoutMs: 20_000,
    });
    const second = await cli.invoke('vdfs/write', { path: inboxAddr(root), text: '第二步：总结。' });
    const third = await cli.invoke('vdfs/write', { path: inboxAddr(root), text: '第三步：收尾。' });
    assertEq([second.status, third.status], [200, 200], '忙碌期间的写入照样成功（入队即返回）');

    // 取消一条**排队中**的条目：delete 条目自身地址
    const thirdAddr = third.body?.data?.path;
    const cancelled = await cli.invoke('vdfs/delete', { path: thirdAddr });
    assertEq(cancelled.status, 200, `取消排队中的条目（${JSON.stringify(cancelled.body)?.slice(0, 300)}）`);

    const queued = await cli.invoke('vdfs/list', { path: inboxAddr(root) });
    const queuedItems = queued.body?.data?.items ?? [];
    assert(
      !queuedItems.some((n) => n.name === thirdAddr.split('/').pop()),
      '被取消的条目不应仍在队列里',
    );

    // ── ④ 最终收敛：取消掉的那条**从未**变成消息；其余两条按序被消费
    await waitFor(() => subMessages().some((m) => textOf(m).includes('收到第二步了')), {
      what: '第二条消息在会话空闲后被消费',
      timeoutMs: 30_000,
    });
    await waitFor(() => subMessages().filter((m) => m.type === 'text' && m.role === 'assistant').length >= 2, {
      what: '两轮回答都落进子智能体自己的存储',
      timeoutMs: 30_000,
    });

    const msgs = subMessages();
    assertTranscriptInvariants(msgs, 'T15');
    const userTexts = msgs.filter((m) => m.role === 'user' && m.type === 'text').map(textOf);
    assertEq(
      userTexts,
      ['第一步：报告。', '第二步：总结。'],
      'FIFO：按入队顺序消费；被取消的那条从未成为消息',
    );

    // 队列已排空（条目出队即消失——「正在处理的那条」在转写里，不在队列里）
    const drained = await cli.invoke('vdfs/list', { path: inboxAddr(root) });
    assertEq((drained.body?.data?.items ?? []).length, 0, '消费完的收件箱应为空');

    // ── ⑤ 子智能体带的是**它自己**的指令层与模型服务
    const bodies = (await llm.requests()).map((r) => JSON.stringify(r.body));
    assert(
      bodies.some((b) => b.includes('评审子智能体')),
      '子空间里的会话注入了它自己的 AGENTS.md（子树 agent 实例的宿主目录就是 agent 目录）',
    );
    assert(
      (await llm.requests()).length >= 2,
      '两次消费各自真的打了一次模型（消息入队不等于被消费）',
    );
  } finally {
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
