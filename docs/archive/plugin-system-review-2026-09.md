# 插件体系评审：架构与协议层的不合理之处

> **文档类型：评审（一次性结论）** — 只读评审的结论清单，不是设计提案。
>
> **归档理由**：本文记录的是 2026-09 这一轮评审**当时**发现了什么、打算怎么改；
> 改进一旦落地，本文与现状的一致既不必要也不可能。**现行规范**见
> [design/third-party-plugin-spec.md](../design/third-party-plugin-spec.md)；
> 单条决策的来龙去脉以 [DECISIONS.md](../DECISIONS.md) 为准。
>
> **方法**：每条都带**精确位置**（`文件:行`）与**可核对的现象**，不含推测。
> 「为什么是问题」一律从**既有文档自己声明的规则**出发——一个体系最值得审的地方，
> 是它**说的**和它**做的**之间的缝。

## 0. 判据

评审一个分形插件体系，只问三个问题：

| 判据 | 对应的问题 | 本文哪一组 |
|---|---|---|
| **契约自洽** | 文档声明的规则，代码是否遵守？ | A 系列（架构层） |
| **边界可判** | 「是不是插件」「能不能跨进程」是否有可机检的判据？ | P 系列（协议层） |
| **扩展成本** | 加一个插件 / 加一个维度，要动几处？ | A5 / P8 |

**不在本文范围**：性能、并发正确性、前端呈现。它们各有专门文档。

---

## 1. 架构层（A 系列）

### A1｜「插件 = 编译期注册的进程内 `Arc<dyn Plugin>`」——等式过窄（**根问题**）

**现象**：`plugin_provider` 的合法取值 = `creator_ids::<dyn Plugin>()`，即编译期
`inventory` 静态表里那 16 个 id。装配方判据是「`has_creator` 命中否」。

**位置**：
- `symbio/src/symbio_core/creator.rs:20` —— 构造函数是**裸函数指针**
  `fn(Arc<dyn InvokeRequest>) -> Box<dyn Any + Send + Sync>`
- `symbio/src/symbio_core/creator.rs:35` —— `inventory::collect!(Submit)`（编译期收集）
- `symbio/src/symbio_core/creator.rs:71` —— `entry.type_id != TypeId::of::<T>()` 严格相等校验
- `symbio/src/symbio_core/plugin_dir.rs:61` —— 加载判据 = `plugin_provider` 指向**已注册**工厂
- `docs/architecture/PROTOCOLS.md:249-251` —— 同一判据的文档表述

**为什么是问题**：这不是「少一个注册 API」，而是**把「插件」定义成了「本进程内的一个
Rust trait object」**。三个机制叠加成一道不可绕的墙：

1. `inventory` 只在**编译期**收集，运行时无法往表里加东西；
2. `TypeId` 是**进程内**概念，跨进程无意义；
3. 裸函数指针不可序列化，无法描述「另一个进程里怎么造出这个东西」。

任何进程外 / 异构语言的实现，都必须先把自己塞进这张编译期表——**不可能**。
这是挡住第三方插件的**唯一根因**，其余 A 系列问题都是它的后果或伴随条件。

**改进方向**：把 `plugin_provider` 的解析从「查表命中 / 未命中」升级为**两级解析**：

```
plugin_provider: "web"            → 内置命名空间（现状完全不变）
plugin_provider: "ext:weather"    → 外部命名空间（运行期注册的 provider）
```

关键是**不改内置路径的任何行为**——`inventory` 保留为「内置来源之一」，
外部 provider 是**并列的第二个来源**，二者在「解析 provider」这一层合流。

**代价**：需 ADR（触及 `Plugin` 装配契约）。**前置**：A2 / A4 / A5 / P3 / P8 都是它的配套。

---

### A2｜插件无生命周期钩子，「停用 = 不构造」把生命周期与装配绑死

