# 插件路由地址规则与入口审计

> **文档类型：Design（设计）** — 地址规则的定义，以及 2026-09-18 对
> `Plugin` trait 两个分形入口（`route` / `traverse`）的审计记录。
>
> 守卫脚本：[`scripts/plugin-entry-audit.mjs`](../../scripts/plugin-entry-audit.mjs)
> （规则 E-001 ~ E-006，随 `scripts/gate.mjs` 阶段 3 运行）。

## 1. 为什么需要这份规则

`Plugin` trait 只有三个方法（`symbio_core/plugin.rs`）：

```rust
pub trait Plugin: Send + Sync + 'static {
    fn meta(&self) -> PluginMeta;
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload>;
    async fn traverse(self: Arc<Self>, path: String, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload>;
}
```

`route` 与 `traverse` 是**全仓仅有的两条**跨插件寻址通道，而它们的地址是**字符串**
——字符串不会因为改名而编译失败，只会静默地指向一个不存在的地方。
HTTP 传输文档里那句「路由是运行时分形分发，各插件内部 `match path`，**无静态注册表**」
（[http-api-transport.md](./http-api-transport.md) §7）说的就是这个代价。

代价具体长什么样，2026-09-18 的审计给出了三个实例（见 §3）。它们**没有任何一个**
会让测试变红，全都是靠人工核对发现的。这份文档把规则写清楚，脚本把规则变成可执行的。

## 2. 地址规则（四条）

### 规则一：地址只有两种形态

| 形态 | 形状 | 谁写它 | 谁读它 |
|---|---|---|---|
| **绝对地址** | `<插件目录名>/<子路径>` | 调用方（`ctx.set(PATH, …)` 后过 `route`） | 容器剥首段 → 目标插件的 `match` |
| **相对臂** | `<子路径>` | 目标插件自己的 `route` 体内 `match` | 容器剥完首段后剩下的那截 |

两者的关系是**一次剥离**。所以：

- 插件里的 `match` **只写相对臂**（`"chat/send"`、`"fire"`）；
- 跨插件调用**只写绝对地址**（`"session/chat/send"`、`"hook/fire"`）。

反过来的两种写法都是 bug：插件内匹配绝对地址永远不命中；调用侧写相对臂在容器层就丢了。

### 规则二：前缀是**插件目录名**，不是 `PluginMeta`

容器（`composite`）扫描插件根下的**一层目录**建实例表，键就是目录名
（`composite.rs` 注释原文：「目录名 = 实例名」），`route` 也按它分发。
`SYSTEM_PLUGINS` 清单里写的也是目录名（`"hook"`，不是 `"hooks"`）。

因此 **`PluginMeta::new` 的首参（`id`）必须等于插件目录名**。
它不是路由前缀——`Plugin::meta()` 全仓**无生产消费方**（`PluginMeta` 事实上是只写字段），
但生成器与文档会拿它当名字用，不一致就会产出幽灵路由。

### 规则三：过路由才设 `PATH`；直连方法**不设**

`PATH` 是给**容器**看的。判据很简单：**下一跳是 `route()` 还是方法调用**。

- `parent.route(ctx)` / `root.route(ctx)` ⇒ 设绝对地址；
- `self.handle_xxx(…)` ⇒ **不设** `PATH`（设了没有任何读者，是死赋值）。

编排侧对此已有明确口径：`orchestrator/entry.rs` 写着「chat_ctx 仅承载 payload
（不再设置跨插件 PATH）」。

### 规则四：`traverse` 的 `PATH` 只有两个合法值

`TRAVERSE_AVAILABLE_TOOLS`（`symbio_core/mod.rs`）与
`TRAVERSE_AVAILABLE_OPTIONS`（`symbio_core/option.rs`）。
它们是**协议端点**，不是插件路径，因此不进 `symbio_core::paths`。

处理哪个、不处理哪个由插件自己决定：不贡献选项的插件对 `available_options`
返回 `NotFound` 是**正确**行为（`collect_options` 会忽略该子树的错误）。

### 规则五：跨插件调用**经父容器**，不按值持有兄弟实例

调用另一个插件时，`ctx.parent()` 取容器、再用**绝对地址** `parent.route(ctx)`
（规则三）。入口容器早就塞好了：`composite.rs` 挂载子插件时用
`SimpleRequest::new(Some(composite_weak), None)`，而 `fork()` 保留整个 extensions 桶
（含 `PARENT`），所以任意深度的子 ctx 都拿得到。

