# 第三方插件体系与规范

> **文档类型：Design（设计）** — 让「插件」不再限于编译期内置的 Rust 实现。
>
> **边界**：本文只回答「怎么设计」。现状不合理之处的清单与优先级见
> [archive/plugin-system-review-2026-09.md](../archive/plugin-system-review-2026-09.md)
> （下文简称「**评审**」，条目号 A1–A6 / P1–P8 均指该文）；单条决策定案后写进
> [DECISIONS.md](../DECISIONS.md)。
>
> **参照系**：MCP（Model Context Protocol）。本文与 MCP 的**关键差异**是范围——
> MCP 的接入点是「工具清单」，本方案的接入点是**完整的 `Plugin` trait**：
> 路由 / 资源树挂载 / 配置表单 / 生命周期。§8 逐项对照。

## 1. 目标与不变量

### 目标

让一个**不在本进程、不必是 Rust** 的实现，成为一个与内置插件**同构**的插件：

- 有挂载点（出现在资源树里，有标题、图标、排序）；
- 有路由（`<挂载名>/<子路径>` 可寻址）；
- 有工具（出现在 LLM 的能力清单里）；
- 有配置表单（在自己的目录里读写 `PLUGIN.yml`）；
- 可启用 / 停用 / 卸载（与内置插件同一套动作）；
- 被约束（声明需要什么，宿主决定授不授）。

### 四条不变量（不得违反）

| # | 不变量 | 出处 |
|---|---|---|
| **I1** | 一个插件 = 一个目录；搬走目录 = 搬走插件（含配置与状态） | `plugin_dir.rs:11-12` |
| **I2** | 挂载名 = 目录名 = 实例名 = 路由前缀 | `plugin_dir.rs:178` |
| **I3** | 插件之间互相不可见，只经 core 契约交互 | `plugins/mod.rs:5-14` |
| **I4** | 容器是通用容器：它不认识任何具体插件，只认 `Arc<dyn Plugin>` | `composite` 设计 |

**I4 是本方案能成立的关键**：容器对子插件的要求只有「实现 `Plugin`」。
因此**只要外部插件能提供一个 `Arc<dyn Plugin>` 的实现，容器就不需要任何改动**。
方案的全部工作，就是造出这个实现。

---

## 2. 现状：八类障碍

以下是「外部实现为什么现在接不进来」的完整清单（详见评审文档，此处只列与设计相关的约束）：

| # | 障碍 | 位置 | 本方案的处置 |
|---|---|---|---|
| 1 | `inventory` 编译期注册 + `TypeId` 严格相等 + 裸函数指针 | `creator.rs:20,35,71` | §3 两级 provider 解析 |
| 2 | `ctx` 扩展桶有 6 个不可序列化键 | `keys.rs:100,259,276,294,326,346` | §5.2 线上可表达子集（P3） |
| 3 | ~~`PluginPayload::Native` 死变体~~ ——**已删除**（评审 P1 已落地） | `transport.rs` | 已完成 |
| 4 | 闭包 sink / 事件总线 channel / `ExecEnv` | `exec.rs`、`keys.rs:326` | §5.4 出口的线上形态 |
| 5 | 共享内存状态（`&self` dispatch / visitor 槽 / `HookRegistry`） | 各插件内部 | §5.3 声明式替代 |
| 6 | 无插件级权限模型 | 仅 `local/policy/mod.rs:37` | §7 manifest 授予 |
| 7 | 无生命周期钩子 | `plugin.rs:300-364` | §6 `init` / `shutdown` 帧 |
| 8 | `gateway` 是**纯入站**（外部调用本应用，反向不成立） | `gateway/server.rs` | §3.3 复用其线上类型，方向相反 |

**障碍 8 值得单独说明**：`gateway` 已经解决了「外部进程怎么调用本应用」——
它有完整的线上类型（`PluginMessageWire` / `PluginPayloadWire` / `PluginFrame`）
与三种传输（Tauri IPC / HTTP / WS）。本方案要的是**同一个问题的反向**：
「本应用怎么调用外部进程」。因此**线上类型可以原样复用**，只是发起方向相反、
且外部侧需要多一组生命周期帧。

