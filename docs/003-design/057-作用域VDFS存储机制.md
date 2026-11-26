---
kind: design
id: DE-057-作用域VDFS存储机制
title: 作用域 VDFS 存储机制
status: active
created: 2026-11-20
impl: partial
deps: []
---

# 057 作用域 VDFS 存储机制

## 作用域提供者

宿主在上下文中提供三个作用域键：

| 键 | 值 | 含义 |
|---|---|---|
| `AgentVdfsKey` / `AGENT_VDFS` | `Option<DynVdfsProvider>` | 当前智能体的数据根 |
| `WorkspaceVdfsKey` / `WORKSPACE_VDFS` | `Option<DynVdfsProvider>` | 当前工作区的数据根 |
| `SessionVdfsKey` / `SESSION_VDFS` | `Option<DynVdfsProvider>` | 当前会话的数据根 |

- 三个键复用内核唯一的 `VfsProvider` / `VdfsProvider` 抽象，`WIRE = false`，不进入网络协议。
- 值是可选提供者。`None` 表示作用域不存在，不是存储故障；插件必须区分“缺席”与“调用失败”。
- 当前磁盘后端为 `ScopeDirVdfs`。绑定固定根，只接受相对地址，并继承容量、路径与符号链接检查。
- 会话开轮重挂 `WORKSPACE_VDFS` 和 `SESSION_VDFS`，保留上下文已有的 `AGENT_VDFS`；`fork()` 继承扩展。
- 子树读写子树自己的作用域。系统树与子树可以拥有同名相对文件，物理根把两者隔开。

## 身份与存储分家

`AGENT_ID`、`SESSION_ID` 仍是授权、归属、审批、选择、血缘与路由身份，不作存储寻址。

- 数据地址由逻辑路径与作用域 provider 决定，不把主体 id 拼进物理路径。
- VDFS 事实源不能被绝对路径、目录接口或错误消息绕开暴露。
- 前端、HTTP 与 metadata 的外部头键保持稳定；本页不新增第五种作用域键。
- shell、hook 的 cwd 是执行环境，不是普通数据存储。物理执行根只由宿主或 internal 受控提供。

## 事实源边界

- `SESSION_VDFS` 是会话的数据目录视图，不是 WAL 或会话状态的事实源。
- 插件不得从中读取隐藏状态文件，拼接 WAL，调用 `open_readonly` 或直接打开 `EventWalStore`。
- 会话状态通过公开事实与查询接口读取；数据文件通过对应作用域的 VDFS 操作读取。
- 工具超限结果写入 `SESSION_VDFS` 根下的 `tool_archives/`。无会话宿主态时降级到
  `WORKSPACE_VDFS`；两者都没有则不存档，但仍返回受容量约束的结果。
- 取回只接受会话逻辑地址，不在模型、前端或错误文本中打印物理绝对路径。

## 使用边界

- 动态记忆三层物理文件名均为 `MEMORY.md`；挂载名分别为 `MEMORY.md`、`WORKSPACE.md`、
  `SESSION.md`。稳定指令与人格不属于 memory。
- 写权限不因知道路径而自动成立；仍受调用主体、作用域与 provider 的安全检查约束。
- 存储失败不回退为另一作用域，除非调用协议明确声明了无会话降级。
- provider 切换不迁移旧数据，也不让旧 provider 与新 provider 同时写同一逻辑地址。

## 实现入口

- 作用域键：`crates/symbio-platform/src/sdk/keys/mod.rs`。
- 磁盘 provider：`crates/symbio-providers/src/vdfs_service/scope.rs`。
- 作用域提供者与开轮重挂：`crates/symbio-plugin-session/src/plugin/vdfs_provider.rs`、
  `crates/symbio-plugin-session/src/plugin/round.rs`。
- 工具超限存档：`crates/symbio-plugin-tool/src/result_guard.rs`。
- memory 三层挂载：`crates/symbio-plugin-memory/src/plugin.rs`。
