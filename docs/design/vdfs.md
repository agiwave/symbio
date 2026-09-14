# VDFS —— 虚拟动态文件系统规范

状态：现行规范（核心接口 + 访问层/拓扑分层）
范围：Symbio 全部「资源与数据访问」能力（前端 + 大语言模型）
关联：
`symbio/src/symbio_core/vdfs_provider.rs`（**纯接口，权威定义：`VdfsProvider` trait
+ 域类型 + 路径工具 + 容器组合视图 `VdfsMountTable`**）、
`symbio/src/symbio_core/vdfs/host.rs`（symbio 桥：上下文注入 + 错误翻译）、
`symbio/src/plugins/vdfs/protocol.rs`（线路信封：`vdfs/*` 请求响应 + 协议路径）、
`symbio/src/plugins/vdfs/host.rs`（**访问层：取根 + 翻译操作 + 树遍历 + 事件投递**）、
`symbio/src/plugins/composite/vdfs.rs`（**拓扑：聚合子插件挂载 + 注册为 VDFS 根**）

> 本文件只写**规范与机制**。具体挂载点（会话、模型、设置分区、工作目录……）
> 一律属于**范例**（§13），不是机制的组成部分；挂载点的增删改不影响本规范的
> 效力，机制演进也不以任何单个挂载点为准。

---

## 1. 目标与不变量

**一个协议、一个页面机制、一处注册**——把「项目里所有的资源与数据」收敛为
同一棵虚拟树上的节点，让**前端与大语言模型用同一组操作、同一种寻址方式**访问：

```
                    ┌────────────── 消费者 ──────────────┐
                    │  前端（导航 / 列表 / 详情 / 表单）   │
                    │  LLM（vdfs_* 工具）                 │
                    └────────────────┬──────────────────┘
                                     │ 同一组 vdfs/* 操作 · 同一份载荷
                          ┌──────────▼──────────┐
                          │  vdfs 插件（访问层） │  取根 → 转发全路径
                          └──────────┬──────────┘
                                     │ get_vdfs_root()
                                     │ （与工具 / 模型 / 提示词同一次广播）
                          ┌──────────▼──────────┐
                          │  composite（容器）   │  虚拟根 = 组合视图
                          │  VdfsMountTable      │  子插件 = 一级子目录
                          └──────────┬──────────┘
                                     │ register_vdfs_provider(插件名, provider)
        ┌───────────────┬─────────────┼─────────────┬───────────────┐
   setting provider  session provider  model provider   …（各模块自持）
```

不变量（不可违反）：

1. **一个协议**：所有资源只用 `vdfs/*` 一组操作访问，**不得**新造私有资源协议
   （`entities/*` 等既有协议在迁移期保留，但不作为新功能的接入面）。
2. **provider 自持**：数据操作、状态、校验、呈现描述全部由实现方负责；
   机制**只认访问位**（§4），不做任何按类型的特判。
3. **路径是唯一地址**：节点地址 = `/<挂载点>/<相对路径>`，无第二种寻址方式。
4. **接口与宿主解耦**：VDFS 接口定义**不依赖任何宿主类型**（§2）。
5. **同一集合**：LLM 能访问的资源集合与前端能访问的集合**恒等**——二者都归结到
   同一个 VDFS 根（§6），不存在「前端有而 LLM 没有」的资源。
6. **挂载是使用方的概念**：provider **不知道也不提供挂载名**；挂载名由注册方
   选定（约定 = 插件名）。
7. **拓扑归容器**：虚拟根 `/` 由**组合容器**（`composite`）拥有并注册；
   访问层（`vdfs` 插件）与 provider **都不持有**拓扑知识（§2.5）。

## 2. 分层与开放边界

### 2.1 centerpiece = 纯接口

VDFS 的核心是**一个包含全部资源操作的接口**——`VdfsProvider` trait。它**不是**
一组请求 / 响应结构；请求响应只是它的线路投影。

这与既有的 provider 组织方式一致（见 `symbio_core/model_provider.rs` 的模块文档）：
**core 只暴露纯 object-safe trait**，协议适配与线路形状留在插件内部。

```
symbio_core/vdfs_provider.rs   ← centerpiece：VdfsProvider trait
                                  + 域类型（VdfsNode / VdfsContent / VdfsAccess /
                                    VdfsWriteResponse / VdfsError / VdfsChange）
                                  + VdfsContext + 路径工具
                                  + VdfsMountTable（容器组合视图，通用机制）
symbio_core/vdfs/host.rs       ← symbio 桥：上下文注入 + 错误翻译
plugins/vdfs/protocol.rs       ← 线路信封：vdfs/* 请求响应 + 协议路径常量
                                  + VdfsMountInfo / VdfsChangeEvent（使用方形状）
plugins/vdfs/host.rs           ← 访问层：取根 + 翻译操作 + 树遍历 + 事件投递
plugins/composite/vdfs.rs      ← 拓扑：聚合子插件挂载 + 注册为 VDFS 根
```

trait 上收拢全部操作（列 / 读 / 写 / 删 / 建 / 移 / 订阅），未实现者保持默认
（`NotImplemented`）。因此「一类资源 = 实现一个 trait」——新增资源不触碰线路层、
不触碰访问层，也不需要考虑自己被挂在哪里（§2.4）。

