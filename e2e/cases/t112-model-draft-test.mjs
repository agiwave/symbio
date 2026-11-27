import './_selfrun.mjs';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

export default defineCase('T112 模型草稿测试：真实连接回执含模型、协议与耗时且不落盘', async () => {
  const llm = await new MockLlm([{ id: 'ping', match: 'ping', content: 'pong' }]).start();
  const port = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: {
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: port,
        inbound_token: '',
        inbound_readonly: false,
      },
    },
  });
  const cli = startLongLivedCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    session: 'unused-t112',
    provider: PROVIDER_ID,
    gatewayPort: port,
  });
  try {
    await cli.waitGatewayReady();
    const dataOf = (resp) => {
      assertEq(resp.status, 200, `网关请求应受理（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
      assert(resp.body?.data != null, '网关应返回 Data 载荷');
      return resp.body.data;
    };
    const root = dataOf(await cli.invoke('vdfs/root', {})).path;
    const mount = `${root.replace(/\/+$/, '')}/model`;
    const before = dataOf(await cli.invoke('vdfs/list', { path: mount }));

    const result = dataOf(await cli.invoke('vdfs/action', {
      path: mount,
      action: 'test',
      payload: { ...providerConfig(llm.port), skip_validation: true },
    }));
    assertEq(result.ok, true, `草稿测试应通过（${result.message}）`);
    assert(result.message.includes('mock-model'), `回执应含模型名（${result.message}）`);
    assert(result.message.includes('openai_chat'), `回执应含协议（${result.message}）`);
    assert(result.message.includes('ms'), `回执应含耗时（${result.message}）`);
    assertEq(result.data?.model, 'mock-model', '结构化回执应含模型名');
    assertEq(result.data?.api_protocol, 'openai_chat', '结构化回执应含协议');
    assert(Number.isInteger(result.data?.elapsed_ms) && result.data.elapsed_ms >= 0, '结构化回执应含非负耗时');

    const after = dataOf(await cli.invoke('vdfs/list', { path: mount }));
    assertEq(
      (after.items ?? []).map((item) => item.path),
      (before.items ?? []).map((item) => item.path),
      '测试草稿不得新建或改变 Provider 条目',
    );

    const failed = dataOf(await cli.invoke('vdfs/action', {
      path: mount,
      action: 'test',
      payload: { ...providerConfig(llm.port), api_protocol: 'not-a-protocol' },
    }));
    assertEq(failed.ok, false, '无效协议应给出失败回执');
    assert(failed.message.includes('not-a-protocol'), '失败回执应保留原因');
    assertEq(failed.data, undefined, '失败回执不应伪装成测试成功数据');
    const afterFailure = dataOf(await cli.invoke('vdfs/list', { path: mount }));
    assertEq(
      (afterFailure.items ?? []).map((item) => item.path),
      (before.items ?? []).map((item) => item.path),
      '失败测试同样不得落盘',
    );
  } finally {
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