---

## 3. 核心决策：把 provider 从「一个 id」扩展为「一个描述」

### 3.1 两级 provider 解析

**现状**：`plugin_provider` 的值必须命中 `inventory` 表（`plugin_dir.rs:61`）。

**改动**：把「查表」升级为「解析」，解析结果有两种来源：

```
plugin_provider: web                             → Builtin("web")     ← 现状完全不变
plugin_provider: "ext:stdio:./weather"           → External(Stdio)    ← 新增
plugin_provider: "ext:http:http://127.0.0.1:8080" → External(Http)    ← 新增
```

**关键性质**：

1. **零新增保留键**——只扩展 `plugin_provider` 的**值语法**，键本身不变。
   这直接服务于「减少新加机制」：`PLUGIN.yml` 的骨架、`creator_has` 判据、
   `PluginEntry` 结构都不动，只多一条解析分支。
2. **内置路径行为完全不变**——无 `ext:` 前缀即走现有 `creator_create_object`，
   一个字节都不改。
3. **解析发生在装配方**（`composite`），与「谁构造插件」同处——不引入新角色。

### 3.2 装配期数据流

```
composite.mount_all()
  └─ 逐目录读 PLUGIN.yml
       ├─ plugin_provider = "web"                 → creator_create_object("web")      → Arc<dyn Plugin>
       └─ plugin_provider = "ext:stdio:./weather" → ExternalPlugin::spawn(..)  → Arc<dyn Plugin>
                                                                                     ↑
                                                              容器从这里往后完全一致
```

**这就是全部**：`ExternalPlugin` 是一个**普通的 `Plugin` 实现**，它的
`route` / `traverse` / `vdfs_dispatch` 三个方法体内不发本地逻辑，而是**把调用翻译成帧、
发给子进程、等响应、翻译回来**。

从容器视角，它与 `web` 插件没有任何区别——因为容器只认 trait。

### 3.3 形态选型

| 形态 | 隔离 | 跨语言 | 复用现有资产 | 结论 |
|---|---|---|---|---|
| **stdio 子进程** | 进程级 | ✅ | `PluginFrame` 帧编解码 | **推荐（首选）** |
| **HTTP/WS 端点** | 网络边界 | ✅ | `PluginMessageWire` / `PluginPayloadWire` | **推荐（次选）** |
| WASM（wasmtime） | 沙箱 | ✅（编译目标） | 无 | 后续评估 |
| 动态库（`abi_stable`） | **无**（同进程） | ❌（Rust ABI） | 无 | **不推荐** |

**为什么动态库不推荐**：它只解决「编译期」不解决「隔离」——而隔离恰是第三方插件的
核心诉求。且 Rust 无稳定 ABI，`abi_stable` 需双方锁定同一版本，生态代价高。
`docs/archive/proj/IMPROVEMENT_PLAN_2026.md:218-228` 曾把 `abi_stable` 与 `wasmtime`
并列，本方案的建议是**去掉前者**。

**为什么 stdio 首选**：它是 MCP 已验证的形态；隔离是进程级的（不共享内存、
崩溃不传染）；且**宿主已有的 `PluginFrame` 就是 JSON 帧**——子进程只需实现
「读一行 JSON、写一行 JSON」。

---

## 4. 外部插件的清单（manifest）

`PLUGIN.yml` 在外部插件上的形态（**复用 §3.1 的值语法，不新增连接类保留键**）：

```yaml
plugin_provider: "ext:stdio:./weather"   # 连接方式（值语法，见 §3.1）
plugin_name: weather                     # 实例名（缺省 = 目录名）
plugin_enabled: true                     # 装配位
plugin_title: 天气                        # 身份：展示标题（ADR-032）
plugin_description: 查天气的外部插件       # 身份：语义描述
plugin_version: 1.2.0                    # 身份：版本
plugin_author: someone@example.com       # 身份：作者
plugin_api: "1"                          # 新增：要求的宿主插件 API 版本
plugin_grants: [fs.read, net.http]       # 新增：宿主授予的能力（§7）

# 以下仍是插件自己的配置字段（现状不变，宿主不解释）
weather_units: metric
```

