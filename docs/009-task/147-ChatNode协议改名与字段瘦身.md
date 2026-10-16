---
kind: task
id: T-147-ChatNode协议改名与字段瘦身
title: ChatMessage → ChatNode 一次原子改——Turn 退役、role 删除、tool_call_id 沉 meta、principal 升级为结构体
status: todo
created: 2026-10-16
track: 会话与输入
deps: [T-146-主体可路由地址]
prio: now
parent: T-145-前端机制化批
where: symbio/src/symbio_core/schemas/session/ · symbio/src/plugins/session/ · symbio/src/plugins/agent/ · cli/src/client.rs · cli/src/render.rs · tauri/src/schemas/ · scripts/protocol-mirror-audit.mjs
exit: `chat_message.rs` → `chat_node.rs`；`ChatMessage` → `ChatNode`；`MessageType` → `NodeKind`（五值闭集：Body/Notice/Skeleton/Interaction/Typing）；`MessageRole` 从协议删除；`tool_call_id`/`name` 从顶层移除（沉 meta 或用 title）；`ChatMessage.principal: Option<String>` → `Option<Principal>`（`{kind: PrincipalKind, id: String, title: Option<String>}` 三字段）；历史 `messages.json` 一次性读侧别名（`"agent:main"` → `{kind:Agent,id:"session/main"}`，`"user"` → `{kind:Person,id:"person/local"}`）；`ResumeAction::RetryTurn` → `RetrySubtree`；前端 `chat_message.ts` → `chat_node.ts` 同步、`principal?: string` → `principal?: { kind, id, title? }`；100+ 单测一次改绿；`protocol-mirror-audit` 指向新文件名与新 principal 结构；一次 diff 内落地，不分批
---

## 内容

决策依据：[ADR-070 Skeleton 取代 Turn](../010-adr/070.md "ADR-070") · [ADR-069 主体可路由地址](../010-adr/069.md "ADR-069")
父设计：[PL-013 §3](../004-plan/013-前端机制化.md "PL-013-前端机制化")

**协议层最大一次原子改**。Turn→Skeleton 的语义解绑是整批的核心；[T-148]~[T-152] 全部依赖它。
「一次改完、一次改绿」不是洁癖——分两批改意味着中间态协议两侧不一致，`protocol-mirror-audit` 会红一片。

principal 从字符串升到结构体一并做：[T-146] 已把值域迁到可路由地址 `<ns>/<name>`，本批把 kind
与 title 从「靠约定读 id 前缀 / 靠 meta」升为「协议字段」。历史 `messages.json` 里旧字符串
`"agent:main"` / `"user"` 靠读侧别名一次到位，不保留双形态。

字段瘦身清单见 ADR-070 §决策 2 的表；Principal 三字段与读侧别名规则见 ADR-069 §决策 6。

## 处理过程

（未开始）

