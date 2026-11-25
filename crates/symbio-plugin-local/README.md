---
kind: design
id: crates/symbio-plugin-local/README
title: Local 插件
status: active
created: 2026-09-09
lists: [local-tools=owner]
impl: done
---

# Local 插件

本地工具集：提供宿主机文件系统与命令执行能力，经 `traverse` 向会话贡献工具。

## 工具

清单的 owner 是本篇的列表标记区（`lists: [local-tools=owner]`）；`docs/004-ref/003-ROUTES.md`
§Local 插件是它的 `view`（只指路，不抄表）。共 5 个原生工具：

<!-- list:local-tools owner -->

| 工具 | 说明 |
|------|------|
| `local/cmd`（Windows）/ `local/sh`（macOS / Linux） | 执行 Shell 命令（受工作目录约束）；工具名按操作系统取 |
| `local/content_search` | 文件内容正则搜索（ripgrep） |
| `local/todo_write` | 会话级任务清单（id/content/status/priority），支持 merge；返回紧凑确认（count+message），**不回显全量清单**——最新一次调用经 `LastOnly` 保留策略始终完整，避免每轮全量回显的 token 冗余 |
| `local/codebase_search` | 语义化代码检索（向量相似度，不可用时降级正则） |
| `local/ask_user` | 向用户提出结构化问题（单问题或 1~4 批量，自动补 `Other`）；返回 `failure_kind = needs_interaction` + `prompt` 载荷（**节点由编排层构造**），等回答；自动模式下降级为 `tool_unavailable` 不阻塞 |

> **文件编辑类能力由 VDFS 端口提供**：`read` / `edit` / `write` / `delete` / `list` / `search` 统一走
> `vdfs_*` 工具；物理文件层不在本插件，本插件只挂一份配置文档（`<根>/local/PLUGIN.yml`）。

## 机制

- **工具贡献**：启动/遍历时经 `traverse` 把上述工具注册为 Capability，由宿主收集后进入模型工具定义。
- **文件能力**：经 VDFS 端口访问，本插件不描述文件层的实现方或分发路径。
- **工作目录**：从 `ctx` 提取 `WORKDIR`，相对路径以此为基准；拒绝越界访问。
- **结果回传**：工具结果作为 Tool 消息交回；超大结果由注入的结果守卫按上限处理（8192 tok / 1MB）。

## 边界

工具收集与结果守卫都由宿主经端口提供；本插件不描述、不假设其它能力的实现。
