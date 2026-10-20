---
kind: task
id: T-203-CLI与src-tauri收口批
title: CLI 与 src-tauri 收口批——交互卡有播报、机制纪律同判据、连接管理有单测
status: done
created: 2026-10-20
track: plan
prio: next
deps: []
parent: T-183-质量与机制清偿总批
where: cli/src/ · scripts/mechanism-audit.mjs · tauri/src-tauri/src/
exit: 'CLI 的自动模式遇到等待用户动作时明确播报（哪张卡、缺什么答复、为什么不自动继续），覆盖「卡在等审批」与「卡在提问」两种形态；`cli/src` 纳入机制门禁扫描范围（至少两条：不得直读业务 `meta` 字段、地址段名常量单点定义，三处常量收进一个单点）；`src-tauri` 补最小组测试（连接超时清理与移除全部、命令层 EOF 取消并发出帧路径各一条）；[T-137](137-CLI交互卡无呈现与应答出口.md "T-137-CLI交互卡无呈现与应答出口") 的不做理由随之改为有名字的有限承诺或由本批取代；全量门禁与 e2e 全绿'
evidence: 'CLI：新增 `cli/src/protocol.rs`（协议读法唯一点：SESSION_SEG / MESSAGES_SEG / OUTCOME_ABORTED 三常量 + meta 字段解释 + 等待卡片识别），`render.rs` 对 WaitingUserAction 的审批卡 / 提问卡播报「哪张卡、缺什么答复、为什么不自动继续」，auto 非交互模式以失败码收尾、interactive 按协议以「停在等用户」正常退出（`render.test.rs` 播报文案两种卡形 + 状态机判据，`protocol.test.rs` 卡片解析）；门禁：`mechanism-audit.mjs` 新增 M-009（cli/src 不得直读 meta）/ M-010（地址与协议词常量只能在 protocol.rs 定义）并补 5 条回归（31 条全绿）；tauri：`route_connection.test.rs` 超时清理 + remove_all 两条、`commands.test.rs` EOF 取消 + 并发帧两条（cargo test -p symbio-tauri 4 passed）；T-137 reason 改为「只播报、不应答」的有限承诺。'
---

## 内容

父设计：15 报告建议 2 / 4 / 5 · 总报告 C7

**三件都是「CLI 与桌面端这两条薄链路」的空白**：

| 空白 | 现状 | 后果 |
|---|---|---|
| CLI 交互卡 | 自动模式下跑到审批就**静默卡住** | 实际可触发的失败模式：用户以为在跑，其实在等一个看不见的回答 |
| `cli/src` 不在机制门禁扫描范围 | 只有 4 文件 1799 行，但零判据 | CLI 与 Tauri 的机制纪律是两套做法；段名失配静默丢弃 |
| `src-tauri` 零单测 | 连接生命周期纯靠人工实机 | 超时清理与 EOF 路径没人守 |

**CLI 交互卡这条要说清分寸**：完整形态（渲染成可回答的一行并收回答）成本更高，
原报告建议先做**播报形态**——「明确告播」本身就把「静默卡住」这个失败模式消掉了。
[T-137](137-CLI交互卡无呈现与应答出口.md "T-137-CLI交互卡无呈现与应答出口") 已判 `wont`
（理由是它假设的渲染对象随前端机制化改造退役了），本批要么按新协议重立这条承诺，
要么明确「CLI 只播报不回答」——**两者都比 `wont` 里那句「等改造完成后按新协议重画」更诚实**。

**为什么 `cli/src` 值得纳入门禁**：它小，但它是**唯一的另一个前端**。
机制纪律只在 Tauri 侧生效，等于「同一份协议两种读法」，而这正是本仓反复消灭的形态。
两条规则（不直读业务 `meta`、段名常量单点）是最小可用集，不追求一次覆盖。

## 处理过程

- 2026-10-20 立批。来源：15 报告建议 2 / 4 / 5 + 总报告 C7 的 cli 一半。
- 2026-10-20 与 [T-202](202-前端协议面守卫批.md "T-202-前端协议面守卫批") 的分工：
  T-202 管 Tauri 侧的跨栈镜像与 `meta` 白名单，本批管 CLI 侧的机制纪律与 `src-tauri` 的单测。
  两批的共同目标是「两条薄链路用同一套判据」，但落点不同（前端协议 vs 机制扫描范围）。
- 2026-10-20 一处待裁决：CLI 交互卡走「只播报」还是「播报 + 应答」。本批允许分批——
  播报先落地（消掉静默卡住），应答出口若成本过高，另立任务而不是拖着整批。
- 2026-10-21 收口。裁决为**只播报**：终端应答要另立行编辑 + `responds_to` 回填协议，
  不在本批成本内；[T-137](137-CLI交互卡无呈现与应答出口.md "T-137-CLI交互卡无呈现与应答出口") 的 `wont` 理由随之改成这条有限承诺。

## 结果

| 空白 | 落点 | 判据 |
|---|---|---|
| CLI 交互卡静默 | `cli/src/protocol.rs` 识别 `Interaction + WaitingUserAction`；`render.rs` 播报卡片名、缺的答复（通过/拒绝、回答几个问题）与不能继续的原因；`main.rs` 在 auto 下以失败码收尾（interactive 下「停在等用户」按协议正常退出） | `protocol.test.rs` 两种卡形解析 + `render.test.rs` 两种卡形的播报文案、等待/收束状态机 |
| CLI 不在机制扫描范围 | `mechanism-audit.mjs` M-009（`cli/src` 不得直读 `meta`）/ M-010（地址与协议词常量只能在 `protocol.rs` 定义，三常量已收进该文件） | `mechanism-audit.test.mjs` 两条规则各一命中 + 一放行，31 条全绿 |
| `src-tauri` 零单测 | 超时清理抽成 `sweep_expired`（清理任务与测试同一实现）；转发泵抽成 `spawn_forward_pump`（只依赖 rx / cancel / emit） | `route_connection.test.rs` 2 条 + `commands.test.rs` 2 条，`cargo test -p symbio-tauri` 4 passed |

**分寸**：CLI 仍然没有应答入口。`auto` 模式下审批与 `ask_user` 大多在产卡前就返回
`PERMISSION_DENIED` / `TOOL_UNAVAILABLE`（后端既有行为），但子智能体转播的待审批与
`--mode interactive` 的会话确实会产出卡片——本批让**只要卡片到达就播报**，不再依赖
「哪种模式不会产卡」的假设；`-q` 仍保留一行紧凑终态，不恢复静默。