**身份键（`plugin_title` / `plugin_description` / `plugin_version` /
`plugin_author`）已在第一期落地**（ADR-032）：运行期唯一来源是 manifest，
出厂声明（`Plugin::meta()`）只在装配期投影一次。对外部插件而言这尤其自然——
它**没有** Rust 侧的实现可承载身份，manifest 是唯一可能的位置。

**新增两个保留键，各有不可省的理由**：

| 键 | 为什么必须在 manifest 里（而不是别处） |
|---|---|
| `plugin_api` | 宿主必须在**启动子进程之前**就知道「这个插件我认不认识」——否则要么盲目启动（可能挂），要么启动后才发现不兼容（已产生副作用） |
| `plugin_grants` | 权限是**宿主**的决定，必须在宿主侧可审计（I1：跟着目录走，搬走目录 = 带走授予） |

**注意**：插件「**需要**什么」**不**在 manifest 里——那是插件自己在 `init` 响应里
声明的（§7）。manifest 只记「宿主**授予**了什么」。两者分开，才能有「授予 < 需要」
这个可检测的状态。

---

## 5. 线上化协议

### 5.1 帧形态

**复用现有类型**：宿主侧的线上格式已是 `PluginMessageWire { metadata, payload }`
（`transport.rs:237`）。外部插件协议在此之上加一个**操作头**与**请求 id**：

```jsonc
// 宿主 → 插件
{
  "op": "route" | "traverse" | "vdfs" | "init" | "shutdown",
  "id": 1,                       // 请求 id，用于配对（流式会话时同一 id 多帧）
  "path": "forecast",            // route / traverse / vdfs 的路径（相对本插件）
  "metadata": { "trace_id": "…", "session_id": "…" },   // 线上可表达子集（§5.2）
  "payload": { … }
}

// 插件 → 宿主（一次性）
{ "id": 1, "ok": true,  "payload": { … } }
{ "id": 1, "ok": false, "error": { "code": "NOT_FOUND", "message": "…" } }

// 插件 → 宿主（流式，多帧同 id）
{ "id": 1, "frame": "data", "value": { … } }
{ "id": 1, "frame": "error", "message": "…", "details": { … } }
{ "id": 1, "frame": "end" }
```

**`op` 的闭集是 5 个**——正好对应 `Plugin` trait 的五个方法（`init`/`shutdown`
对应 §6 新增的生命周期钩子，`meta` 不需要，因为身份来自 manifest）：

| `op` | 对应 trait 方法 | 响应形态 |
|---|---|---|
| `route` | `Plugin::route` | 一次性 或 流式 |
| `traverse` | `Plugin::traverse` | 一次性 |
| `vdfs` | `Plugin::vdfs_dispatch` | 一次性（`VdfsRequest`/`VdfsResponse` 本就可序列化） |
| `init` | （新增，§6） | 一次性，带声明（§5.3） |
| `shutdown` | （新增，§6） | 一次性 |

**`frame` 与 `error.code` 都复用现有资产**：帧的三态（`data`/`error`/`end`）与
`PluginFrame` 的两态（`Data`/`Error`）同构；`error.code` 直接用
`PluginErrorCode`（`error.rs:38`）——**这是评审 P4 记录的「可直接复用」**。

### 5.2 线上可表达的上下文子集

`ctx` 的 26 个键分三类（评审 P3）：

| 类 | 键 | 外部插件可见 |
|---|---|---|
| **字符串键**（17 个） | `PATH` / `WORKDIR` / `AGENT_ID` / `SESSION_ID` / `TRACE_ID` / `VDFS_PARENT_ADDR` / `TOOL_CALL_ID` / `RESULT_MSG_ID` / `MODE` / `PROVIDER_ID` / `RISK_LEVEL` / `ID` / `NAME` / `KIND` / `SCOPE` / `CONTENT` / `DESCRIPTION` | ✅ 原样进 `metadata` |
| **可序列化对象键**（3 个） | `CONFIG`(Value) / `PLUGIN_DIR`(路径串) / `REQUIRED_PLUGINS` | ✅ 序列化后进 `metadata` |
| **进程内对象键**（6 个） | `PARENT` / `CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIG_VISITOR` / `EVENT_SINK` / `ABORT_SIGNAL` | ❌ 见 §5.4 |

