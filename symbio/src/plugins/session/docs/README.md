---
kind: design
id: symbio/src/plugins/session/docs/README
title: Session 插件文档
status: active
created: 2026-09-16
lists: [session-docs=owner]
impl: done
---

# Session 插件文档

本目录是 **session 插件相关知识的唯一归处**（高内聚：会话的机制、设计、审计、性能、模块分工都在这里，
不再散落在系统级 `docs/`）。系统级目录只保留跨模块规范（VDFS / 传输层等）。

## 与 `../README.md` 的分工

| 文档 | 写什么 |
|---|---|
| [`../README.md`](../README.md "symbio/src/plugins/session/README") | **现行机制 + 配置面**：五大压缩策略、参数 Schema、策略对比矩阵。要查"当前行为是什么"看这里。 |
| 本目录 | **深度设计与专题**：核心循环怎么收口、模块怎么分工、历史审计结论、前端性能、VDFS 会话消息映射等。要查"为什么这样设计"看这里。 |

## 文档清单

<!-- list:session-docs owner -->

| 文档 | 状态 | 一句话 |
|---|---|---|
| [DE-102-core-loop](102-core-loop.md) | 现行设计 | LLM × 工具会话主循环的结构与四个收口点（`gate_turn` / `prepare_turn_inputs`（含轮首派发点）/ `policy.compress` 的执行体 / `finish_turn`）；§6 记执行期出口/信号双原语（`EventSink` + `AbortSignal`） |
| [DE-104-module-layout](104-module-layout.md) | 现行规范 | 插件怎么分域：五个判据、编排层/领域层落点、测试布局、可见性与共享面、纯搬移自证 |
| [DE-101-context-compression-design](101-context-compression-design.md) | 现行设计总览 | 上下文压缩 L0–L6 分层机制、优先取舍、不变量（实现细节见 `../README.md`） |
| [DE-108-turn-tool-mechanisms](108-turn-tool-mechanisms.md) | 现行机制 | 工具失败如何回传（信息性、不中断循环）、Turn 终态如何定 |
| [DE-113-heartbeat-mechanism](../../heartbeat/docs/113-heartbeat-mechanism.md) | 现行设计 | 会话空闲心跳：调度语义、设置工具、CLI 守护模式、homedir 传导 |
| [DE-106-perf](106-perf.md) | 现行设计 | 会话/转写的前端性能设计 + 存储拆分（元数据与消息分文件，清单慢的根因解药） |
| [DE-109-vdfs-session-messages](109-vdfs-session-messages.md) | 现行设计 | 会话消息的 VDFS 化：转写即列表、流式即追加；地址与节点形状、变更语义、不变量 |
| [DE-107-session-options](107-session-options.md) | 现行规范（2026-09-23 旧机制下线） | 会话选项 = 会话配置表单：同一份 `DetailDefinition` 的两种渲染形态（选项栏 / 详情页），三条通路（`node.schema` / `attributes.metadata` / `vdfs/write`），收集走统一 `Registry`（选项行）|

> **已归档的过程产物**（一次性审计 / 迁移记录，进 `docs/archive/`）：
> [`legacy-route-migration.md`](../../../../../docs/archive/legacy-route-migration.md)（会话旧路由审计，迁移已全部落地）、
> [`session-mechanism-audit.md`](../../../../../docs/archive/implementation-logs/session-mechanism-audit.md)（机制审计与简化方案，方案已全部落地）
> 与 [`session-complexity-audit.md`](../../../../../docs/archive/implementation-logs/session-complexity-audit.md)（更早的复杂度审计）。
> 它们的结论已体现在现行代码与上面各文档里，不再驱动任何待做项——**不随重构回改**。

## 建议阅读路径

- **要改核心循环** → [`015-core-loop.md`](102-core-loop.md "DE-102-core-loop") → [`../README.md`](../README.md "symbio/src/plugins/session/README") 五大策略 → [`017-module-layout.md`](104-module-layout.md "DE-104-module-layout") 找文件落点
- **要改压缩** → [`014-context-compression-design.md`](101-context-compression-design.md "DE-101-context-compression-design")（分层与不变量）→ [`../README.md`](../README.md "symbio/src/plugins/session/README")（阈值与函数）
- **要查历史为什么这么改** → 已归档的审计与迁移记录（见上表下方），以及 `git log --grep=<词>` / `git log -S<符号>`（本仓库不另维护变更日志）
- **要动前端转写/会话列表** → [`019-perf.md`](106-perf.md "DE-106-perf") → [`022-vdfs-session-messages.md`](109-vdfs-session-messages.md "DE-109-vdfs-session-messages")

## 约定

- **路径书写**：本目录内引用同目录文档用 `./x.md`，引用插件 `README.md` 用 `../README.md`，
 引用系统级文档用 `../../../../../docs/...`。代码注释里的文档路径统一用**仓库根相对**写法
 <!-- doc-link-allow D-006: 以下两个反引号路径是"路径该怎么写"的示意，并非入链 -->
 （`symbio/src/plugins/session/docs/x.md`），插件内部注释可用 `docs/x.md`。
- **新增文档**：放本目录，并在上表登记一行；改完跑 `node scripts/doc-link-audit.mjs --strict`。
- **历史实施记录**不写在这里，进 `docs/archive/implementation-logs/`。
