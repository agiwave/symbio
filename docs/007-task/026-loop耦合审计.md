---
kind: task
id: T-026-loop耦合审计
title: loop 耦合审计：端口名统一、世界语义下沉与 turn 骨架归属
status: doing
track: arch
prio: now
created: 2026-11-02
parent: T-001-能力全面拆分
deps: []
where: "crates/symbio-plugin-loop/src, crates/symbio-plugin-api/src/port.rs, crates/symbio-kernel/src/capability"
exit: 五项输出（四类耦合清单 / 目标形状 / D-298 修订案 / 分批计划 / T-025 处置）全部落进本文档并经用户裁决；「单族按名唯一」的修订在 DE-045 落 `### D-NNN` 之后 T-027 才动代码；T-025 按裁决改档或重开；本文档自身不含代码改动
---

# loop 耦合审计：端口名统一、世界语义下沉与 turn 骨架归属

## 内容

用户裁决：loop 层不区分 Model / Tool，只认端口名与契约。据此对 `symbio-plugin-loop`
做耦合审计（先不改代码），交付五项：四类耦合清单（A 命名 / B 世界语义 / C turn-session
骨架 / D 真环机制）、目标形状（含 turn 生命周期归属判断）、`D-298` 修订案（单族按名唯一
+ 最小通用增强）、分批计划（T-023 / T-024 / T-025 的保留与重做，对齐 T-014 账本）、
T-025 暂停处置。审计正文即本文档其余各节。

## 结论（先给）

1. **T-025 暂停时零未提交改动**：提交链 `62d046ed`（T-023）→ `12232ea6` → `ea297306`
   （T-024）→ `fe760937`（修红收口），工作区干净；四笔全部保留、不回退。
2. loop 的 1.4 万行里，**真环机制（D 类）约占四成**且形状基本正确（单一行表、协议五事实、
   幂等键、租约）；其余是三类债——命名耦合（A，改字符串即可）、世界语义（B，该下沉）、
   turn/session 骨架（C，该搬走）。
3. `port.model` / `port.tool` 两族**确认应改为单族按名唯一**：`Registry` 的 `single` 是
   族级单槽（`put` 降同族全部旧行），表达不了按名唯一，需要一处**最小通用增强**
   （槽位策略加「按名」一档，不新开静态表）。
4. **turn 生命周期归 session**，不归端口：turn 的身份键、收束格、18 格状态全是会话轴；
   端口的生命周期单位是 `Call`（attempt + 租约），归环记账。
5. 分批：T-023 / T-024 **保留**（T-023 的读数面随修订跟进）；T-025 **重做**为 T-029
   （exit 按本审计改写）；新增 T-027（D-298 修订）/ T-028（A 类清耦）/ T-030（C 类搬出）。

## 一、耦合清单（文件:行，按四类归档）

### A 通用环机制，仅命名耦合

改字符串 / 派生常量即可，无行为差异；但**行名变更会动事实面与 e2e 基线**，须走裁决。

| 位置 | 是什么 | 耦合形态 |
|---|---|---|
| `port.rs:870-878`（`Round::id`） | 幂等键 id 段 `model-{turn}-d{drive}-{step}` | 前缀硬写 `model-`；应为 `{port}-…` 由调用侧传端口名 |
| `tool_dispatch.rs:390-414`（`call_signature` 一带） | 工具调用 id 按 `model_id` 识别「产生这批调用的模型往返」并生成同前缀 | 同上，前缀与识别都写死 model |
| `loop_rows.rs:744-777`（`PORTS`） | `PortDecl` 表 `port: "model"` / `"tool"` 字面量 | 应从 `ports::MODEL/TOOL` 常量派生（闭集单一真源） |
| 行名 `dispatch-model` / `await-model` / `dispatch-tools` / `await-tools` | 行表的派发 / 等待行 | 单数 vs 复数不一致；统一 `dispatch-{port}` / `await-{port}` 需改 `dispatch-tools`→`dispatch-tool` 等，**事实 payload 与 PTS / e2e 基线随之重生成**——裁决点，见 §四 |
| `port.rs:933` / `port.rs:1101` | `Call::with_via("model", …)` | 字面量，应为 `ports::MODEL` |
| `control_decl.rs:736` / `:775` | 求解器侧按 `str_field(e,"port") == Some("model")` 配对模型响应 | 配对判据写死端口名；应参数化为「请求的发起端口」 |
| `chat_loop.rs:249`、`tool_dispatch.rs:748` 等注释与 `plugin_warn` 文案 | 行名 / 端口名散落注释 | 随行名统一一并收口 |

