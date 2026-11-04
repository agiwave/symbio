---
kind: task
id: T-034-旧loop门禁资产下线清单
title: 旧 loop 门禁资产下线清单：删 / 撤接线 / 保留改造
status: doing
track: arch
prio: now
created: 2026-11-04
deps: []
where: "零代码改动；只清点 scripts/ 与 docs/005-spec 里绑定旧 symbio-plugin-loop 的判据资产"
exit: 逐项列出绑定旧 crate 的门禁资产与三档处置；标出所有写死旧路径 / 旧测试总数的位置；给出删除后 gate 各阶段的可运行结论
---

# 旧 loop 门禁资产下线清单

## 内容

## 0. 结论

用户裁决：`symbio-plugin-loop` 与全部旧代码可删，过渡态不要求绿。因此**只有两条屏障保留改造**，其余按「删」或「撤接线」处理：

- **直接删**：`control-plane-audit`（CP-001…009 / TC-1…6）、`turn-root-audit`（F1…F5）、`baseline-scope-audit`（BS-001…005），连同它们的基线/测试。
- **留文件、撤接线**：`engine-ledger-audit`、`obligation-ring-accept`、`capability-migration-audit`、`extension-locality-audit`、`facts-boundary-audit`；逻辑可改指新 crate，但当前不跑。
- **保留并改造**：① 环 crate 零世界知识检查（旧 LP-003 / LP-006 精简后指向新 crate，含 `known-red` 机制）；② e2e 保留集——**定义权在新会话的 e2e 分类表**，`t96` / `t97` 断言的 `turn.stage` / `stage` 取值 / `round` 编号 / 幕与阶段事件都属于引擎细节，属删除或改写对象，不是保留契约。
- **其余连带**：`grep-audit` 的四条 loop 规则、`arch-shape-audit` 的 CS-010、`plugin-surface-audit` 的控制面名单，以及 `_shared.mjs` 的测试总数——它们不是旧 loop 专属，但会在删 crate 时连带红。
- **与 session 同批删**：`scripts/exemptions/plugin-session-sdk.json`（E-009 对 `symbio-plugin-session` 的 `symbio-sdk` 限期豁免）。**session 退役时同批删除本豁免**；只删 crate 不删豁免 = 空头豁免，E-009 直接判红。

删完后的「绿」只剩：**未删的架构（kernel / sdk / 前端）自己的判据 + 上面两条屏障**。旧控制面终态（六条不变量、TC-1…6）不再有判据，也不再是「绿」的一部分。

## 1. 资产逐项

