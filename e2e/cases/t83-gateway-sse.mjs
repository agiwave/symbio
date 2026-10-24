import './_selfrun.mjs';
// T83 SSE 实时面：`GET /api/v1/events` 是 `event_bus/subscribe` 的只读包装。
//
// ## 为什么必须走真实 HTTP 边界
//
// SSE 是**传输层语义**：状态行、`text/event-stream`、事件分隔（空行）、长连接上的
// 逐帧下发，单测只能覆盖「一条信封怎么编码」，覆盖不到「路由拿到 Session 后真的
// 一帧一帧写出去」。这里用真实 `fetch` 读流，判据全部取**另一侧观测到的事实**：
//
// 1. 首条事件是 `event_bus` 的 `connected` 信封（订阅真的建立了）；
// 2. 订阅期间 `event_bus/ping` 的订阅者数 +1（端点的清理责任因此有对照基线）；
// 3. 登记 `vdfs/watch` 后创建会话，SSE 流上收到 `kind = "vdfs"` 且 `path` 落在
//    该会话子树内（信封原样过，可以当会话实时面用）；
// 4. 客户端断开后订阅者数回到基线（**不留下幽灵订阅**——这一条是本轮顺带修的缺口，
//    没有它，调试连接每断一次就漏一个订阅者）。
//
// ## 边界：SSE 不登记 watch、不过滤 kind
//
// 与 WS 完全同构：总线只负责广播，**收不收得到会话流取决于 `vdfs/watch`**。
// 因此本用例自己登记 watch——端点不替调用方做这件事，也不按 `kind` 过滤。
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

/** `event_bus/ping` 的回包形如 `pong (N subscribers)`。 */
function subscriberCount(resp) {
  const message = resp?.body?.data?.message ?? '';
  const m = /pong \((\d+) subscribers\)/.exec(message);
  assert(m, `event_bus/ping 应回订阅者数（实得 ${JSON.stringify(resp?.body)?.slice(0, 200)}）`);
  return Number(m[1]);
}

export default defineCase(
  'T83 SSE 实时面：总线信封经 text/event-stream 原样下发、断开后订阅被摘除',
  async () => {
    const llm = await new MockLlm([{ id: 'unused', match: '（本用例不发言）', content: '—' }]).start();
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
      session: 't83',
      gatewayPort: GATEWAY_PORT,
    });
    try {
      await cli.waitGatewayReady(20_000, 't83');

      const rootResp = await cli.invoke('vdfs/root', {});
      const root = rootResp.body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const mount = `${root.replace(/\/+$/, '')}/session`;

      // 基线订阅数：CLI 自身可能已有一条（不假设是 0）。
      const baseline = subscriberCount(await cli.invoke('event_bus/ping', {}));

      // ── ① 打开 SSE，按事件块解析 ──
      const controller = new AbortController();
      const resp = await fetch(`http://127.0.0.1:${GATEWAY_PORT}/api/v1/events`, {
        signal: controller.signal,
      });
      assertEq(resp.status, 200, `SSE 应回 200（实得 ${resp.status}）`);
      assert(
        (resp.headers.get('content-type') ?? '').startsWith('text/event-stream'),
        `SSE 的 Content-Type 应为 text/event-stream（实得 ${resp.headers.get('content-type')}）`,
      );

      const events = [];
      const comments = [];
      let stopped = false;
      const parser = (async () => {
        const reader = resp.body.getReader();
        const decoder = new TextDecoder();
        let raw = '';
        try {
          for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
            raw += decoder.decode(value, { stream: true });
            let sep;
            while ((sep = raw.indexOf('\n\n')) >= 0) {
              const block = raw.slice(0, sep);
              raw = raw.slice(sep + 2);
              if (block.startsWith(':')) {
                comments.push(block);
                continue;
              }
              const data = block
                .split('\n')
                .filter((l) => l.startsWith('data: '))
                .map((l) => l.slice('data: '.length))
                .join('\n');
              if (data) events.push(JSON.parse(data));
            }
          }
        } catch (e) {
          // 主动 abort 会让 read() 抛 AbortError——那是本用例的收尾动作，不是失败。
          if (!stopped) throw e;
        }
      })();

      const waitForEvent = async (pred, what, ms = 10_000) => {
        const deadline = Date.now() + ms;
        for (;;) {
          const hit = events.find(pred);
          if (hit) return hit;
          if (Date.now() > deadline) {
            throw new Error(
              `等待「${what}」超时；已收 ${events.length} 条事件：${JSON.stringify(events).slice(0, 800)}`,
            );
          }
          await new Promise((r) => setTimeout(r, 50));
        }
      };

      // ── ② 首条事件：握手信封 ──
      // SSE 的 `data:` 承载的是总线信封本身：`{type:"bus_event", data:{kind,…}}`
      // （WS 客户端看到的是同一份东西，只是外面多包了一层 `{"Data": …}`）。
      const connected = await waitForEvent(
        (e) =>
          e?.type === 'bus_event' &&
          e?.data?.kind === 'system' &&
          e?.data?.data?.event === 'connected',
        'connected 信封',
      );
      assert(
        typeof connected.data.data.connection_id === 'string' &&
          connected.data.data.connection_id.length > 0,
        'connected 信封应带 connection_id',
      );
      assertEq(
        subscriberCount(await cli.invoke('event_bus/ping', {})),
        baseline + 1,
        'SSE 订阅期间订阅者数应比基线多一',
      );

      // ── ③ 登记 watch 后创建会话：SSE 上应收到该子树的 vdfs 变更 ──
      const watched = await cli.invoke('vdfs/watch', { path: mount });
      assertEq(watched.status, 200, `vdfs/watch 应成功（${watched.status}）`);

      const created = await cli.invoke('vdfs/write', {
        path: mount,
        create: true,
        text: JSON.stringify({ metadata: { risk_level: 'low' } }),
      });
      assertEq(created.status, 200, `创建会话的 vdfs/write 应成功（${created.status}）`);
      const sid = created.body?.data?.name;
      assert(typeof sid === 'string' && sid.length > 0, '创建会话应回执会话名');

      const change = await waitForEvent(
        (e) =>
          e?.type === 'bus_event' &&
          e?.data?.kind === 'vdfs' &&
          typeof e?.data?.data?.path === 'string' &&
          e.data.data.path.startsWith(`${mount}/${sid}`),
        `会话 ${sid} 的 vdfs 变更`,
      );
      // 无载荷变更（`path` only）是合法形状：它表示「这个地址变了，回读」。
      // 因此这里只钉住信封与归属地址，不假设每条变更都带节点快照。
      assert(
        change.data.data.path.startsWith(`${mount}/${sid}`),
        `变更必须落在该会话子树内（实得 ${change.data.data.path}）`,
      );

      // ── ④ 断开：订阅者回落到基线（不留幽灵订阅） ──
      stopped = true;
      controller.abort();
      await Promise.allSettled([parser]);

      const deadline = Date.now() + 8_000;
      let after = subscriberCount(await cli.invoke('event_bus/ping', {}));
      while (after !== baseline && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 100));
        after = subscriberCount(await cli.invoke('event_bus/ping', {}));
      }
      assertEq(after, baseline, 'SSE 断开后订阅者应回到基线（清理任务没被触发 = 幽灵订阅）');

      // 心跳注释行不是事件：它不得被当成 data 下发（空闲期只在测试里可能观察不到，
      // 这里只断言已收事件全部可解析——解析失败会在 parser 里抛出）。
      assert(events.length >= 2, `应至少收到 connected 与会话变更两条事件（实得 ${events.length}）`);
    } finally {
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