**这一分类已是类型事实**（评审 P3，**已落地**）：`SymbioKey::WIRE`（缺省 `true`）
声明「本键能否在进程外表达」，上表第三类的 6 个键标 `false`——「哪些键能进
`metadata`」现在可静态读取、可机检，不必靠人记住。

### 5.3 能力声明：用「声明」替代「宿主拉」（评审 P8）

**问题**：内置插件的能力清单是**宿主 `traverse` 时现场算出来的**（`traverse` 是同步
调用栈里的回调）。外部插件做不到——它不能在宿主的遍历中「被调用一下然后返回」。

**解法**：外部插件在 `init` 时**一次性声明**自己提供什么：

```jsonc
// 宿主 → 插件
{ "op": "init", "id": 1, "metadata": { "api": "1" } }

// 插件 → 宿主
{
  "id": 1, "ok": true,
  "declare": {
    "api": "1",                                  // 确认版本（不匹配则宿主拒绝）
    "capabilities": [                             // = CapabilityMeta 清单
      { "name": "forecast", "description": "…", "input_schema": { … } }
    ],
    "options": [ … ],                             // = 选项（可展示的数据节点）
    "configurable": { "label": "天气设置", "definition": { … } },  // = 配置文档声明
    "vdfs_root": true,                            // = 是否暴露资源树挂载点
    "needs": ["fs.read", "net.http"]              // = 需要的能力（§7）
  }
}
```

**宿主侧的接线**（这是本设计的**关键**）：

外部插件对应的 `ExternalPlugin` 实现里，`traverse` 方法**不转发给子进程**，
而是**返回 init 时缓存下来的声明**：

```rust
async fn traverse(self: Arc<Self>, path: String, ctx: Arc<dyn PluginInvokeRequest>) -> … {
    match path.as_str() {
        // 宿主遍历到本插件时，把「声明」原样交出去——与内置插件的回填同形
        "available_tools"    => Ok(PluginPayload::new(&self.declared.capabilities)),
        "available_options"  => Ok(PluginPayload::new(&self.declared.options)),
        _ => /* 其他路径转发给子进程 */,
    }
}
```

**于是容器完全不需要知道「这个子插件是内置的还是外部的」**——它遍历时收到的都是
一份清单。内置插件是「现场算的」，外部插件是「init 时缓存下来的」，
**对汇聚点完全同构**。

这正是评审 P8 那句「声明是外部插件的入口，visitor 仍是宿主的唯一汇聚点」的落地。

### 5.4 四条进程内通道的处置

| 通道 | 用途 | 外部插件的对应物 |
|---|---|---|
| `CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIG_VISITOR` | 宿主拉清单 | §5.3 声明（**推**模型） |
| `EVENT_SINK`（执行期出口） | 工具执行时推事件 | 流式帧（§5.1 的 `frame: data`，同 id 多帧） |
| `ABORT_SIGNAL`（执行期中止） | 编排层中止工具 | 带外帧 `{ "op": "abort", "id": 1 }`（宿主 → 插件） |
| `PARENT`（父插件引用） | 子插件回看父 | ❌ 不提供——**违反 I3**（插件互相不可见）。若某插件真需要，那是它该经 core 契约表达的需求 |

**`EVENT_SINK` 的对应物已经存在**：现有 `PluginPayload::Session(PluginChannel)` 正是
「长连接 + 多帧」的机制，而 `gateway` 的两个入口已经在处理它（`server.rs:439`、`:549`）。
外部插件的流式响应**直接复用这条路径**——不需要新机制。

---

## 6. 生命周期（评审 A2）

### 钩子

在 `Plugin` trait 上加**可选**钩子（有默认空实现，不波及现有 16 个 impl）：

