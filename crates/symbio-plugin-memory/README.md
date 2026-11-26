---
kind: design
id: crates/symbio-plugin-memory/README
title: Memory 插件
status: active
created: 2026-09-28
impl: done
---

# Memory 插件

## 职责

Memory 管理智能体、工作区和会话三层动态记忆，并在模型请求中注入对应提示段。
同一 owner 还负责事件形态记忆的轮末编码、活集巩固、轮首召回与语义读数；索引与打分不进入 VDFS 事实。
它保存用户希望跨轮保留的动态内容；稳定指令与人格仍归 setting 的 `AGENTS.md`。

专题说明见 [长期记忆与语义检索](src/docs/116-长期记忆与语义检索.md "DE-116-长期记忆与语义检索")。

## 公开用法

插件提供 `<根>/memory` 的 VDFS 挂载，并通过 `vdfs_read`、`vdfs_write`、`vdfs_list` 等通用地址面读写：

| 逻辑文件 | 物理文件 | 挂载名 | 作用域 |
|---|---|---|---|
| 智能体记忆 | `MEMORY.md` | `MEMORY.md` | `AGENT_VDFS` |
| 工作区记忆 | `MEMORY.md` | `WORKSPACE.md` | `WORKSPACE_VDFS` |
| 会话记忆 | `MEMORY.md` | `SESSION.md` | `SESSION_VDFS` |

三层可以同时挂载，相对地址相同但物理根独立。子树实例只读写子树自己的作用域；
系统树与子树并集不会因同名文件冲突。

## 配置

配置位于 `<根>/memory/PLUGIN.yml`，只存容量闸门与注入开关，不存记忆正文。

- 每层写入闸门默认 16 KiB；超限拒绝，不静默截断。
- 每层注入闸门默认 4 KiB；超限截断，并在提示中指向 `vdfs_read` 以读取全文。
- 三层注入可以独立关闭；关闭只停止提示注入，不删除文件或禁用 VDFS 挂载。
- 旧位置文件不自动迁移。需要保留时由用户显式复制到对应作用域。

## 行为与限制

- 记忆文件不可删除；清空时写入空内容。
- 智能体记忆为空时省略注入段；工作区记忆为空时注入“当前为空”的提示。
- 容量检查按编码后字节计算；注入截断不改变磁盘中的原文。
- 文件形态记忆统一走作用域 VDFS；事件形态记忆经声明端口落格与召回，不增加第二套持久化后端。
- `AGENTS.md` 不参与记忆注入，也不会被 Memory 插件自动改写。

## 测试入口

主要实现位于 `src/plugin.rs`、`src/memory.rs`、`src/workspace.rs`、`src/session.rs` 与 `src/memory_file.rs`。

```sh
cargo test -p symbio-plugin-memory
```

测试覆盖三层读写与注入、空文件行为、容量闸门、挂载并存和子树作用域隔离。