**现象**：`Plugin` trait 只有 5 个方法，没有任何 `init` / `start` / `stop` /
`on_disable` / `on_unload`。「停用」的实现是：写 `PLUGIN.yml` 的 `plugin_enabled: false`
→ 从实例表移除 → 下次装配不再构造。

**位置**：
- `symbio/src/symbio_core/plugin.rs:300-364` —— trait 全部方法（`meta` / `route` /
  `traverse` / `get_vfs_provider` / `vdfs_dispatch`）
- `symbio/src/symbio_core/plugin_dir.rs:108-124` —— `KEY_ENABLED` 注释：
  「停用的插件**刻意不被构造**（它不该启动任何后台行为）」

**为什么是问题**：进程内插件「被 drop ≈ 停止」之所以成立，靠的是 RAII 兜底——
但这是**巧合而非设计**：

- 若插件持有后台任务 / 长连接 / 临时文件 / 子进程，drop 不保证优雅停止；
- 进程外插件**根本没有 drop 语义**：不显式通知它，它就是一个孤儿进程。

也就是说：这条设计在**纯进程内 + 无副作用插件**的前提下成立，而第三方插件恰好
两条都不满足。A1 一旦松绑，A2 立刻变成必须解决的问题。

**改进方向**：加**可选**钩子（有默认空实现，不波及现有 16 个 impl），在
`set_enabled(false)` 与 `uninstall` 的路径上调用。语义上要区分三件事：
「停用」（可恢复）/「卸载」（数据是否保留）/「进程退出」（全局收尾）。

**代价**：需 ADR（`Plugin` 是公开契约）。

---

### A3｜唯一一处真跨插件引用，违反自己声明的「插件独立原则」

**现象**：`agent` 插件直接 `use` `session` 插件的内部符号（`message_of_node` 函数与
`SEG_MESSAGES` 常量）。这是全仓**唯一**一处真跨插件引用。

**位置**：
- 违反点：`symbio/src/plugins/agent/host/subagent.rs:42`
  `use crate::plugins::session::plugin::{message_of_node, SEG_MESSAGES};`
- 被违反的规则：`symbio/src/plugins/mod.rs:5-14` ——「插件独立原则：所有 plugin
  子模块都是私有，插件之间互相不可见，不能直接相互引用」

**为什么是问题**：`agent` 与 `session` 编译期耦合，二者不能独立替换——`session`
改一个内部函数签名，`agent` 会编译失败。而这条耦合**恰好落在「子智能体」这个最像
「插件复用」的场景上**：子 Agent 是「一棵与父树同构的 composite 子树」，它复用的是
`session` 的**消息节点构造逻辑**。第三方插件体系最需要的就是这种复用，而当前唯一的
复用实例是通过**破坏原则**实现的——说明原则缺一个出口。

**改进方向**：把 `message_of_node` / `SEG_MESSAGES` 上移到
`symbio_core::schemas::session`（跨插件共享契约层）。它们本就是「会话消息节点的
线上形状」，属于 core 的既有职责范围（`schemas/` 已按业务域拆分）。

**代价**：**机械改进**，面窄（一处 `use` + 两个符号搬迁 + 调用点更新）。

---

### A4｜安全模型是「工具执行策略」，不是「插件能力授权」

**现象**：`SecurityPolicy` 有完整的字段（自主级别 / 命令白名单 / 路径边界 / 风险
阈值 / 审批要求 / 频次上限），但它**只作用于 `local` 插件自己的工具**。

**位置**：
- `symbio/src/plugins/local/policy/mod.rs:37-47` —— `SecurityPolicy` 定义
- `symbio/src/plugins/local/plugin.rs:4,61-68,137,175` —— 只在 local 内部构造与消费
- `symbio/src/plugins/local/plugin.rs:239` —— `SecurityPolicy::default()` 在 local 构造时

**为什么是问题**：这是**一个插件内部怎么安全地执行它自己的工具**，不是
**宿主怎么约束一个插件**。两者方向相反：

