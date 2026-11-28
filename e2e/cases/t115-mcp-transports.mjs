import './_selfrun.mjs';
// T115 MCP 多 server：stdio 与 HTTP 两条真实 transport 都能完成连接测试，
// 启用开关按 server 隔离，HTTP 自定义头由客户端下发且保留头不被覆盖。
import { createServer } from 'node:http';
import { join } from 'node:path';
import {
  E2E_ROOT,
  makeHomedir,
  cleanupHomedir,
  nextPort,
  startLongLivedCli,
  addMcpServer,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

const STDIO = 'stdio-audit';
const HTTP = 'http-audit';

export default defineCase(
  'T115 MCP：stdio + HTTP 多 server 连接测试、启停隔离与 HTTP 自定义头',
  async () => {
    const gatewayPort = nextPort();
    const httpPort = nextPort();
    const seenHeaders = [];
    const server = createServer((req, res) => {
      let raw = '';
      req.on('data', (chunk) => { raw += chunk; });
      req.on('end', () => {
        seenHeaders.push(req.headers);
        const msg = (() => { try { return JSON.parse(raw); } catch { return {}; } })();
        const reply = (result) => {
          res.writeHead(200, {
            'content-type': 'application/json',
            'mcp-session-id': 't115-session',
          });
          res.end(JSON.stringify({ jsonrpc: '2.0', id: msg.id ?? null, result }));
        };
        if (msg.method === 'initialize') {
          return reply({
            protocolVersion: '2025-03-26',
            capabilities: { tools: {} },
            serverInfo: { name: 'mock-http-mcp', version: '1.2.3' },
            instructions: 'T115 本地模拟市场服务',
          });
        }
        if (msg.method === 'tools/list') {
          return reply({
            tools: [{
              name: 'http_echo',
              description: 'HTTP 传输的回显工具',
              inputSchema: { type: 'object', properties: { text: { type: 'string' } } },
            }],
          });
        }
        return reply({});
      });
    });
    await new Promise((resolve) => server.listen(httpPort, '127.0.0.1', resolve));

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
    addMcpServer(hd, STDIO, {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs')],
      enabled: true,
    });
    addMcpServer(hd, HTTP, {
      type: 'http',
      url: `http://127.0.0.1:${httpPort}/mcp`,
      headers: { 'x-audit-marker': 'T115' },
      enabled: true,
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'unused-t115',
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
      const mount = `${root.replace(/\/+$/, '')}/mcp`;
      const listed = dataOf(await cli.invoke('vdfs/list', { path: mount }), 'mcp list');
      const names = (listed.items ?? []).map((item) => item.path?.split('/').pop());
      assert(names.includes(STDIO) && names.includes(HTTP), `两个 server 都应可见（实际: ${names}）`);

      for (const [name, expected] of [
        [STDIO, { toolCount: 2, serverName: 'mock-mcp' }],
        [HTTP, { toolCount: 1, serverName: 'mock-http-mcp' }],
      ]) {
        const result = dataOf(
          await cli.invoke('vdfs/action', { path: `${mount}/${name}`, action: 'test', payload: {} }),
          `${name} test`,
        );
        assertEq(result.ok, true, `${name} 连接测试应通过（${result.message}）`);
        assertEq(result.data?.tool_count, expected.toolCount, `${name} 工具数`);
        assertEq(result.data?.server_name, expected.serverName, `${name} server 名`);
        assert(result.data?.protocol_version, `${name} 应回传协商协议版本`);
      }
      assert(
        seenHeaders.some((h) => h['x-audit-marker'] === 'T115'),
        'HTTP server 应收到配置里的自定义头',
      );
      assert(
        seenHeaders.some((h) => String(h.accept).includes('text/event-stream')),
        '客户端标准 Accept 头不得被自定义头覆盖',
      );

      const disabled = dataOf(await cli.invoke('vdfs/write', {
        path: `${mount}/${HTTP}`,
        text: JSON.stringify({
          type: 'http',
          url: `http://127.0.0.1:${httpPort}/mcp`,
          headers: { 'x-audit-marker': 'T115' },
          enabled: false,
        }),
      }), 'disable http');
      assert(disabled != null, '停用一个 server 应受理');
      const stillWorks = dataOf(
        await cli.invoke('vdfs/action', { path: `${mount}/${STDIO}`, action: 'test', payload: {} }),
        'stdio after http disabled',
      );
      assertEq(stillWorks.ok, true, '停用 HTTP 不影响另一个 stdio server');
    } finally {
      cli.stop();
      await new Promise((resolve) => server.close(resolve));
      cleanupHomedir(hd);
    }
  },
);
