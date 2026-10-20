import './_selfrun.mjs';
// T70 telegram 上游路由判据：`send` / `get_updates` / `start_listener` 经 `api_base` 打本地靶子。
//
// ## 这一条补的是哪一格
//
// RE-005「通道与自动化面」行的 `telegram` 格：t69 已入三条**纯本地**路由
// （`status` / `set_chat_id` / `stop_listener`），另三条都打 `api_url()`，而它曾把
// `https://api.telegram.org/bot<token>` **写死** ⇒ 无靶子可打（转交 T-114）。
// T-114 给 `TelegramConfig` 加了 `api_base`（缺省即官方端点，对现有部署零行为变化），
// 本用例把它指向 `MockHttp`，三条上游路由各钉一条判据。
//
// ## 判据为什么取「靶子收到什么」+「回包被消化」
//
// 三条路由的回包都薄（`sent` 计数 / `updates` 数组 / `success`），只断回包的话
// 「根本没出网」也照样绿。真正能红的证据两条腿：
//
// 1. **靶子留痕**：请求真的发到了 `{api_base}/bot<token>/<op>`（路径含 token、
//    正文含载荷）⇒ `api_base` 真的被读走，不是摆设；
// 2. **回包被消化**：`getUpdates` 回包的 `update_id` 被推进成 `status` 里的
//    `update_offset` ⇒ 上游数据真的进了被测系统，不是 mock 自嗨。
//
// ## `start_listener` 的「停」必须钉住（t36 心跳同款纪律）
//
// 监听器是**永不自行停止**的后台任务：只断「轮询起来了」会让一条 1s 一次的
// getUpdates 挂在用例收尾上（进程泄漏）。所以判据把「停」一并钉住：
// `stop_listener` 后 `listener_running` 翻回 `false`、靶子计数在 2.5s 窗口内不再增长。
//
// ## ④ 为什么要真的重启一次进程
//
// 游标与 `chat_id` 曾经纯内存：重启 = 从头再拉一遍历史更新 / 绑定丢失。它们现在
// 落在 `PLUGIN.yml`（T-194），而「落盘了」只有**从同一目录重建插件树**才证明得了——
// 单进程内读到的是同一个 `RwLock`，那份读数与磁盘无关。故末尾停掉 CLI 再起一个。
import {
  MockHttp,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

const TOKEN = 'T114TESTTOKEN';
const CHAT_ID = 't114-chat';
const TEXT = 't114 上游正文';

export default defineCase(
  'T70 telegram 上游路由：send / get_updates / start_listener 经 api_base 打本地靶子',
  async () => {
    const mock = await new MockHttp({ marker: 'T114TG', slowMs: 0 }).start();
    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        telegram: { bot_token: TOKEN, api_base: mock.base },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 't70',
      gatewayPort: GATEWAY_PORT,
    });
    let cli2 = null;
    try {
      await cli.waitGatewayReady(20_000, 't70');

      // 靶子留痕按 telegram 形状过滤（`/bot<token>/<op>`）；控制面 `/_health` 不进判据。
      const tgHits = async () => (await mock.requests()).filter((r) => r.path.startsWith('/bot'));
      const opHits = async (op) => (await tgHits()).filter((r) => r.path.endsWith(`/${op}`));

      // ── ① `send`：靶子收到 `POST /bot<token>/sendMessage`，正文含 chat_id 与 text ──
      const sent = await cli.invoke('telegram/send', { text: TEXT, chat_id: CHAT_ID });
      assertEq(sent.status, 200, `telegram/send 应 200（实得 ${sent.status}）`);
      assertEq(sent.body?.data?.sent, 1, '一条短正文 ⇒ 恰发送 1 条');

      const sendReqs = await opHits('sendMessage');
      assertEq(sendReqs.length, 1, `靶子应恰收到 1 次 sendMessage（实得 ${(await tgHits()).length} 次总计）`);
      assertEq(sendReqs[0].path, `/bot${TOKEN}/sendMessage`, '路径应是 `{api_base}/bot<token>/sendMessage`');
      assert(sendReqs[0].body.includes(CHAT_ID), `正文应含 chat_id（实得 ${sendReqs[0].body.slice(0, 200)}）`);
      assert(sendReqs[0].body.includes(TEXT), `正文应含 text（实得 ${sendReqs[0].body.slice(0, 200)}）`);

      // ── ② `get_updates`：靶子收到 getUpdates，回包 update_id 被推进成 update_offset ──
      const upd = await cli.invoke('telegram/get_updates', {});
      assertEq(upd.status, 200, `telegram/get_updates 应 200（实得 ${upd.status}）`);
      assertEq(upd.body?.data?.success, true, '回包应 success');
      assertEq(
        upd.body?.data?.updates?.[0]?.update_id,
        100100,
        '回包应透传靶子的 updates 数组',
      );

      const getReqs = await opHits('getUpdates');
      assertEq(getReqs.length, 1, '靶子应恰收到 1 次 getUpdates');
      assertEq(getReqs[0].path, `/bot${TOKEN}/getUpdates`, '路径应是 `{api_base}/bot<token>/getUpdates`');
      assert(getReqs[0].body.includes('"offset":0'), `首次请求应带 offset=0（实得 ${getReqs[0].body}）`);

      // 回包被消化：status 的 update_offset = update_id + 1。
      const st1 = await cli.invoke('telegram/status', {});
      assertEq(st1.body?.data?.update_offset, 100101, 'update_id=100100 ⇒ offset 应推进到 100101');

      // ── ③ `start_listener`：轮询真的起来（靶子持续收到 getUpdates）+ 可停 ──
      const start = await cli.invoke('telegram/start_listener', {});
      assertEq(start.status, 200, `telegram/start_listener 应 200（实得 ${start.status}）`);

      // 轮询周期 1s：等「第二次」getUpdates 落地 ⇒ 不是只打了一发就停。
      await waitFor(
        async () => (await opHits('getUpdates')).length >= 3,
        { what: 'start_listener 后靶子持续收到 getUpdates（≥3 次）', timeoutMs: 15_000 },
      );
      const stRun = await cli.invoke('telegram/status', {});
      assertEq(stRun.body?.data?.listener_running, true, 'listener_running 应翻 true');

      const stop = await cli.invoke('telegram/stop_listener', {});
      assertEq(stop.status, 200, `telegram/stop_listener 应 200（实得 ${stop.status}）`);
      const stStop = await cli.invoke('telegram/status', {});
      assertEq(stStop.body?.data?.listener_running, false, 'stop 后 listener_running 应翻回 false');

      // 「停」要真停：2.5s（> 轮询周期 2 倍）窗口内靶子计数不再增长。
      const stopped = (await tgHits()).length;
      await new Promise((r) => setTimeout(r, 2500));
      assertEq(
        (await tgHits()).length,
        stopped,
        'stop_listener 后靶子不应再收到任何请求（后台轮询必须真的停了）',
      );

      // ── ④ 重启后游标与绑定仍在：两者都落在 `PLUGIN.yml`，不是进程内存 ──
      const bound = await cli.invoke('telegram/set_chat_id', { chat_id: CHAT_ID });
      assertEq(bound.status, 200, `set_chat_id 应 200（实得 ${bound.status}）`);

      cli.stop();
      cli2 = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 't70',
        gatewayPort: GATEWAY_PORT,
      });
      await cli2.waitGatewayReady(20_000, 't70-restart');
      const stRestart = await cli2.invoke('telegram/status', {});
      assertEq(
        stRestart.body?.data?.update_offset,
        100101,
        '重启后游标应从 PLUGIN.yml 恢复（否则会重复处理历史更新）',
      );
      assertEq(
        stRestart.body?.data?.has_chat_id,
        true,
        '重启后自动/显式绑定的 chat_id 应仍在',
      );
    } finally {
      cli.stop();
      cli2?.stop();
      mock.stop();
      cleanupHomedir(hd);
    }
  },
);
