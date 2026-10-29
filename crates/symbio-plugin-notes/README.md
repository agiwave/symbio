---
kind: design
id: crates/symbio-plugin-notes/README
title: Notes 插件（扩展夹具）
status: active
created: 2026-10-28
impl: done
---

# Notes 插件（扩展夹具）

本插件存在的理由不是「便签功能」，而是 [DE-025](../../docs/003-design/025-能力包与扩展面.md "DE-025-能力包与扩展面") `EX-001` 的夹具：
**一个新增能力包在不改 `core` / `kernel` / `loop` / `session` 的前提下，同时接上六条扩展面**。

| 面 | 实现 | 接入方式 |
|---|---|---|
| 工具 | [`src/tool.rs`](src/tool.rs) 的 `notes_add` | `Plugin::traverse` 注册 `Capability` |
| 领域事实 | [`src/decl.rs`](src/decl.rs) 的 `notes.created`（`Domain × Opened`） | `submit_declarations!` |
| 上下文段 | [`src/context.rs`](src/context.rs) 的 `notes_context` | `submit_context_provider!` |
| 效果行为 | [`src/behavior.rs`](src/behavior.rs) 的 `note_capture` | `submit_behavior!` |
| 前置判断 | [`src/behavior.rs`](src/behavior.rs) 的 `ToolBehavior::before`（重复便签在执行前拒） | 同一个 `submit_behavior!`（D-219） |
| 对外可读的折叠 | [`src/decl.rs`](src/decl.rs) 的 `ViewDecl`（`notes.recent`） | 自家 `PLUGIN.yml` 的 `shared_views` → 生成 `view_names.rs`，读方经 `plugin-api::views`（D-221） |

## 为什么不声明 `tool_namespace`

`tool_namespace` 是部署事实，声明它会把生成物写进
`crates/symbio-kernel/src/chat/tool_namespaces.gen.rs`。本夹具的验收判据是
**`kernel` 零 diff**，因此工具主体地址落 `legacy/notes_add`——它只影响转写里的展示身份；
授权判的是调用方主体（[D-176](../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。

## 停用行为

- **工具**随装配位消失：停用 ⇒ 插件不构造 ⇒ `traverse` 不注册（T88 同一条语义）。
- **行为**不会被触发：没有工具调用就没有声明，效果端口选不到它，不再产生新事实。
- **声明 / 上下文 provider / 视图读口** 是链接期贡献（`inventory` 与清单生成物），运行期停用**不**移除它们；
  provider 在本插件被停用后仍会渲染**历史事实**里的便签。要把它们从进程里彻底拿掉，
  单位是**构建**（不链接本 crate），不是运行期开关——两种开关的分界见 DE-025 D-217。

## 关联

- 扩展面契约：[DE-025](../../docs/003-design/025-能力包与扩展面.md "DE-025-能力包与扩展面") §3
- 判据：`EX-001`（构建 diff 落在本节）
