# Skill 插件

技能插件：加载与执行技能定义，经 `traverse` 向会话贡献技能工具。

## 路由

只有一条自有路由 `skill/execute`（载荷 `{name, args}`，按名称执行技能）。
技能清单与内容**不设私有路由**——已安装技能经 `.vdfs/skill` 寻址（一个技能 = 一个目录，主文件 `SKILL.md`）。
见 `docs/reference/ROUTES.md` §Skill 插件（**权威**）。

## 机制

- 技能定义以文件形式存放在主目录 skills 目录下，启动时加载。
- 经 `traverse` 注册为 Capability，session 收集后进入模型工具定义，执行结果回传会话。

## 关联

- 工具收集管线：`plugins/session/chat_pipeline.rs`
