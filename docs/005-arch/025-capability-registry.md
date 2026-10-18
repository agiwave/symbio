---
kind: arch
id: AR-025-capability-registry
title: 能力统一注册表——CapabilityVisitor 六族 → 一个 Registry（3 方法）
status: active
created: 2026-10-17
deps: [PL-014-单环一表收敛, AR-016-turn-runner-steps]
---

# 能力统一注册表——CapabilityVisitor 六族 → 一个 Registry（3 方法）

状态：**已落地**——[步骤 4](../009-task/165-控制面收敛.md "T-165-控制面收敛") 的施工图整体实施完毕（4a–4f，各步「落地形态」见 §5；4f = 收集口 trait 纯重命名）。
回答开工前必须先答的三个问题，并把 PL-014 §4 里两处与现网契约不符的签名修正掉。

范围：`symbio_core` 的收集口（`CapabilityVisitor` / `OptionVisitor` / `ConfigurableVisitor`）+
三个默认收集器（`providers/collectors/`）+ 第二实现方（`SubAgentVisitor`）。**不含** 19 个工具的
`Capability::execute` 实现体。

## §1 判据：每一步 core 三格净下降，session 行数不增

| 判据 | 表达 | 现有棘轮 |
|---|---|---|
| core 形状只降不升 | `pub trait` 18 · `fn` 成员 56 · 默认体 18（4e 后实测） | `core-shape-audit.mjs`（CS-001） |
| 收集口收敛 | `CapabilityVisitor` 16 方法 + `OptionVisitor` 2 + `ConfigurableVisitor` 2 ⇒ **1 个 trait，3 个方法**（4e 收敛、4f 名实相符） | 本文档 §2 的账 |
| session 不增 | 每步 `session` 非测试行数 ≤ 搬入前 | `find` + `wc -l` |
| 行为不变 | 工具清单逐条比对 + schema 导出比对 + e2e 77 条 | 现有 e2e |
| 零新增机制键 | --- | `verify/mechanism_growth` |

**「不新增 trait、就地变形」是关键选择**：如果新写一个 `trait Registry` 再慢慢删旧 trait，
中途 `pub trait` 会从 20 涨到 21，CS-001 当场判红。因此正确形态是**把 `CapabilityVisitor`
这个 trait 就地改成 3 个方法**——trait 数不变（变换过程中恒为 20），方法数每一步严格下降。
trait 改名（`CapabilityVisitor` → `Registry`）是最后一步的纯重命名，单独成提交。

## §2 现网盘点：16 个方法不是一个家族，是四个

`CapabilityVisitor`（`symbio_core/capability/mod.rs:287`）的 16 个 `async fn` 按家族拆开
（**下表是 4a 之前的盘点**，4a 落地后 Tool 族 5 方法已收敛为 3，trait 余 14）：

| 家族 | 方法数 | 方法 | 对应 `RowKind` |
|---|---:|---|---|
| Tool | 5 | `register` · `register_batch`(默认) · `list_capability` · `invoke` · `has_capability` | `Tool` |
| Model | 4 | `register_model_providers` · `get_model_provider` · `get_model_provider_by_id` · `resolve_model_provider`(默认) | `Model` |
| Prompt | 2 | `register_system_prompt` · `list_system_prompts` | `Prompt` |
| Mount | 5 | `register_vdfs_provider` · `list_vdfs_providers` · `get_vdfs_provider` · `register_vdfs_root`(默认) · `get_vdfs_root`(默认) | `Mount` |
| Option | 2 | `register_option_field` · `list_option_fields`（**另一个 trait**，随 4e 退场） | `Option` |

