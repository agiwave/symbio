<!-- doc-link-allow D-004: 本文定义「现行文档不写变更史」这条规矩，需引用反例措辞（「曾经…已改为…」）界定禁止范围 -->
# Symbio 文档中心

> **文档驱动开发**：本文档体系是 Symbio 项目的"单一事实来源"。

## 文档下沉原则

文档**就近放置、单一来源**：

- **单模块文档**放在该模块目录内（`README.md` + 可选 `docs/`），只写该模块自身机制，不重复系统级内容；
- **系统级文档**（本目录）只保留**跨模块**的核心逻辑、边界与约定，涉及单模块内部实现时**引用**模块文档，不复制细节；
- **历史实施记录**一律进 `archive/`，现行文档只描述当前行为。

四条由 `node scripts/doc-link-audit.mjs` 机械判定（任一命中即失败）：**站内链接必须有效**、
**过程文档不得滞留活跃目录**（D-002）、**活跃文档不得超过 800 行**（D-003）、**正文不得写
变更史**（D-004——历史归 `git log`）。行数上限的作用不是控制篇幅，而是给「职责失守」一个
不会误报的信号：超限即按下面这张表处置，没有豁免。

文档是**下沉**的，所以「某条约定写在哪」要靠检索而不是靠记：
`node scripts/doc-find.mjs <关键词>` 会同时搜 `*.md` 与源码里的 `//!` / `///`
（相当一部分机制就写在模块文档注释里，如 `symbio_core::memory`）。
**知识只写一处**——不要以摘要形式复制到别处，复制必然漂移。

示例：会话上下文压缩的 L0-L6 分层总览在 [session/docs/context-compression-design.md](../symbio/src/plugins/session/docs/context-compression-design.md)，各层阈值与代码实现在 [session/README.md](../symbio/src/plugins/session/README.md)——**会话相关的一切文档都在 `symbio/src/plugins/session/docs/` 内**；系统级目录只保留跨模块规范（如 [design/vdfs.md](./design/vdfs.md)）。

## 文档职责边界（一个事实只有一个 owner）

**重复描述 = 双倍维护成本。** 同一事实写两处，改代码时就要改两处，且两处必然漂移。
因此每类事实只有一个 owner，其余位置一律**引用**，不复述：

| 事实 | 唯一 owner | 其它文档 |
|---|---|---|
| 现在是什么（插件 / 挂载点 / 路由 / 工具 / 规模） | [CURRENT.md](./CURRENT.md)（代码生成） | **不手抄**，需要就引用 |
| 路由语义与调用方 | [reference/ROUTES.md](./reference/ROUTES.md) | 模块 `README.md` 不抄路由表 |
| 错误码 | [reference/ERROR_CODES.md](./reference/ERROR_CODES.md) | PROTOCOLS 不再列错误码表 |
| 配置项 | [reference/CONFIGURATION.md](./reference/CONFIGURATION.md) | 模块 `README.md` 只写机制 + 指向它 |
| 协议线上形状（帧 / 载荷 / 通道 / 线格式） | [architecture/PROTOCOLS.md](./architecture/PROTOCOLS.md) | — |
| 系统拓扑（谁挂在谁下面） | [SYSTEM_MAP.md](./SYSTEM_MAP.md) | 其它图只画自己那一层 |
| 排障链路（一次请求经过哪些代码） | [architecture/DATA_FLOW.md](./architecture/DATA_FLOW.md) | — |
| 为什么这样设计（含被否决的方案与理由） | [DECISIONS.md](./DECISIONS.md) | 其它文档只引用 ADR 编号 |
| 模块内部机制 | 该模块 `README.md` | 系统级文档不复述 |
| 模块深度设计与不变量 | 该模块 `docs/` | — |
| 变更历史（改了什么、何时改的） | `git log` | **现行文档不写变更史**（见下） |
| 一次性评审 / 迁移记录 / 体检报告 | `docs/archive/` | 活文档不保留过程日志 |

**变更时只改 owner 那一处**：结构事实重跑 `node scripts/gen-current-facts.mjs`（CI 有 `--check` 门禁）；跨模块架构改 [OVERVIEW.md](./architecture/OVERVIEW.md) + [SYSTEM_MAP.md](./SYSTEM_MAP.md) 各自己那一层；常见问题进 [TROUBLESHOOTING.md](./guides/TROUBLESHOOTING.md)。

### 两条硬规则

1. **现行文档只描述「现在是什么」**，不写「曾经是什么、后来改成了什么」。
   这类追溯一律走 `git log -S<符号>` / `git log --grep=<词>`；需要保留「为什么否决 A 选 B」的
   结论，就写进 `DECISIONS.md` 的一条 ADR——**那才是它的 owner**。
   判据：一条叙述如果随每次重构都要跟着改，它就不该活在现行文档里。
2. **代码注释与文档各有边界**：注释写「读这个文件需要知道的不变量与陷阱」，
   文档写「机制与取舍」。**注释不复述文档，文档不复述注释**（签名、方法清单、字段表以代码为准）。
   详见 [CONTRIBUTING.md §4](../CONTRIBUTING.md)。

## 快速导航

