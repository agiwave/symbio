---
kind: design
id: DE-057-作用域VDFS存储机制
title: 作用域 VDFS 存储机制
status: active
created: 2026-11-20
impl: partial
deps: []
---

# 057 作用域 VDFS 存储机制（D-515）

> 状态：**实施中**。本文登记一次上下文存储面的统一改造：把「目录字符串」换成
> 「作用域 VDFS provider」，让所有插件经同一套虚拟文件系统接口读写数据，
> 不再各自拼接物理路径。

### D-515 作用域 provider 取代 ctx 目录参数

本次决策：上下文新增 `AGENT_VDFS` / `WORKSPACE_VDFS` / `SESSION_VDFS` 三个
provider 键，插件只经相对地址读写，物理路径只留在 provider 后端；`WORKDIR` /
`SESSION_DIR` 保留为对外协议键，但插件内部不再拿它们寻址。身份键 `AGENT_ID` /
`SESSION_ID` 经业务意义分析后保留，但不参与存储寻址（见 §2）。

---

## 1. 背景与诉求

上下文当前带着四个「目录/身份」键：`AGENT_ID` / `SESSION_ID` / `WORKDIR` /
`SESSION_DIR`。其中后两个是纯存储定位参数，插件拿到后自行 `Path::join` 落盘——
`memory` 拼 `AGENTS.md` / `MEMORY.md`、`local` 拼索引与工作区路径、`skill` 拼
`SKILL.md`、`hook` 把进程 cwd 设到目录、`vdfs/physical` 真读磁盘。

问题有三：

1. **存储实现泄漏**：每个插件都得知道「目录 + 文件名」这条物理布局，
   换后端（内存 / 网络 / 只读快照）就要改所有插件；
2. **地址不一致**：同一份数据有物理路径和 VDFS 挂载名两套地址，
   谁读写、谁注入靠约定维系；
3. **语义混杂**：`AGENT_ID` / `SESSION_ID` 既当身份、又被当存储寻址
   （heartbeat 用会话 id 定位配置、todo_write 拿它做内存键），
   边界不清。

用户裁决（2026-11-20）：

- 机制是**统一机制**，不是只为记忆插件服务；
- 先系统分析身份键是否真有业务意义——有则保留，无则去除；
- 存储一律换成三个作用域 provider：`agentVdfsProvider` /
  `workspaceVdfsProvider` / `sessionVdfsProvider`；
- 本轮迁移覆盖记忆插件与所有直读文件系统的插件。

---

## 2. 身份键业务意义分析

逐个读点核查 `AGENT_ID` / `SESSION_ID`。判据：**去掉这个键、换成 provider
之后，这条业务是否还成立**。

### 2.1 `AGENT_ID` 读点

| 读点 | 用途 | 是否身份语义 | 结论 |
|---|---|---|---|
| `symbio-sdk/authz::request_principal` | `principal_of(AGENT_ID)` → 授权矩阵判定主体 | **是**（安全核心） | 保留 |
| `symbio-plugin-tool/src/port.rs`（立约格 `from`） | 声誉聚合主体（DE-038） | **是**（归属） | 保留 |
| `symbio-plugin-tool/src/port.rs`（before-hook 闸门） | 判的主体 | **是**（授权） | 保留 |
| `symbio-plugin-tool/src/port.rs`（fork 子 ctx） | 转发 | 转发（无独立语义） | 随保留 |
| owner 插件的转发位（`forward_to_sub_agent`） | 选中 agent 目录实例 | **是**（选择） | 保留 |
| `symbio-plugin-session/src/tools/subagent.rs`（默认 agent 选择） | 子会话默认 agent | **是**（继承选择） | 保留 |
| `symbio-plugin-local/src/todo_write.rs`（内存键） | `wd::aid` 内存键 | 否（寻址替代） | 迁 provider |

### 2.2 `SESSION_ID` 读点