| # | 资产（路径） | 它守的是什么 | 处置 | 折/依赖 |
|---|---|---|---|---|
| 1 | `scripts/loop-purity-audit.mjs`、`scripts/loop-purity-audit.test.mjs`、`scripts/loop-purity-baseline.json` | LP-001 端口载荷 import、LP-002 Cargo 依赖、LP-003 世界语义词根、LP-004 端口受理面命名、LP-005 端口签名、LP-006 行表序号键 / 阶段值 | **保留并改造**：只留 LP-003 + LP-006，指向新 crate；LP-001/002/004/005 随旧端口面退役 | 基线按**文件**记 41 个数（`control_decl 109`、`entry 183`、`tool_dispatch 236`、`chat_loop.rs 85`、`ring 3`、`stage_events 38`…）；测试夹具写死 `loop_rows.rs` / `control_decl.rs` / `ring.rs` / `stage_events.rs` 与 `SESSION_ID_*` / `STEP_*` 常量 |
| 2 | `scripts/control-plane-audit.mjs`、`.test.mjs`、`scripts/baselines/control-plane.json` | CP-001…009：驱动循环 / 选债派发 / 守卫 / 内核行名 / 收束格 / 机制冻结 / 规范 / seal / 第二推进循环；TC-1…6 的验收 | **直接删** | 基线写死 `mechanism.file = crates/symbio-plugin-loop/src/ring.rs`、`parts = [drive loop body, fn next, fn waiting]`、`hash = sha256:907124…`、ADR 链 `D-234…D-320`；测试夹具现造 `ring.rs` / `loop_rows.rs` / `turn_exec.rs` |
| 3 | `scripts/baseline-scope-audit.mjs`、`.test.mjs` | BS-001…005：两个棘轮基线的新增键 / 计数上调 / 删键 / 机制面 file·hash 变化 | **留文件、撤接线**（逻辑可改指新基线；当前无对象） | 写死 `scripts/loop-purity-baseline.json` 与 `scripts/baselines/control-plane.json` 两个路径 |
| 4 | `scripts/known-red.json`、`scripts/known-red.mjs` | 红债务台账：16 条（U3×11 / U4×3 / U2×2）+ D-321 豁免 | **保留并改造**：机制留，16 条随旧文件清除而删；新 crate 的债从零起。**D-321 的安排随 `turn.stage` 删除失去依据**（用户裁决：`turn.stage` 属引擎细节，不保留） | 每条 `file` 都指向 `crates/symbio-plugin-loop/src/*`；`adrExists` 读 `docs/003-design/048` |
| 5 | `scripts/engine-ledger-audit.mjs`、`.test.mjs` | EL-001 行表行数（`Decl` 计数）、EL-002 通道、EL-003 全量读点数 | **留文件、撤接线**（`Decl` 解析器可复用） | 写死 `LOOP_ROWS` / `DISPATCH` / `SCAN_FILES`（`loop_rows.rs` / `tool_dispatch.rs` / `port.rs`）；`BASELINE = { rows:16, channels:1, fullScans:9 }` |
| 6 | `scripts/turn-root-audit.mjs`、`.test.mjs`、`scripts/turn-root-baseline.json` | F1…F5：`Turn` 标识符、`turn.*` 字面量、DTO 字段 | **直接删**（turn 整体废弃，空棘轮无意义） | 基线按面记 `F1 1251/870`、`F2 114/264`、`F4 4/12`、`F5 1`；扫 `crates/**` |
| 7 | `scripts/obligation-ring-accept.mjs`、`.test.mjs` | J1…J9 义务环运行期验收（配对、幂等、恢复） | **留文件、撤接线**（J 逻辑可改指新环的事实） | 消费 `turn.stage` 与 `st-t{turn}-{stage}` 事件 ID；`30-docs` 的 GUARDS 与 LIBS 各接一处 |
| 8 | `scripts/capability-migration-audit.mjs`、`.test.mjs` | 迁移批次的能力归属：换实现不换能力 | **留文件、撤接线**（可改指新 control tree） | `CONTROL_TREES = ['crates/symbio-plugin-loop', 'crates/symbio-plugin-session']` |
| 9 | `scripts/extension-locality-audit.mjs`、`.test.mjs` | 扩展局部性：四棵树的机制面指纹 | **留文件、撤接线** | `CONTROL_TREES` 写死 `crates/symbio-plugin-loop`；指纹按文件 |
| 10 | `scripts/facts-boundary-audit.mjs`、`.test.mjs` | FB-002：能力文件零存储知识 | **留文件、撤接线**（能力文件表改指新 crate） | `CAPABILITY_FILES` 写死 `.../memory_events.rs`、`.../derived_facts.rs` 等 |
| 11 | `scripts/grep-audit.mjs` 的 S-011 / S-015 / S-016 / S-017 | 注入点次序、收束闸单 owner、停机原因单形状、恢复无旁路 | **保留文件、摘这四条规则** | 规则锚点写死 `turn_runner/stream.rs`、`turn_runner.rs`、`turn_runner/facts.rs`、`closure_gate`、`resume.rs`；文件删后 S-011 会「跳过」而不是失败 |
| 12 | `scripts/arch-shape-audit.mjs` 的 CS-010 | 控制面草稿 `TurnState` 字段数 | **保留文件、摘 CS-010 段**（CS-001/009 的 core 形状继续） | `TURN_STATE_REL = .../chat_loop/state.rs`；`TURN_STATE_BASELINE = 18`；`35-baseline` 的 `turn-state` 条目 |
| 13 | `scripts/plugin-surface-audit.mjs` | HS-001/002/006：插件只认一个宿主门面 | **保留文件、改名单**：从 `CONTROL_PLANE` 去掉 loop | `CONTROL_PLANE` 含 `symbio-plugin-loop`；`WHITELIST_BASELINE 0`、`SDK_BASELINE 109`、`GRAMMAR_BASELINE 138`、`MODULE_REEXPORT_BASELINE 6`；消费者计数会下降（需同步下调） |
| 14 | `scripts/gate.d/30-docs.mjs` 的 GUARDS / LIBS | 把上述判据接进 docs 阶段 | **撤接线 8 条**：`facts-boundary`、`control-plane`、`baseline-scope`、`engine-ledger`、`turn-root`、`extension-locality`、`capability-migration`、`obligation-ring`；LIBS 去掉 `known-red` | 行 277 / 323 / 332 / 356 / 367 / 381 / 396 / 630 / 789；`loop-purity` 保留（改造后继续接） |
| 15 | `scripts/gate.d/35-baseline.mjs` 的 `turn-state` 棘轮 | CS-010 的基线不许放宽 | **摘条目** | `RATCHETS` 里的 `id: 'turn-state'`，读 `arch-shape-audit.mjs` 的三元尾部 |
| 16 | `scripts/gate.d/55-verify.mjs` 的 `recall_eval` | 检索排序 recall@k | **摘该程序**（或改指新实现） | 写死 `#[path = "crates/symbio-plugin-loop/src/recall_score.rs"]`；文件删后**编译失败**，阶段直接红 |
| 17 | `scripts/gate.d/58-e2e.mjs`、`e2e/cases/t96-effect-idempotency.mjs`、`t97-trivial-value-equivalence.mjs` | e2e 执行面 | **等新会话的分类表落地后按表处理**。`t96` / `t97` 断言的 `turn.stage` / `stage` 取值 / `event_id` 都是引擎细节（用户裁决），删或改；保留集的定义权在新分类表，不在这两例 | `BASELINE.e2eCases = 96`：**不下调、不对齐**；新结构前无意义，改为从实际执行结果读取，做不到就标「待新结构重设」并注释原因 |
| 18 | `scripts/gate.d/10-backend.mjs` + `BASELINE.ciRustTestsTotal` | workspace 测试数只许涨 | **保留文件；数值处置见第 19 项** | `_shared.mjs` 的 `rustTests: 2172`、`ciRustTestsTotal: 2634` |
| 19 | `scripts/gate.d/_shared.mjs` 的 `BASELINE` | 后端 / e2e / dead-code 的账本 | **保留文件、改为实际读数**：`rustTests` / `ciRustTestsTotal` / `e2eCases` **不下调、不对齐**（新结构前没有意义）；脚本结构允许就从实际执行结果读取，做不到就标「待新结构重设」并在脚本里注释原因 | 三个总数 + 整段 `baseline-allow` 历史注释 |
| 20 | `scripts/gate.d/06-facts-write.mjs` + `docs/004-ref/006-当前事实表.md`、`GATE_CODES.md` | 结构事实与判据码登记 | **自动重生成**，无人工 | 删 crate 后 CURRENT 的插件行、GATE_CODES 的 `LP/CP/BS` 码自动消失；`gate-codes-audit` 随之通过 |
| 21 | `docs/005-spec/033.md`、`docs/005-spec/README.md` | CP 终态规范与判据表 | **同批改指**（本次不动） | 033 有 10 处指名 `control-plane-audit` / `obligation-ring-accept` / `port-surface-audit` / `arch-shape-audit`；README 1 处。不改则 `spec-coverage-audit` 红 |
| 22 | `docs/007-task/032-turn废弃审计与替代映射.md` | turn 废弃映射 | **同批改指**（本次不动） | 正文 6 处指名旧脚本路径；不改则 `doc-symbol-audit` / `registry-audit` 有红风险 |
| 23 | `agent-ownership-audit`、`doc-symbol-audit`、`shared-event-vocab-audit`、`dead-code-audit` | 归属表 / 现行文档符号 / 共享事件消费方 / 死码棘轮 | **保留，但会连带红**：删 crate 时需核对 | 归属表与现行文档仍指向 loop 模块；共享事件的消费方少一个；死码 / 测试计数下降 |
| 24 | `scripts/exemptions/plugin-session-sdk.json` | E-009 对 `symbio-plugin-session` → `symbio-sdk` 的限期豁免：对象精确匹配 / `expires` 到点即红 / `symbols` 冻结既有用法 | **session 退役时同批删除**（P4–P6，与 crate 同批） | `crate = symbio-plugin-session`、`expires = 2027-06-30`、`symbols` 列既有符号；新增一条 `symbio_sdk::` 用法或对象扩散到别的 crate 即红 |

