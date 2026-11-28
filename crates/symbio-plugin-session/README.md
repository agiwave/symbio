---
kind: design
id: crates/symbio-plugin-session/README
title: Session Orchestration Architecture (会话编排架构说明书)
status: active
created: 2026-07-20
impl: done
---

# Session Orchestration Architecture (会话编排架构说明书)

## 职责与边界

Session 是会话持久化、事实源、地址面、装配与生命周期中心。

- 收件箱队首被写成 `user.message` 事实，宿主看到新事实后单次驱动该轮。
- Session 只写事实、读事实，不直接调用模型、执行工具或拥有运行时 worker。
- WAL 是事实真源；转写、请求投影与 UI 树是读面。当前只承诺磁盘驻留会话。
- 每轮 `round_ctx` 独立，重置 Agent 权限、沙箱句柄与端口容器；开轮重挂工作区和会话 provider，
  保留已有 `AGENT_VDFS`。
- 流式进行态与增量不落盘；只有收口节点和终态事实进入持久层。

## 公开地址面

### 会话生命周期

- 创建、读取、更新、删除、停止与恢复会话。
- 会话删除由控制面先停 worker 子树，再删除对应 VDFS 数据。
- 嵌套会话按显式逻辑资源路径路由，metadata 中的会话上下文不能覆盖路径叶节点。

### 收件箱与补充整合

- 收件箱支持追加、列出、认领与删除；同一会话严格 FIFO，忙时排队，不抢占正在执行的轮次。
- 判空、占轮和出队在单会话状态锁内完成。首条消息未落盘时回滚占轮并回队；
  已落盘但后续装配失败时不重放，补失败终态。
- 启用补充整合时，批次边界最多抽取 `supplements_max_per_drain` 条，合并为一条 User 消息。
  纯文本以空行连接，混合片段保持顺序与类型；元数据保留全部原条目 id 与数量。
- 合并消息经唯一写入口落为 `message.added`。关闭补充整合时退化为一条消息一轮。
- 仍在排队的条目可以删除；已经出队的条目通过会话 abort 中止，不按队列项删除。

### 委托执行

`agent_run` 是当前公开委托入口，支持三种形态：

| 形态 | 主要参数 | 行为 |
|---|---|---|
| 新建 | `agent_id`、`prompt`、`working_dir?`、`session_id?` | 在目标作用域创建子会话并入队 |
| 续会话 | 已有 `session_id`、新 `prompt` | 作为目标会话下一轮追加 |
| 恢复 | `session_id` 或 `target_id` | 走普通工具恢复机制 |

- 候选来自作用域 VDFS，执行期重新校验目标与 epoch。
- 子会话经 `session.subagent` 请求级端口和 durable inbox 启动或复用。
- 过程经 `ExecEventSink` 转播，工具不返回事件通道；结果末尾附 `[subagent_session_id]`。
- 审批使用 collectors 写入的可信标记，不认模型伪造的 `args.approved`。

### 工具结果读取

工具结果文本按以下顺序抽取：

1. 字符串 `content`；
2. 字符串 `output`；
3. `success` 布尔：成功时序列化整包，失败时输出 `Error: <error>`，缺错误时为 `unknown error`；
4. 顶层字符串；
5. 其余对象序列化为 JSON。

控制流只使用明确的 `failure_kind`，不从 JSON 文案猜测审批、取消或重试意图。

## 配置

配置 owner 是 `SessionConfig`；完整字段与默认值见 `src/config.rs`。

| 字段 | 默认值 | 当前用途 |
|---|---:|---|
| `max_messages` | 100 | 按 User 轮 FIFO 裁剪历史，0 表示不限制 |
| `context_messages` | 6 | 当前只作为历史工具链裁剪分水岭 |
| `prune_tool_history` | true | 删除分水岭前的 Tool/ToolCall/Reasoning 及子节点 |
| `supplements_enabled` | true | 是否把收件箱补充条合并为一条 User 消息 |
| `supplements_max_per_drain` | 20 | 每次抽干最多合并的条目数 |
| `opening_enabled` | false | 是否生成开轮提示 |
| `progress_enabled` | false | 是否启用进度汇报 |
| `progress_interval_ms` | 60000 | 最小汇报间隔 |
| `progress_max_per_turn` | 5 | 每轮汇报上限 |
| `progress_min_rounds` | 1 | 开始汇报前的最小轮数 |
| `progress_lease_ms` | 60000 | 汇报租约时长 |
| `progress_max_attempts` | 1 | 单轮最大汇报尝试 |
| `presentation_tree` | `grouped` | 展示树投影形态 |

会话只保存 `sandbox_id` 选择与 `risk_level` 自动放行偏好；最终有效阈值、审批范围与永久授权
由 sandbox 档案策略决定，会话配置不再持有 `approval_grants`。

`context_messages` 不是加载窗口。历史仍从 WAL 读取并投影，只有工具链裁剪以该字段为分水岭。

## 裁剪与投影限制

- `max_messages` 只裁剪消息节点，不删除 `tool_archives/` 中的关联存档。
- `prune_tool_history` 只删除工具与推理节点，不删除存档；存档回收归 tool 插件的容量守卫。
- `replace_all` 只统计孤儿存档，不执行存档删除。
- 请求体来自事实网格投影；提示词、用户输入与工具结果是事实，展示树不是事实真源。
- 同一会话同一时刻只允许一个活动执行体；恢复与审批不得绕过 epoch 或作用域校验。

## 配置与文档入口

- 公开设置与插件声明：`PLUGIN.yml`、`src/setting.rs`。
- 轮首装配与收件箱：`src/plugin/round.rs`。
- 持久化与裁剪：`src/chat_session/write.rs`。
- 委托工具：`src/tools/subagent.rs`。
- 专题说明见 [Session 文档入口](src/docs/README.md "crates/symbio-plugin-session/src/docs/README")。

## 测试入口

```sh
cargo test -p symbio-plugin-session
```

测试覆盖 WAL 重建、transcript、轮首装配、历史裁剪、补充提示、委托续会话与恢复、作用域 provider 和删除路由。