```rust
/// 装配后、开始服务前调用一次。默认无操作。
async fn start(self: Arc<Self>, _ctx: Arc<dyn PluginInvokeRequest>) -> Result<(), PluginError> { Ok(()) }

/// 停用 / 卸载 / 进程退出前调用。默认无操作。
///
/// `reason` 区分三种情形（可恢复停用 / 卸载 / 全局收尾），
/// 因为插件的处置可能不同（如卸载时是否保留数据）。
async fn stop(self: Arc<Self>, _reason: PluginStopReason) -> Result<(), PluginError> { Ok(()) }
```

### 调用点（现有动作的钩子接线）

| 动作 | 现有实现 | 加钩子后 |
|---|---|---|
| 装配 | `registry.mount_all()` 构造 | 构造后调 `start()` |
| 停用 | `registry.set_enabled(name, false)` 移除实例 | 移除前调 `stop(Disabled)` |
| 卸载 | `registry.uninstall(name)` 移除实例 + 删目录 | 删目录前调 `stop(Uninstalled)` |
| 退出 | （无） | 逐插件调 `stop(Shutdown)` |

### 外部插件的实现

`ExternalPlugin::stop()` 发 `shutdown` 帧 → 等 `ok` → 关闭 stdin → 等进程退出
→ **超时则 kill**（宽限期如 5 秒）。`start()` 在 spawn 后发 `init` 帧并校验版本。

**对内置插件**：`start`/`stop` 默认空实现，**行为与今天完全一致**——这是「加钩子
不改变现状」的保证。

---

## 7. 权限与信任（评审 A4）

### 两侧声明，取交集

```
manifest（宿主写）  plugin_grants: [fs.read, net.http]      ← 宿主决定「我允许你用什么」
init 响应（插件给） "needs": ["fs.read", "net.http", "proc.spawn"]  ← 插件声明「我需要什么」

                    授予 ⊇ 需要  → 通过
                    授予 ⊉ 需要  → 拒绝 init（记录缺失的能力）
```

**为什么是两侧而非一侧**：只有「授予」没有「需要」，宿主不知道插件要干什么，
只能全授（等于没约束）；只有「需要」没有「授予」，插件自己说了算（等于没权限）。
两侧都有，才有「**授予 < 需要**」这个可检测、可审计的状态。

### 能力清单（已定案，闭集）

| 能力 | 含义 | 风险 |
|---|---|---|
| `fs.read` / `fs.write` | 读写**本插件目录之外**的文件 | 高 |
| `net.http` | 出站 HTTP | 中 |
| `proc.spawn` | 启动子进程 | 高 |
| `env.read` | 读环境变量 | 中 |
| `host.ctx` | 读 `metadata` 里的会话 / 追踪信息 | 低 |
| `vdfs.read` / `vdfs.write` | 经宿主读写**共享资源空间**（会话 / 技能 / 记忆 / 其它插件的数据） | 高 |
| `event.publish` | 向事件总线投递事件（可被其它插件与前端订阅） | 中 |

**为什么是这九个而不是最小集**：前三项（`fs` / `net` / `proc`）是「进程能对外做什么」，
后三项（`host.ctx` / `vdfs` / `event`）是「**插件能对宿主做什么**」——后者才是本体系
真正的权限面：一个第三方插件若能任意写 VDFS，它就能改会话转写、改别人的配置、
伪造记忆，**而它根本不需要碰文件系统**。只列前三项会让权限模型看起来完整、
实际上漏掉最大的那一片。`event.publish` 单列（而非并入 `vdfs.write`）：投递事件是
**广播**语义，影响面跨插件，与写一个文件不是同一类风险。

闭集的意义：新增能力必须改本表 + 一条 ADR，**不允许插件自定义能力名**——
否则「授予 ⊇ 需要」退化成字符串游戏。

**与 `SecurityPolicy` 的关系**（评审 A4）：**分层共存，不替换**。
`SecurityPolicy`（`local/policy/mod.rs:37`）管「local 插件**自己的工具**怎么安全执行」；
本表管「**宿主**允不允许一个插件碰文件系统 / 共享空间」。前者是插件内部的实现细节，
后者是装配契约。

