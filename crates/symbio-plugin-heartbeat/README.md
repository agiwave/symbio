---
kind: design
id: crates/symbio-plugin-heartbeat/README
title: Heartbeat 插件（空闲会话的自主开轮调度）
status: active
created: 2026-10-18
impl: done
---

# Heartbeat 插件（空闲会话的自主开轮调度）

## 职责

Heartbeat 只回答“何时给空闲会话开一轮”。达到间隔后，它把提示词写入目标会话收件箱并推进心跳元数据；
不负责模型调用、工具执行、会话恢复或运行时进程管理。

专题说明见 [Heartbeat 机制](src/docs/113-heartbeat-mechanism.md "DE-113-heartbeat-mechanism")。

## 公开地址面

插件无独立 `route`、无 VDFS 挂载点，只使用以下会话地址面：

| 地址 | 用途 |
|---|---|
| `vdfs/list(session)` | 读取状态、更新时间与 `metadata.heartbeat` |
| `vdfs/write(session/<id>/inbox)` | 写入 `meta.trigger` 触开一轮 |
| `vdfs/write(session/<id>)` | 替换 `metadata.heartbeat` |

写收件箱优先使用运行时的 `dispatch_session_invoke`；没有运行时时才使用构造期 provider 兜底。
触发失败不推进调度锚点，下一个 tick 重试。

## 配置

配置位于 `Session.metadata.heartbeat`：

| 字段 | 含义 |
|---|---|
| `enabled` | 是否启用，默认 `false` |
| `interval_seconds` | 空闲多久后触发，默认 300 秒 |
| `prompt` | 写入收件箱的提示词，默认空；空白提示词不触发 |
| `include_history` | 是否随触发提示带入历史，默认 `true` |

固定机制参数为 tick 15 秒、每 tick 最多 2 个会话、相位抖动 0–30 秒。
会话正在工作时不触发；空闲基线取最近一次开轮锚点与 `updated_at` 中较晚者。

## 生命周期与限制

- 同步 `start` 中启动后台任务；停止使用停止位、唤醒信号与有界等待。
- 先挂唤醒再读取停止位，每次触发前再检查停止位，避免停止与唤醒之间丢失信号。
- 插件只写当前会话允许的地址面，不扫描任意目录，不直接读取模型或工具实现。
- 心跳是自动开轮信号，不是保活进程；宿主停机后不再产生新触发。

## 组成与测试入口

主要目录为 `src/plugin.rs`、`src/scheduler.rs`、`src/setting.rs`、`src/tool.rs`、`src/option.rs`。

```sh
cargo test -p symbio-plugin-heartbeat
```

单元测试覆盖调度、提示词与停止竞态；composite 的真实 `mount_all` 停用测试覆盖插件的启停边界。
