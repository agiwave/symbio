---
kind: task
id: T-147-ChatNode协议改名与字段瘦身
title: ChatMessage → ChatNode 一次原子改——Turn 退役、role 删除、tool_call_id 沉 meta、principal 升级为结构体
status: doing
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

按「协议两侧同时改绿」的原子约束分五档推进——每档前后端一起动、审计一起绿、一次原子提交：

- **147a**（additive）：`Principal` 结构体 + `PrincipalKind` 枚举 + `from_legacy` 读侧别名 helper；未接消费方。→ commit `910c4214`
- **147b**（wire 翻转）：`ChatMessage.principal: Option<String>` → `Option<Principal>`；`Principal` 自定义 `Deserialize` 同吃字符串与对象两种 wire 形态；写侧 `attributed` 构造 `Principal`；读侧 `visible_for` / `actor_and_ts` 通过 `.id` 消费；TS 加 `Principal` 接口 + `normalizePrincipal` / `principalId` / `principalKind` 三个 helper 与后端 `from_legacy` 逐字一致。
- **147c-i**（机械改名）：`chat_message.rs` → `chat_node.rs`；`ChatMessage` → `ChatNode`；`MessageType` → `NodeKind`；`MessageStatus` → `NodeStatus`；`MessageRole` → `NodeRole`；前端 `chat_message.ts` → `chat_node.ts` 同步。枚举**未**收五值——六值变体名与 wire 词都保留。
- **147c-ii-prep**（判据 helper，本批）：`NodeShape` 目标五值 + `legacy_shape_to_style_tag` 六→五映射 + `CHAT_STYLE_TAG_*` 词表 + `ChatNode::shape()` / `style_tag()` 判据入口 + 谓词 `is_run` / `is_tool_call` / `is_reasoning` / `is_text` / `is_compression` / `is_interaction`——谓词**签名**即未来形态判据入口，五值收敛那一档消费方代码不动、改点收在构造侧。四条单测（映射表、谓词互斥、shape 折叠、style_tag 从 meta 读）钉住基线。
- **147c-ii**（六→五收敛，延后）：`NodeKind` 变体收 Body/Notice/Skeleton/Interaction/Typing；写侧把 `NodeKind::Turn` 构造改成 `Skeleton + meta.style_tag="run"`，其余五档同理；前端 TS 同步。**规模**：~300 Rust 站点 + 250+ TS 站点，一次原子改。
- **147d**（字段瘦身）：`role` 从协议删除（`kind` 已承载归属）；`tool_call_id` 沉 meta；`name` 迁到 `principal.title`。
- **147e**（动词改名）：`ResumeAction::RetryTurn` → `RetrySubtree`；前后端常量与分派同步。