**强制点**：本期**只做声明与校验**（拒绝不匹配的 `init`），不做运行时沙箱
（如 seccomp / 网络隔离）。理由：stdio 子进程已是**进程级隔离**，宿主能做的事
（拒绝启动）已经足够表达「不信任就不装」。运行时沙箱留给后续按需引入。

---

## 8. 与 MCP 的对照

| 维度 | MCP | 本方案 |
|---|---|---|
| 传输 | stdio / HTTP+SSE / streamable HTTP | stdio / HTTP（**复用 gateway 线上类型**） |
| 报文 | JSON-RPC 2.0 | JSON 帧（`op` + `id`，§5.1） |
| 能力协商 | `initialize` → `capabilities` | `init` → `declare`（§5.3） |
| **覆盖范围** | **只有工具**（+ 资源 + 提示词） | **完整插件**：路由 / 工具 / 资源树挂载 / 配置表单 / 生命周期 |
| 宿主角色 | LLM 客户端 | **插件容器**（插件可嵌套，分形） |
| 挂载语义 | 工具加前缀（`server__tool`） | **挂载名 = 目录名 = 路由前缀**（I2），且出现在资源树里 |
| 配置 | 无（由客户端自行处理） | 插件在自己的目录里读写 `PLUGIN.yml`（I1） |
| 权限 | 无（靠用户逐次确认） | manifest 授予 + init 需要，取交集（§7） |
| 隔离 | 进程（stdio）/ 网络 | 同 |

**「MCP 只管工具」的具体含义**：MCP 的插件是一个**工具提供者**，接入点是「工具清单」；
本方案的插件是**智能体的组成单元**，接入点是完整的 `Plugin` trait。因此本方案多出
四件 MCP 没有的事：**资源树挂载点**（§5.3 `vdfs_root`）、**配置文档声明**
（§5.3 `configurable`）、**生命周期**（§6）、**权限授予**（§7）。

**可以借鉴 MCP 的两点**：
1. **stdio 优先**——已验证、跨语言、隔离好；
2. **能力协商先于服务**——`init` 一次说清，避免「调用时才发现不支持」。

**不必借鉴 MCP 的一点**：JSON-RPC 2.0 的完整报文规范（`jsonrpc`/`method`/`params`）。
本方案已有 `PluginMessageWire`，加 `op` + `id` 即可；引入 JSON-RPC 是**第三套**
线上约定（现有已有两套：Tauri IPC 与 HTTP/WS），徒增不一致。

---

## 9. 分期落地

**顺序不是任意的**：前三期的每一项都是后一项的前置（§9 末尾说明依赖链）。

### 第一期｜manifest 成形（不改运行时行为）—— **已完成**

- ✅ **A6**：身份单源——身份归 manifest（ADR-032）；
- ✅ **A5**：`PLUGIN.yml` 升级为 manifest（`plugin_api` / `plugin_grants` 两个保留键已定义）；
- ✅ **A4**：能力清单定案（§7 的表，闭集九项）；
- ✅ 内置 16 个插件**零改动**迁移（出厂身份声明仍在代码，装配期投影进 manifest——
  见下方「已落地进度」的说明）。

**验收**：`cargo test --lib` 全绿；门禁 38/38；内置插件行为**零变化**。

> **「迁移」的含义**：内置插件的 `metadata()` 一行未改——出厂身份仍声明在代码里，
> 只是消费方式从「运行期每次读 `meta()`」变成「装配期投影一次进 manifest」。
> 因此这次迁移的产物是**机制**（保留键 + 投影 + 单源读取），不是 16 份手写 YAML。

### 第二期｜开放 provider 与生命周期（跑通一个外部插件）

- **A1**：`plugin_provider` 两级解析（§3.1）；
- **P8**：声明式能力（§5.3）；
- **A2**：`start` / `stop` 钩子（§6）；
- ~~**P3**：`SymbioKey` 的线上可表达标记（§5.2）~~ —— **已完成**；
- **ExternalPlugin**（stdio）：spawn / init / 帧往返 / shutdown；
- 一个**样例外部插件**（如 `ext:stdio:./weather`）走通全链路：挂载 → 资源树可见 →
  工具进能力清单 → 配置表单可编辑 → 停用 / 卸载。

