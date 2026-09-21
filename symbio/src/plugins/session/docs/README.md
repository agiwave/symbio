# Session 插件文档

本目录是 **session 插件相关知识的唯一归处**（高内聚：会话的机制、设计、审计、性能、模块分工都在这里，
不再散落在系统级 `docs/`）。系统级目录只保留跨模块规范（VDFS / 传输层等）。

## 与 `../README.md` 的分工

| 文档 | 写什么 |
|---|---|
| [`../README.md`](../README.md) | **现行机制 + 配置面**：六大压缩策略、参数 Schema、策略对比矩阵。要查"当前行为是什么"看这里。 |
| 本目录 | **深度设计与专题**：核心循环怎么收口、模块怎么分工、历史审计结论、前端性能、心跳、VDFS 会话消息映射等。要查"为什么这样设计"看这里。 |

## 文档清单

| 文档 | 状态 | 一句话 |
|---|---|---|
| [core-loop.md](./core-loop.md) | 现行设计（批次 A/B/C 已落地，D 待授权；**批次 E 已落地**） | LLM × 工具会话主循环的结构与四个收口点（`gate_turn` / `prepare_turn_inputs` / `apply_compaction` / `finish_turn`）；§8 记执行期出口/信号双原语（`EventSink` + `AbortSignal`） |
| [module-layout.md](./module-layout.md) | 现行（S1–S4 已落地） | 插件模块分工评审：单文件过长问题、目标目录结构、S1–S4 执行顺序与验收、可见性口径 |
| [context-compression-design.md](./context-compression-design.md) | 现行设计总览 | 上下文压缩 L0–L6 分层机制、优先取舍、不变量（实现细节见 `../README.md`） |
| [turn-tool-mechanisms.md](./turn-tool-mechanisms.md) | 现行机制 | 工具失败如何回传（信息性、不中断循环）、Turn 终态如何定 |
| [heartbeat-mechanism.md](./heartbeat-mechanism.md) | 现行设计 | 会话空闲心跳：调度语义、设置工具、CLI 守护模式、homedir 传导 |
| [perf.md](./perf.md) | 现行设计 | 会话/转写的前端性能设计 + 存储拆分（元数据与消息分文件，清单慢的根因解药） |
| [vdfs-session-messages.md](./vdfs-session-messages.md) | 现行设计（S16–S19 已完成） | 会话消息的 VDFS 化：转写即列表、流式即追加；地址与节点形状、变更语义、迁移路线 |
| [cascading-options-mechanism.md](./cascading-options-mechanism.md) | 现行规范 | 会话页「选项行」的级联选项机制与规范（宿主 = `options.rs`，收集走 core 的 `OptionVisitor`） |
| [legacy-route-migration.md](./legacy-route-migration.md) | 审计（待执行，S1–S7） | 会话旧路由审计：11 条路由分「可删（死路由）/ 可迁 VDFS / 必须保留」三档，附影响面与执行顺序 |

> **已归档的过程产物**（两份审计，`docs/archive/implementation-logs/`）：
> [`session-mechanism-audit.md`](../../../../../docs/archive/implementation-logs/session-mechanism-audit.md)（机制审计与简化方案，方案已全部落地）
> 与 [`session-complexity-audit.md`](../../../../../docs/archive/implementation-logs/session-complexity-audit.md)（更早的复杂度审计）。
> 它们的结论已体现在现行代码与上面各文档里，不再驱动任何待做项。

## 建议阅读路径

- **要改核心循环** → [`core-loop.md`](./core-loop.md) → [`../README.md`](../README.md) 六大策略 → [`module-layout.md`](./module-layout.md) 找文件落点
- **要改压缩** → [`context-compression-design.md`](./context-compression-design.md)（分层与不变量）→ [`../README.md`](../README.md)（阈值与函数）
- **要查历史为什么这么改** → 已归档的两份审计（见上表下方的归档说明），以及 [`docs/CHANGELOG.md`](../../../../../docs/CHANGELOG.md)
- **要动前端转写/会话列表** → [`perf.md`](./perf.md) → [`vdfs-session-messages.md`](./vdfs-session-messages.md)

## 约定

- **路径书写**：本目录内引用同目录文档用 `./x.md`，引用插件 `README.md` 用 `../README.md`，
  引用系统级文档用 `../../../../../docs/...`。代码注释里的文档路径统一用**仓库根相对**写法
  （`symbio/src/plugins/session/docs/x.md`），插件内部注释可用 `docs/x.md`。
- **新增文档**：放本目录，并在上表登记一行；改完跑 `node scripts/doc-link-audit.mjs --strict`。
- **历史实施记录**不写在这里，进 `docs/archive/implementation-logs/`。