**不要**把兄弟插件当字段存起来（`llm_plugin: Arc<dyn Plugin>`）。两个代价：
① 绕过容器的地址分发——直接调 `session.route(ctx)` 拿到的是绝对地址，
对方的 `match` 只认相对臂，必然 `NotFound`；② 插件重建后钉住旧实例。
守卫 **E-007** 盯这个形态。

允许的两种持有：`Weak<dyn Plugin>`（向上引用父）与 `HashMap<String, Arc<dyn Plugin>>`
（容器按名持有子实例）。可运行先例：`agent/host/subagent.rs` 的三处 `parent.route(ctx)`。

### 常量放在哪

| 位置 | 收什么 |
|---|---|
| `symbio_core::paths` | **有真实调用方的绝对地址**（`&'static str`），命名 `<PLUGIN>_<OP>` |
| 前端 `tauri/src/constants/pluginPaths.ts` | 同一批地址的前端侧常量（模板串链，含 `worker/` 前缀） |
| 定义它的模块（如 `schemas::options::OPTIONS_LIST`） | **相对臂**常量 |

`paths.rs` **不为「将来可能用到」的路由预置常量**——那正是它在 2026-09-18 清掉的
那类腐烂（见 §3.2）。

## 3. 审计发现的四处漂移

### 3.1 `hook` 的幽灵命名空间（已修）

`hook/plugin.rs` 写的是 `PluginMeta::new("hooks", "钩子插件")`——与目录名 `hook`、
工厂 id `PLUGIN_HOOK` 都**不一致**。

**它不影响运行**（`Plugin::meta()` 无人读），但它足以让
`scripts/gen-current-facts.mjs` 与三处文档写出 **`hooks/fire`、`hooks/list`、
`hooks/register`** 三条**不存在的路由**：

- `docs/CURRENT.md` §1 的自有路由列（生成器按 `PluginMeta` 首参渲染前缀）；
- `docs/reference/ROUTES.md` §Hook 插件；
- `symbio/src/plugins/hook/README.md`。

而真正在用的 `symbio_core::paths::HOOK_FIRE = "hook/fire"` 是对的——
**代码是对的，文档是错的**，两者长期并存。

修复：① 首参改为 `PLUGIN_HOOK`；② 生成器的路由前缀改用**目录名**（防御，即使
首参再写错也不会污染生成物）；③ 三处文档改正并记录原因。

### 3.2 `paths.rs` 里的两个幽灵常量（已删）

`AGENT_CHAT = "agent/chat"` 与 `AGENT_CREATE = "agent/create"` **全仓零调用方**，
而且描述的路径**根本不存在**——`agent` 插件的 `route` 恒返回 `NotFound`
（agent 目录一律经 VDFS 访问），子智能体派生走的是 `session/chat/send`。

`AGENT_CHAT` 的文档还写着「子智能体会话执行入口（仅 `agent_run` 能力内部调用）」
——而没有任何代码那样调用。`chat_pipeline.rs` 的注释里留着线索：
「**重构前**，会话的工具集由 agent 插件在 `agent/chat` 路由里独家装配」。
它们是被重构淘汰后忘了删的常量。

修复：删除两个常量，并把「不为将来的路由预置常量」写进 `paths.rs` 的模块文档。

### 3.3 Telegram 的 `session/chat`（已修：地址 + 注入路径）

`telegram/plugin.rs` 用 `SESSION_CHAT = "session/chat"` 路由——**该路径不存在**。
session 的 `route` 只认 `chat/send` 与 `chat/abort` 两条相对臂，所以那处调用
**必定**落到 `_ => NotFound`。

**地址修好之后，功能仍然不通**——真正的病灶在调用方式上，分两层：

1. **`llm_plugin` 字段从未被写入。** 它只在 `handle_start_listener(Some(plugin), …)`
   里赋值，而唯一的调用点 `invoke_start_listener` 传的是 **`None`**。于是
   `process_update` 里 `if let Some(ref plugin) = *llm` 永远不成立，
   每条消息都被回成「LLM 插件未配置」。`git log` 显示这个形态**从初始提交
   （`e00832d`）就是这样**：这条链路自始至终没通过。

2. **病灶是「按值持有兄弟插件」这个设计本身**（而不是漏传了一个参数）。
   它违反地址规则：跨插件调用必须**经父容器**——`telegram` 与 `session` 在
   worker 下平级，只有容器的 `route` 认识绝对地址 `session/chat/send`。
   按值持有还有第二个代价：插件重建后钉住的是**旧实例**。

   容器其实早就把入口塞好了：`composite.rs` 挂载子插件时用
   `SimpleRequest::new(Some(composite_weak), None)`，`fork()` 又保留 `PARENT`
   （`plugin.rs:206`），所以 `ctx.parent()` 一直可用。仓库里的**可运行先例**
   是 `agent/host/subagent.rs`（三处 `parent.route(ctx)` + 绝对地址常量）。