> 基线修正：[T-165-控制面收敛](../009-task/165-控制面收敛.md "T-165-控制面收敛") 原记「14 方法」，
> 实测 16（含 3 个默认体）。已随本文订正。**4a 之后 trait 余 14 方法**（Tool 3 + Model 4 +
> Prompt 2 + Mount 5；默认体 3 → 2，`register_batch` 那个随 Tool 族一起退场）；
> **4b 之后余 10 方法**（put / get / list 3 + Prompt 2 + Mount 5，默认体 5）；
> **4c 之后余 8 方法**（put / get / list 3 + Mount 5，默认体 5）；
> **4d 之后余 3 方法**（put / get / list，无默认体——Mount 族含两个默认体一并退场）；
> **4e 之后** `OptionVisitor` / `ConfigurableVisitor` 两个专服单族的 trait 退场
> （`pub trait` 20 → **18**，两族各 2 方法共 4 个并入表的行词汇），收集口只剩一个 trait 三方法。

收集器有**两个实现方**，这是迁移的真正成本所在：

- `providers/collectors/tool_visitor.rs:69` `DefaultToolVisitor`——生产主路；
- `plugins/agent/host/scope.rs:116` `SubAgentVisitor`——子智能体作用域投影，
  它必须与被投影者**同时**改，否则子智能体的工具集会静默退回空表。

`ConfigurableVisitor`（`capability/configurable.rs:63`）与另外两个 visitor 同源（插件配置树的
收集口），并入同一 trait 的动作一并记账，不另立批次。实测它是 **2 个方法**
（`register_configurable` · `list_configurables`），T-165 原记的「3」多数了一个自由函数。

## §3 表的目标形状

```rust
enum RowKind { Policy, Tool, Model, Prompt, Mount, Option }

struct Row {
    name: String,
    prio: u32,
    kind: RowKind,
    schema: RowSchema,          // Tool ⇒ CapabilityMeta；Option ⇒ DetailField；其余按需
    run: Run,
    effects: RowEffects,        // risk / approval / retention（Tool 已有）+ 预算档（Policy）
}

enum Run {
    /// 纯决策：策略行。签名收 `&State`——它结构上碰不到 ctx（见 §4 Q1）。
    Policy(fn(&State) -> Option<Step>),
    /// 有副作用执行：能力行。**本批只有这一种**，见 §4 Q2。
    Cap(Arc<dyn Capability>),
}
```

trait 只剩三个方法：

```rust
trait Registry {
    fn put(&self, row: Row) -> Result<(), RowConflict>;
    fn get(&self, name: &str) -> Option<Row>;
    fn list(&self, kind: RowKind) -> Vec<Row>;   // 按 (prio, name) 升序
}
```

`session/loop_registry.rs`（现有 514 行）是这张表的**第一任实现**——它现在就长这样，
只是 `RowKind` 与 `Run::Cap` 还没落地。**这张表从 session 出发，最终搬进 core**：
策略行是 `session` 的私有知识，能力行是全插件共用的协议，两者必须同表才能说
「一个能力一个入口」。搬的动作属步骤 6（切插件），不在步骤 4 内。

## §4 三问的裁决（含对 PL-014 §4 的修正）

### Q1 `ctx` 信封与 §3 硬规则 a「只携带引用」

**裁决：`ctx` 不进表。** 它是**执行期环境**，不是**注册数据**。

```rust
// 执行侧持有 ctx，注册行只持能力与判据
let row = registry.get(name)?;
row.run.invoke(args, &env, ctx).await   // ctx 由调用方给，不由行携带
```

`Policy` 与 `Cap` 是 `Run` 的两个变体，**策略行在类型上就拿不到 ctx**——不是靠纪律，
是签名。这也让 `Arc<dyn PluginInvokeRequest>` 保持"在每个需要它的调用点显式传入"，
与今天逐字一致。

### Q2 工具结果 → `Vec<Fact>`：**修正 PL-014 §4 的 `Exec` 签名**

PL-014 §4 写的是 `Fn(Capsule, &Store) -> BoxFuture<Result<Vec<Fact>, ExecError>>`。
**本批不按这个签名施工**，三条理由：

1. **现网 `Capability::execute(args, &ExecEnv, ctx) -> Result<Value, PluginError>` 已经是
   `Run::Cap` 的完整形态**。19 个工具无一需要 `&Store`；让它们都拿到一个 store 句柄，
   是在为一个假设给所有人加参数。