### 2.2 分层与依赖

| 层 | 文件 | 允许依赖 | 说明 |
|---|---|---|---|
| 纯接口（centerpiece） | `symbio_core/vdfs_provider.rs` | `std` / `serde` / `serde_json` / `async_trait` | `VdfsProvider` trait、域类型、`VdfsContext`、路径工具、回填、`VdfsMountTable` |
| 宿主桥 | `symbio_core/vdfs/host.rs` | 宿主自有 | 上下文注入、`VdfsError` ↔ `PluginError` |
| 线路信封 | `plugins/vdfs/protocol.rs` | 上面两层 | `vdfs/*` 请求 / 响应、协议路径常量与 `VFDS_OPS`、使用方形状（`VdfsMountInfo` / `VdfsChangeEvent`） |
| 访问层 | `plugins/vdfs/host.rs` | 宿主自有 | 取根、翻译操作、树状遍历、事件总线投递 |
| 拓扑 | `plugins/composite/vdfs.rs` | 宿主自有 | 逐子插件收集挂载、组合为根、注册为 VDFS 根 |

硬约束：

- **纯接口层不得出现 `use crate::…`**。`vdfs_provider.rs` 可原样抽出为独立
  crate，供任何宿主（不限于 symbio）复用。
- **core 不暴露线路类型**：`VdfsPathRequest` / `VdfsListResponse` /
  `VdfsMountInfo` … 只存在于 `plugins/vdfs/protocol.rs`，core 的任何 `pub` 面都
  不得出现它们。这是与 `model_provider` 一致的分层（core = 纯 trait；协议适配在插件）。
- **trait 上不得出现挂载相关成员**（`mount()` / `mount_info()` 之类，§2.4）。
- **宿主专有状态只能经 `VdfsContext` 注入**：它是一个不透明袋子
  （`Arc<dyn Any + Send + Sync>`），provider 用 `ctx.require::<T>()` 取回。
  新增宿主服务**不需要改动接口**。
- 错误类型由接口定义（`VdfsError`），不沿用任何宿主错误枚举；宿主在桥层
  做一次性翻译。
- 呈现描述（如表单定义）作为**不透明 JSON** 放在节点的 `schema` 字段里：
  VDFS 只透传、不解释，因此接入方可自由选择方言（JSON Schema、自有表单定义……）。

### 2.3 实现方为什么依赖 core 而不是插件

实现方（如 `setting` 插件）是**插件**而非宿主本身，它需要两件宿主能力：
从 `VdfsContext` 取回宿主句柄、把下游 `PluginError` 翻回 `VdfsError`。
这两件事放在 `symbio_core::vdfs::host`（桥层），使实现方**只依赖 core**，
不去依赖 `plugins/vdfs`——否则插件之间就产生了横向依赖。访问层（`dispatch`）
不是实现方需要的，因此留在 vdfs 插件内部。

### 2.4 挂载名不属于 provider

**挂载是使用方的概念**：谁使用 provider，谁决定它在虚拟树上的名字。因此：

- trait 上**没有** `mount()`——provider 既不管理也不提供挂载名；
- 挂载名在注册时由注册方给出：`register_vdfs_provider(mount, provider)`，
  **约定用插件名**（`PLUGIN_*` 常量）。插件名在宿主内唯一，天然是合格的挂载名，
  无需另立命名机制；
- provider 的方法只接收**自身子树内的相对路径**（`""` = 自身根）；
  全路径 `/<挂载名>/<rel…>` 由使用方（容器）拼接与回填；
- 挂载根节点、`vdfs/providers` 的使用方视图（`VdfsMountInfo`）都由使用方合成，
  provider 不参与；
- 变更事件（`VdfsChange`）同样不含挂载名——provider 只报子树内相对路径，使用方
  补挂载名并拼成全路径（使用方形状 `VdfsChangeEvent`，见 §9）。

好处：provider 实现者不必先想一个名字、也不必担心重名；同一个实现可被挂在
任何位置；换使用方策略（改名字、改顺序）不需要触碰任何 provider。

### 2.5 拓扑只归容器

「谁挂在哪儿」只由**组合容器**知道。容器实现 `VdfsProvider`，把自己聚合出的
根视图注册到 `CapabilityVisitor` 的**单槽根位**：

- **容器**（`composite`）＝ 唯一持有拓扑的角色：它逐个子插件收集挂载、组装成
  `VdfsMountTable`，并 `register_vdfs_root(...)`；
- **访问层**（`vdfs` 插件）＝ 只认「根 + 全路径」：`get_vdfs_root()` 取根，把
  `vdfs/*` 翻译成对根的方法调用。它**不认识任何挂载点**，因此「新增挂载点」
  不需要改访问层；
- **provider**（各插件）＝ 只认自身子树内相对路径。

于是拓扑知识的落点**有且只有一处**（容器），且容器不依赖访问层、访问层不依赖
任何具体资源——三者可以独立替换（§10.3）。

## 3. 路径与节点模型

### 3.1 路径