修复：删掉 `llm_plugin` 字段与 `handle_start_listener` 的注入参数，
`process_update` 改为 `ctx.parent()` → `parent.route(ctx)`。
守卫 **E-007** 盯住这个形态（§5）。

> 顺带纠正上一轮的一个**错误结论**：当时把 `telegram/*` 六条读成「休眠」，
> 理由是「零调用方」。**零调用方 ≠ 不可达**——网关会把外部 `path` 原样转发给
> 容器 `route`（`gateway/server.rs` 的 `dispatch_once` / `handle_ws`，
> 只过一层只读白名单），对外 API 天然是 `refs=0`。见 §4.2。

### 3.4 `session/heartbeat.rs` 的死赋值（已删）

```rust
let ctx = SimpleRequest::new(None, None);
ctx.set(PATH, "chat/send".to_string());   // ← 删掉
ctx.set(SESSION_ID, session_id.to_string());
…
self.handle_chat_send_oneoff(Arc::new(ctx)).await   // 直连方法，不过路由
```

两处都不对：`PATH` 没有任何读者（规则三），而且值还是**相对臂**（规则一）。
`entry.rs` 早已明说编排不再设跨插件 PATH。

## 4. 审计结果：路由表与消费方

`refs` = 代码里对该绝对地址的引用数（字面量 + 解析到它的常量，定义处不计）。
**`refs=0` 不是判决**，有两条理由，第二条很容易被漏掉：

1. 运行期拼路径（工具名、子插件名、`VDFS_OPS`）数不出来；
2. **网关是对外入口**：`gateway/server.rs` 把请求体里的 `path` 原样交给
   `router.route(ctx)`（`dispatch_once` / `handle_ws`，只过一层只读白名单），
   所以**对外 API 天然是 `refs=0`**。

它的用途是**指出需要人工判断的位置**。

### 4.1 `route` 侧

| 插件 | 分派形态 | 路由 | refs |
|---|---|---|---|
| `agent` | 恒 `NotFound` | — | — |
| `composite` | 运行期动态（子插件名） | — | — |
| `event_bus` | 静态 | `event_bus/subscribe` | 3 |
| | | `event_bus/pending/snapshot` | **0** |
| | | `event_bus/ping` | **0** |
| `gateway` | 静态 | `gateway/status` | **0** |
| `home` | 静态 | `home/reload` | 1 |
| | | `home/get_homedir` | 2 |
| | | `work/set_workspace` | 1 |
| | | `work/get_workspace` | 2 |
| `hook` | 静态 | `hook/fire` | 4 |
| | | `hook/register` | **0** |
| | | `hook/list` | **0** |
| `local` | 运行期动态（工具名） | — | — |
| `mcp` | 恒 `NotFound` | — | — |
| `model` | 恒 `NotFound` | — | — |
| `session` | 静态 | `session/chat/send` | 9 |
| | | `session/chat/abort` | 5 |
| | | `session/get_messages` | 4 |
| | | `session/update` | 9 |
| `setting` | 恒 `NotFound` | — | — |
| `skill` | 静态 | `skill/execute` | **0** |
| `telegram` | 静态 | `telegram/send` · `get_updates` · `set_chat_id` · `start_listener` · `stop_listener` · `status` | **全 0** |
| `vdfs` | 运行期动态（`VDFS_OPS`） | — | — |
| `web` | 运行期动态（工具名） | — | — |
| `work` | 恒 `NotFound` | — | — |

**`home` 的两条 `work/*` 臂值得单说**：`work/set_workspace` 与 `work/get_workspace`
挂在 **`home`** 的 `route` 里（`home` 是系统根，不挂前缀，臂已是全名），
与 `work` **插件**的命名空间重名。`work` 插件的 `route` 恒 `NotFound`，
所以不冲突——但这条「同名不同主」的事实只存在于代码里，本表是它唯一的文字记录。

### 4.2 「零消费方」路由的性质各不相同

先记住 §4 的前提：**`refs=0` 只说明仓内没有调用方**，不代表不可达（网关是对外入口）。

