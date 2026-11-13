---
kind: design
id: crates/symbio-plugin-task/README
title: 任务能力包（task）
status: active
created: 2026-11-13
impl: done
---

# 任务能力包（task）

任务清单 → 任务事实 → 三个消费方。它拥有「任务」这条能力在一个回合里的三个时点：

| 时点 | 形态 | 扩展面 |
|---|---|---|
| 工具执行之后 | `todo_write` 声明 → `task.*` 事实草稿（含返工节点） | 效果端口（`task::TaskTable`） |
| 请求组装之前 | 就绪集渲染成一段提示（段身份 `READYSET`，prio 70） | 上下文 provider |
| 本轮收束之后 | 就绪集 → `task.ready` / `task.assigned` | 收束贡献（`D-250`） |

三者都只**交草稿**：写口仍是宿主的唯一写入口（I1），判断与成格住本插件。`core` / `kernel`
不认识「任务」这个词——格位、角色与名字都在 `decl`，读侧按 `TASK_VOCAB` 认名字，
跨插件消费经 `PLUGIN.yml` 的 `shared_events` 投影（`task.asserted` / `task.held` /
`task.opened`；`task.assigned` 的租约无外部消费方，名字随 owner 走）。

## 域内分工

| 文件 | 职责 |
|---|---|
| `decl` | 事件声明、角色（`CountsRework`）、词汇表与 `task.readyset` 的报表登记 |
| `readyset` | 就绪集折叠：谁就绪、什么序；容量是**读者**套上的预算 |
| `task` | 声明 → 事实的折叠（开 / 推进 / 终态 / 返工）、调度判据、调度段排版、收束贡献 |
| `context` | 把调度段交给请求视图 |
| `plugin` | 插件本体：无工具、无路由，只是装配挂载点 |

内部依赖单向：`context` / `task` / `readyset` 只读 `decl` 的名字，`plugin` 不认识其余模块。

## 六条口径（为什么是这样）

- **只记声明，不记全量**：`merge = true` 是增量、`false` 是整体替换，本模块两种模式行为
  相同——事实源只认被声明到的项，清单里没提到的任务一律不动。
- **整体替换移出清单的项保持未终态**：append-only 不替模型编一条「已验收」的假事实。
  代价写在明处：它仍留在事实源里、也可能仍在就绪集里，读出口会念出一个清单上已没有的 id。
  要表达「不再做」得先有 cancel 语义的事件，而 `task.*` 词汇表里没有这一格。
- **降级不产事实**（进行中 → 未开始）：`task.progress` 是不可撤销的推进记录，清单改回
  `pending` 只是模型改了主意，没有新事实发生。

就绪判定是**纯函数**、从事件序列算：`ready = 非终态 ∧ depends_on 全部终态 ∧ 无活租约 ∧
未挂起`。调度器不持任何内部状态——同一份事件切片永远算出同一个就绪集（N1）。容量
（`readyset_cap`）是本轮配置**事实**，不是折叠的形参：调度段、提示段与统计列读同一份史，
回放必然得到同一个容量（`D-252`）。`cap = 1` 是本阶的平凡值——调度器、租约、仲裁一行不改，
系统退化为串行 FIFO。

## 授权

调度者只持 `assign.work`：发放的权威动作是**派活**，不立任务（`define.work`）、不发言
（`reply`）。这一行写在 `PLUGIN.yml` 的 `principal_scheduler`，由装配投影进授权面。

## 关联

- 任务族与时点：`docs/003-design/033-多步任务与返工.md`、`docs/003-design/034-并发调度与租约.md`
- 就绪集折叠搬 owner 的裁决：`D-289`、`D-490`
- 收束贡献的形状：`crates/symbio-plugin-api/src/ring.rs`