- 虚拟根：`/`——其目录内容 = **挂载点清单**（由容器的组合视图合成）。
- 节点：`/<mount>/<rel…>`，如 `/session`（挂载根）、`/setting/session`（节点）。
- 规范化由**访问层**在调用根之前完成（`normalize_path`）：折叠空段与前导斜杠、
  拒绝 `..`。**根与 provider 收到的永远是已规范化的路径**，无需重复校验。
- `split_mount` / `join_path` 是路径的唯一构造与拆分入口；首段 = 挂载名的拆分
  只发生在容器（`VdfsMountTable`）内。

### 3.2 节点

`VdfsNode` 是文件与目录的统一表达：

| 字段 | 语义 |
|---|---|
| `path` / `name` | 全路径（使用方回填） / 父内唯一标识（路径段） |
| `title` / `description` | 展示标题 / 语义说明（缺省 `title` = `name`） |
| `kind` | **场景**类型（`dir` / `file` / `mount` / 场景自定义）；机制不据此判定 |
| `status` | `active` / `working` / `disabled` / `error` / `unknown` |
| `access` | 访问位（§4），**机制唯一的能力依据** |
| `ext` | 呈现扩展名——**前端据此选择详情页面**（§7） |
| `size` / `updated_at` / `children` / `binary` | 元数据 |
| `schema` | 不透明呈现描述（宿主方言，VDFS 透传） |
| `attributes` | 场景扩展字段（flatten 到顶层） |

**目录与文件不做类型区分**：`access` 含 `l` 即可列（目录），含 `r` 即可读
（文件）。`is_dir()` 的实现就是 `access.list`。

### 3.3 内容

`VdfsContent` 承载文本或二进制（互斥）：`text` 或 `b64`，由 `binary` 显式标注，
不做猜测；`size` / `mime` / `etag`（乐观并发令牌，可选实现）。

**二进制写入 = 整包导入**：目录型资源（skill / agent bundle…）以新建类型
`ext = zip` + `source = file` 声明「内容来自本地文件」，使用方选文件后走
`vdfs/write { create: true, b64 }`；provider 把它解释为**导入一个完整目录包**
（语义自持，VDFS 不解释）。因此导入不额外占一个操作（详见
[vdfs-frontend.md](vdfs-frontend.md) §5 与 §7 的 S12）。

**导出是导入的逆动作**：它不新增第二个操作，而是一个**节点动作**——
`vdfs/action { action: "export" }`，zip 随 `VdfsActionResult.data` 回传
（载荷 `{id, filename, b64}`，与 `VdfsContent.b64` 同构）。provider 不支持
时返回 `NotImplemented`，使用方据此不给出入口；支持与否由 provider 自陈，
与「新建类型」同理（详见 [vdfs-frontend.md](vdfs-frontend.md) §7 的 S13）。

## 4. 访问位（r / w / l / t）

访问位是**机制唯一的能力依据**，线上表示为紧凑字符串（按 `r` `w` `l` `t` 顺序）：

| 位 | 语义 | 典型 |
|---|---|---|
| `r` | 读取内容 | 文件 |
| `w` | 写入内容 | 文件 |
| `l` | 列出直接子节点 | 目录 |
| `t` | 树状递归遍历 | 目录 |

- `"rl"` = 可读 + 可列；`"rw"` = 可读可写；`"wlt"` = 可写可列可遍历；`""` = 不可访问。
- 预置组合：`VdfsAccess::{READ, READ_WRITE, LIST, LIST_TRAVERSE, LIST_WRITE_TRAVERSE}`。
- **树状遍历 `t` 是声明式的**：provider 只需实现 `list` 并在目录节点上声明 `t`，
  访问层即可递归展开（`vdfs/tree`）；provider **不需要**实现遍历逻辑。
- 消费者（前端 / LLM）在动作前应依据访问位判定可做性，**不得**按 `kind` 猜测。

## 5. 协议操作

`plugins/vdfs/protocol.rs` 的 `VFDS_OPS` 是唯一操作清单（**线路信封只存在于插件内，
core 不暴露**）：

| 操作 | 请求载荷 | 响应 | 说明 |
|---|---|---|---|
| `vdfs/providers` | `{}` | `VdfsProvidersResponse` | 挂载点清单（= 根的 `list("/")` 的使用方视图） |
| `vdfs/list` | `{path}` | `VdfsListResponse` | 列目录；`/` 返回挂载点 |
| `vdfs/tree` | `{path, depth?, limit?}` | `VdfsTreeResponse` | 递归遍历（只下钻 `t` 位目录） |
| `vdfs/stat` | `{path}` | `VdfsNode` | 读元数据 |
| `vdfs/read` | `{path}` | `VdfsContent` | 读内容（`r`） |
| `vdfs/edit` | `{path, old_string, new_string?}` | `VdfsEditResponse` | 精确字符串替换（**访问层组合操作**：`read` → 替换 → `write`） |
| `vdfs/search` | `{path?, pattern}` | `VdfsSearchResult` | 文件名 Glob 搜索（**访问层组合操作**：递归 `list` + 过滤） |
| `vdfs/write` | `{path, text\|b64, create?, etag?}` | `VdfsWriteResponse` | 写内容（`w`） |
| `vdfs/delete` | `{path, recursive?}` | `VdfsDeleteResponse` | 删除 |
| `vdfs/mkdir` | `{path}` | `VdfsWriteResponse` | 新建目录 |
| `vdfs/move` | `{from, to}` | `VdfsMoveResponse` | 移动 / 重命名（同挂载点内） |
| `vdfs/watch` | `{path}` | `SuccessResponse` | 订阅该子树变更 |
| `vdfs/unwatch` | `{path}` | `SuccessResponse` | 取消订阅（与 watch 配对） |
| `vdfs/action` | `{path, action, payload?}` | `VdfsActionResponse` | 执行**节点动作**（provider 自持动词；未实现返回 `NotImplemented`） |