**验收**：样例插件的 e2e 用例（挂载 / 调用 / 停用 / 卸载）进 `e2e/cases/`。

### 第三期｜HTTP 传输与权限强制

- HTTP/WS 端点形态（复用 `PluginMessageWire`）；
- `init` 时的权限校验强制（授予 ⊉ 需要 ⇒ 拒绝）；
- 文档与 ADR 定案。

### 依赖链

```
第一期  A6 → A5 → A4          身份 → 载体 → 声明内容（同一份 manifest 的三个字段群）
第二期  A1 + P8 + A2 + P3     A5 成形后才能解析 ext:；P3 是 A1 的准入判据
第三期  HTTP + 权限强制        第二期跑通后，传输与授权是正交扩展
```

### 已落地进度

评审第一组（机械改进）与 P3 **已同批完成**（它们不阻塞第三方插件，但先做完能显著
减少实现期噪音——尤其 P1：删除 `Native` 后，「载荷有几种形态」这个基础问题才有
唯一答案）：

| 项 | 落地物 |
|---|---|
| **P1** 删除 `PluginPayload::Native` 死变体 | `transport.rs` 的枚举收成 3 态，gateway 两处「拒绝」分支消失 |
| **P2** 收口载荷转换 | `gateway/server.rs::classify_payload` —— 唯一分类点，两入口共用 |
| **A3** 消除跨插件引用 | `SEG_MESSAGES` / `message_of_node` 上移 `symbio_core::schemas::session::chat_message`；全仓跨插件 `use` 归零 |
| **P6** `payload` 键收口 | `KEY_PAYLOAD` 取代 deprecated 的 `PayloadKey`/`PAYLOAD`，裸字符串 `"payload"` 消除 |
| **P3** 线上可表达标记 | `SymbioKey::WIRE`（6 个进程内键标 `false`） |
| **P5** gateway 术语统一 | 「native」→「进程内（in-process）」，与协议层一词两义消除 |
| **P7** 文档编号 | `PROTOCOLS.md` 重编号 |

**待办**：A1 / A2 / P8（均需 ADR，见 §10）。

### 第一期落地物（ADR-032）

| 项 | 落地物 |
|---|---|
| **A6** 身份单源 | `PluginDir::identity()` 从 manifest 读；`PluginEntry` 与 `dir_node` 都改走它 ⇒ **停用插件也有身份** |
| **A5** manifest 升级 | `RESERVED_KEYS` 单一清单（读写都遍历它）；`plugin_api` / `plugin_grants` 键定义与解析 |
| **A4** 能力闭集 | 九项（§7），定案 |
| 出厂种子投影 | `PluginDir::seed_identity(&PluginMeta)`——装配期一次性，**只补缺失键**（不覆盖用户改过的值） |

**下一期**：A1（provider 两级解析）——它同时是「删除 `PluginMeta` 身份字段」的前置
（需要一张按工厂 id 索引的**静态自述注册表**，才能在不构造插件时拿到出厂身份，
见 ADR-032「后果」）。

---

## 10. 待确认的决策

| # | 决策点 | 选项 | 本文倾向 |
|---|---|---|---|
| **D1** | 身份归谁（评审 A6） | ✅ **已定案 = 甲**（ADR-032）：归 manifest，`meta()` 降为「出厂自述」 |
| **D2** | 外部插件形态（§3.3） | stdio / HTTP / WASM / 动态库 | **stdio 首选 + HTTP 次选**，去掉动态库 |
| **D3** | 权限强制程度（§7） | 仅声明校验 / 加运行时沙箱 | **仅声明校验**（stdio 已隔离） |
| **D4** | 外部插件连接信息放哪（§4） | 扩展 `plugin_provider` 值语法 / 新增保留键 | **扩展值语法**（零新增键） |
| **D5** | 是否引入 JSON-RPC（§8） | 是 / 否 | **否**（复用 `PluginMessageWire`） |
| **D6** | 生命周期钩子形态（§6） | `start`/`stop` 两钩子 / 更多细粒度钩子 | **两钩子**（最小可用） |

定案后，每条决策写成一条 ADR（编号接 ADR-031）。
