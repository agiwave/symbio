import './_selfrun.mjs';
// T69 通道面·路由级判据：`hook` 三路由经 gateway HTTP 边界直发（fire / register / list）。
//
// ## 这一条补的是哪一格
//
// RE-005 的「通道与自动化面」行（`telegram` 6 路由 · `hook` · `home`）此前**整面零用例**。
// 本用例先落 `hook` 格——它是三格里唯一**驱动面已在生产里存在**的一格：
// `hook/fire` / `hook/register` / `hook/list` 都是插件路由，经 gateway 的 HTTP 边界
// （与 Tauri 前端同构的 `invoke`）直发即可，不需要 mock 任何外部服务。
//
// ## 为什么读法取「靶子收到请求」+「路由回包」
//
// 钩子的语义是「触发一个外部观察者，它的回答可以否决主流程」。所以判据不能只看回包
// 形状——回包恒为 `HookOutput`，看不出「钩子到底跑没跑」。真正能红的证据有两条：
//
// 1. **靶子（`MockHttp`）收到了一次 POST**，且正文是**本次事件**的 JSON ⇒ 注册真的生效、
//    钩子真的执行、事件载荷真的透传；
// 2. **未注册事件** fire 时靶子计数**不增**，回包是默认值 ⇒ 上面那条不是恒真。
//
// 第 2 条是必须的：只有第 1 条时，「execute 无条件跑一遍」也能绿。
//
// ## 为什么用 `Http` 钩子而不是 `Command`
//
// `HookType::Command` 在 Windows 上走 `cmd /C <command> <event_path>`，命令串含空格时
// 被 Rust 转义成 `"echo X"`，`cmd` 对 `/C "..."` 的引号解析会把事件路径并进命令名 ⇒
// 实测回包 `Command error: 文件名、目录名或卷标语法不正确。 (os error 123)`。
// 那是**被测系统的真实行为**（钩子作者的命令在 Windows 上确实跑不动），但拿它当
// 主判据会让这条用例变成「平台行为记录」而不是「钩子机制判据」。`Http` 钩子
// 跨平台、且能把「事件 JSON 透传」一并钉住，故主判据取它。
//
// ## 顺带钉住的一条边界事实：`home/*` 不经 gateway
//
// `gateway` 插件挂在 **worker composite 之下**（`gateway/plugin.rs` 模块文档），
// 入站请求的 router 是它的 `parent` = worker composite。而 `home` 是 composite 的
// **父**（`home` 挂 worker），`Composite::route` 对认不出的名字直接 `NotFound`、
// **不向上转发** ⇒ `home/get_homedir` / `home/reload` / `work/*`（这两组是 home 的臂）
// 经 gateway **不可达**（实测 400）。它们的生产驱动面是 Tauri 的 `callPlugin`
// （`tauri/src/services/home.ts`，进程内 `root.route`），归前端形态 e2e。
//
// 这条断言是**事实钉住**而不是「期望如此」：若将来 gateway 的 router 抬到 home
// （`gateway/server.rs` 声称「第三方拿到的能力与前端完全一致」，当前实现少了这四条臂），
// 本断言会变红——那正是它该做的事，届时按新的边界重审。
//
// ## telegram：六条路由里**三条是纯本地**的，本轮先落这三条
//
// `telegram` 六路由中，`set_chat_id`（写内存配置）· `status`（读配置）· `stop_listener`
// （取消令牌 + 置标志）**不碰上游**，可直发驱动；另三条（`send` · `get_updates` ·
// `start_listener`）都打 `api_url()`，而它把 `https://api.telegram.org/bot<token>` **写死**
// （`telegram/plugin.rs:117-122`）⇒ 无靶子可打，转交 T-114（需先加地址配置位）。
// 所以本用例覆盖 6 条中的 3 条，另 3 条有明确归属——不是「只补一条就宣布整面已覆盖」。
import {
  MockHttp,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

/** 已注册的钩子事件（载荷里带一个可断言的正文标记）。 */
const REGISTERED_EVENT = {
  event: 'Notification',
  data: { message: 't69 通知正文', notification_type: 'info' },
};

/** 未注册的事件——用来做「靶子计数不增」的负向自检。 */
const IDLE_EVENT = { event: 'SessionEnd', data: { reason: 't69 done' } };

export default defineCase(
  'T69 通道面路由级：hook 三路由 + telegram 本地三路由经 gateway 直发（home 不经 gateway）',
  async () => {
    const mock = await new MockHttp({ marker: 'T69HOOKMARK', slowMs: 0 }).start();
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
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 't69',
      gatewayPort: GATEWAY_PORT,
    });
    try {
      await cli.waitGatewayReady(20_000, 't69');

      // 靶子的控制面（`/_health` / `/_requests`）也会进留痕表，故判据一律按**路径过滤**——
      // 只看钩子真正该打到的那条 `/echo`，否则「读留痕」这个动作本身会改计数。
      const hookHits = async () => (await mock.requests()).filter((r) => r.path === '/echo');

      // ① 初始：没有任何已注册钩子。
      const before = await cli.invoke('hook/list', {});
      assertEq(before.status, 200, `hook/list 应 200（实得 ${before.status}）`);
      assertEq(
        Object.keys(before.body?.data ?? {}).length,
        0,
        `未注册任何钩子时 hook/list 应为空表（实得 ${JSON.stringify(before.body?.data)}）`,
      );

      // ② 负向基线：未注册事件 fire → 默认输出（「不否决主流程」的旁路观察者语义），
      //    且靶子**一次都没被打到**。
      const idle = await cli.invoke('hook/fire', { session_id: 't69', event: IDLE_EVENT });
      assertEq(idle.status, 200, `hook/fire 应 200（实得 ${idle.status}）`);
      // 逐字段比（`assertEq` 对对象走 JSON.stringify，键序敏感，不可直接比整包）
      const idleOut = idle.body?.data ?? {};
      assertEq(idleOut.should_proceed, true, '未注册事件应返回默认 HookOutput（不否决主流程）');
      assertEq(idleOut.context, null, '未注册事件不应带 context');
      assertEq(idleOut.block_reason, null, '未注册事件不应带 block_reason');
      assertEq((await hookHits()).length, 0, '未注册事件不应触达任何钩子靶子');

      // ③ 注册一个 Http 钩子，指向本地靶子。
      const reg = await cli.invoke('hook/register', {
        event: 'Notification',
        hooks: [{ name: 't69-http', hook_type: 'Http', url: mock.url('/echo') }],
      });
      assertEq(reg.status, 200, `hook/register 应 200（实得 ${reg.status}）`);

      // ④ 注册可见：list 里出现该事件与它的 url（配置的可见变化）。
      const after = await cli.invoke('hook/list', {});
      const listed = after.body?.data?.Notification ?? [];
      assertEq(listed.length, 1, `注册后 hook/list 的 Notification 应恰有 1 条（实得 ${listed.length}）`);
      assertEq(listed[0].url, mock.url('/echo'), '列出的钩子 url 应是注册时给的那个');

      // ⑤ 正向：fire 已注册事件 → 靶子收到一次 POST，正文是本次事件 JSON；
      //    回包 context 带上靶子的回答（钩子输出经路由透传给调用方）。
      const fired = await cli.invoke('hook/fire', { session_id: 't69', event: REGISTERED_EVENT });
      assertEq(fired.status, 200, `hook/fire 应 200（实得 ${fired.status}）`);
      const out = fired.body?.data ?? {};
      assertEq(out.should_proceed, true, '钩子允许时 should_proceed 应为 true');
      assert(
        typeof out.context === 'string' && out.context.includes('T69HOOKMARK'),
        `回包 context 应带上靶子的回答（实得 ${JSON.stringify(out.context)?.slice(0, 200)}）`,
      );

      const reqs = await hookHits();
      assertEq(reqs.length, 1, `注册的钩子应恰被调用一次（实得 ${reqs.length} 次）`);
      assertEq(reqs[0].method, 'POST', '钩子应以 POST 投递事件');
      assert(
        reqs[0].body.includes('t69 通知正文'),
        `靶子收到的正文应是本次事件载荷（实得 ${reqs[0].body.slice(0, 200)}）`,
      );
      assert(reqs[0].body.includes('Notification'), '事件载荷里应带事件名');

      // ⑥ telegram 三条**纯本地**路由（不碰上游，故无需地址配置位即可驱动）。
      const tgBefore = await cli.invoke('telegram/status', {});
      assertEq(tgBefore.status, 200, `telegram/status 应 200（实得 ${tgBefore.status}）`);
      assertEq(tgBefore.body?.data?.has_chat_id, false, '初始未绑定 chat id');
      assertEq(tgBefore.body?.data?.listener_running, false, '初始监听未运行');

      const setChat = await cli.invoke('telegram/set_chat_id', { chat_id: 't69-chat-1' });
      assertEq(setChat.status, 200, `telegram/set_chat_id 应 200（实得 ${setChat.status}）`);

      // 配置的**可见变化**：写完再读，status 必须反映出来（否则「写了没生效」）。
      const tgAfter = await cli.invoke('telegram/status', {});
      assertEq(tgAfter.body?.data?.has_chat_id, true, 'set_chat_id 后 status 应反映已绑定');

      const stop = await cli.invoke('telegram/stop_listener', {});
      assertEq(stop.status, 200, `telegram/stop_listener 应 200（实得 ${stop.status}）`);
      assertEq(stop.body?.data?.success, true, '未运行时 stop_listener 也应成功返回');

      // ⑦ 边界事实：home 的臂经 gateway 不可达（router 是 worker composite，不向上转发）。
      const home = await cli.invoke('home/get_homedir', {});
      assertEq(home.status, 400, `home/get_homedir 经 gateway 应不可达（实得 ${home.status}）`);

      // ⑧ 再负向：又一次未注册事件 → 靶子计数不增（证明 ⑤ 的「收到」是注册带来的）。
      const seen = (await hookHits()).length;
      await cli.invoke('hook/fire', { session_id: 't69', event: IDLE_EVENT });
      assertEq((await hookHits()).length, seen, '未注册事件仍不应新增靶子请求');
    } finally {
      cli.stop();
      mock.stop();
      cleanupHomedir(hd);
    }
  },
);