**每个操作都是对「根 provider」的一次方法调用**（`vdfs/providers` = `list("/")`；
`vdfs/unwatch` = `unwatch`），访问层不含任何资源语义。唯一由访问层自身实现的是
`vdfs/tree`——它按 `t` 位递归调用根的 `list`，属于**机制级**通用能力，不是场景逻辑。

机制级守卫分两处（provider 都无需重复）：

**访问层统一施加**（`plugins/vdfs/host.rs`，与资源语义无关）：

- 路径穿越 → `ValidationError`；
- `write` 必须携带 `text` 或 `b64`；
- 响应中的 `path` 缺省回填；`ext` 缺省由 `name` 推导；`title` 缺省 = `name`。

**根（容器组合视图）统一施加**（`VdfsMountTable`，与资源语义无关）：

- 虚拟根 `/` 不是可操作节点（读 / 写 / 删 / 移一律拒绝）；
- 挂载根不可读 / 写 / 删 / 移动，也不可 `mkdir`；
- `move` 跨挂载点被拒（只允许同一挂载点内）；
- 子节点 / 内容 / 写入响应的 `path` 回填为全路径；
- 变更事件的相对路径补全为全路径。

错误经 `VdfsError` 表达，桥层翻译为宿主错误（symbio → `PluginError`）。
`VdfsError::Invalid(VdfsValidationError)` 的结构化字段级错误序列化为 JSON 置于
错误文案位，前端可解析后逐字段高亮；解析失败按纯文本展示（向前兼容）。

## 6. 注册与收集

### 6.1 provider 注册（各插件 → 容器）

`CapabilityVisitor` 上的三方法（均有默认实现，不破坏既有实现方）：

```rust
// mount = 使用方选定的挂载名（约定 = 插件名），provider 自身不含此概念
async fn register_vdfs_provider(&self, mount: &str, provider: Arc<dyn VdfsProvider>);
async fn list_vdfs_providers(&self) -> Vec<(String, Arc<dyn VdfsProvider>)>;  // order 升序
async fn get_vdfs_provider(&self, mount: &str) -> Option<Arc<dyn VdfsProvider>>;
```

插件在 `traverse` 的 `TRAVERSE_AVAILABLE_TOOLS` 分支里，**在注册工具的同一处**
顺带注册自己的资源，挂载名用插件名：

```rust
if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
    // 工具…
    let me: Arc<dyn VdfsProvider> = self.clone();
    visitor.register_vdfs_provider(PLUGIN_SETTING, me).await;   // 插件名即挂载名
}
```

### 6.2 根注册（容器 → 系统）

虚拟根是**单槽**，由组合容器占据：

```rust
async fn register_vdfs_root(&self, provider: Arc<dyn VdfsProvider>);   // 重复注册覆盖
async fn get_vdfs_root(&self) -> Option<Arc<dyn VdfsProvider>>;        // 无容器时 None
```

`composite` 在**同一次** `TRAVERSE_AVAILABLE_TOOLS` 广播里注册根：

```rust
if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
    if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
        visitor.register_vdfs_root(self.vdfs.clone()).await;   // 组合视图 = 虚拟根
    }
}
```

### 6.3 组合视图：容器如何聚合子插件

`VdfsMountTable`（core，通用机制）把「一串挂载」组合成以 `/` 为顶的子树，并
**它本身就是一棵可用的 provider**，因此容器可以直接把它注册为根。

容器**逐个子插件**广播一次收集、每个子插件配一个**独立**收集器：

```rust
for child in children {
    let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
    let sub = host.fork();
    sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    sub.set(CAPABILITY_VISITOR, visitor.clone());
    child.clone().traverse(String::new(), sub).await?;
    mounts.extend(visitor.list_vdfs_providers().await);   // 必然属于该子插件
}
Ok(VdfsMountTable::new(mounts))
```

一次广播把整棵树收进同一个收集器，就无法区分某个 provider 是谁注册的；
**逐子插件 + 独立收集器**保证挂载名天然归属于注册它的那个子插件。

### 6.4 一次广播的语义，两条消费链路

**不新增第二条收集通道**。资源与工具、模型服务、系统提示词**共用同一次
能力广播**：

- **LLM 链路**：会话编排方 `collect_capabilities` → 广播 → 容器注册根、
  各插件注册 provider 进入 `CAPABILITY_VISITOR` → 工具执行时 `tool_ctx` 携带该
  访问器 → `vdfs_*` 工具经 `get_vdfs_root()` 取根。