| 读点 | 用途 | 是否身份语义 | 结论 |
|---|---|---|---|
| `symbio-plugin-api/src/hooks.rs`（事件载荷） | hook 事件载荷 | **是**（通知身份） | 保留 |
| `symbio-plugin-heartbeat/src/tool.rs`（配置读取） | 会话配置的 VDFS 寻址键 | 否（存储寻址） | 迁 provider |
| `symbio-plugin-local/src/todo_write.rs`（内存键） | 内存键 | 否（寻址替代） | 迁 provider |
| `symbio-plugin-session/src/route_guard.rs`（只读路由守卫） | 路由守卫 | **是**（安全） | 保留 |
| `symbio-plugin-session/src/tools/subagent.rs`（派生子会话） | 父会话 id（派生子会话） | **是**（血缘） | 保留 |
| `symbio-plugin-session/src/wording.rs`（措辞请求载荷） | 措辞请求载荷 | **是**（请求身份） | 保留 |
| `symbio-plugin-tool/src/port.rs`（审批窗口 `window_for`） | 审批窗口 `window_for(sid)` | **是**（并发闸门） | 保留 |
| `symbio-plugin-vdfs/src/provider.rs`（越界告警日志） | 越界告警日志 | 弱（观测） | 可随保留 |

### 2.3 结论

`AGENT_ID` 与 `SESSION_ID` **都有真实且不可替代的业务意义**：授权主体派生、
立约/声誉归属、审批窗口、agent 选择、子会话血缘、hook 载荷、路由守卫。
它们不是存储定位参数，VDFS provider 无法替代。**两键保留**，定位为
「身份键」，与本条新增的「存储键」明确分家。

边界纪律：凡把身份键**当存储寻址**用的读点（heartbeat / todo_write），
本条一并迁到 provider；保留的读点只读身份，不得再 `join` 物理路径。

---

## 3. 三个 provider 键

在 `symbio-sdk::keys` 定义，经 `symbio-plugin-api` 转出：

| 键类型 | 实例 | 值类型 | Wire | 作用域根 |
|---|---|---|---|---|
| `AgentVdfsKey` | `AGENT_VDFS` | `Option<DynVdfsProvider>` | false | `agent_dir` |
| `WorkspaceVdfsKey` | `WORKSPACE_VDFS` | `Option<DynVdfsProvider>` | false | `work_dir` |
| `SessionVdfsKey` | `SESSION_VDFS` | `Option<DynVdfsProvider>` | false | `session_dir` |

- `None` = 作用域不存在（没选工作区 / 无会话），是正常状态而非故障；
  插件据此降级，语义等同 `MemoryFile::absent`。
- `WIRE = false`：进程内 trait object，无字符串形态，不进 `metadata`
  （与 `PLUGIN_PARENT` 同一纪律）。
- 插件不再读 `WORKDIR` / `SESSION_DIR` 拼路径；两键的**对外协议面**
  （metadata / 前端头 / CLI 参数）保持不动，只在**内部 ctx 消费面**被
  provider 取代——这是「内部实现替换，不改外部契约」。

### 3.1 provider 契约

复用既有 `VdfsProvider`（内核唯一抽象，单方法 `dispatch(ctx, path, req)`），
不新造 trait。作用域 provider 是「绑定到一个根、只暴露相对路径」的
`VdfsProvider` 实现：

```text
dispatch(vctx, "AGENTS.md", Read/Write)  →  <根>/AGENTS.md
```

实现形态（`symbio-provider-vdfs_service` 新增 `ScopeDirVdfs`）：
持有一个固定根 `PathBuf`，把相对地址拼上去，走 std::fs；继承既有容量与
路径安全检查。它只是**当前后端**——插件只见 `dyn VdfsProvider`，后续可
换成 `MemoryVdfs` / 网络实现而插件无感。这是本条的核心收益。

### 3.2 事实源边界（FB-002 延续）

`SESSION_VDFS` 是**插件数据目录视图**，不是事实源：能力面不得借它拼
`V2_WAL_FILE`、不得 `open_readonly` / `EventWalStore::open`。事实仍只经
`FACT_SOURCE` / `FactSource`。`facts-boundary-audit` 的禁词扩展到三个
provider 键名。

---

## 4. 注入链路

三个 provider 由**持有该目录信息的层**构造，统一挂进 ctx：