### B 世界语义，必须下沉到 provider / 能力包

| 位置 | 是什么 | 下沉去向 |
|---|---|---|
| `tools/tool_executor.rs:260-297` | **线上名解析**：`mcp__fs__read` → 能力名（`tool_metas` + `capability_resolve`，失败不猜按原样路由交 NotFound） | tool 端口的 PortProvider 合同 |
| `tools/tool_executor.rs:379-446` | **空闲超时**：`TOOL_EXEC_IDLE_TIMEOUT_SECS=600`，进展判据 `sink.progress().emitted()`，`biased` select 三出口 | 同上 |
| `tools/tool_executor.rs:61-131` + `:154-190` + `:448-496` | **结果契约**：`extract_result` / `PendingPrompt` / `RESULT_CONTENT`·`RESULT_OUTPUT` 解码 | 同上 |
| `tools/tool_executor/decls.rs:33-68` | `ToolDecls` / `behaviors_of`：模型可见工具清单构造，名字匹配先等值再取末段 | tool 端口合同（清单即端口的请求形状） |
| `tools/tool_executor/effects.rs:31-60` | `record_effect`：工具效果落格 | tool 端口合同的效果面 |
| `context.rs:294-376`（`should_start_compression` / `prepare_compression`）+ `chat_loop/inputs.rs:548`（`run_compress`）+ `context/pipeline.rs:996`（`run_compression_llm`）+ `turn_exec.rs:1047` | **压缩链**：阈值判定、迟滞、切分、摘要请求 | 压缩是模型使用策略（世界语义）——判定与摘要构造下沉；内部往返的 `via` 记账（`Call::with_via("compress/summary")`）留环（D） |
| `tokenizer.rs`（318 行）+ `context.rs` 水位计算 | token 计数与请求水位 | 模型世界语义，随压缩链一并评估 |
| `context/prompt.rs` / `conversation_view.rs` / `providers.rs` / `view.rs` | 对话视图与 prompt 段构造 | 模型对话面语义；`build_user_prompt_message`（工具结果节点，`tool_executor.rs` 内）同属此层 |
| `memory_events.rs` | 记忆召回读数落格 | 读数已住能力包（D-264），loop 只带读数——**已基本就位**，仅登记 |

### C turn / session 骨架，应从 loop 搬出到 session

| 位置 | 是什么 | 备注 |
|---|---|---|
| `chat_loop/state.rs:129-150`（`SessionContext`） | 消息列表 + `LoopSession` + 压缩寄存 | 会话历史管理是 session 语义 |
| `chat_loop/state.rs:188+`（`TurnState`，18 格） | 轮次状态 | T-014 步 1 已逐格核账无删格；搬走不删格，`CS-010` 读数随归属变 |
| `turn_session.rs`（`LoopSession`） | 会话身份与事实包 | — |
| `provider.rs:27-107`（`run_turn` 入口） | `<sid>` 上 `LOOP_RUN_TURN` 动作、装配 `ChatOrchestrator` | 入口应归 session，loop 只被调用 |
| `turn_runner.rs`（1028 行；`begin` / `settle` / `close` / `await_model:675`） | 轮次生命周期：开轮、收束、落 `chat.assistant.final` / `fallback` | 收束格是会话事实；`await_model` 的 poll 骨架是 D（见下） |
| `chat_loop/turn.rs` / `wake.rs` / `decide.rs` / `delegate.rs` / `compose.rs` | 轮首判决、唤醒 resume 幂、委派、措辞 | 会话语义 |
| `stage_events.rs` | `turn.stage` 事实 | 会话语义 |
| `control_decl.rs` 中会话词汇（`turn.dispatch` 等） | 轮次派发事实 | 与 D 类求解器声明同文件，搬迁时拆开 |

### D 真环机制（选债、派发、等待、配对、记账、预算 / 租约、重放）——留 loop

| 位置 | 是什么 |
|---|---|
| `port.rs`（协议五事实：`announce` / `dispatch:373` / `reap`；`Dispatch:126`；`Round:861`） | `request`→`sent`→`response`/`failed`+`retry` 的唯一写方；派发前问表（`row_owed`）；幂等键 `(port, id, attempt)`；租约看护 |
| `loop_rows.rs`（`ROWS` 行表 + `PORTS`） | 选债：`owes` / `until` / `unless`、授权级联两行（`D-265` 终态、`D-290` 不补合取）、`PortDecl` 的 lease / attempts / concurrency |
| `tool_dispatch.rs`（1620 行主体） | 批编排四不变量、`process_tool_calls_async`、同批 N 条 `request` |
| `tools/dispatch/batch.rs` / `parallel.rs` / `gates.rs`（`GateReadings`、`read_batch_gates:102-146`，`port_of(…)` 按端口名读环声明——形状已对）/ `unfinished.rs`（`close_unfinished`） | 批调度、风险档并发、门档、收尾 |
| `tools/tool_executor/breaker.rs` | 熔断 |
| `turn_exec/drive.rs`（环体、幕迭代） | 环的主体；`consolidate_into_wal` 的落格令牌分割 |
| `control_decl.rs` 求解器声明主体 | 判据面（`PTS-003` 冻结） |

