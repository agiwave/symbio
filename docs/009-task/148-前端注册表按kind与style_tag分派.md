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

- **148-i**（`(kind, style_tag)` 表驱动折叠，本批）：`foldNodeKind` 的 `switch` 控制流降为数据——新增 `NODE_KIND_PRESENTATION: Record<NodeKind, {default, byStyleTag}>` 分派表，外层按 wire 五值建（`Record<NodeKind,…>` 让漏登记一个形态是编译错误），内层 `byStyleTag` 只列该形态确有专属变体的 tag，未命中（含缺 tag / 畸形裸骨架 / **未登记的新 tag**）一律落 `default` **不判红**。折叠语义逐条对齐旧 `switch`（body±reasoning、skeleton±{run,tool}、notice、interaction、typing 原样透出、呈现键幂等直通）。判据单测三条钉死「加一个 style_tag 只加表条目不改组件」：已登记 tag→专属键、未登记 tag→默认臂且等价于缺 tag、未登记 tag 折叠后仍命中已登记渲染器（非 fallback 非空白）。前端 typecheck + vitest 884（+3）、`protocol-mirror-audit` 0 错全绿（纯 registry 重构，wire 与后端未动）。
- **148-ii**（principal 前缀 → 身份呈现独立表，本批）：新增 `PRINCIPAL_NS_PRESENTATION`（命名空间主键：`person`/`session`/`system`/`vdfs`/`web`/`local`/`mcp`/`agent`/`legacy` 各一行 icon/tone/label）+ `PRINCIPAL_KIND_PRESENTATION`（四族兜底）。读入口 `principalPresentationOf`——命名空间精确命中优先、未登记退回 kind、缺主体 → `undefined`（与 148-i 默认臂同一「不判红」纪律）；`principalNamespace` 取 `<ns>/<name>` 的 ns 段。主键是命名空间而非 kind（`vdfs/write` 与 `web/fetch` 同为 tool 却各有徽标，正合 [ADR-069]「kind 不参与主键」）。`MessageFacets` 加可选 `principal` 呈现字段、`facetsOf` 经此表填充（表被消费非死数据）。历史裸字符串 principal 经 `normalizePrincipal` 归一后进表（`"user"`→person/local、`"agent:main"`→session/main）。单测 6 条钉死命名空间差异、kind 兜底、缺主体 undefined、字符串归一、facets 携带。纯 registry 增量，现有 icon/label 行为与既有断言不动。typecheck、vitest 890（+6）、`protocol-mirror-audit` 0 错全绿。
