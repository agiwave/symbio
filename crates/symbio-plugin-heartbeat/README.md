---
kind: design
id: crates/symbio-plugin-heartbeat/README
title: Heartbeat 插件（空闲会话的自主开轮调度）
status: active
created: 2026-10-18
impl: done
---

# Heartbeat 插件（空闲会话的自主开轮调度）

「会话空闲达到间隔之后，自动把一条提示词交进这个会话的收件箱」。**只回答什么时候该开一轮**——
间隔、错峰、空闲基线、那条常驻扫描循环，加上这一设置的两个入口（模型侧工具 + 会话选项栏那一行）。

「这次开轮是系统自己发起的」在事实源里的样子（触发 / 自检 / 欲 / 闸门四格）与运行参数的推导
**不在本插件**：那是会话存储所有者的写窗口纪律，住在
[`../session/transcript/autonomous.rs`](../symbio-plugin-session/src/transcript/autonomous.rs)，经会话的收件箱写臂进入
（归属裁决 [D-076](README.md "crates/symbio-plugin-heartbeat/README")）。机制的完整语义与实测节奏见
[`src/docs/113-heartbeat-mechanism.md`](src/docs/113-heartbeat-mechanism.md "DE-113-heartbeat-mechanism")。

## 为什么是一个独立插件，而不是 session 里的一个模块

判据是 [D-077](README.md "crates/symbio-plugin-heartbeat/README") 裁决一的三条，**缺一都不成立**：

| 判据 | 这一片怎么满足 |
|---|---|
| **卸载单位** | 启停动词只认**一层插件目录**；拆出独立插件之前，它是 session 里的一个模块，只能随 session 整体启停（唯一的停用对象是 session 整体，而停用它少掉的显然不是心跳一个功能） |
| **第二个消费单位** | 写标记的一方（本插件）与识别标记的一方（会话写臂）分属两个插件 ⇒ `meta.trigger` 这套词汇才是**共享**词汇，其单点定义住 `symbio_core::event` |
| **分形** | 本插件在 `ASSEMBLY_SUB_AGENT_PLUGINS` 清单里 ⇒ 逐项进每一棵子树，每个智能体各有一条自己的调度循环——与切片前「每个 `SessionPlugin` 实例各跑一条」数量相等 |

## 它对世界只说三句话，全部走地址面

| 一句 | 地址 | 用途 |
|---|---|---|
| 读清单 | `vdfs/list(session)` | 会话的 `status` / `updated_at` / `metadata`（含 `heartbeat` 子对象） |
| 开一轮 | `vdfs/write(session/<id>/inbox)` | 写体带 `meta.trigger` 声明；落格与入队由会话侧完成 |
| 存设置 | `vdfs/write(session/<id>)` | `metadata.heartbeat` 整块替换（浅合并按顶层键） |

**没有路由**（`route` 恒 `NotFound`）、**没有 VDFS 挂载点**、**没有自己的配置文档**。
它不持有会话插件实例，不认识事实源，也不认识会话的运行时状态表（E-007 / E-009）。

为什么每句都走地址面而不是进程内调用：切片要消掉的正是「一片代码通过宿主的私有项读写别人的存储」。
读写别人的存储就用**给别人用的那条通道**——于是这一片能不能被停掉，是个真问题而不是修辞。

## 组成

| 文件 | 职责 |
|---|---|
| [`plugin.rs`](src/plugin.rs) | 插件本体：能力行贡献（Tool + Option 两行）、常驻循环的启动与停机 |
| [`scheduler.rs`](src/scheduler.rs) | 扫描循环：tick、错峰、空闲基线（判空闲的算术是纯函数 `due`，逐条可测） |
| [`setting.rs`](src/setting.rs) | `Session.metadata.heartbeat` 那块数据的形状（本片写、会话侧透传） |
| [`tool.rs`](src/tool.rs) | 模型侧设置入口（`heartbeat` 工具：set / get / cancel，两跳：先读后写补齐部分更新） |
| [`option.rs`](src/option.rs) | 会话选项栏那一行的声明（字段 + 子表单定义，保存走前端已在用的 `option` 写通道） |

## 生命周期

- **启动点在 `build`**：`Plugin::start` 是**同步**钩子（D-033，装配路径整条同步），循环体是 task，
 只能在构造点 `Handle::try_current` + `handle.spawn`。没有 runtime 时一条不起（裸构造的单测）。
- **停机** = 一个停机位 + 一个唤醒 + 一个有界等待（沿用 D-075 P1 的形状，本片只有一条循环，
 句册因此退化成一格，判据不变）。循环体**先挂唤醒再读位**，反序会留一条竞态窗口。
- **每 tick 在触发前重查停机位** ⇒ `stop` 之后最多走完当前这一轮，此后一条也不触发。

## 配置

**没有 `PLUGIN.yml` 的可调项**：可调的东西全是**会话的**属性（间隔 / 提示词 / 携带历史），
落在 `Session.metadata.heartbeat`；扫描节奏（tick 15s / 每 tick 上限 2 / 抖动 0-30s）是机制参数，
不开放配置——做成一份配置文档只会让「一个会话的空闲判定」有两处可调的地方。

## 守卫

停用它 ⇒ 少的是这一个功能，不是整个界面。判据两侧各钉一半（跨插件的判据不许靠 import 兄弟模块，E-009）：

- 本片 [`plugin.test.rs`](src/plugin.test.rs)：`traverse` put 两行 + 没有第三行 + `stop` ⇒ 句柄已 finished。
- 容器侧 [`../composite/composite.test.rs`](../symbio-plugin-composite/src/composite.test.rs)：经**工厂 id + 真实 `mount_all`**
 装配，`set_enabled("heartbeat", false)` ⇒ Tool 行与 Option 行的 `heartbeat` 都不在，会话自有三项与
 `session` 挂载点照旧，`vdfs/list` / `write(session/<id>)` / `write(<id>/inbox)` 三条通道照常。

## 关联

- 切分与落点裁决：[D-077](#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出)（本插件的存在理由）
 / [D-076](#d-076-心跳片的八个宿主触点逐条落位调度随片走触发事实落进收件箱写臂)（谁有资格开这个会话的事实源）
- 会话那一侧的入口：`../session/plugin/nodes.rs` 的 `trigger_declaration`、
 `../session/transcript/autonomous.rs` 的 `request_autonomous_round`

## 关键决策

> 本节是**仍然约束现行代码的取舍**：每条一句决策、一句理由。已退场或主题已消失的决策不在其中（`git log` 是历史）。

### D-076 心跳片的八个宿主触点逐条落位：调度随片走，触发事实落进收件箱写臂

[D-075](../../docs/003-design/002-架构总览.md "DE-002-架构总览") 把切片判据立成「被切的那片不再 `use super::` 宿主的私有项」，并点名心跳为第一片。逐口核对后，心跳对宿主的全部私有触点共八条；本决策逐口给出归属。

### D-077 心跳片的落点定为一层的插件目录：卸载单位与第二个消费单位都由它给出

[D-076](README.md "crates/symbio-plugin-heartbeat/README") 把心跳裁成两件事并逐口定了归属，落格与参数推导已经搬进 `transcript/autonomous.rs`；那条末尾留下一道没答的题——「标记词汇表进不进 core 由调度半的落点决定」，而那个落点本身是新的形状裁决。本决策就答这一题：调度半（间隔 / 抖动 / 空闲基线 / 常驻循环 / 设置工具 / 选项族那一行）住在哪。