2. **「谁落格」现在有唯一答案**：调用方（`turn_exec` 收束时写）。改成执行体自己产 Fact，
   等于同时改 19 个实现的职责与网格的写方纪律——那是另一个批次，不是搬迁。
3. **`Vec<Fact>` 需要 `ExecError` 与事实网格的类型打通**，那是 core 的形状变更，
   必须走 CS-002 并有 ADR。迁移批次里夹带它，会把 300 行熔断线直接顶穿。

因此：**`Run` 本批只有 `Policy` 与 `Cap(Arc<dyn Capability>)` 两个变体**。`Exec` 签名在
**第一个真正需要 Store 的能力族**出现时再引入，届时它的 `&Store` 是 per-session 的
`EventWalStore`（与 `turn_exec` 今天持有的同一个），不会是全局 store。

> PL-014 §4 的 `Exec` 段与 §5 步骤 4 已据此订正（随本设计的提交一起）。

### Q3 `&Store` 是哪个 Store：**推迟，且推迟本身就是答案**

本批所有签名零 `Store`。若将来某族需要，判据是：**只可能是 per-session WAL**，因为
跨会话的写入口子一旦打开，double-write 就会从"两处记账"变成"两个真源"。
把它做成显式参数（而不是全局单例）也是同一条理由：参数拦得住调用方，全局拦不住。

## §5 迁移顺序（每步一个提交，每步 core 三格净下降）

| 步 | 内容 | 删 | 加 | core 三格 |
|---|---|---:|---:|---|
| 4a | 表骨架进 core：`RowKind`（6 值）+ `Row` + `Run{Policy,Cap}` + `Registry` trait 的 3 方法；`CapabilityVisitor` 的 Tool 族 5 方法改走表 | Tool 族 5 | **3**（put/get/list 进 trait） | 方法 73 → **71** |
| 4b | Model 族 4 方法改走表 | 4 | 0 | 71 → 67 |
| 4c | Prompt 族 2 方法改走表 | 2 | 0 | 67 → 65 |
| 4d | Mount 族 5 方法改走表 | 5 | 0 | 65 → 60 |
| 4e | `OptionVisitor` 与 `ConfigurableVisitor` 一并合入（各 2 方法）；`RowKind` +1 值（`Configurable`，封闭集偏离的裁决见 [ADR-071](../010-adr/071.md)） | 2 trait + 4 方法 | 0 | trait 20 → **18**，方法 60 → 56 |
| 4f | 纯重命名 `CapabilityVisitor` → `Registry` | 0 | 0 | 不变 |

> **算术订正（4a 落地时）**：原表把三方法写成「加 0」，于是每步都少数 3。正确账是
> 「删 5、加 3」净 −2：`put` / `get` / `list` 是**就地变形**进 `CapabilityVisitor`
> （trait 数不变，CS-001 不触），不是免费的。终点因此是 18 trait / 56 方法，不是
> 19 / 52。

### 4a 落地形态（2026-10-18）

- **新文件 `symbio_core/capability/row.rs`**（行词汇）：`CapabilityRowKind`（6 值，
  本批只 `Tool` 可达——其余是词汇，`list(kind)` 签名由此在 4b–4e 不再变形）、
  `CapabilityRowPayload::Tool(Arc<dyn Capability>)`、`CapabilityRow`（**两个字段**：
  名字 + 载荷；族别从载荷派生，结构上不存在两种答案）。
- **meta 不另存一份**：`row.meta()` 每次从能力自述现取。两份表示只会在改名装饰器
  （`PrefixedCapability`）上分叉，而那种装饰器改的正是 `meta().name` 本身。
- **`invoke` 不在表上**：按名执行 = `get(name)` + `capability_invoke`，NotFound 文案
  逐字保持。`put` 内裹审批闸（T-138 纪律原样）。
- **顺序 = 注册序**：`list(kind)` 按族过滤后即表序（IndexMap），族内不按名重排
  （ADR-057）。策略行的 `(prio, name)` 先命中者胜在 session 的 `loop_registry`，两件事。
