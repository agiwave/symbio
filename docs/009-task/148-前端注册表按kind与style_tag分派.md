---
kind: task
id: T-148-前端注册表按kind与style_tag分派
title: 呈现分派从 MessageType 枚举换成 (kind, style_tag, principal) 三元组
status: todo
created: 2026-10-16
track: frontend
deps: [T-147-ChatNode协议改名与字段瘦身]
prio: now
parent: T-145-前端机制化批
where: tauri/src/registry/messageTypes.ts · tauri/src/registry/messageRenderers.ts · tauri/src/components/MessageNode.vue · tauri/src/components/NodeShell.vue · tauri/src/components/MessageChildren.vue
exit: 渲染器注册表按 `(kind, style_tag)` 二元组分派，`kind` 五值全覆盖（Body/Notice/Skeleton/Interaction/Typing）；`style_tag` 未识别时落到 default 臂不判红；`principal` 前缀 → 图标/颜色/标签一张独立表；组件里的 `role === 'user'` / `type === 'tool_call'` 判据全清；单测覆盖"新增一个 style_tag 只加注册表条目不改组件"这一判据
---

## 内容

父设计：[PL-013 §3 · §4](../004-plan/013-前端机制化.md "PL-013-前端机制化")
决策依据：[ADR-070 §决策 3](../010-adr/070.md "ADR-070")

今天的 `registry/messageTypes.ts` 已经是"注册表按 type 分派"的形状，只是 type 是 MessageType 枚举，
且业务专名（`tool_call` / `turn` / `compression` / `user_prompt`）都写进了词表。本任务把它换成
`(kind, style_tag)` 二元组——**协议 enum 只承担"呈现方式"这一族决策，业务分类走 style_tag**。

Claude Code UI 的 `ToolDisplayConfig` 表是这条形状的近邻参考（配置驱动，加一个工具只改表）。

## 处理过程

（未开始）