- **前端链路**：`vdfs` 插件收到 `vdfs/*` 请求后调
  `host::resolve_root(parent, ctx)`：`ctx` 无能力管理器时，它从父插件广播
  **同一次** `traverse(TRAVERSE_AVAILABLE_TOOLS)`，容器在该广播中把根注册进新的
  收集器，随即取回。

两条链路因此归结到**同一个根**（不变量 5）——不存在第二处拓扑来源。
取不到根（无组合容器）时降级为**空文件系统**（空 `VdfsMountTable`）：`/` 可列出
但无内容，具体路径 `NotFound`。于是「系统没有资源」与「资源为空」表现一致，
前端与 LLM 都不必特判。

### 6.5 上下文携带

大语言模型工具调用时，宿主在 ctx 中挂载 `CAPABILITY_VISITOR`
（会话侧 `attach_capabilities`，工具执行侧 `tool_ctx = ctx.fork()`）。
`vdfs_*` 工具据此取得"本次调用实际可用的资源树"，因此**新增资源不需要
改动 vdfs 插件或工具集**。

上下文分两层：

| 层 | 形态 | 用途 |
|---|---|---|
| **宿主句柄** | `VdfsContext::host::<H>()`（类型化 downcast） | 宿主专有对象（如能力管理器） |
| **调用级参数** | `VdfsParams`（`serde_json::Map` + 约定键） | 运行时状态（如 `workdir`） |

调用级参数是**开放约定**而非类型化槽位：键名由使用方与 provider 用共享常量对齐
（`VFDS_PARAM_WORKDIR` = `"workdir"`），因此**新增一个约定参数不需要改接口**，
资源语义也不会渗进 `VdfsProvider`。翻译只在访问层发生一次
（`host::call_params`：宿主 ctx 的 `WORKDIR` → 约定键 `workdir`），provider 因此
**不认识宿主 ctx 的键名约定**。

`host::dispatch_with(root, path, ctx, params)` 是唯一分发入口；不需要参数时传空的
`VdfsParams`。两条链路（前端协议入口 / LLM 工具）都经它，因此"前端能做什么"与
"LLM 能做什么"始终同源。

## 7. 前端机制：ext 决定详情页面

后端下发的 `ext`（扩展名）是**前端选择详情渲染器的唯一键**：

| `ext` | 渲染器 | 说明 |
|---|---|---|
| （目录，`l` 位） | 列表 / 树 | 中栏结构由访问位与 `t` 位决定 |
| `form` | 定义驱动表单 | 解析 `node.schema`（不透明 JSON，symbio 方言 = DetailDefinition） |
| `session` | 会话工作区 | 实时对话 |
| `md` / `json` / `text` | 文本类编辑器 | 内容经 `vdfs/read` 获取 |
| 其他 | 通用兜底 | 只读呈现 + 通用文本视图 |

约定：

- 扩展名**缺省由 `name` 推导**（`a.md` → `md`），provider 可显式覆盖
  （设置分区名无扩展名，显式声明 `form`）。
- 前端只持有**纯 UI 映射**（ext → 组件、挂载点 → 图标），不硬编码资源类型、
  标签、能力或路径模板。分层：`schemas/vdfs.ts`（数据契约，零组件知识）→
  `registry/vdfsTypes.ts`（ext → 渲染器标识，零组件导入）→
  `registry/vdfsRenderers.ts`（标识 → 组件，唯一装配点）。
- 页面：`views/VdfsView.vue`（路由 `/vdfs/:mount?`，可选 `mount` 支持深链），
  页面逻辑全部在 `composables/useVdfs.ts`；左栏 = 挂载点、中栏 = 当前目录、
  右栏 = 按 `ext` 分发的渲染器。中栏当前为列表形态（点目录逐级进入，配面包屑）；
  服务层已绑定全部 11 个操作（含 `vdfs/tree` 与 `vdfs/stat`），树视图与
  二进制预览留待后续接入，不影响机制。
- 由此，整个前端逐渐收敛为「**通用资源状态查看 / 管理工具**」：
  新增一类资源只需后端新增一个 provider，前端零页面开发。

## 8. 校验职责

**校验属于 provider，不属于机制**：

- 机制只做与资源语义无关的守卫（§5）。
- provider 在 `write` 内完成必填 / 范围 / 枚举 / 类型 / 跨字段校验，失败返回
  `VdfsError::Invalid(VdfsValidationError { message, fields })`。
- **校验先于副作用**：校验不过时 provider 不得触达被代理的下游写入
  （设置模块在转发 `config/set` 之前完成校验）。
- 前端据 `fields[].field` 逐字段提示，与 `schema` 中的字段键对齐。

## 9. 实时性

- provider 在 `watch(path, sink)` 中开始监听，变化时调用 `sink(VdfsChange)`
  上报；`unwatch` 严格配对。
- **provider 报出的 `VdfsChange` 不含挂载名**：`path` / `to` 都是该 provider 子树
  内的**相对路径**（与 `list` / `stat` 同一坐标系）。挂载名是使用方的概念（§2.4），
  provider 无从得知。
