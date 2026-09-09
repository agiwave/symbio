# Skill 插件

技能插件：加载与执行技能定义，经 `traverse` 向会话贡献技能工具。

## 路由

| Path | 说明 |
|------|------|
| `skill/list` | 列出已加载技能 |
| `skill/search` | 技能检索 |
| `skill/run` | 执行指定技能 |

## 机制

- 技能定义以文件形式存放在主目录 skills 目录下，启动时加载。
- 经 `traverse` 注册为 Capability，session 收集后进入模型工具定义，执行结果回传会话。

## 关联

- 工具收集管线：`symbio_core/chat_pipeline.rs`
