---
kind: arch
id: AR-025-capability-registry
title: 能力统一注册表——CapabilityVisitor 六族 → 一个 Registry（3 方法）
status: active
created: 2026-10-17
deps: [PL-014-单环一表收敛, AR-016-turn-runner-steps]
---

# 能力统一注册表——CapabilityVisitor 六族 → 一个 Registry（3 方法）

状态：**设计文档，未施工**。它是 [PL-014 §5 步骤 4](../004-plan/014-单环一表收敛.md "PL-014-单环一表收敛") 的施工图，
回答开工前必须先答的三个问题，并把 PL-014 §4 里两处与现网契约不符的签名修正掉。

范围：`symbio_core` 的收集口（`CapabilityVisitor` / `OptionVisitor` / `ConfigurableVisitor`）+
三个默认收集器（`providers/collectors/`）+ 第二实现方（`SubAgentVisitor`）。**不含** 19 个工具的
`Capability::execute` 实现体。

## §1 判据：每一步 core 三格净下降，session 行数不增

| 判据 | 表达 | 现有棘轮 |
|---|---|---|
| core 形状只降不升 | `pub trait` 20 · `fn` 成员 73 · 默认体 25 | `core-shape-audit.mjs`（CS-001） |
| 收集口收敛 | `CapabilityVisitor` 16 方法 + `OptionVisitor` 2 + `ConfigurableVisitor` 3 ⇒ **1 个 trait，3 个方法** | 本文档 §2 的账 |
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
| Option | 2 | `OptionVisitor::register_option_field` · `list_option_fields`（**另一个 trait**） | `Option` |

> 基线修正：[T-165-控制面收敛](../009-task/165-控制面收敛.md "T-165-控制面收敛") 原记「14 方法」，
> 实测 16（含 3 个默认体）。已随本文订正。**4a 之后 trait 余 14 方法**（Tool 3 + Model 4 +
> Prompt 2 + Mount 5；默认体 3 → 2，`register_batch` 那个随 Tool 族一起退场）。

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
2. **「谁落格」现在有唯一答案**：调用方（`v2_exec` 收束时写）。改成执行体自己产 Fact，
   等于同时改 19 个实现的职责与网格的写方纪律——那是另一个批次，不是搬迁。
3. **`Vec<Fact>` 需要 `ExecError` 与事实网格的类型打通**，那是 core 的形状变更，
   必须走 CS-002 并有 ADR。迁移批次里夹带它，会把 300 行熔断线直接顶穿。

因此：**`Run` 本批只有 `Policy` 与 `Cap(Arc<dyn Capability>)` 两个变体**。`Exec` 签名在
**第一个真正需要 Store 的能力族**出现时再引入，届时它的 `&Store` 是 per-session 的
`EventWalStore`（与 `v2_exec` 今天持有的同一个），不会是全局 store。

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
| 4e | `OptionVisitor` 与 `ConfigurableVisitor` 一并合入（各 2 方法）；`RowKind` +1 值（`Configurable`） | 2 trait + 4 方法 | 0 | trait 20 → **18**，方法 60 → 56 |
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
| [T-162-ClassifyOnly快速档路径裁决](../009-task/162-ClassifyOnly快速档路径裁决.md) | 它的 `where` 含 `v2_exec.rs`，与 `policy.reflex` 的接线点相邻 | 冲突面仅邻行；`policy.classify` 按胶囊名派发，不受令牌接线影响 |
| [T-163-插件层特判审计守卫](../009-task/163-插件层特判审计守卫.md) | 它给插件层补审计脚本——「按名硬编码」的守卫正对着本表的 `RowKind` | 4a 落地时**同时**请它加一条「按名硬编码工具」的守卫，否则回流无人拦 |
| [T-160-v2文件前缀退役](../009-task/160-v2文件前缀退役.md) | `v2_exec` 等 8 个文件改名——`loop_registry` 的接线点在这些文件里 | 顺序无强约束；若 T-160 先动，`loop_registry` 的 `super::` 路径不变 |

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
   （4e 时按实测修订：`ConfigurableVisitor` 是 2 方法不是 3，且三方法要计入「加」）。
2. `providers/collectors/` 三个 visitor 收敛为一个 registry 实现；
   `SubAgentVisitor` 改为对同一张表的作用域投影。
3. 工具清单两种口径逐条相等，e2e 77 条全过，core 三格棘轮不升。
4. `session/loop_registry.rs` 从策略表长成策略 + 能力同表，行数不含工具实现体。