| | 谁设规则 | 约束谁 | 现有形态 |
|---|---|---|---|
| 工具执行策略 | 插件自己 | 插件自己的工具 | ✅ `SecurityPolicy` |
| 插件能力授权 | **宿主** | **插件整体** | ❌ 不存在 |

引入第三方插件后，缺的是后者：插件声明「我需要文件系统 / 网络 / 子进程」，宿主
决定授不授、授多少。当前体系里没有任何一处表达「这个插件被允许做什么」——
因为内置插件是可信的，这个问题从未被提出。

**改进方向**：新增**插件级**能力声明（落在 manifest 里，见 A5）+ 宿主侧授予点。
与 `SecurityPolicy` **分层共存**，不替换它：`SecurityPolicy` 继续管 local 的工具
执行细节，插件级声明管「整个插件能不能碰文件系统」。

**代价**：需 ADR。

---

### A5｜插件自述分散在三处，「清单」不是单一事实源

**现象**：关于「这个插件是什么」，事实散在三处，**互不校验**：

| 位置 | 承载 | 权威性 |
|---|---|---|
| `<插件>/PLUGIN.yml` | 身份（`plugin_provider`/`plugin_name`）+ 装配位 + 插件自己的配置 | 运行期读 |
| `keys.rs:180,227` | `SYSTEM_AGENT_PLUGINS` / `SUB_AGENT_PLUGINS`（挂载清单） | **机制级常量**（编译期） |
| `creator.rs:29-35` | 工厂注册表（有哪些 provider 可用） | 编译期 |

**位置**：`symbio/src/symbio_core/plugin_dir.rs:105-124`；
`symbio/src/symbio_core/keys.rs:180`、`:227`；`symbio/src/symbio_core/creator.rs:29-35`

**为什么是问题**：没有一处能回答「这个插件依赖什么、需要什么权限、提供什么能力」。
更具体地说，三个清单之间**没有任何一致性检查**：

- `SYSTEM_AGENT_PLUGINS` 列了 `telegram`，但若 `telegram` 的工厂被删，装配期只会
  「扫描时跳过」，不会报错——**清单说要有、注册表说没有，无人核对**；
- `PLUGIN.yml` 的 `plugin_provider` 可以指向任意已注册工厂，与它所在目录名无关。

第三方插件无法自述的后果是：宿主只能靠**编译期常量**认识它——这又把 A1 的墙
加固了一层。

**改进方向**：把 `PLUGIN.yml` 从「配置」升级为「**清单（manifest）**」，承载
身份 + 依赖 + 能力声明（A4）；插件自己的配置字段仍归插件自己（现状不变）。
关键是不新增文件——`PLUGIN.yml` 已经在每个插件目录里，且「搬走目录 = 搬走插件」
这个不变量（`plugin_dir.rs:11-12`）要求清单必须与配置同处一个文件。

**代价**：需 ADR + 迁移（保留键从 3 个扩到一组）。

---

### A6｜`PluginMeta` 与 `PLUGIN.yml` 是两份身份，注释自承只有一处消费

**现象**：`Plugin::meta()` 返回 `PluginMeta`（含 `id`/`name`/`version`/`author`/…），
但 trait 注释**自承**：「唯一运行期消费方是 `composite/vdfs.rs`」「插件的身份实际
来自各自的 `PLUGIN.yml`」。

**位置**：`symbio/src/symbio_core/plugin.rs:301-312`（注释本身）；
`symbio/src/symbio_core/plugin_dir.rs:419-425`（`PluginEntry` 的 `title`/`version`
「来自 `PluginMeta`，未构造时为空」）

**为什么是问题**：同一事实两个来源，且其中一个是「刻意的但几乎没人用」。后果已经
显形：`PluginEntry` 的 `title`/`version` 依赖 `meta()`，而**停用的插件不被构造**
（A2）⇒ 那几个字段为空 ⇒ 消费方按目录名兜底。于是「插件名」在不同地方可能不同：
`PLUGIN.yml` 一个、`meta()` 一个、目录名一个。

