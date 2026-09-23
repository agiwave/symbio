# 插件路由地址规则

> **文档类型：Design（设计）** — `Plugin` trait 两个分形入口（`route` / `traverse`）的地址规则。
>
> 守卫脚本：[`scripts/plugin-entry-audit.mjs`](../../scripts/plugin-entry-audit.mjs)
> （规则 E-001 ~ E-007，随 `scripts/gate.mjs` 的 docs 阶段运行）。

## 1. 为什么需要这份规则

`Plugin` trait 只有三个方法（`meta` / `route` / `traverse`，签名见 [PROTOCOLS.md](../architecture/PROTOCOLS.md)）。

`route` 与 `traverse` 是**全仓仅有的两条**跨插件寻址通道，而它们的地址是**字符串**
——字符串不会因为改名而编译失败，只会静默地指向一个不存在的地方。
HTTP 传输文档里那句「路由是运行时分形分发，各插件内部 `match path`，**无静态注册表**」
（[http-api-transport.md](./http-api-transport.md) §7）说的就是这个代价。

这类错误**没有一个**会让测试变红，全靠人工核对发现。这份文档把规则写清楚，脚本把规则变成可执行的。

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
| 定义它的模块 | **相对臂**常量（不带 `worker/` 前缀）；新增时照此落位，别塞进 `paths.rs` |

`paths.rs` **不为「将来可能用到」的路由预置常量**。

## 3. 守卫：`scripts/plugin-entry-audit.mjs`

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

## 4. 与既有文档的关系

- [`docs/reference/ROUTES.md`](../reference/ROUTES.md) 是**人工维护**的路由清单，
  E-006 就是守它的（`CURRENT.md` 由代码生成，必然一致，不需要守）。
- [`symbio_core/paths.rs`](../../symbio/src/symbio_core/paths.rs) 的模块文档是
  地址规则的**权威表述**，本文 §2 是它的展开。
- [`http-api-transport.md`](./http-api-transport.md) §7 讨论了「给 `Plugin` trait
  加 `manifest()` 自动聚合路由清单」的方案。若将来落地，E-006 的判据会从
  「人工清单」换成「manifest 与代码一致」。