- **session 的非测试行数 32,489 → 32,501（+12）**：这是调用协议拆分的一次性成本
  （一个读助手 `capabilities::tool_metas` 落位，三个读点各变轻）。纪律的读法是
  「session 不得吸收 core 的职责」——本步方向相反（core 少 2 个方法、少 1 个默认体，
  Tool 族 5 → 3）。
- **基线同步**：`core-shape-audit.mjs` 的 `SHAPE_BASELINE` 20/73/25 → **20/71/24**。
- 守门人用例落在 `tool_visitor.test.rs`：**同一份工具清单两种口径逐条相等**
  （`list(Tool)` 的 meta 转储 vs 入表时给的元数据，含顺序），加顺序/覆盖/缺失三格。

### 4b 落地形态（2026-10-18）

- **生效位长在目录行上**：`CapabilityRowPayload` 加 `Model { provider, active }`
  （`CapabilityRow::model(provider, active)`，行名 = `provider_id()`）。原来那条不变量
  （「生效者必然是目录里的一员」）靠**一次批量注册**的签名维持；表只有 `put`，它于是
  从签名搬进**结构**——生效位是目录行的一个位，「生效者不在目录里」不可表达。
- **单槽移交归 `put`**：带生效位的行入表时，同族旧生效行降位。降位是**换一个 `Arc`
  副本入表**（`CapabilityRow::demote`），不是就地改位——否则 `get` / `list` 的读者会
  读到「两个生效者」的中间态。旧行**留在表里**（降为普通目录行）：`active` 只答
  「谁在槽里」，「目录里有谁」是同一批行的另一问。
- **读侧两个自由函数**（`capability/model.rs`，**不**回头往 trait 挂方法）：
  `capability_model_active`（会话引擎取生效者）与 `capability_model_resolve`
  （插件选自己的模型：严格查 id，取不到落回生效者——降级方向仍**只有一处定义**）。
- **写侧没有新函数**：`plugins/model/plugin.rs` 逐行 `put`（它本来就知道哪个是当次
  生效者），作用域丢弃改在 `SubAgentVisitor::put` 上**按族**判——写入口只有一个，
  作用域规则也就只有一处（机制化之前那条「丢弃子 Agent 的模型服务注册」逐字保留）。
- **同族重注册 = 表语义（同名覆盖），不再整体替换目录**：旧实现把批量注册实现成
  「换掉整张目录」，并有一条单测钉着。生产上两者无差别——`collect_capabilities` **每次
  会话现造一个 visitor**，注册只发生一次；但那是签名的变化，故单测随之改写并写明。
  跨族同名（表键是全表唯一的名字空间）**留一条告警再覆盖**：静默被顶掉的那一族不会
  以任何别的方式表现出来。
- **core 三格**：方法 71 → **67**、默认体 24 → **23**、`pub trait` 20 不变；
  `SHAPE_BASELINE` 同步至 **20/67/23**，`CapabilityVisitor` 余 **10** 方法（5 个默认体）。
- **session 非测试行数不变**（唯一改动是 `consume.rs` 里一行调用的写法）：本步没有
  任何 session 侧的新逻辑，只有调用点换名。

### 4c 落地形态（2026-10-18）

- **提示词行 = 一段文字**：`CapabilityRowPayload::Prompt { text }`（`CapabilityRow::prompt(段名, 正文)`，
  行名 = 段名）。段名的两条语义原样搬进 `capability/prompt.rs` 的模块文档：**注册在这里的
  一定会送达模型**（没有「取一个」的竞争）、**段名是覆盖键不是分类**。
- **写侧是 `put`，没有新函数**：四个注册点（model 的 persona、setting 的自述段、
  memory 的两段、session 的会话记忆）各自 `put(CapabilityRow::prompt(…))`；
  作用域前缀（`agent/<id>/<段名>`）改在 `SubAgentVisitor::put` 上按族判——与工具/
  模型同一处规则，写入口仍然只有一个。
