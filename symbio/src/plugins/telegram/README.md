# Telegram 插件

Telegram Bot 集成插件。

## 功能特性

- 消息发送
- 消息接收
- 命令处理
- 多会话管理

## 配置

```yaml
telegram:
  bot_token: "123456:ABC-DEF"
  allowed_chat_ids:
    - 123456789
```

## Actions

| Action | 说明 |
|--------|------|
| `send` | 发送消息 |
| `get_updates` | 获取更新（轮询） |
| `set_chat_id` | 设置当前会话 chat id |
| `start_listener` | 启动消息监听 |
| `stop_listener` | 停止消息监听 |
| `status` | 查询 Bot 状态 |

## 配置

`bot_token` / `chat_id` / `streaming_enabled` / `poll_enabled` / `allowed_users`
是**可寻址的配置文件**：`.vdfs/telegram/PLUGIN.yml`（`ext = form`，字段定义随节点
`schema` 下发）。读写走 `vdfs/read` / `vdfs/write`，落盘就是本插件写自己目录里的
那个文件（`ConfigFile::apply`）——不再有 `config/get` / `config/set` 路由，
也不再经父插件转发。