注释说「若将来确认要删，请连同各 impl 一并清理，不要只删这一行声明」——说明这已被
识别为一个待决问题，只是没有决策。

**改进方向**：**二选一**，不要双源：

- 方案甲：身份统一来自 manifest，`meta()` 退役（只剩「挂载点呈现自述」那部分，
  如 `order`/`icon`/`hidden`/`root_access`，可以留在 trait 上）；
- 方案乙：manifest 不存身份，`meta()` 是唯一来源（但停用插件无身份的问题需解决）。

从「一个插件 = 一个目录、目录可整体搬走」的不变量看，**甲更自洽**：身份是目录的属性，
不是构造物的属性。

**代价**：需 ADR（`Plugin` 是公开契约）。

---

## 2. 协议层（P 系列）

### P1｜`PluginPayload::Native` 是死变体：协议写 4 态，实际 3 态可达

**现象**：`Native(Arc<dyn Any + Send + Sync>)` 在**全仓没有任何构造点**。唯二的匹配点
在 gateway，且都是「拒绝」语义。

**位置**：
- 定义：`symbio/src/symbio_core/transport.rs:149`
- 分支：`transport.rs:174`（`get`）、`:184`（`serialize`）、`:195`（`Debug`）——三处都是 Err / 占位
- 匹配点：`symbio/src/plugins/gateway/server.rs:438`
  （`Err("该路径返回进程内原生对象，不支持跨传输调用")`）、`:593`（WS 侧同款）
- 文档：`docs/architecture/PROTOCOLS.md:20,31`、`docs/architecture/OVERVIEW.md:20`

**为什么是问题**：协议文档把它写成「**4 态穷尽枚举**」的正式成员，还给了用途
（「进程内原生接口，不序列化，直接 downcast」）。实现者读到这里会以为存在一条
「用载荷返回进程内对象」的正式路径——**文档描述了一个不存在的世界**。而真相是：
进程内对象走的是 `ctx` 的扩展桶（`set_raw`），与 `PluginPayload` 无关。

这是「文档说的」与「代码做的」之间最直白的一道缝，且它**同时污染三处文档**
（`PROTOCOLS.md` 两处 + `OVERVIEW.md` 一处）。

**改进方向**：删除该变体（连带 3 处分支 + 2 处匹配 + 3 处文档）。若将来确有
「进程内原生载荷」需求，那是扩展桶的职责，不该占用协议层枚举——协议层枚举是
**跨进程可表达**的东西，而 `Arc<dyn Any>` 恰恰不是。

**代价**：**机械改进**（编译器会逐个报出所有匹配点，不会漏）。

---

### P2｜进程内载荷与线上载荷是两套枚举，映射逻辑在两个入口各写一遍

**现象**：`PluginPayload`（4 态，进程内）→ `PluginPayloadWire`（2 态，线上）的转换，
在 HTTP 入口与 WS 入口**各写了一遍**。

**位置**：
- 两套枚举：`symbio/src/symbio_core/transport.rs:142`（`PluginPayload`）、`:245`（`PluginPayloadWire`）
- 映射一（HTTP）：`symbio/src/plugins/gateway/server.rs:433-460`
- 映射二（WS）：`symbio/src/plugins/gateway/server.rs:548-606`

**为什么是问题**：同一个映射两份实现 ⇒ 新增 / 删除一个变体要改**三处**
（枚举 + 两个入口）。漏一处的后果是**不对称**：「HTTP 能调、WS 不能调」这类 bug
不会让任何测试变红（现有 e2e 只覆盖少数路径），只会在用户手里显形。

注意 `PluginPayload::Session` 在两处的处理**本来就不同**（HTTP 侧「消费到 EOF 取
最后一帧」；WS 侧「双向转发」）——这个差异是**真差异**（HTTP 无双向能力），
但「4 态 → 2 态的**分类**」是同一件事，应当只有一份。

