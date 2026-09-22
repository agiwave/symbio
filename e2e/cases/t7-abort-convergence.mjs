// T7 中止收敛：长驻 CLI（REPL）+ gateway HTTP 边界中途 chat/abort → 存储终态收敛。
import './_selfrun.mjs';
//
// 走真实双入口（REPL stdin 发送 + gateway `POST /api/v1/invoke` 中止），与
// Tauri 前端同构。one-shot CLI 一轮即退、进程内 gateway 随之消失，因此必须长驻。
// 钉的不变量：
// - 中止后**不得有节点停在 streaming/pending**（S20.3 的 converge_inflight 职责）；
// - 会话节点 attributes.outcome = "aborted"（经 vdfs/stat 真实边界读取）；
// - Turn 根 → Aborted，Turn 子节点 → Completed（abort_terminal_of 契约）。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  readMessagesJson,
  waitFor,
  assert,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T7 中止收敛：运行中 chat/abort，节点全部终态 + outcome=aborted', async () => {
  // 流式场景：正文分片慢吐（拉长运行窗口），给 abort 留出时序空间
  const llm = await new MockLlm([
    { id: 'slow', match: '慢慢说', chunks: ['很', '长', '很', '长', '的', '回', '答'], chunkDelayMs: 300 },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: {
      // 无此文件时网关回退默认配置（inbound_enabled: false）——监听不会被拉起
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
    session: 'e2e-t7',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });
  try {
    await cli.waitGatewayReady();

    // ① 发送：REPL stdin 进一轮慢速对话
    cli.send('慢慢说');

    // ② 等会话真正开跑（LLM 收到请求）
    await waitFor(async () => (await llm.requests()).length >= 1, { what: '首轮 LLM 请求', timeoutMs: 20_000 });

    // ③ 中止：走 gateway HTTP 边界（session_id 经 metadata 透传进 ctx）
    const ab = await cli.invoke('session/chat/abort', { session_id: 'e2e-t7' }, { session_id: 'e2e-t7' });
    assert(ab.status === 200, `abort 调用应成功（${ab.status}: ${JSON.stringify(ab.body)?.slice(0, 200)}）`);

    // ④ 收敛判据：轮询存储直到无节点停在 streaming/pending
    await waitFor(
      () => {
        const msgs = readMessagesJson(hd.homedir, 'e2e-t7') ?? [];
        const nonTerminal = msgs.filter((m) => ['streaming', 'pending', 'waiting_user_action'].includes(m.status));
        return msgs.length > 0 && nonTerminal.length === 0;
      },
      { what: '中止后节点全部收敛到终态', timeoutMs: 20_000 },
    );

    // ⑤ 不变量断言
    const msgs = readMessagesJson(hd.homedir, 'e2e-t7');
    assertTranscriptInvariants(msgs, 'T7');

    // Turn 根 → Aborted；Turn 子节点（本用例无工具）→ 正文不悬空
    const turnRoots = msgs.filter((m) => m.type === 'turn' && !m.parent_id);
    assert(turnRoots.length >= 1, '至少应有一个 Turn 根节点');
    for (const t of turnRoots) {
      assert(t.status === 'aborted', `Turn 根应为 aborted（实际 ${t.status}）`);
    }
    const streaming = msgs.filter((m) => m.status === 'streaming' || m.status === 'pending');
    assert(streaming.length === 0, '不得有节点停在 streaming/pending');

    // ⑥ outcome=aborted 经 vdfs/stat 真实边界读取（会话节点 attributes）。
    // 回执形状：PluginPayloadWire = { type: 'Data', data: <节点> }
    const root = await cli.invoke('vdfs/root', {});
    const rootPath = root.body?.data?.path;
    assert(typeof rootPath === 'string' && rootPath.length > 0, `vdfs/root 应返回根地址（${JSON.stringify(root.body)?.slice(0, 200)}）`);
    const stat = await cli.invoke('vdfs/stat', { path: `${rootPath.replace(/\/+$/, '')}/session/e2e-t7` });
    const node = stat.body?.data;
    assert(!!node, `vdfs/stat 应返回会话节点（${JSON.stringify(stat.body)?.slice(0, 200)}）`);
    assert(node.outcome === 'aborted', `会话节点 outcome 应为 aborted（实际 ${JSON.stringify(node.outcome)}）`);
  } finally {
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