| 我想... | 查阅 |
|---------|------|
| 核对"现在是什么"（插件 × 挂载点 × 路由 × 工具，自动生成） | [CURRENT.md](./CURRENT.md) |
| 了解系统全貌 | [SYSTEM_MAP.md](./SYSTEM_MAP.md) |
| 理解架构设计 | [OVERVIEW.md](./architecture/OVERVIEW.md) |
| 查看协议规范 | [PROTOCOLS.md](./architecture/PROTOCOLS.md) |
| 追踪请求全链路 | [DATA_FLOW.md](./architecture/DATA_FLOW.md) |
| 查找路由路径 | [ROUTES.md](./reference/ROUTES.md) |
| 理解错误码 | [ERROR_CODES.md](./reference/ERROR_CODES.md) |
| 配置系统 | [CONFIGURATION.md](./reference/CONFIGURATION.md) |
| 快速上手 | [QUICK_START.md](./guides/QUICK_START.md) |
| 开发新插件 | [PLUGIN_DEVELOPMENT.md](./guides/PLUGIN_DEVELOPMENT.md) |
| 排查问题 | [TROUBLESHOOTING.md](./guides/TROUBLESHOOTING.md) |
| 了解设计决策 | [DECISIONS.md](./DECISIONS.md) |
| 查「某条约定写在哪」 | `node scripts/doc-find.mjs <词>`（全仓 md + Rust 文档注释检索） |
| 看某个插件/前端的职责与机制 | 各模块 `README.md`（见下方模块文档地图） |

## 文档放在哪

**目录清单以文件系统为准**（`ls docs/`、`ls docs/archive/`）——本文不抄一份会漂移的副本。各目录的定位：

| 位置 | 放什么 |
|------|--------|
| `docs/`（本目录） | 系统级跨模块文档；`CURRENT.md` 自动生成、勿手改 |
| `docs/architecture/` | 架构总览 / 协议形状 / 排障链路 |
| `docs/reference/` | 路由 / 错误码 / 配置的权威参考页 |
| `docs/guides/` | 上手 / 插件开发 / 排障 |
| `docs/design/` | 跨层设计规范（VDFS、agent 目录、HTTP 传输…）；**只放规范** |
| `docs/archive/` | 历史归档：已废止的旧机制 / 旧规范、一次性评审与体检、已落地的实施方案与迁移；**变更历史以 `git log` 为准，本仓库不维护 CHANGELOG** |
| 模块目录 | `symbio/src/plugins/<plugin>/README.md`（+ 可选 `docs/`）、`tauri/`、`cli/`——就近放置，受同一「过程文档必须归档」约束 |

## 模块文档地图

| 模块 | 文档 | 一句话职责 |
|------|------|-----------|
| symbio_core | [symbio_core/README.md](../symbio/src/symbio_core/README.md) | 内核契约层：命名与结构规范 + 域清单（trait / 协议类型 / 词表 / 纯工具） |
| session | [plugins/session/README.md](../symbio/src/plugins/session/README.md) | 会话编排唯一入口：工具循环、提示词组装、上下文压缩 |
| model | [plugins/model/README.md](../symbio/src/plugins/model/README.md) | 无状态单轮 LLM 网关（execute_turn），多协议适配 |
| agent | [plugins/agent/README.md](../symbio/src/plugins/agent/README.md) | 智能体域唯一所有者：agent 目录库、子树装配、两作用域 `AGENTS.md` |
| mcp | [plugins/mcp/README.md](../symbio/src/plugins/mcp/README.md) | MCP 外部工具接入与能力注册 |
| skill | [plugins/skill/README.md](../symbio/src/plugins/skill/README.md) | 技能脚本（loader/plugin）发现与装载 |
| local | [plugins/local/README.md](../symbio/src/plugins/local/README.md) | 本地原生工具（shell / 内容与语义搜索 / 任务清单）；文件操作已迁 VDFS |
| vdfs | [plugins/vdfs/README.md](../symbio/src/plugins/vdfs/README.md) | 文件系统本身：`vdfs/*` 协议入口 + `vdfs_*` LLM 工具（规范见 design/vdfs.md） |
| web | [plugins/web/README.md](../symbio/src/plugins/web/README.md) | 网页抓取/搜索工具 |
| home | [plugins/home/README.md](../symbio/src/plugins/home/README.md) | 根插件：持应用级状态（`<homedir>/PLUGIN.yml`），构造 worker(Composite) 并传入必需插件清单 |
| composite | [plugins/composite/README.md](../symbio/src/plugins/composite/README.md) | 子插件容器：扫描自己的目录（其下一层目录即一个插件）+ 路径合并分发 |
| work | [plugins/work/README.md](../symbio/src/plugins/work/README.md) | 工作区记忆：注入 `{workdir}/AGENTS.md`，可经 `<根>/work` 编辑 |
| gateway | [plugins/gateway/README.md](../symbio/src/plugins/gateway/README.md) | HTTP/WS 入站网关 |
| plugin_manager | [plugins/plugin_manager/README.md](../symbio/src/plugins/plugin_manager/README.md) | 插件管理入口：列全部插件 + 启用/停用/添加/卸载 + 各插件配置条目 + 自有分区（appearance/about） |
| hook | [plugins/hook/README.md](../symbio/src/plugins/hook/README.md) | 生命周期钩子 |
| event_bus | [plugins/event_bus/README.md](../symbio/src/plugins/event_bus/README.md) | 进程内事件总线 |
| telegram | [plugins/telegram/README.md](../symbio/src/plugins/telegram/README.md) | Telegram 通道接入 |
| tauri 前端 | [tauri/README.md](../tauri/README.md) → [docs/FRONTEND.md](../tauri/docs/FRONTEND.md) | Vue3 + Tauri2 桌面前端 |

> **维护原则**：本文是文档体系的入口；结构性变更（新增/删除目录、职责改属）先改此文件。单条事实的增改只动它的 owner，不必回头改本文。