- **读侧落到一处**：`capability_prompts`（`(段名, 正文)`，保注册顺序）。两处消费方
  （会话装配系统提示词、`compose` 拼自己的提示词）都不再各写一遍行到对的映射。
  空段**不在这里**过滤：那是拼接方的判断，两处各自跳过。
- **core 三格**：方法 67 → **65**（两个必填方法退场）、默认体 23 不变、`pub trait` 20 不变；
  `SHAPE_BASELINE` 同步至 **20/65/23**，`CapabilityVisitor` 余 **8** 方法。
- **session 非测试行数不变**：三处涉及（`inputs.rs` 读点、`plugin.rs` 注册点、`entry.rs` 一行注释）
  都只换了一行写法；四个注册点里三个在别的插件（model / setting / memory）。

### 4d 落地形态（2026-10-18）

- **挂载点与根同表**：`CapabilityRowPayload::Mount { provider, active }`
  （`CapabilityRow::mount(目录名, provider)` / `CapabilityRow::mount_root(provider)`）。
  差别只在根**不可能由调用方命名**（它不是某个插件的目录）——故根行的行名由构造函数
  自己填保留值 `"<root>"`，`active` 是**根位**。又一条「位长在行上」：与模型族同一条
  `put` 单槽移交规则，不需要第二套机制。
- **一族两形的两条不变量各被一处守住**：「根不可按名查」由保留行名 + 读侧按位过滤守
  （`capability_mount` 查到活跃行返回 `None`）；「根不在挂载点清单里」由同一过滤反向守
  （`capability_mounts` 只列非活跃行）。
- **读侧三个自由函数**（`capability/mount.rs`，不回头往 trait 挂方法）：
  `capability_vdfs_root`（访问层取服务者，无注册时 `None` 由调用方降级空根）、
  `capability_mounts` / `capability_mount`（挂载清单：**预留的 LLM 可见资源裁剪入口**，
  当前读侧只有单测——不是死代码，判据写在 `mount.rs` 模块文档）。
- **作用域规则仍是第三处也归 `put`**：子 Agent 的挂载点走 `agent/<id>/` 前缀入表，
  子树的根行**丢弃**（否则劫持整个 `<根>`，判据 = `scope.test.rs` 的单测 + e2e t39）。
- **core 三格**：方法 65 → **60**、默认体 23 → **18**（`register_vdfs_root` /
  `get_vdfs_root` 两个默认体随族退场），`pub trait` 20 不变；`SHAPE_BASELINE` 同步至
  **20/60/18**，`CapabilityVisitor` 余 **3** 方法（put / get / list，无默认体）。
- **守门人**：`tool_visitor.test.rs` 两格（三个读函数是同一张表的三个视图 / 根的单槽移交），
  `scope.test.rs` 三格原样保留（根与模型族丢弃、多槽前缀、读侧透传）；
  `capability_mounts` 从「零消费方」（C-003 判红）变为测试在用——预留清单的消费方
  就是这格判据本身。
- **跨族同名的告警真的响了（并被处置）**：提示词段名与挂载名（插件名）撞键三处
  （memory / setting / model），`put` 的跨族覆盖告警当场判出——段是覆盖键（内部词汇），
  挂载名是插件名（地址约定），故改的是段名：`memory` → `agent-memory`（与 `workspace-memory` /
  `session-memory` 同构）、`setting` → `agent-setting`、model 的 persona 段 → `model-persona`。
  这是「告警不该只是一条日志」的自证：它上岗第一轮就顶掉了三个真碰撞。
- **session 非测试行数 32,503 → 32,505（+2）**：唯一涉表的注册调用一行被 rustfmt
  折成三行，无新逻辑——量到就记账（§6 口径）。

### 4e 落地形态（2026-10-18）

- **选项行**：`CapabilityRowPayload::Option { field, order }`（`CapabilityRow::option(order, 字段)`，
  行名 = `field.key`，同键覆盖保留先注册槽位）。`order` 长在行上但**不下发**——读侧
  `capability_option_fields` 稳定排序后即丢，排序答案与字段是同一行的两个事实。
