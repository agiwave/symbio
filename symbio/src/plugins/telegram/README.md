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
| `config/get` / `config/set` | 读写 `bot_token` / `allowed_chat_ids` 等配置 |