**改进方向**：抽出唯一转换函数（如 `PluginPayload::to_wire(...)`），两个入口共用；
两处**真差异**（Session 的消费方式）作为参数或回调传入。P1 删掉 `Native` 后，
这个函数变成**穷尽且无拒绝分支**的纯映射，可读性显著提升。

**代价**：**机械改进**（**依赖 P1 先做**——否则要为一个将删的变体设计参数）。

---

### P3｜上下文键没有「可否跨进程」的类型级分类

**现象**：`SymbioKey` 的 `parse → None` 是「进程内专用」的**唯一**标记，而它是个
**约定**，不是可枚举的事实。当前 6 个键带这个标记。

**位置**（`symbio/src/symbio_core/keys.rs`）：

| 键 | 行 | Value 类型 |
|---|---|---|
| `PARENT` | `:100` | `Option<Weak<dyn Plugin>>` |
| `CAPABILITY_VISITOR` | `:259` | `Arc<dyn CapabilityVisitor>` |
| `OPTION_VISITOR` | `:276` | `Arc<dyn OptionVisitor>` |
| `CONFIG_VISITOR` | `:294` | `Arc<dyn ConfigurableVisitor>` |
| `EVENT_SINK` | `:326` | `EventSink`（闭包） |
| `ABORT_SIGNAL` | `:346` | `AbortSignal`（`CancellationToken`） |

**为什么是问题**：要判断「一次调用能否送到进程外插件」，得**逐个键去读注释**。
而这正是第三方插件体系的**准入判据**——它必须可枚举、可机检，否则每个新键都要靠
人记住「这个能不能跨进程」。

反过来看：现有键里有一批**天然可跨进程**的（`PATH`/`SESSION_ID`/`TRACE_ID`/…
17 个字符串键，加上 `CONFIG`/`PLUGIN_DIR`/`REQUIRED_PLUGINS`）——它们已经是
「线上可表达子集」的**事实成员**，只是这个子集从未被显式定义。

**改进方向**：给 `SymbioKey` 加一个关联常量（如 `const WIRE_SAFE: bool`），或按
trait 分层（`WireKey` / `LocalKey`）。目标：让「线上可表达子集」成为**类型事实**，
从而可以加守卫脚本（如「`route` 转发给外部插件的路径不得引用 `LocalKey`」）。

**代价**：需小设计（trait 加一个关联常量，波及 ~26 个键实现，机械）。

---

### P4｜错误分类已健全——列此以说明「可直接复用」

**现象**：`ErrorCode` 已是跨插件边界的错误分类**真源**，三段链路齐备：
生产侧 `PluginError::code` → 传输侧 `PluginError::to_frame` 写 `code.as_str()`
→ 消费侧 `PluginFrame::error_code` 解析回枚举。分派逻辑比较**枚举值**而非字符串。

**位置**：`symbio/src/symbio_core/error.rs:38-50`

**为什么列在这里**：**这不是缺陷**。评审要同时给出「可复用的资产」，否则改进方案会
重复造轮子。这一项的价值是：第三方插件体系**不需要**新建错误分类机制，只需复核
`ErrorCode` 的变体是否覆盖进程外场景（如「对端不可达」「协议版本不符」「初始化超时」）。

**改进方向**：无（进程外场景落地时补变体；`ErrorCode` 是 ABI，加变体需谨慎）。

**代价**：无。

---

### P5｜「native」一词两义，文档与代码撞车

**现象**：`gateway` 文档用「native」表示「不跨传输（进程内）」，而
`PluginPayload::Native` 表示「进程内原生对象」。**同名不同义**。

**位置**：`symbio/src/plugins/gateway/README.md:9`
（「本插件自身接口**恒走 native**」）

**为什么是问题**：P1 删除 `Native` 变体后，这个词在仓库里只剩 gateway 那一处的含义。
现在留着两种用法，读文档的人会以为它们相关——尤其 `gateway` 正是那个**唯一**
拒绝 `PluginPayload::Native` 的地方，两义在同一文件族里相遇，最易误读。