## 2. 写死旧路径 / 旧总数的位置（删代码前先知道）

按「一删就红」的顺序：

| 位置 | 写死了什么 |
|---|---|
| `scripts/gate.d/_shared.mjs` | `rustTests: 2172`、`ciRustTestsTotal: 2634`、`e2eCases: 96`（三个总数都含旧 crate 的测试）。**不下调、不对齐**：改为实际读数或标「待新结构重设」并注释 |
| `scripts/loop-purity-baseline.json` | 41 个按文件的词根计数：`control_decl 109`、`entry 183`、`tool_dispatch 236`、`chat_loop.rs 85`、`loop_rows 75`、`stage_events 38`、`ring 3`… |
| `scripts/known-red.json` | 16 条全部指向 `crates/symbio-plugin-loop/src/*`（U3×11 / U4×3 / U2×2） |
| `scripts/baselines/control-plane.json` | `mechanism.file = .../ring.rs`、三段 `parts`、`hash = sha256:907124…`、ADR `D-320` |
| `scripts/turn-root-baseline.json` | `F1 1251/870`、`F2 114/264`、`F4 4/12`、`F5 1` |
| `scripts/engine-ledger-audit.mjs` | `LOOP_ROWS` / `DISPATCH` / `SCAN_FILES` 三条旧路径；`BASELINE = { rows:16, channels:1, fullScans:9 }` |
| `scripts/facts-boundary-audit.mjs` | `CAPABILITY_FILES` 里的 `memory_events.rs`、`derived_facts.rs` |
| `scripts/arch-shape-audit.mjs` | `TURN_STATE_REL = .../chat_loop/state.rs`、默认 `18` 字段 |
| `scripts/gate.d/55-verify.mjs` | `recall_eval` 的 `#[path]` → `.../recall_score.rs`（**删文件即编译失败**） |
| `scripts/gate.d/58-e2e.mjs` + `e2e/cases/t96`、`t97` | 96 例棘轮（**改成实际读数 / 待新结构重设，不下调对齐**）；两例断言的 `turn.stage` / `event_id` 属引擎细节，删或改，保留集等新分类表 |
| `scripts/grep-audit.mjs` | S-011/S-015/S-016/S-017 的锚点文件（`turn_runner/stream.rs` 等） |
| `scripts/capability-migration-audit.mjs` / `extension-locality-audit.mjs` | `CONTROL_TREES` 含 `crates/symbio-plugin-loop` |
| `scripts/plugin-surface-audit.mjs` | `CONTROL_PLANE` 含 `symbio-plugin-loop` |
| `docs/005-spec/033.md` / `README.md` / `docs/007-task/032…` | 10 / 1 / 6 处指名旧脚本或旧符号 |

