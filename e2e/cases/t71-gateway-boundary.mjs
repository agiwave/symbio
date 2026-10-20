import './_selfrun.mjs';
// T71 入站边界：令牌不进 URL · `Idempotency-Key` · 连接上限（T-194）。
//
// ## 这三条为什么必须走真实边界
//
// 三条都是**传输层语义**，单测只能覆盖谓词（`ws_token` 怎么取、幂等表怎么记），
// 覆盖不到接线：令牌改走子协议后，握手响应里回选的是哪个协议、查询串里的旧令牌
// 是否真的不再被接受、409 是在分派**之前**发出的——这些都只在一条真实连接上成立。
//
// 判据都取「另一侧观测到的事实」：
//
// 1. **WS 令牌**：带 `?token=` 的裸握手必须回 401；只带子协议
//    `bearer.<token>` 的 `ws` 客户端必须连上，且回选协议是 `symbio.v1`
//    （回显 `bearer.*` 等于把令牌写进响应头——见 `server.rs::ws_select_subprotocol`）；
// 2. **幂等**：同键第二次 409、换键 200；而**失败没有副作用**时同键重试必须仍是
//    400 而不是 409（键在失败分支被释放）——这条负向自检证明 409 不是「一律拒绝」；
// 3. **连接上限**：`inbound_max_connections: 1` 时，占住唯一配额的那条连接
//    （只发了半个请求头）必须让第二个请求**排队**，释放后它才被受理。
//    判据用「800ms 内无响应 + 释放后 200」，而不是问插件「你到顶了吗」——后者
//    在限额根本没接上线时也会说「是」。
import net from 'node:net';
import WebSocket from '../../tauri/node_modules/ws/index.js';
import {
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

const TOKEN = 'T194TOKEN';
const AUTH = { authorization: `Bearer ${TOKEN}` };

/** 读响应头（读到第一行即可判状态；不消费 body）。 */
function readHead(socket) {
  return new Promise((resolve, reject) => {
    let buf = '';
    const onData = (d) => {
      buf += d.toString('latin1');
      if (buf.includes('\r\n')) {
        socket.off('data', onData);
        resolve(buf);
      }
    };
    socket.on('data', onData);
    socket.once('error', reject);
  });
}

function delay(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

export default defineCase(
  'T71 入站边界：WS 令牌走子协议、幂等键拒绝重放、连接上限背压',
  async () => {
    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: TOKEN,
          inbound_readonly: false,
          inbound_max_connections: 1,
        },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 't71',
      gatewayPort: GATEWAY_PORT,
      headers: AUTH,
    });
    try {
      await cli.waitGatewayReady(20_000, 't71');

      // ── ① 旧通道已关闭：WS 升级只带 `?token=` ⇒ 401 ──
      const raw = net.connect(GATEWAY_PORT, '127.0.0.1');
      await new Promise((res, rej) => {
        raw.once('connect', res);
        raw.once('error', rej);
      });
      raw.write(
        `GET /api/v1/ws?token=${TOKEN} HTTP/1.1\r\n` +
          'Host: 127.0.0.1\r\n' +
          'Upgrade: websocket\r\n' +
          'Connection: Upgrade\r\n' +
          'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n' +
          'Sec-WebSocket-Version: 13\r\n\r\n',
      );
      const rejected = await readHead(raw);
      assert(
        rejected.startsWith('HTTP/1.1 401'),
        `查询串里的令牌不得再被接受（实得 ${rejected.split('\r\n')[0]}）`,
      );
      raw.destroy();

      // ── ② 新通道可用：子协议携带令牌，握手回选 `symbio.v1` ──
      const ws = new WebSocket(`ws://127.0.0.1:${GATEWAY_PORT}/api/v1/ws`, [
        'symbio.v1',
        `bearer.${TOKEN}`,
      ]);
      const wsOpened = new Promise((res, rej) => {
        ws.once('open', res);
        ws.once('error', rej);
      });
      await wsOpened;
      assertEq(ws.protocol, 'symbio.v1', '握手应回选 symbio.v1，而不是 bearer.<token>');

      const framePromise = new Promise((res, rej) => {
        const timer = setTimeout(() => rej(new Error('WS 首帧超时')), 5_000);
        ws.once('message', (data) => {
          clearTimeout(timer);
          res(JSON.parse(data.toString()));
        });
      });
      ws.send(JSON.stringify({ metadata: { path: 'telegram/status' }, payload: {} }));
      const frame = await framePromise;
      // `PluginFrame::Data(Arc<Value>)` 的线上形态是 `{"Data": <路由回包>}`
      assert(frame?.Data && typeof frame.Data === 'object', `首帧应是 Data 帧（实得 ${JSON.stringify(frame)?.slice(0, 200)}）`);
      assertEq(frame.Data.listener_running, false, '未启动监听时 listener_running 应为 false');
      await new Promise((res) => {
        ws.once('close', res);
        ws.close();
      });

      // ── ③ 幂等键：同键重复 ⇒ 409；换键 ⇒ 正常；失败释放键 ⇒ 重试仍是 400 而不是 409 ──
      const keyed = (key, path = 'telegram/status') =>
        cli.invoke(path, {}, {}, { 'idempotency-key': key });

      const first = await keyed('t71-once');
      assertEq(first.status, 200, `首次带键调用应 200（实得 ${first.status}）`);
      const dup = await keyed('t71-once');
      assertEq(dup.status, 409, `同键未过期时重复调用应 409（实得 ${dup.status}）`);
      const other = await keyed('t71-other');
      assertEq(other.status, 200, `不同键互不影响（实得 ${other.status}）`);

      // 分派失败没有副作用 ⇒ 键被释放，同键重试应再次执行（400），而不是被判为重复（409）。
      const fail1 = await keyed('t71-fail', 'no/such/route');
      assertEq(fail1.status, 400, `未知路由应 400（实得 ${fail1.status}）`);
      const fail2 = await keyed('t71-fail', 'no/such/route');
      assertEq(fail2.status, 400, `失败后同键应可重试（实得 ${fail2.status}）`);

      const badKey = await keyed('x'.repeat(201));
      assertEq(badKey.status, 400, `超长幂等键应 400（实得 ${badKey.status}）`);

      // ── ④ 连接上限：配额占满时第二个请求排队，释放后受理 ──
      const blocker = net.connect(GATEWAY_PORT, '127.0.0.1');
      await new Promise((res, rej) => {
        blocker.once('connect', res);
        blocker.once('error', rej);
      });
      // 只发请求头的一部分：`read_request` 停在等头，连接与配额都活着。
      blocker.write('GET /api/v1/health HTTP/1.1\r\nHost: blocked\r\n');
      await delay(150);

      const health = fetch(`http://127.0.0.1:${GATEWAY_PORT}/api/v1/health`).then((r) => r.status);
      const early = await Promise.race([health, delay(800).then(() => 'queued')]);
      assertEq(early, 'queued', '连接上限已满时，第二个请求不得在配额释放前被受理');

      blocker.destroy();
      const status = await Promise.race([
        health,
        delay(5_000).then(() => {
          throw new Error('占位连接释放后，排队的请求仍未受理（配额没有归还）');
        }),
      ]);
      assertEq(status, 200, `配额释放后排队请求应被受理（实得 ${status}）`);
    } finally {
      cli.stop();
      cleanupHomedir(hd);
    }
  },
);