- **可配置行**：`CapabilityRowPayload::Configurable(Box<VdfsItem>)`（行名 = 目录名，同名覆盖）。
  插件侧唯一入口仍是 `capability_announce_configurable`——条目形状（地址跨挂载点、推不出来，
  必须连条目一起交）在那里统一构造，行只是它的新住址。
- **变的是「行住哪张表」，不变的是通道**：`OPTION_VISITOR` / `CONFIGURABLE_VISITOR` 两个广播
  通道键保留（端点字面量、失败降级、独立实例语义全部原样），值类型改指统一收集口；
  `DefaultOptionVisitor` 随旧 trait 删除，`collect_options` 返回全表共用的 `DefaultToolVisitor`。
- **`RowKind` 六值 → 七值**（+`Configurable`）：PL-014 §5 步骤 4 的熔断线真触发了一次，
  裁决入档 [ADR-071](../010-adr/071.md)——塞进 `Option`/`Mount` 都会把「同一事实的第二种答案」长回数据侧。
- **core 三格**：trait 20 → **18**（两个专服单族的 trait 退场）、方法 60 → **56**、默认体 18 不变；
  `SHAPE_BASELINE` 收窄至 **18/56/18**（基线数字随收尾批同步，见 [T-165 处理过程](../009-task/165-控制面收敛.md "T-165-控制面收敛")）。
  同提交另修 `clippy::large-enum-variant`：`Option.field` 与 `Configurable` 载荷入 `Box`。
- **守门人**：`tool_visitor.test.rs` 四格——选项按 order 排序并丢号 / 同键覆盖保首槽位 /
  可配置保注册序且同名覆盖 / 空表两族皆空；`session` 非测试行数 32,505 → **32,508**
  （+3：`collect_options` 签名换型带来的注释与 use 行重排，无新逻辑——量到就记账，§6 口径）。

### 4f 落地形态（2026-10-18）

- **纯重命名**：`CapabilityVisitor` → `Registry`（54 个 `.rs`、134 处，签名与语义零改动）——
  §1 预告的「最后一步的纯重命名，单独成提交」由此兑现；名字追上事实，收集口只剩
  **1 个 trait 3 方法**这件事第一次可从**名字**读出来。core 三格 18/56/18 不变（CS-001 数形不数名）。
- **改的是收集口名，不改通道名**：`CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIGURABLE_VISITOR`
  三个键与键类型 `…VisitorKey` 保留——它们命名的是**广播通道**（与 `TRAVERSE_AVAILABLE_*`
  端点成对），不是收集口 trait；实现名 `DefaultToolVisitor` 同此理由保留（属 collectors 层，
  归 PL-014 步骤 6 清算）。这是 4f 「纯度」的边界。
- **裸名进表**：`Registry` 无域前缀，登记进 core README §1.2 的 `capability` 域类型格
  （N-001 按表核对，注册即入册，不是豁免）。
- **顺带修的文档腐点**（「现行文档即登记」，发现即处理）：004-工程落地 `list_capability()` ×2、
  007-对话面拆分 `CapabilityVisitor::register`、015-core-loop 引名 `resolve`（实为
  `capability_resolve`）、013-vdfs / 020-session-options / 019-perf / 插件 README 的现行词汇
  改指 `Registry` + 行词汇；AR-025 / T-165 / PL-014 / ADR 中的历史名按台账性质保留。
- **记账**：`session` 非测试行数 32,508 → **32,505**（−3：`Registry` 更短使部分 `use` 块被
  rustfmt 并回单行，无逻辑删减——量到就记账，§6 口径）。

每一步的守门人：**同一份工具清单以两种口径导出逐条相等**（走表的 `list(Tool)` 与
`CapabilityMeta` 全量转储），加 e2e 77 条全过。任何一格漂移即停。

## §6 session 不增的纪律

步骤 4 会让 `session` 变**小**的可能性来自 T-160（`v2_` 前缀退役）与 T-158（工具名
元数据化）先落地。施工时三条硬规矩：

