---
kind: task
id: T-114-telegram上游端点可配批
title: telegram 上游端点可配批：`api_url()` 把 `api.telegram.org` 写死 ⇒ `send` / `get_updates` / `start_listener` 三条路由无靶子可打
status: todo
created: 2026-10-15
track: e2e
deps: []
prio: later
exit: `TelegramConfig` 有地址位（`api_base`，缺省即官方 `https://api.telegram.org`）；`api_url()` 读它而不是拼字面量；配置面三处同步（[RE-001](../ref/RE-001-CONFIGURATION.md "RE-001-CONFIGURATION") 登记 + `scripts/facts/_sets.mjs` 的 `plugin-configs` 封闭集 + 出厂 `.symbio/telegram/PLUGIN.yml`）；三条依赖上游的路由（`telegram/send` · `telegram/get_updates` · `telegram/start_listener`）各自有一条经 gateway 直发的判据——上游请求打到**本地靶子**且回包形状正确（[RE-005](../ref/RE-005-e2e覆盖矩阵.md "RE-005-e2e覆盖矩阵") 的「通道与自动化面」行 `telegram` 格随之补齐六条）；若裁决「不做」，须写明「telegram 只对官方端点说话」的理由与**撤销条件**，并把 RE-005 那一格从「缺口」改成「明确不做」
---

## 内容

本任务由 [T-110](T-110-通道与自动化面e2e补位.md "T-110-通道与自动化面e2e补位") 的第一批**实测**开出：
核实 `telegram` 六条路由的驱动面时发现它们**不是一体的**——六条里有一半根本不碰上游。

### 六条路由的两半

`telegram` 的六条路由逐条登记在 [RE-006-当前事实表.md](../ref/RE-006-当前事实表.md "RE-006-当前事实表") §2.3（地址清单的 owner 在那里，此处不复制）。按**是否打上游**，它们分成两半：

- **不碰上游的三条**——`handle_status`（`plugin.rs:389`，只读内存配置）·
  `invoke_set_chat_id`（630，只写内存配置）· `handle_stop_listener`（374，取消令牌 + 置标志）
  ⇒ 无上游依赖，经 gateway 直发可驱动 ⇒ **已由 t69 入格**（T-110 闭批）。
- **打上游的三条**——`send_message`（151，`POST {api_url}/sendMessage`）·
  `handle_get_updates`（214，`POST {api_url}/getUpdates`）·
  `handle_start_listener`（283，后台轮询每秒打一次 `getUpdates`）
  ⇒ 都先取 `self.api_url()` ⇒ **本任务**。

### 卡在哪一行

`api_url()`（`plugin.rs:117-122`）是唯一构造上游地址的地方：

```rust
Some(format!("https://api.telegram.org/bot{}", config.bot_token))
```

`TelegramConfig`（`telegram/schemas.rs`）的五个字段是
`bot_token` / `chat_id` / `streaming_enabled` / `poll_enabled` / `allowed_users`——
**没有地址位**。于是这三条路由**无法被指向本地靶子**：e2e 只能打真外网，而 CI 无网。

这与 [T-109](T-109-web面读数与配置残余批.md "T-109-web面读数与配置残余批") ⑤（`web_search` 的三个上游地址写死）
是**同款病**：上游地址是**供应商契约的一部分**，但把它写成不可改的字面量之后，
这一族能力就**永久停在零覆盖**——而且不只是 e2e 的问题：真实环境里「走自建 / 代理端点
访问 Telegram Bot API」是一个常见需求（网络受限环境），当前没有任何配置位能满足它。

### 落地要做什么

1. **配置位**：`TelegramConfig` 加 `api_base: String`，`Default` 给官方地址；
   `api_url()` 改成 `format!("{}/bot{}", config.api_base.trim_end_matches('/'), config.bot_token)`。
   缺省值取官方地址 ⇒ **对现有部署零行为变化**（这是加这一位的前提：不能因为
   「让 e2e 能测」而改变默认行为）。
2. **配置面三处同步**（缺一处会被门禁判红）：
   - `docs/ref/RE-001-CONFIGURATION.md` 的 `telegram` 插件键表；
   - `scripts/facts/_sets.mjs` 的 `plugin-configs` 封闭集；
   - 出厂 `.symbio/telegram/PLUGIN.yml`。
3. **靶子**：需要一个按 Telegram Bot API 形状回包的 mock——
   `sendMessage` → `{ok:true,result:{message_id:…}}`；`getUpdates` → `{ok:true,result:[…]}`。
   `e2e/mock-http.mjs` 现有端点（`/page` `/json` `/echo` …）都是 web 外取的形状，
   与 Bot API 不同 ⇒ 可给它加两个 telegram 形状的端点，或另起 `e2e/mock-telegram.mjs`
   （与 `mock-mcp.mjs` 同款：一个协议一个夹具）。
4. **判据**（三条路由各一）：`send` ⇒ 靶子收到 `POST /bot<token>/sendMessage` 且正文含
   `chat_id` / `text`，回包 `message_id` 正确；`get_updates` ⇒ 靶子收到 `POST …/getUpdates`
   且回包的 `update_offset` 被推进；`start_listener` ⇒ 靶子**持续**收到 `getUpdates`
   （轮询真的起来了）+ `status` 的 `listener_running` 翻 `true` + `stop_listener` 后
   轮询停止（`listener_running` 翻回 `false`）。**注意**：`start_listener` 的判据要
   把「停」也钉住，否则一条永不停止的后台任务会挂在用例收尾上（与 `t36` 心跳同款纪律）。

### 撤销条件

若裁决**不做**（即认定「telegram 只对官方端点说话」），须：
- 在 RE-005 的「通道与自动化面」行把 `telegram` 那一格写成**「明确不做」**而不是留 `缺口`；
- 写明撤销条件：**一旦出现「telegram 需指向自建 / 代理端点」的真实需求**
  （网络受限环境是主要触发场景），本任务重开——因为那时 `api_base` 不再是「为了测试」
  而是**产品能力**。

## 处理过程

- 2026-10-15 登记：T-110 第一批实测 `telegram` 六路由的驱动面时，
  逐条核对 `api_url()` 的引用点，确认「三条本地 / 三条上游」的两半结构；
  本地三条已由 `e2e/cases/t69-channel-routes.mjs` 驱动（T-110 闭批），
  上游三条因地址写死转交本任务。
