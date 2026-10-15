---
lists: [session-docs=owner]
kind: arch
id: symbio/src/plugins/session/docs/README
title: Session 插件文档
status: active
created: 2026-09-16
---

# Session 插件文档

本目录是 **session 插件相关知识的唯一归处**（高内聚：会话的机制、设计、审计、性能、模块分工都在这里，
不再散落在系统级 `docs/`）。系统级目录只保留跨模块规范（VDFS / 传输层等）。

## 与 `../README.md` 的分工

| 文档 | 写什么 |
|---|---|
| [`../README.md`](../README.md "symbio/src/plugins/session/README") | **现行机制 + 配置面**：六大压缩策略、参数 Schema、策略对比矩阵。要查"当前行为是什么"看这里。 |
| 本目录 | **深度设计与专题**：核心循环怎么收口、模块怎么分工、历史审计结论、前端性能、心跳、VDFS 会话消息映射等。要查"为什么这样设计"看这里。 |

## 文档清单

<!-- list:session-docs owner -->

| 文档 | 状态 | 一句话 |
|---|---|---|
| [015-core-loop.md](015-core-loop.md "AR-015-core-loop") | 现行设计 | LLM × 工具会话主循环的结构与四个收口点（`gate_turn` / `prepare_turn_inputs` / `apply_compaction` / `finish_turn`）；§6 记执行期出口/信号双原语（`EventSink` + `AbortSignal`） |
| [017-module-layout.md](017-module-layout.md "AR-017-module-layout") | 现行规范 | 插件怎么分域：五个判据、编排层/领域层落点、测试布局、可见性与共享面、纯搬移自证 |
| [014-context-compression-design.md](014-context-compression-design.md "AR-014-context-compression-design") | 现行设计总览 | 上下文压缩 L0–L6 分层机制、优先取舍、不变量（实现细节见 `../README.md`） |
| [021-turn-tool-mechanisms.md](021-turn-tool-mechanisms.md "AR-021-turn-tool-mechanisms") | 现行机制 | 工具失败如何回传（信息性、不中断循环）、Turn 终态如何定 |
| [016-heartbeat-mechanism.md](016-heartbeat-mechanism.md "AR-016-heartbeat-mechanism") | 现行设计 | 会话空闲心跳：调度语义、设置工具、CLI 守护模式、homedir 传导 |
| [019-perf.md](019-perf.md "AR-019-perf") | 现行设计 | 会话/转写的前端性能设计 + 存储拆分（元数据与消息分文件，清单慢的根因解药） |
| [022-vdfs-session-messages.md](022-vdfs-session-messages.md "AR-022-vdfs-session-messages") | 现行设计 | 会话消息的 VDFS 化：转写即列表、流式即追加；地址与节点形状、变更语义、不变量 |
| [020-session-options.md](020-session-options.md "AR-020-session-options") | 现行规范（2026-09-23 旧机制下线） | 会话选项 = 会话配置表单：同一份 `DetailDefinition` 的两种渲染形态（选项栏 / 详情页），三条通路（`node.schema` / `attributes.metadata` / `vdfs/write`），收集走 core 的 `OptionVisitor` |

> **已归档的过程产物**（一次性审计 / 迁移记录，进 `docs/archive/`）：
> [`legacy-route-migration.md`](../../../../../docs/archive/legacy-route-migration.md)（会话旧路由审计，迁移已全部落地）、
> [`session-mechanism-audit.md`](../../../../../docs/archive/implementation-logs/session-mechanism-audit.md)（机制审计与简化方案，方案已全部落地）
> 与 [`session-complexity-audit.md`](../../../../../docs/archive/implementation-logs/session-complexity-audit.md)（更早的复杂度审计）。
> 它们的结论已体现在现行代码与上面各文档里，不再驱动任何待做项——**不随重构回改**。

## 建议阅读路径

- **要改核心循环** → [`015-core-loop.md`](015-core-loop.md "AR-015-core-loop") → [`../README.md`](../README.md "symbio/src/plugins/session/README") 六大策略 → [`017-module-layout.md`](017-module-layout.md "AR-017-module-layout") 找文件落点
- **要改压缩** → [`014-context-compression-design.md`](014-context-compression-design.md "AR-014-context-compression-design")（分层与不变量）→ [`../README.md`](../README.md "symbio/src/plugins/session/README")（阈值与函数）
- **要查历史为什么这么改** → 已归档的审计与迁移记录（见上表下方），以及 `git log --grep=<词>` / `git log -S<符号>`（本仓库不另维护变更日志）
- **要动前端转写/会话列表** → [`019-perf.md`](019-perf.md "AR-019-perf") → [`022-vdfs-session-messages.md`](022-vdfs-session-messages.md "AR-022-vdfs-session-messages")

## 约定

- **路径书写**：本目录内引用同目录文档用 `./x.md`，引用插件 `README.md` 用 `../README.md`，
  引用系统级文档用 `../../../../../docs/...`。代码注释里的文档路径统一用**仓库根相对**写法
  <!-- doc-link-allow D-006: 以下两个反引号路径是"路径该怎么写"的示意，并非入链 -->
  （`symbio/src/plugins/session/docs/x.md`），插件内部注释可用 `docs/x.md`。
- **新增文档**：放本目录，并在上表登记一行；改完跑 `node scripts/doc-link-audit.mjs --strict`。
- **历史实施记录**不写在这里，进 `docs/archive/implementation-logs/`。