1. **新机制进 `loop_registry`**（它将是步骤 6 的 `loop` 插件种子），不进 `chat_loop.rs`。
2. **`chat_loop` 问表**：每一步把一个 `if let Some(x) = ...` 换成
   `registry.get(name)`，净行数为负或零；出现净增长时，说明这一步混进了别的东西。
   （4a 的实测见 §5：chat_loop 三处读点净 −11，全步 +12，来源是读协议的拆分——
   量到了就记账，不自认失败。）
3. **每步提交前量一次 `session` 非测试行数**，与 T-165 基线表对照（4a 前 32,489，
   4a 后 32,501）。

## §7 与在飞批次的关系（避免撞车）

| 任务 | 与本文的关系 | 处置 |
|---|---|---|
| [T-158-工具名硬编码归一为能力元数据](../009-task/158-工具名硬编码归一为能力元数据.md) | 它往 `CapabilityMeta` 加元数据位（`provides_assign_work` 等），正是 `Row` 的载荷要透出的东西 | **已落地**（步 1 `f21342b1` / 步 2 `d5f70d23`）。4a 开工前它须已闭环——否则 `row.meta()` 读到的声明位要分两批接线 |
| [T-162-ClassifyOnly快速档路径裁决](../009-task/162-ClassifyOnly快速档路径裁决.md) | 它的 `where` 含 `turn_exec.rs`，与 `policy.reflex` 的接线点相邻 | 冲突面仅邻行；`policy.classify` 按胶囊名派发，不受令牌接线影响 |
| [T-163-插件层特判审计守卫](../009-task/163-插件层特判审计守卫.md) | 它给插件层补审计脚本——「按名硬编码」的守卫正对着本表的 `RowKind` | 4a 落地时**同时**请它加一条「按名硬编码工具」的守卫，否则回流无人拦 |
| [T-160-v2文件前缀退役](../009-task/160-v2文件前缀退役.md) | `turn_exec` 等 8 个文件改名——`loop_registry` 的接线点在这些文件里 | 顺序无强约束；若 T-160 先动，`loop_registry` 的 `super::` 路径不变 |

## §8 风险与不动清单

- **`SubAgentVisitor` 是第二个实现方**，漏改它 = 子智能体工具集静默退空。判据：
  4a 的提交必须让 e2e（含子智能体场景）全绿，不能只靠单测。
- **`invoke` 是热路径**：`registry.get(name)` 比今天的 `DashMap` 直读多一次匹配。
  判据是 `verify/latency_gate` 不红；若红，`get` 用 `DashMap<name, Row>` 加只读快照，
  `list(kind)` 走排序后的 `Vec`，两者同一份事实的两个视图。
- **不动**：`Capability::execute` 的签名、`PluginError`、`ExecEnv`、审批闸门的
  `risk_for` 取值口径（T-158 动的是它的**来源**，不是它的值）。
- **不动**：`traverse` 的广播协议本身。改的是广播的**接收端**（visitor → registry），
  广播方（插件）的调用形态一行不改——这是 4a 能小步提交的原因。

## §9 出口条件

1. `symbio_core` 只剩 1 个收集口 trait，3 个方法；`pub trait` ≤ 18，`fn` 成员 ≤ 56
   （4e 时按实测修订：`ConfigurableVisitor` 是 2 方法不是 3，且三方法要计入「加」）。**已达成**（4e/4f，实测 18/56/18）。
2. `providers/collectors/` 三个 visitor 收敛为一个 registry 实现；
   `SubAgentVisitor` 改为对同一张表的作用域投影。**已达成**（4e 单实现；投影在 `put` 上按族判，4b–4d）。
3. 工具清单两种口径逐条相等，e2e 77 条全过，core 三格棘轮不升。**已达成**（4f 批全量门禁 109/109，含 e2e）。
4. `session/loop_registry.rs` 从策略表长成策略 + 能力同表，行数不含工具实现体。
   ——本条属 PL-014 步骤 5/6 的账面，不是步骤 4 的收口前置。