## 3. 删除后 gate 各阶段还剩什么

| 阶段 | 删干净后 | 说明 |
|---|---|---|
| `05-fmt` | **可跑** | 只对现存 crate 跑 |
| `06-facts-write` | **可跑** | 生成物变小（插件行、旧判据码消失） |
| `10-backend` | **可跑；计数改为实际读数 / 待重设** | 不把 `rustTests` / `ciRustTestsTotal` 下调对齐；脚本能读实际结果就读，不能就标「待新结构重设」并注释 |
| `20-frontend-static` | **可跑** | 与旧 crate 无关 |
| `30-docs` | **部分可跑** | 摘掉 8 条接线后其余继续；但 `spec-coverage` / `agent-ownership` / `doc-symbol` 会因文档仍写旧路径而红，需同批改文档 |
| `35-baseline` | **可跑；先摘条目** | 摘 `turn-state`；`_shared` 的账本按上一条处置（不靠下调数字过关） |
| `50-msrv` | **可跑** | 与旧 crate 无关 |
| `55-verify` | **直接失败** | `recall_eval` 的 `#[path]` 指向已删文件 ⇒ 编译错误 |
| `56-frontend-build` | **可跑** | 与旧 crate 无关 |
| `58-e2e` | **等新分类表** | `t96` / `t97` 属删除或改写对象；保留集按新会话的分类表重建，本清单不预设 |
| `60-facts` | **可跑** | 重算结构事实 |

**一句话**：删干净后，`gate.mjs` 里真实可跑的是 `05 / 06 / 20 / 50 / 56 / 60`；`10 / 35` 可跑，但三个测试总数须改为实际读数或标「待新结构重设」，不靠下调对齐；`30` 是「摘线 + 改文档」后的部分可跑；`55` 直接报错；`58` 等新分类表，不再以 `t96` / `t97` 为屏障。届时「绿」的含义从「旧控制面终态成立」收窄为「留下的架构自洽 + 新分类表定义的 e2e 保留集 + 零世界知识检查」。

## 处理过程

本清单是清点与裁决记录，未改动任何门禁脚本或基线；后续按下表逐项执行时再逐条勾销。
