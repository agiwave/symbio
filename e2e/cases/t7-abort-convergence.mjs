// T7 中止收敛：长驻 CLI（REPL）+ gateway HTTP 边界 action(会话地址, "abort") → 存储终态收敛。
import './_selfrun.mjs';
//
// 走真实双入口（REPL stdin 发送 + gateway `POST /api/v1/invoke` 中止），与
// Tauri 前端同构。one-shot CLI 一轮即退、进程内 gateway 随之消失，因此必须长驻。
// 钉的不变量：
// - 中止后**不得有节点停在 streaming/pending**；
// - 会话节点 attributes.outcome = "aborted"（经 vdfs/stat 真实边界读取）；
// - D-335 后没有 Turn 节点：中止落在**本轮正文节点**的 status=aborted 上，
//   且不落失败节点、不落兜底（中止不是失败，D-044 / D-165）。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  readSessionMessages,
  abortSession,
  waitFor,
  assert,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgTypeOf,
} from '../helpers.mjs';

export default defineCase('T7 中止收敛：运行中 action(会话地址, abort)，节点全部终态 + outcome=aborted', async () => {
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
    //    入口 = 节点动作 `action(<会话地址>, "abort")`（input-face 双轨退役后唯一路径）
    const ab = await abortSession(cli, 'e2e-t7');
    assert(ab.status === 200, `abort 调用应成功（${ab.status}: ${JSON.stringify(ab.body)?.slice(0, 200)}）`);
    assert(ab.body?.data?.ok === true, `abort 应真的停下一轮（ok = false 说明会话没在跑）：${JSON.stringify(ab.body)?.slice(0, 200)}`);

    // ④ 收敛判据：轮询**生产读出口**（vdfs/read）直到 aborted 节点落格。
    // 只等「无 streaming/pending」会在助手节点尚未落格时空转通过——用户节点没有
    // 在途状态，条件在发送后就恒真；中止节点出现才说明本轮真的收束了。
    await waitFor(
      async () => {
        const msgs = (await readSessionMessages(cli, 'e2e-t7')) ?? [];
        return msgs.some((m) => m.status === 'aborted');
      },
      { what: '中止节点落格（本轮收束）', timeoutMs: 20_000 },
    );

    // ⑤ 不变量断言
    const msgs = await readSessionMessages(cli, 'e2e-t7');
    assertTranscriptInvariants(msgs, 'T7');

    // D-335：整轮骨架已折叠，不再有 Turn 节点——中止落在**本轮正文节点**的状态上。
    // 把「没有 Turn 节点」从隐含变成显式回归钉子，同时钉住「中止不落失败 / 兜底」。
    const turnNodes = msgs.filter((m) => msgTypeOf(m) === 'turn');
    assert(turnNodes.length === 0, `D-335 后不应再有 Turn 节点（实际 ${turnNodes.length}）`);

    const aborted = msgs.filter((m) => m.status === 'aborted');
    assert(
      aborted.length === 1,
      `应恰好有一个 aborted 节点（实际 ${aborted.length}：${aborted.map((m) => `${msgTypeOf(m)}:${m.id}`).join(', ')}）`,
    );
    assert(msgTypeOf(aborted[0]) === 'text', `aborted 应落在正文节点（实际 ${msgTypeOf(aborted[0])}）`);

    // 中止不是失败：不推失败节点、不带错误信息（兜底是失败路径，D-044 / D-165）。
    const failedNodes = msgs.filter((m) => m.status === 'failed');
    assert(failedNodes.length === 0, `中止不应产生失败节点（实际 ${failedNodes.length}）`);
    assert(msgs.every((m) => m.error == null), '中止不应带错误信息（error 应为空）');

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
    console.log('DEBUG homedir:', hd.homedir);
    if (!process.env.E2E_KEEP) {
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  }
});