**改进方向**：gateway 文档里的「native」改为「in-process（进程内）」。
与 P1 一并做，则「native」这个词在协议层彻底消失。

**代价**：**机械改进**（纯文档）。

---

### P6｜已废弃的 `PAYLOAD` 键仍在三处活着，且有一处绕过常量

**现象**：`PayloadKey` / `PAYLOAD` 已标 `#[deprecated(since = "3.1.0")]`，指向
`ctx.payload::<T>()`。但：

1. 常量仍留在 `keys.rs`；
2. `plugin.rs` 用**裸字符串** `"payload"` 存取，**绕过常量**——正是 `grep-audit`
   的 S-009 想拦的形态（裸字面量）；
3. `PROTOCOLS.md` 仍把它列为标准上下文键。

**位置**：`symbio/src/symbio_core/keys.rs:71-97`；
`symbio/src/symbio_core/plugin.rs:171`（`let key = "payload";`）、`:208`；
`docs/architecture/PROTOCOLS.md:76`

**为什么是问题**：这是「废弃但仍在用」的**悬置状态**——既没删干净，又给了一个
反面示范（绕过常量）。更根本的是**标注名不副实**：`payload` 是**事实上的核心键**
（`ctx.payload()` 就靠它），废弃的是「用 `PAYLOAD` 键直接存取 `Value` 这种用法」，
不是「payload 这个概念」。给一个仍在核心路径上的概念贴 deprecated，会让人误以为
整条路都该弃用。

**改进方向**：把键名收进一个**非废弃**常量（如 `KEY_PAYLOAD`），`plugin.rs` 与
gateway 都引它；`PayloadKey` / `PAYLOAD` 删除；文档同步。

**代价**：**机械改进**。

---

### P7｜`PROTOCOLS.md`「新增插件只需」有两个「3.」

**位置**：`docs/architecture/PROTOCOLS.md:254-258`

**为什么是问题**：纯笔误——但这一节是「**怎么加一个插件**」的入口说明，正是
第三方插件作者会读的地方，编号错了会让人以为漏了一步（实际上原文把「加进
`SYSTEM_PLUGINS`」和「在配置里挂载」都编成了 3）。

**改进方向**：重编号。

**代价**：**机械改进**（纯文档）。

---

### P8｜「插件能提供什么」每加一个维度，就要加一个键 + 一套 traverse 约定（**扩展成本**）

**现象**：已有 3 条「宿主拉」的收集通道（能力 / 选项 / 可配置）+ 1 条执行出口，
各自一个 `ctx` 键、一套 `traverse` 约定、一处容器转发。

**位置**：`symbio/src/symbio_core/keys.rs:259`（`CAPABILITY_VISITOR`）、`:276`
（`OPTION_VISITOR`）、`:294`（`CONFIG_VISITOR`）、`:326`（`EVENT_SINK`）；
`docs/architecture/PROTOCOLS.md:90-118`（traverse 发现流程）

**为什么是问题**：这套「**宿主拉、插件同步回填**」模型对进程内插件非常优雅
（对称、无静态注册表、容器只做转发）。但它有一个隐含前提：**插件能在宿主的
`traverse` 调用栈里同步执行代码**。进程外插件不满足——它不能在宿主的遍历中
「被调用一下然后返回一份清单」。

于是：每加一个维度，就要再加一个键、一套约定、一处转发；而**这些新增的键对
进程外插件全都不可用**（P3）。

**改进方向**：**不是**合并 visitor（它们语义确实不同，合并会损失清晰度），而是为
外部插件引入**声明式对应物**：外部插件在初始化时**声明**自己提供的能力 / 选项 /
配置，宿主把这些声明**注入**到与内置插件相同的 visitor 管道里。

关键洞察：**声明是外部插件的入口，visitor 仍是宿主的唯一汇聚点**——汇聚点不变，
只多一个「声明的来源」。这样 P8 与 A1 是同一个动作的两面：A1 开放「谁来构造插件」，
P8 开放「插件怎么自述」。