| 路由 | 判断 |
|---|---|
| `event_bus/pending/snapshot` | **有意保留**：前端已不再调用（见 `node-state-streaming.md`），但它是**网关对外 API** 的一部分。删它属另一件事。 |
| `event_bus/ping` | 疑似**未接线**：全仓（含文档）零提及。 |
| `gateway/status` | 疑似**未接线**：`gateway` 的入站端点（`/api/v1/*`）有真实用户，但这条 `route` 没有。 |
| `hook/register` · `hook/list` | 疑似**未接线**：钩子目前只有「触发」在用（`hook/fire`），注册走的是别处。 |
| `skill/execute` | **重复实现**：`SkillExecuteTool`（`skill_tool.rs`）直接读 `self.skills` 完成同一件事，不经过这条路由。这是「同一能力两份实现」的典型形态。 |
| `telegram/*`（6 条） | **对外 API，本轮已修复**：原先因 §3.3 的 `llm_plugin` 恒 `None`，每条消息都被回成「LLM 插件未配置」。地址与注入路径已修（§3.3）。 |

除 `telegram/*` 外，其余**本轮一律未动**：删路由是能力取舍（网关对外 API /
未接线的插件面），不属于「地址规则规范化」的范围。脚本把它们报出来，判定留给人。

### 4.3 `traverse` 侧

| 端点 | 处理它的插件 |
|---|---|
| `available_tools` | 除 `event_bus` / `gateway` / `home` / `hook` / `setting` 外的全部 |
| `available_options` | `agent` · `model` · `session` |

`traverse` 侧**没有发现漂移**：13 个插件的 `route` 里没有一处对端点写字面量
（E-004 全绿），端点集合也全部落在两个合法值内。

## 5. 守卫：`scripts/plugin-entry-audit.mjs`

| 编号 | 规则 | 级别 |
|---|---|---|
| E-001 | `PluginMeta::new` 首参 == 插件目录名 | ERROR |
| E-002 | 代码里路径字面量的首段必须是插件目录名（或容器前缀 `worker`） | ERROR |
| E-003 | `set(PATH, "<字面量>")` 一律违规 | ERROR |
| E-004 | `traverse` 内不得出现 `available_tools` / `available_options` 字面量 | ERROR |
| E-005 | 引用的路径必须对应到某条真实 `route` 臂 | WARNING |
| E-006 | **权威清单**（`ROUTES.md` / `CURRENT.md` / 插件 README）里的路径前缀必须合法 | WARNING |
| E-007 | 插件不得按**强引用**持有兄弟插件实例（`Arc<dyn Plugin>` 字段） | ERROR |

ERROR 判据是 airtight 的（不依赖任何白名单），因此可以进 `--strict`；
WARNING 需要「动态命名空间」白名单配合，宁可先报给人看。

E-007 有四条豁免，逐条对应仓里的真实形态：`Weak`（向上引用父）·
`HashMap`/`Vec`/`BTreeMap`（容器按名持有多个子实例）· 字段名 `parent`/`router`
（本仓专指向上引用）· 类型以 `&` 开头（借用，字段不可能是这个形态 ⇒ 必是形参）。

### 为什么 E-006 只判**前缀**，不判整条路径是否存在

权威清单里**必须**能提到已退役的路由（`session/append`、`model/chat`、
`telegram/config/get`…），否则迁移记录就无从写起。第一版把「整条路径必须存在」
也加上，结果 22 条告警**全部**是这类正当的历史提及。前缀判据没有这个问题：
退役路由的前缀仍然是对的（`session/append` 的 `session` 合法），
而 `hooks/fire` 的 `hooks` 一眼就是错的。

### 已知边界

判定基于**去注释后的文本**，不是 AST。因此运行期拼出来的路径看不见（列在
`DYNAMIC_NAMESPACES` 里，E-005 对它们不判定）；变量别名与 `concat!` 能绕过。
它挡的是「顺手写回去」，不是「刻意绕过」。

## 6. 与既有文档的关系

- [`docs/reference/ROUTES.md`](../reference/ROUTES.md) 是**人工维护**的路由清单，
  本审计的 E-006 就是守它的（`CURRENT.md` 由代码生成，必然一致，不需要守）。
- [`symbio_core/paths.rs`](../../symbio/src/symbio_core/paths.rs) 的模块文档是
  地址规则的**权威表述**，本文 §2 是它的展开。
- [`http-api-transport.md`](./http-api-transport.md) §7 讨论了「给 `Plugin` trait
  加 `manifest()` 自动聚合路由清单」的方案。若将来落地，本文 §4.1 的表格可以由
  它生成；届时 E-006 的判据也会从「人工清单」换成「manifest 与代码一致」。