- 消费能力由访问位与 provider 实现共同决定；默认 `watch` 为 no-op（无实时能力
  的 provider 直接成功，消费者退回手动拉取）。
- 宿主侧投递分两跳，**各补一次它那层才知道的信息**：
  1. **容器**（`VdfsMountTable::watch`）把 provider 的相对路径**补成全路径**
     （`join_path(mount, …)`）后交给上层 sink；
  2. **访问层**（`plugins/vdfs/host.rs`）从全路径**取回顾首段作为挂载名**，
     映射为使用方形状 `VdfsChangeEvent`（`mount` + 全路径），发布到全局事件总线
     （`kind = "vdfs"`，无会话关联、不入回放缓冲）。

  前端 `subscribe({ kind: 'vdfs' })` 据此按挂载名过滤、按 `path` 精确刷新。
- **禁止轮询、禁止私有刷新通道**；细粒度帧不得触发列表刷新。

## 10. 扩展指引

### 10.1 新增一类资源（= 新增一个挂载点）

1. 为模块实现 `VdfsProvider` —— **没有必填方法**：
   - 自描述按需：`label`（缺省由使用方以挂载名代替）/ `description` / `order`
     / `icon` / `root_access` / `root_status`；
   - 数据操作按需实现 `list` / `stat` / `read` / `write` / `delete` / `mkdir`
     / `move_item` / `watch` / `unwatch`；未实现者保持默认（`NotImplemented`）。
   - **不需要、也不应该提供挂载名**（§2.4）——`impl VdfsProvider for X {}`
     即可编译。
2. 在插件 `traverse` 的 `TRAVERSE_AVAILABLE_TOOLS` 分支中
   `register_vdfs_provider(挂载名, provider)`；挂载名用插件名（`PLUGIN_*`）。
3. 校验写在 `write` 里；呈现需求放在节点的 `ext` + `schema`。

**零改动面**：访问层（`plugins/vdfs/*`）、前端（导航 / 列表 / 树 / 详情）、
容器（`composite`）都无需改动——容器逐子插件收集，新挂载点自动出现在根下。
仅当需要一类全新的详情渲染器时才在前端登记一个 ext → 组件映射。

**迁移期的唯一例外**：LLM 工具链路把地址软编码到 `local`（§13.3），因此新挂载点
**不会**自动进入大模型的可用地址空间——把多个挂载点暴露给大模型属于后续设计
（届时替换 `LOCAL_MOUNT` 这处软编码，而不是改 provider）。

### 10.2 新增一个容器层

需要「容器套容器」（例如某个插件内部再挂若干子资源）时，**不要**新造私有协议、
也不要改访问层：

1. 容器实现 `VdfsProvider`，内部用 `VdfsMountTable` 聚合自己的子项；
2. 容器在 `traverse` 中把该视图 `register_vdfs_root(...)`（成为子树的根）
   **或** `register_vdfs_provider(自己的插件名, 视图)`（作为父容器下的一级目录）。

两种挂法都由既有机制支持：前者成为独立根，后者成为父容器下的一级子目录。

### 10.3 换宿主

重写三个文件即可：

- `symbio_core/vdfs/host.rs`（桥：装上下文、翻错误）；
- `plugins/vdfs/host.rs`（访问层：取根、翻译操作、树遍历、事件投递）；
- 容器的注册接线（子插件收集方式随宿主插件机制而变）。

`symbio_core/vdfs_provider.rs`（纯接口 + `VdfsMountTable`）与
`plugins/vdfs/protocol.rs`（线路信封）**原样复用**——前者零宿主依赖，
后者只依赖前者。

## 11. 与既有机制的关系与迁移

- **`entities/*`**：VDFS 是**替代性**核心接口。迁移期两者并存：`entities/*`
  继续服务既有 UI；新能力一律接入 VDFS。逐模块把 `EntityProvider` 的实现
  改写为 `VdfsProvider`（数据操作与校验逻辑可整体搬移），最终下线 `entities/*`。
- **本地文件工具**（`read_file` / `write_file` / `file_edit` / `delete_file` /
  `dir_list` / `glob_search`）：**已迁入 VDFS 并下线原生实现**（§13.4）。`local`
  插件实现 `VdfsProvider`，以插件名注册为 `/local` 挂载；原生能力由
  `vdfs_read` / `vdfs_edit` / `vdfs_write` / `vdfs_delete` / `vdfs_list` /
  `vdfs_search` 统一暴露（§13.3）——每个工具持有封装 provider `ToolVdfs`，
  把对文件系统的操作原样改为对它的操作，在虚拟地址空间里复现「相对路径从工作
  目录开始」的既有规则——行号分页、`ignore` 过滤、精确字符串替换（保持换行符
  风格）、文件名 Glob 均已对齐。
- **`CapabilityVisitor`**：由「工具 + 模型服务 + 系统提示词」扩展为
  「+ VDFS 挂载点 + VDFS 根」。全部共用一次 traverse 广播，不引入新通道。