| provider | 构造点 | 根来源 |
|---|---|---|
| `WORKSPACE_VDFS` | 宿主 `drive.rs` / `Host::gather` | `DriveTarget.workdir` / metadata |
| `SESSION_VDFS` | session `plugin/round.rs`（能力收集前） | `SessionStore::session_dir` |
| `AGENT_VDFS` | 装配层 / 宿主 | `{homedir}/subagent/<id>`（`SYMBIO_HOMEDIR`） |

- 宿主基础 ctx 与驱动 ctx 都要注入，覆盖「gather 收集」与「drive 驱动」两条链；
- session 编排器在真实能力收集前注入（延续 D-513 的教训：`fork()` 只 clone
  extensions，必须在实际收集点补挂）；
- `fork()` 出的子上下文继承三者（extensions 克隆），无需逐跳重挂；
- 网关 / CLI / telegram / 前端的 metadata 注入点**不改**（外部契约不变）。

---

## 5. 插件迁移规则

### 5.1 记忆插件（核心）

三层落位从「路径 + 文件名」改为「provider + 相对地址」：

- `agent_dir` → `AGENT_VDFS`，相对地址 `AGENTS.md`
- `work_dir` → `WORKSPACE_VDFS`，相对地址 `AGENTS.md`，挂载名 `WORKSPACE.md`
- `session_dir` → `SESSION_VDFS`，相对地址 `MEMORY.md`，挂载名 `SESSION.md`

`MemoryFile` 改为 provider-backed：持有 `Option<(DynVdfsProvider, 相对地址)>`
与两道容量闸门；`read` / `write` / `exists` / `size` 经 `dispatch` 完成
（读不存在 → 空串；写超限 → 拒绝，口径不变）。因 provider 是异步接口，
相关调用链改 `async`。

### 5.2 其他直读插件

| 插件 | 现状 | 迁移 |
|---|---|---|
| `local/codebase_search` | `PathBuf(workdir)` 索引遍历 | 经 `WORKSPACE_VDFS` 列目录 / 读文件 |
| `local/content_search` | 同上 | 同上 |
| `local/shell` | `cmd.current_dir(workdir)` | 工作区访问走 provider 提供的执行根 |
| `skill/loader` | `workdir.join(...)/SKILL.md` | 经 `WORKSPACE_VDFS` |
| `hook/executor` | `.current_dir(workdir)` | 同上 |
| `vdfs/physical` | `VDFS_PARAM_WORKDIR` 真读盘 | 改造为 `WORKSPACE_VDFS` 的后端 |
| `host/lib.rs` scratch | `PathBuf(workdir)` | 经 `WORKSPACE_VDFS` |

> 注：`shell` / `hook` 的 cwd 属「进程执行环境」而非「插件存数据」。
> 实现上由 `ScopeDirVdfs` 暴露一个受控的执行根解析口（仅宿主/internal 可用，
> 不对扩展插件暴露物理路径），避免把物理目录重新泄漏回插件。

### 5.3 身份读点的存储用途迁移

- `heartbeat/tool.rs`：从 `SESSION_ID` 寻址改为 `SESSION_VDFS` + 固定相对地址；
- `todo_write.rs`：内存键改从 `SESSION_VDFS` 的作用域标识取（不读物理路径）。

---

## 6. 判据

- 生产代码中不再有插件把 `WORKDIR` / `SESSION_DIR` 拼进 `Path::join` /
  `PathBuf::from` / `current_dir`；
- `memory` 三层读写全经 provider，物理文件名只出现在 provider 后端；
- `grep-audit` 新增规则禁止 `ctx.get(WORKDIR|SESSION_DIR)` 后接路径拼接；
- `facts-boundary-audit` 禁词扩展到三个 provider 键；
- `plugin-surface` / `arch-shape` / `ciRustTestsTotal` 基线同步；
- e2e 记忆读写、工作区文件访问用例保持绿。

---

## 7. 不做什么

- 不改外部 metadata / HTTP / 前端头的键名（`agent_id` / `session_id` / `workdir`）；
- 不把事实源经 provider 暴露（FB-002 不变）；
- 不在本条替换 provider 的具体后端实现（只建立抽象与注入，后端仍是磁盘）；
- 不合并插件、不为降基线数字削契约面。
