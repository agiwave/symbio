# Local 插件

本地工具集：提供宿主机文件系统与命令执行能力，经 `traverse` 向会话贡献工具。

## 工具

| 工具 | 说明 |
|------|------|
| `local/shell` | 执行 Shell 命令（受工作目录约束） |
| `local/content_search` | 文件内容正则搜索（ripgrep） |
| `local/todo_write` | 会话级任务清单（id/content/status/priority），支持 merge 合并；返回紧凑确认（count+message），**不回显全量清单**——最新一次调用经 `LastOnly` 保留策略始终完整，避免每轮全量回显的 token 冗余 |
| `local/ask_user` | 向用户提出结构化问题（含选项），阻塞等待用户输入 |
| `local/codebase_search` | 语义化代码检索（向量相似度，不可用时降级正则） |
| `local/system` | 系统信息查询 |

> **文件编辑类能力已迁入 VDFS**：`read`/`edit`/`write`/`delete`/`list`/`search` 统一由 `vdfs` 插件以 `vdfs_read` / `vdfs_edit` / `vdfs_write` / `vdfs_delete` / `vdfs_list` / `vdfs_search` 暴露（详见 `../vdfs/README.md`）。本地文件挂在 `local` 挂载点下，与任意已挂载资源走同一条分发链路——「LLM 能做的 = 前端能做的」。

## 机制

- **工具贡献**：启动/遍历时经 `traverse` 把上述工具注册为 Capability，session 的 chat_loop 收集后进入模型工具定义。
- **文件能力**：本地文件树经 `traverse` 注册为 `local` 挂载点的 VDFS provider；读/写/编辑/列举/搜索统一经 `vdfs` 插件分发。
- **工作目录**：从 `ctx` 提取 `WORKDIR`，相对路径以此为基准；拒绝越界访问。
- **结果回传**：工具结果作为 Tool 消息回给 session；超大结果受 session 侧 L0 守卫（8192 tok）与 1MB 物理上限约束。

## 关联

- 工具收集管线：`plugins/session/chat_pipeline.rs`
- 压缩守卫：`../session/README.md` 策略③ / `docs/design/context-compression-design.md`