- **容器**：容器是「目录 + 拓扑」的自然落点——它实现 `VdfsProvider` 并拥有虚拟根。
  容器**不必**认识任何具体资源：它只逐子插件收集挂载，挂载内部层级由各 provider
  的 `list` 表达。
- **详情定义**：`DetailDefinition` 从 VDFS 协议中**移出**，降级为 symbio 的
  `schema` 方言，从而不污染开放接口。

## 12. 一致性要求

- 任何新的资源访问功能 **不得** 绕开 `vdfs/*` 新造私有协议。
- `symbio_core/vdfs_provider.rs` **不得**引入 `crate::` 依赖；宿主专有类型一律经
  `VdfsContext` 注入。
- 线路类型（请求 / 响应信封、`VFDS_OPS`、`VdfsMountInfo`）**不得**上浮到 core；
  它们只属于 `plugins/vdfs/protocol.rs`。
- provider **不得**提供或假设挂载名（trait 上无 `mount()`）；只接收自身子树内的
  相对路径。
- 挂载名**只能**由注册方在 `register_vdfs_provider(mount, …)` 处给出（约定 =
  插件名）；**不得**出现第二处命名来源。
- **拓扑只归容器**：访问层与 provider **不得**持有挂载点表、不得解析路径首段、
  不得合成虚拟根；访问层只允许「取根 + 转发全路径」。挂载名 → 子树的映射只允许
  出现在 `VdfsMountTable`（以及容器的注册接线）。
- **地址规则归使用方**：把 LLM 地址软编码到某个挂载点（`tools.rs` 的 `LOCAL_MOUNT`）
  是**使用方的迁移期策略**，不是机制的一部分——机制只做「全路径 → 挂载名 +
  相对路径」的解析。provider **不得**假设自己被挂在哪里，也不得依赖任何前缀。
- 运行时状态（workdir 之类）**不得**进入 provider 的路径语义，只能经调用级参数
  （§6.5）透传。
- 虚拟根 `/` **只能**由组合容器经 `register_vdfs_root` 占据；**不得**出现第二个
  根注册点。
- 消费者 **不得**按 `kind` 判定能力；只能依据 `access`。
- 前端 **不得**硬编码资源类型清单、标签、路径模板、能力开关；只允许登记
  ext → 渲染器、挂载名 → 图标这类纯 UI 映射。
- 接口变更先改 `vdfs_provider.rs`（trait 是 centerpiece），线路变更先改
  `plugins/vdfs/protocol.rs`，再改访问层与前端两侧契约。

## 13. 范例（实例，非机制组成部分）

### 13.1 设置（setting）——原生 provider + 自持校验

- 注册名 = 插件名 `PLUGIN_SETTING`（= `/setting` 挂载）；`root_access = l`
  （分区清单固定，每一项是叶子文档，不可 `t`）。provider 自身不含挂载名。
- 分区节点：`ext = form`，`schema` = 该分区的 `DetailDefinition`；
  `access = rw`（可写分区）或 `r`（前端自持分区，如 `appearance` / `about`）。
- `read` → 转发目标插件的 `config/get`，返回格式化 JSON 文本。
- `write` → **先按定义逐字段校验**（必填 / 范围 / 枚举 / 类型 / 条件显隐），
  通过后才转发 `config/set`；失败返回字段级错误。
- 分区清单是**单一真相源**：统一实体机制与 VDFS 资源共用同一份声明。

### 13.2 组合容器（composite）——虚拟根的拥有者

- `composite` 实现 `VdfsProvider`（`CompositeVdfs`），`label = "系统"`、
  `order = 0`、`root_access = lt`。
- 八个原子操作都是同一件事：现场调用 `table_of(ctx)` 取组合视图，再委派
  （`list` / `stat` / `read` / `write` / `delete` / `mkdir` / `move` /
  `watch` / `unwatch`；未实现者默认 `NotImplemented`）。
  **不缓存**——子插件集合由配置与生命周期决定，每次现取才与容器一致。
- `table_of` 逐子插件广播 `TRAVERSE_AVAILABLE_TOOLS`（每个子插件独立收集器），
  汇总为 `VdfsMountTable`。
- 在 `traverse` 中把该视图注册为 VDFS 根（§6.2）。

### 13.3 VDFS 插件（vdfs）——访问层

- 路由：`vdfs/<op>` → `host::resolve_root`（取容器注册的根）→ `host::dispatch_with`。
- **LLM 工具链路（`provider.rs` + `tools/`）**：vdfs 插件封装一个工具链路 provider
  `ToolVdfs`——它**持有 `CapabilityVisitor`**；`traverse` 广播中由 `plugin.rs`
  构造（`ToolVdfs::new(visitor)`）并把工具注册进同一个 visitor。每个工具
  （`tools/` 下一工具一文件，均为框架原生 `Capability`）构造时持有这同一个
  `Arc<ToolVdfs>`，`execute` 内只是「**对文件系统的操作改为对它的操作** + LLM 封装」
  （行号分页 / ignore 过滤 / 成功 message），不认识能力管理器、不走协议信封。
  工具：`vdfs_mounts` / `vdfs_list` / `vdfs_tree` / `vdfs_stat` / `vdfs_read` /
  `vdfs_edit` / `vdfs_search` / `vdfs_write` / `vdfs_delete` / `vdfs_mkdir` /
  `vdfs_move`，共十一个。