**代价**：需 ADR（与 A1 同批）。

---

## 3. 改进清单（按可执行性分组）

### 第一组｜可机械改进（不改架构契约，编译器/守卫兜底）

| # | 改进 | 波及面 | 依赖 |
|---|---|---|---|
| **P1** | 删除 `PluginPayload::Native` 死变体 | 1 枚举 + 3 分支 + 2 匹配 + 3 文档 | — |
| **P2** | 收口「进程内载荷 → 线上载荷」为唯一转换 | gateway 两入口 | P1 |
| **A3** | 跨插件引用上移（`message_of_node` / `SEG_MESSAGES` → core） | 1 `use` + 2 符号 + 调用点 | — |
| **P6** | `payload` 键收进非废弃常量，删 `PayloadKey`/`PAYLOAD` | `keys.rs` + `plugin.rs` + gateway + 文档 | — |
| **P5** | gateway 文档「native」→「in-process」 | 1 文档 | P1（借势统一术语） |
| **P7** | `PROTOCOLS.md` 重编号 | 1 文档 | — |

这一组的共同点：**改动方向由既有事实唯一确定**，没有设计选择空间，验证靠编译
与守卫脚本。**建议本轮直接推进。**

### 第二组｜需小设计（改契约，但面窄、方向明确）

| # | 改进 | 波及面 | 产出 |
|---|---|---|---|
| **P3** | 给 `SymbioKey` 加「线上可表达」的类型级标记 | ~26 个键实现（机械）+ trait 1 处 | 一条可机检的准入判据 |

### 第三组｜需 ADR（架构决策，第三方插件体系的前置）

| # | 改进 | 与第三方插件的关系 |
|---|---|---|
| **A1** | provider 解析开放（内置 ∪ 外部） | **核心**——开放「谁来构造插件」 |
| **P8** | 外部插件声明式能力（注入 visitor 管道） | **核心**——开放「插件怎么自述」 |
| **A5** | `PLUGIN.yml` 升级为 manifest | 承载 A4 的声明与 A1 的外部标识 |
| **A4** | 插件级能力声明 + 宿主授予点 | 让第三方插件可被约束 |
| **A2** | 生命周期钩子 | 进程外插件的启动 / 停止 |
| **A6** | 身份单源（`meta()` vs manifest 二选一） | manifest 成为单一事实源的前提 |

**第三组的顺序不是任意的**：A6（身份单源）→ A5（manifest 成形）→ A4（声明能力）
→ A1 + P8（开放构造与自述）→ A2（生命周期）。前四项是**同一份 manifest 的四个
字段群**，天然一起设计；A1/P8 是**消费**这份 manifest 的两个方向。

---

## 4. 与第三方插件体系的关系

上面 14 条不是并列的，而是一棵树：

```
A1（「插件 = 编译期 Arc<dyn Plugin>」等式过窄）  ← 根
├── P8  外部插件怎么自述            （消费侧）
├── A5  manifest 承载什么           （载体）
│   ├── A4  能力声明
│   └── A6  身份单源
├── A2  进程外插件的生命周期        （伴随条件）
└── P3  线上可表达子集              （准入判据）

P1 / P2 / P5 / P6 / P7  ← 协议层的卫生问题，与第三方插件无关，但先修更省事
A3                       ← 契约自洽问题，独立
```

**一个结论**：第三方插件体系的**最小**前置是
**A6 + A5 + A4 + A1 + P8**（五条），A2 与 P3 紧随。其余（P1/P2/P5/P6/P7/A3）
是**顺路可修**的卫生问题——它们不解决也能上第三方插件，但会在实现时制造额外的
噪音（例如 `Native` 死变体让「载荷有几种形态」这个基础问题说不清）。

完整设计见 [design/third-party-plugin-spec.md](../design/third-party-plugin-spec.md)。
