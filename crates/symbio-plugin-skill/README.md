---
kind: design
id: crates/symbio-plugin-skill/README
title: Skill 插件
status: active
created: 2026-09-09
impl: done
---

# Skill 插件

技能插件：加载与执行技能定义，经 `traverse` 向会话贡献技能工具。

## 路由

只有一条自有路由 `skill/execute`（载荷 `{name, args}`，按名称执行技能）。
技能清单与内容**不设私有路由**——已安装技能经 `<根>/skill` 寻址（一个技能 = 一个目录，主文件 `SKILL.md`）。
见 `docs/004-ref/003-ROUTES.md` §Skill 插件（**权威**）。

## 机制

- 技能定义以文件形式存放在主目录 skills 目录下，启动时加载。
- 经 `traverse` 注册为 Capability，session 收集后进入模型工具定义，执行结果回传会话。

## 专题设计

- 技能编译与自我改进（编译写方、校准账、退避阈值、开关与翻默认值前置条件、测试落点）：
  `src/docs/115-技能编译与自我改进.md`（DE-115）。系统级机制与跨插件判据见
  `docs/003-design/041-技能编译与自我改进.md`（DE-041）。

## 关联

- 工具收集管线由宿主提供；本插件只经 `traverse` 交出能力行