- **`ToolVdfs` 只做三件事**：
  1. **地址翻译**（见下）→ `normalize_path`（拒绝 `..` 穿越）→ `split_mount`
     拆出「挂载名 + 子树相对路径」；
  2. **按挂载名直调**：`visitor.get_vdfs_provider(挂载名)` 取 provider，
     直接调用其方法——**不经 composite 的根，也不经协议信封**；
  3. **调用级参数与守卫**：经 `host::call_params` 透传 `workdir`（与前端链路共用
     同一份翻译）；挂载根本体不可读 / 写 / 删 / 建 / 移（列目录与元数据不受限，
     与根 `VdfsMountTable` 的守卫语义一致），跨挂载点移动拒绝。
- **组合操作只写一次（`host::edit_via` / `host::search_via`）**：`VdfsProvider`
  trait 只含**原子操作**（`list` / `stat` / `read` / `write` / `delete` / `mkdir` /
  `move_item` / `watch` / `unwatch`）——原生文件系统没有 edit / search 对应的原子
  调用，读改写、遍历过滤属于组合逻辑，放 trait 里会逼每个实现方重复实现。二者
  落在访问层各一份：编辑 = `read` → 精确替换 → `write`；搜索 = 递归 `list` +
  Glob 过滤（安全规则经 provider 的 `read` / `write` / `list` 自持生效）。前端
  协议入口（`vdfs/edit` / `vdfs/search` handler）与 LLM 工具链路（`ToolVdfs`）
  共用同一份组合实现；`VdfsEditResponse` / `VdfsSearchResult` 因此归
  `plugins/vdfs/protocol.rs`（访问层形状），不再属于 core。
- **地址规则（迁移期，两类）**——裸地址即**会话工作目录地址空间**，挂载点名字与
  local 插件注册名共用 `PLUGIN_LOCAL` 常量（唯一的软编码点）：
  1. **本地文件地址**（无 `.vdfs/` 前缀）：挂到 `local` 下，**拼接而非解析**——
     `Readme.md` → `local/Readme.md`、`/` → `local//`
     （经 `normalize_path` 规范成 `/local`，即工作目录根）；工具描述与示例按
     **本地文件语义**书写。
  2. **虚拟地址**（以 `.vdfs/` 开头）：剥掉前缀，首段 = 在 `CapabilityVisitor`
     里注册的挂载名，直调对应 provider（`.vdfs/setting/appearance` →
     `setting/appearance`）——任意已挂载插件都可通过 `.vdfs/<插件>/…` 暴露给大模型，
     `ToolVdfs` 不持有任何拓扑知识。
- 前端链路给的是全路径（`/local/README.md`），走 `dispatch_with` 经根分发；
  工具链路与前端消费**同一批注册的 provider**，不存在第二套实现。`vdfs_mounts`
  经 `visitor.list_vdfs_providers()` 列出挂载点清单。

### 13.4 本地文件（local）——真实文件树 provider

- 注册名 = 插件名 `PLUGIN_LOCAL`（= `/local` 挂载）；`label = "本地文件"`、
  `order = 10`、`root_access = lw+t`（可列 / 可遍历 / 可在其下创建）。
- 地址规则与既有本地工具**完全一致**：相对路径从 `workdir` 开始，绝对路径直用。
  `workdir` **不是 provider 的状态**，而是经调用级参数 `VFDS_PARAM_WORKDIR`
  透传（§6.5）；缺失即 `Internal`——那是接线错误，不是用户错误。
- 进入 provider 的路径已由容器剥掉挂载名；其首部可能残留一个 `/`（VFS 分隔符，
  如前端 `/local/README.md` → 容器剥 `local` → `/README.md`），`join_target`
  会归一为 workdir 相对，确保 LLM（`README.md`）与前端（`/README.md`）落到同一文件。
- 安全规则**自持、不经机制**，与既有工具共用同一份 `SecurityPolicy`：
  读写路径白名单、解析符号链接后复验（防链接逃逸）、拒绝写入 / 删除符号链接、
  速率限制、读上限 10MB、列目录上限 2000 项。
- `list` 目录在前、各自按名升序；文件声明 `rw`、目录 `lwt`；图片扩展名映射为
  `binary = true` + MIME，`read` 返回 base64（多模态）。
- **不实现 `edit` / `search`**——二者已从 `VdfsProvider` trait 移除（provider 只出
  原子操作），由访问层组合实现（§13.3 的 `edit_via` / `search_via`）：编辑 = `read`
  → 精确替换 → `write`，搜索 = 递归 `list` + Glob 过滤。安全规则经本 provider 的
  `read` / `write` / `list` 自持生效，无需任何 provider 重复实现；精确替换的语义
  （`old_string` 必填非空、保持原换行符风格 CRLF/LF、`replacen(..,1)` 只替换一次、
  匹配 0 次或多次均报错）与 Glob 约定（拒绝对 / 遍历、上限 1000 项截断）不变。
- `watch` / `unwatch` 保持 trait 默认（未实现）——实时刷新待后续接入。