## 二、目标形状

**环只认「端口名 + 请求 / 应答 / 效果契约」**：

- **单一派发路径**：`dispatch(port, call)` → 行表按 `dispatch-{port}` 开账 → `PortProvider`
  合同执行 → 协议五事实。今天存在的两条分叉（`turn_runner` 的模型 inflight 路径与
  `tool_dispatch` 的批路径）统一为「`await-{port}` 行动作 poll 在飞 future」——批只是
  多个 future 的 join，由并发档管理，不是第二条等待机制。
- **无 model / tool 分支**：`control_decl` 的响应配对按请求发起端口参数化（A 类）；
  授权级联（`authorize-tool` 两行）是否存在是**端口属性**，进 `PortDecl`（或行声明
  参数化），不再由字面量决定。
- **provider 按端口名注册**：单族按名唯一（§三）；`PortProvider` 的合同方法按端口分
  （`llm_round_trip` / tool 三样 glue）是**合同形状差异**，不是环的分支——环经
  `Arc<dyn PortProvider>` 统一取用，方法默认体 `None` 表「本端口无此合同」
  （T-024 已立的形状）。
- **turn 生命周期归 session**，理由：
  1. turn 的身份键是 `session_id`（`TurnDispatch.spec`；`run_turn` 校验 sid 与交付包一致）；
  2. turn 的收束格是会话事实（`chat.assistant.final` / `fallback`、开场白、`user.message` 锚）；
  3. `TurnState` 18 格的输入格（`turn_no` / `user_seq` / `resume_root` / `retry_of`）全是会话轴；
  4. 端口的生命周期单位是 `Call`（attempt 级幂等键 + 租约），归环（D 类）——
     **端口不拥有 turn，只拥有 attempt**。搬出后 loop 只剩 D 类 + 端口派发面。

## 三、`D-298` 修订：单族按名唯一

**确认改**。现状 `port_family(port)` 每端口一个 `single` 族（`port.model` / `port.tool`），
把世界类型留在注册层：加端口 = 加族，且 `single` 的槽位语义（`put` 降**同族全部**旧行，
`capability/mod.rs:377-382`）意味着单族装不下两个端口。

修订案（最小通用增强，不新开静态表）：

1. **单一 `port` 族**；`CapabilityFamily` 的槽位策略从 `single: bool` 增一档
   「按名单槽」——`put` 收到生效行时降位范围从「同族全部」收窄为「同族**同名**」。
   `multi` / `single` 语义不动（`row.rs:62-94`）。这是槽位策略的通用化，不是端口特例。
2. **行名固定 `<port:{port}>`**：`provider_id` 退出表键（判重文案仍从
   `PortProvider::provider_id()` 取，指认谁被顶掉）。尖括号防撞工具名，手法不变。
3. **`capability_port_of` 改按名分组**：`list(port 族)` 后按行名归组——每组 1 行
   `Ready`、>1 行 `Duplicate`（demote 保先注册）；`Absent` = 无该名行。闭集判定
   （`ports::KNOWN`）仍在消费侧。
4. **门禁影响**：kernel `CapabilityFamily` 加一个 `pub const fn` ⇒ `CS-001` 的
   `symbio_kernel` 形状变宽，需挂 `core-shape-allow <格>: D-NNN`（决策号 = 本修订条目，
   落 DE-045「关键决策」节，先裁后动）。结构性用例（port 行在表 ⇒ 六消费族看不见）
   随行名改跟进。

## 四、分批计划

**保留 / 重做裁决**：

- **T-023 保留**（`62d046ed`）：词汇与判重 / 缺席读数面正确；单族化后 `port_family` /
  `port_row_name` / `capability_port_of` 随 T-027 改写，不回退。
- **T-024 保留**（`ea297306`）：`ModelPort` 即目标形状的 model 实例（合同带默认体、
  环只按端口名取）；`ports::MODEL` 常量替字面量随 T-028 收口。
