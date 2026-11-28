import './_selfrun.mjs';
// T114 四协议草稿测试：同一条 `vdfs/action test` 入口逐项验证线格式适配，不落盘。
import { createServer } from 'node:http';
import {
  makeHomedir,
  cleanupHomedir,
  nextPort,
  startLongLivedCli,
  providerConfig,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

const PROTOCOLS = [
  'openai_chat',
  'openai_responses',
  'anthropic_messages',
  'gemini_api',
];

export default defineCase(
  'T114 模型协议：四协议草稿测试全部返回真实回执且不落盘，未知协议失败',
  async () => {
    const port = nextPort();
    const gatewayPort = nextPort();
    const server = createServer((req, res) => {
      let body = '';
      req.on('data', (chunk) => { body += chunk; });
      req.on('end', () => {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ ok: true, path: req.url, bodyLength: body.length }));
      });
    });
    await new Promise((resolve) => server.listen(port, '127.0.0.1', resolve));

    const hd = makeHomedir({
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: gatewayPort,
          inbound_token: '',
          inbound_readonly: false,
        },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'unused-t114',
      gatewayPort,
    });
    const dataOf = (resp, what) => {
      assertEq(resp.status, 200, `${what} 应受理（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
      assert(resp.body?.data != null, `${what} 应返回 Data 载荷`);
      return resp.body.data;
    };

    try {
      await cli.waitGatewayReady();
      const root = dataOf(await cli.invoke('vdfs/root', {}), 'vdfs/root').path;
      const mount = `${root.replace(/\/+$/, '')}/model`;
      const before = (dataOf(await cli.invoke('vdfs/list', { path: mount }), 'list before').items ?? [])
        .map((item) => item.path)
        .sort();

      for (const protocol of PROTOCOLS) {
        const payload = {
          ...providerConfig(port, {
            api_base: `http://127.0.0.1:${port}/v1`,
            api_protocol: protocol,
            api_key: 'audit-only-key',
            model: 'mock-model',
          }),
          skip_validation: true,
        };
        const result = dataOf(
          await cli.invoke('vdfs/action', { path: mount, action: 'test', payload }),
          `${protocol} test`,
        );
        assertEq(result.ok, true, `${protocol} 草稿测试应通过（${result.message}）`);
        assertEq(result.data?.api_protocol, protocol, `${protocol} 回执应带回协议 id`);
        assertEq(result.data?.model, 'mock-model', `${protocol} 回执应带回模型名`);
        assert(
          Number.isInteger(result.data?.elapsed_ms) && result.data.elapsed_ms >= 0,
          `${protocol} 回执应带非负耗时`,
        );
        assert(result.message.includes(protocol), `${protocol} 提示应含协议 id`);
      }

      const failed = dataOf(await cli.invoke('vdfs/action', {
        path: mount,
        action: 'test',
        payload: {
          ...providerConfig(port, { api_protocol: 'not-a-protocol' }),
          skip_validation: true,
        },
      }), 'invalid protocol');
      assertEq(failed.ok, false, '未知协议必须给出失败回执');
      assert(failed.message.includes('not-a-protocol'), '失败原因必须保留协议词');

      const after = (dataOf(await cli.invoke('vdfs/list', { path: mount }), 'list after').items ?? [])
        .map((item) => item.path)
        .sort();
      assertEq(after, before, '草稿测试无论成功失败都不得落盘');
    } finally {
      cli.stop();
      await new Promise((resolve) => server.close(resolve));
      cleanupHomedir(hd);
    }
  },
);
