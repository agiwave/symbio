# 结构化 vs 拍平：真实模型上的对照读数

> ⚠️ **生成物，不手改。** 唯一写者是 `node e2e/ab/run.mjs`。
> 判据口径归 `e2e/ab/tasks.mjs` 与 `e2e/ab/flatten.mjs`，
> **结论与边界**归 [`docs/registry/task/S-end③.md`](../registry/task/S-end③.md)（本文件不写结论）。

生成于 2026-10-12T05:07:12.486Z · 模型 `MiMo-V2.6-Flash`（provider `AMD-MiMo-V2.6-Flash`）· 题 3 道 × 两臂 × 1 次
温度 0（两臂同值，实验只认「两臂一样」不认「出厂默认」）· max_tokens 4096 · 两臂的信封差别只有 `tools` / `tool_choice` 是否存在
上下文来源：真实链路捕获（场景「full-主线装配-工具轮-对话面关闭」，captured 2026-10-12T04:28:14.815Z）· system 前缀 2 条 · tools 21 个

## 汇总（判据列）

| 判据 | 臂 A 结构化（出厂） | 臂 B 拍平 |
|---|---|---|
| select：选对工具（`tool_name_hit`） | 1/1 | 0/1 |
| select：参数是合法 JSON（`args_json_ok`） | 1/1 | — |
| select：必填键齐全（`required_keys_hit`） | 1/1 | — |
| select：调了不存在的工具名（越少越好） | 0/1 | 0/1 |
| control：不该调用却调用（越少越好） | — | — |
| attribution：结果↔调用对应得回来 | 0/2 | 0/2 |
| 拍平侧：文本协议被遵守（臂 A 不适用） | — | — |

## 观测列（**不进判据**，与 ADR-057 同一取舍）

| | 臂 A | 臂 B |
|---|---|---|
| prompt token 合计 | 17891 | 14843 |
| 请求体字节合计 | 42857 | 42743 |
| 墙钟合计（ms） | 14330 | 15473 |
| provider 自报账单（usage.cost，合计） | 0.000205 | 0.000827 |
| 未收束的题数（已从判据分母剔除） | 0 | 0 |
| 请求出错的题数 | 0 | 0 |

**样本健康度**：两臂合计 0 道题未收束、0 道请求出错；重复之间可比对 0 组，其中 0 组换一次时刻就跑出了不同结论。「未收束」不计入上面任何一条判据的分母——本机服务坏过一整段时会把回答刷成 `<unused32>` 复读，把它读成「模型没做到」就是拿环境噪声当读数。

## 逐题（同一道题两臂并排；`×k` 是第 k 次重复）

| 题（臂 A vs 臂 B） | A 调用 | A hit | A 参数 | A 关联 | A 答 | B 调用 | B hit | B 参数 | B 关联 | B 答 | B 协议 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sel-agent（select） | agent_run | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `{"tool": "agent_run", "arguments": {"pro` | – |
| attr-out-of-order-same-name（attribution） | — | – | –/– | ✗ | `plan` | — | – | –/– | ✗ | `plan` | – |
| attr-five-way-reversed（attribution） | — | – | –/– | ✗ | `200` | — | – | –/– | ✗ | `200` | – |

`类`：`select` = 该调哪个工具；`control` = 不该调用；`attribution` = 结果与调用的对应关系。
`hit` 后的 ⚠ = 调了工具面上不存在的名字。`参数` 列是 `合法JSON/必填键齐`。`—` = 未调用/未作答，`–` = 该题不适用。

## 逐题原文（臂各自实际送出与收到的东西）

### sel-agent（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 9965ms · prompt 5860tok · 13692B
  - 送出：`agent_run({"prompt": "请调研当前工作仓库中的「压缩策略」（compression strategy），并写一份小结。具体要求：\n\n1. 在仓库中搜索与压缩相关的实现与配置（关键词可包括：compress、compression、gzip、deflate、brotli、zstd、lz4、zip、archive、pack、compact 等，视仓库实际技术栈调整）。\n2. 梳理清楚：仓库在哪些环节/模块使用了压缩？分别采用什么算法或库？触发条件、配置项（如压缩级别、阈值）、以及默认行为是什么？\n3. 关注压缩的输入输出（如网络传输、文件存储、数据序列化等场景）以及是否`
- **B 拍平**：finish=stop · 10821ms · prompt 4835tok · 13860B
  - 正文：`{"tool": "agent_run", "arguments": {"prompt": "调研本仓库的压缩策略并写一份小结：请先了解仓库整体结构，然后重点查找与「压缩」（如 compression、gzip、deflate、brotli、zstd、pack、archive 等关键词）相关的实现、配置与调用链路，最后`
### attr-out-of-order-same-name（attribution）· 第 1 次
- 测的是：同名调用 + 乱序结果 + 正文不回显路径 ⇒ 判据测的就是 tool_call_id 这一层关联
- 候选条数 2 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 2 分之一
- **A 结构化**：finish=stop · 3255ms · prompt 5965tok · 14294B
  - 正文：`plan`
- **B 拍平**：finish=stop · 2477ms · prompt 4948tok · 14276B
  - 正文：`plan`
### attr-five-way-reversed（attribution）· 第 1 次
- 测的是：候选到 5 条且完全逆序 ⇒ 猜的基线降到 1/5，而「按到达顺序」的启发式在这里必然指错
- 候选条数 5 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 5 分之一
- **A 结构化**：finish=stop · 1110ms · prompt 6066tok · 14871B
  - 正文：`200`
- **B 拍平**：finish=stop · 2175ms · prompt 5060tok · 14607B
  - 正文：`200`
