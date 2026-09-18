# Telegram 插件

Telegram Bot 集成插件：长轮询收发与"继续会话"交互。

## 功能特性

- 消息发送与接收
- 命令处理
- 多会话管理

## Actions

| Action | 说明 |
|--------|------|
| `send` | 发送消息 |
| `get_updates` | 获取更新（长轮询） |
| `set_chat_id` | 设置当前会话 chat id |
| `start_listener` | 启动消息监听 |
| `stop_listener` | 停止消息监听 |
| `status` | 查询 Bot 状态 |

> 清单以 `docs/reference/ROUTES.md` §Telegram 插件为准（**权威**）。

## 怎么被调用

本插件是**对外通道**：`start_listener` 等路由仓内没有调用方，但网关会把外部请求
里的 `path` 原样转发给容器 `route`（`gateway/server.rs`），所以它们是**对外开放
接口**，不是死代码。（审计报告里的 `refs=0` 只统计仓内调用方。）

监听器收到消息后要调 LLM，走的是 **`ctx.parent()` → `parent.route(ctx)`**，
地址 `session/chat/send`（绝对地址由容器分发）。**不要**按值持有 session 实例——
`docs/design/plugin-route-address.md` 规则五，守卫 E-007 会拦。

## 配置

`bot_token` / `chat_id` / `streaming_enabled` / `poll_enabled` / `allowed_users`
是**可寻址的配置文件**：`.vdfs/telegram/PLUGIN.yml`（`ext = form`，字段定义随节点
`schema` 下发）。读写走 `vdfs/read` / `vdfs/write`，落盘就是本插件写自己目录里的
那个文件（`ConfigFile::apply`）——不再有 `config/get` / `config/set` 路由，
也不再经父插件转发。

```yaml
# .vdfs/telegram/PLUGIN.yml（顶层扁平键）
bot_token: "123456:ABC-DEF"
chat_id: "123456789"        # 可选；未指定时按会话推送
streaming_enabled: true
poll_enabled: true
allowed_users: [123456789]
```
