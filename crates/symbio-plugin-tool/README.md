---
kind: design
id: crates/symbio-plugin-tool/README
title: 工具世界能力包（tool）
status: active
created: 2026-11-13
impl: done
---

# 工具世界能力包（tool）

`port.tool` 的提供者：把一次工具调用（`PortInput`）执行成结果事实（`PortOutcome`）。
**不持有轮次语义**、不做批编排、不写协议事实——那些归环的通用受理入口
（`execute_one` / `run_port`）。

端口名 `tool` 由本插件自己定义，行声明与端口声明都随本插件提交，所以接入工具世界
**不需要改 loop 任何文件**。

## 它交什么

| 面 | 交出物 | 说明 |
|---|---|---|
| 端口 | `tool`（执行 / 路由 / 结果提取） | 环按端口名解析提供者 |
| 债 | **一调用一笔**：键 `tool:{request_id}:{call_id}` | 只问「有没有 `tool.call.requested` 这条标记」，不读 `tool.requested` 的 `calls[]` |
| 事实 | `artifact.added`（本次调用产物）、`authz.granted`（授权草稿）、`tool.calls.done`（调用收束） | 名字经 `shared_events` 投影，`artifact.added` 由本插件端口写出（`after_port`） |

## 五个执行体内部的判据

| 文件 | 判什么 |
|---|---|
| `breaker` | 时延 / 预算熔断：环**不判快慢**，只把中性输入（授权结论 / 已耗 / 申请 / 总预算）随债交过来；判定、「熔断中不发」与那条 `control × opened` 事实都由执行体自己落 |
| `result_guard` | L0 单次结果守卫：超 token 预算时把全文存档到会话存档目录（可用 `local/file_read` 取回），正文做 head/tail 摘要（前 60% + 后 40%，尾部通常是结论/错误），中间插占位说明；存档按修改时间保留最新 20 份 |
| `round_done` | `tool.calls.done` 的簿记写方：收尾是**投影**，不是执行体的最后一步——宿主每次驱动前后把全量投影交过来，它按「每一笔的最大 attempt 都有收束」重算并补写 |
| `debt` | 债键与恢复：**恢复按 `call_id` 一对一**，同一请求的多张卡各欠各的恢复（不再「一次恢复消费整批」），且与首派行同 id ⇒ 收束落在同一 `(port, id, attempt)` 上，`attempt` 不加、不开新债 |

「这批调用齐了」**不写在任何进程的收尾语句里**：执行体中途死掉也不会有「永久未完成」——
那一笔由租约收束（重试或判死），收束落进同一投影后，`round_done` 下一圈就把它算进去。
幂等由事实键承担：同一 `request_id` 只长出一条完成事实。

## 关联

- 端口与派发面：`docs/003-design/022-行表与派发面.md`、`docs/003-design/048-端口机制裁决.md`
- 外部执行与熔断：`docs/003-design/039-外部执行与熔断.md`
- 工具调用与产物：`docs/003-design/032-工具调用与产物.md`