- **T-025 重做为 T-029**：原 exit「三样 glue 搬进端口适配器」方向不变，但落点与
  合同面按本审计改写（见下）。

**迁移顺序与每批验证读数**（一批一提交、`Migration:` trailer、`EX-007` 零新增行）：

| 批 | 内容 | 验证读数 |
|---|---|---|
| T-026（本文档） | 审计落档，零代码 | registry-audit 绿 |
| T-027 | DE-045 落 `### D-NNN`（单族按名唯一）+ kernel 槽位策略增强 + plugin-api 单族改写 + model 插件行名跟进 | 门禁 11 阶段；`CS-001` 变宽挂 `core-shape-allow`；`CC-001` 挂 `Core-Change:`；结构性用例绿 |
| T-028 | A 类清耦：常量替字面量、id 前缀参数化、`control_decl` 配对参数化；**行名统一（`dispatch-{port}` / `await-{port}`）在本批裁**——若裁「改」，事实 payload、`PTS` / e2e 基线同提交重生成（`EL-001` 仍 16 行，只换名） | 门禁全绿；`EL-001…003` 只降；`EX-003` 净删 loop 非测试行 |
| T-029（原 T-025 重做） | tool 三样 glue + `ToolDecls` + `record_effect` 下沉为 tool 端口 `PortProvider` 合同；**落点 = session 插件**（顶层收集宿主，`capabilities.rs` 的 `collect_capabilities` 在彼处；不新建 `symbio-plugin-tool`，agent 只是 traverse 参与者）；压缩链（判定 + 摘要构造）是否同批，裁于 T-028 之后 | 装 / 不装 / 缺失三路径 e2e 等价；缺失响亮失败并落事实；`EL-003` 11 读点只降 |
| T-030 | C 类搬出：`SessionContext` / `TurnState` / `LoopSession` / 轮次收束从 loop 搬 session；loop 只剩 D + 端口派发面 | `CS-010` 读数随归属变（18 格不删，搬家）；`EX-003` 净删 loop 行；e2e 全绿 |

**与 T-014 对齐**：本审计四类映射五类残留——D↔环、B↔过程体、C↔会话基底、A↔命名债；
`EL-001`（16 行）/ `EL-002`（1 通道）/ `EL-003`（11 读点）全程只降；步 3 合流
（「五类之外没有名字」复核 + 与 `EX-006` 并账）改在 **T-030 收口后**执行，比原计划
（等 T-007 / T-008）多等两批。`D-265` 两行终态与 `D-290` 不补合取的裁决不受本审计影响。

## 边界

- 本审计不改任何代码；`### D-NNN` 修订条目未落 DE-045 之前，T-027 不开工
  （架构改动只认方案为源）。
- B 类中 prompt / 对话视图的世界语义归属（留在 loop 的 chat 装配还是随 T-029 / T-030
  分批下沉）在 T-029 落地前复核一次，不在本审计强行拆完。
- `PortDecl` 的 `concurrency` / `max_attempts` / `lease_ms` 口径不变；授权级联是否进
  `PortDecl` 是 T-028 的裁决项，不是本审计的既定结论。

## 处理过程

1. 确认 T-025 暂停状态：`git status` 干净，四笔提交保留（见「结论」第 1 条）。
2. 两路勘察回收 Model / Turn 与 Tool / Registry 的耦合点；关键文件
   （`tool_executor.rs`、`port.rs`、`capabilities.rs`、`provider.rs`、`loop_rows.rs`、
   `chat_loop/state.rs`、`turn_runner.rs`、kernel `capability/{mod,row}.rs`）精读定位，
   逐条落进 §一 的四类清单（file:line 均经本会话核对）。
3. 目标形状（§二）与 turn 归属判断基于 `TurnDispatch` / `TurnState` 18 格 / 协议五事实
   的既有裁决（`D-086` / `D-222` / `D-138`）推导，不新立判据。
4. `D-298` 修订案（§三）先核对 `Registry::put` 的族级单槽语义（`capability/mod.rs:377-382`）
   与 `CapabilityFamily` 形状（`row.rs:62-94`），确认 `single` 表达不了按名唯一后才给
   增强案；`CS-001` 变宽影响同条记明。
5. 分批计划（§四）与 T-014 的 `EL-001…003` 棘轮、步 3 合流条件、五类残留定义逐项对齐；
   T-025 的 exit 改写点（落点 session、合同面）记入 T-029 行。
6. 待办：用户裁决本文档五项输出后，T-027 起按 §四 顺序开工；T-025 任务档在裁决时
   一并改档或关闭重开。
