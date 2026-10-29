---
kind: design
id: crates/symbio-plugin-session/README
title: Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书)
status: active
created: 2026-07-20
impl: done
---

# Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书)

Session 插件是 Symbio 架构中的**会话持久化与生命周期中心**：持有事实源、转写与运行态，负责加载历史、**解析**系统提示词（收集各插件经提示词段行注册的段，本层不自己拼）、经 `Registry`（收集口，put/get/list 三方法）汇集工具与选项，并在**一轮的边界**上派发 / 等待 / 收尾（[D-086](../../docs/003-design/014-单环一表.md "DE-014-单环一表")）。一轮的执行本体（主循环、上下文压缩、工具链）住在独立插件 [`crates/symbio-plugin-loop`](../symbio-plugin-loop/README.md "crates/symbio-plugin-loop/README")：session 经 `loop/<sid>` 的 `run_turn` 动作把交付包交给它，loop 只经地址面 `session/<sid>/_run/<op>` 回读 / 写回会话状态。Model 插件是**无状态 LLM 网关**（按上下文注册 Provider + 协议适配），对 session 零依赖。

本文档将系统性地阐述 Symbio 的会话保存、内容压缩、工具迭代限制以及发送过滤策略，说明其具体规则、参数配置及 Rust 底层实现策略。

> **目标形状**见 [DE-001 §4.3 / §4.4](../../docs/003-design/001-核心架构.md "DE-001-核心架构")
> 与 [DE-015](../../docs/003-design/015-义务环.md "DE-015-义务环")。
> 本文只写**当前实现**，不复述目标裁决。

> **深度设计与专题文档**（核心循环收口、模块分工、历史审计、前端性能、VDFS 会话消息映射、级联选项……）
> 见 [`src/docs/`](src/docs/README.md "crates/symbio-plugin-session/src/docs/README")——本文只写**现行机制与配置面**，`src/docs/` 写**为什么这样设计**。

---

## 一、 系统架构数据流图 (Architecture & Data Flow)

下列架构图展示了一个完整的“用户请求 -> Session（存储与生命周期）-> Loop 驱动器（视图组装/推理/工具）-> 磁盘持久化（会话目录）”的生命周期：

```mermaid
flowchart TD
 User([1. 用户消息：写 <sid>/inbox]) --> SaveUser[2. 入队收件箱 → 会话自己消费<br>空闲时取队首 → 保存/追加用户消息]
 SaveUser --> LoadHistory[3. 加载历史上下文<br>get_context_messages 轮次窗口对齐]
 LoadHistory --> InvokeLoop[4. 派发 loop/&lt;sid&gt; run_turn<br>TurnDispatch：TurnSpec + facts + sink]

 subgraph Loop 驱动器插件（crates/symbio-plugin-loop）
 LoadHistory --> BuildPrompt[5. 解析系统提示词<br>收集各插件注册的段<br>人格/记忆/指令]
 BuildPrompt --> AggTools[6. 工具汇集<br>Registry 收集口]
 AggTools --> Resolve[7. 获取生效 ModelProvider<br>capability_model_active 单次解析]
 Resolve --> BuildView[8. 构建请求视图<br>build_request_sections<br>记忆/调度/委派三段注入]
 BuildView --> LLMCall[9. ModelProvider 推理调用<br>round_trip 直连无状态 LLM 网关]
 LLMCall --> ToolAction[10. 执行工具链]
 ToolAction --> LoopCheck{11. 迭代完成?}
 LoopCheck -- No --> BuildView
 end

 LoopCheck -- Yes --> EndTurn[12. 助理回答定格]
 EndTurn --> SaveFinal[13. 转写写柄落格 + TurnReport 交回 session]
 SaveFinal --> Disk[(会话目录<br>&lt;homedir&gt;/session/&lt;id&gt;/<br>session.json + messages.json)]
```

---

## 二、 核心参数与配置 Schema (Configuration Details)

会话与压缩系统的全部表现都由会话配置参数控制。配置落在**本插件自己的目录**：

```text
{homedir}/session/PLUGIN.yml 本插件的配置（可在 <根>/session/PLUGIN.yml 上编辑）
```

字段真源是本插件的 `config.rs::SessionConfig`（**定义由配置的拥有者产出**：设置页的表单定义从 `SessionConfig::default` 读出，不写第二份字面量，避免「面板显示值与实际行为漂移」）。

以下是**与本节机制直接相关的键**的示例（不是全表）：全字段真源见
`config.rs::SessionConfig`，配置面总表见 [`docs/004-ref/001-CONFIGURATION.md`](../../docs/004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")。
（`classify_enabled` / `compose_enabled` / `progress_*` / `supplements_*` /
`conation_enabled` / `skill_compile_enabled` 等键不在下方示例中，
它们归「对话面机制」与「自主与学习」两节。）

```yaml
session:
 # 存储目录：本插件自己的目录（父插件经 PLUGIN_DIR 告知，装配态即 <homedir>/session/），
 # 跟随系统目录 (homedir) 切换，不作为配置项暴露。
 # 注：会话只有一种落盘布局，没有可切换的"存储后端"。临时会话（`_t_` 前缀 / 空 id）
 # 不落盘，由构造点选择，与用户配置无关。
 # 注：会话默认不绑定智能体（纯工具模式）。智能体选择属于前端会话级偏好，
 # 由 session.metadata.agent_id 按会话记录，后端不提供 default_agent。
 
 # 1. 存储级策略
 max_messages: 100 # 单会话本地保存的最大对话轮数限制 (以 User 消息计数；0 = 不限制，默认 100)
 
 # 2. 单轮迭代边界：v2 **没有**这一项配置（工具轮数不设上限，理由见三.2）
 
 # 3. 宏观语义压缩策略
 auto_compress: true # 是否启用基于 Token 溢出 (70% 触发) 的大模型自我语义合并
 context_messages: 6 # 发送给 AI 的对话上下文滑动窗口轮数 (以 User 消息轮次对齐截取；也是存储期 prune 的保留分水岭)
 
 # 4. 存储期工具链裁剪
 prune_tool_history: true # 是否在落库时物理裁剪 context_messages 分水岭之前的历史工具链（Tool/ToolCall/Reasoning 及其子节点；只删节点不删归档；内存临时会话自动跳过）
 
 # 5. 会话记忆（<会话目录>/MEMORY.md，可编辑地址 <根>/session/<id>/MEMORY.md）
 memory_max_bytes: 16384 # 单次**写入**的字节上限，超出直接拒绝（不截断、不部分写入）
 memory_inject_max_bytes: 4096 # 每轮**注入**系统提示词的字节上限，超出部分截断并指路 vdfs_read 取全文
```

---

## 三、 五大核心策略的具体规则与 Rust 实现机制 (Implementation Strategies)

### 1. 单会话对话轮数上限存储策略 (`max_messages`)

> **目标**：防止单个会话历史由于对话轮数过多而无限膨胀，导致磁盘溢出或加载序列化变慢。

* **具体规则**：
 * 对每个 Active Session 物理存储的历史记录实施**最大保存对话轮数限制**。
 * `max_messages` 表示最大保存的对话轮数，每一轮以一个 `User` 消息起始；默认 `100`，`0` 表示不限制（历史上此处曾有 `.max(500)` 硬下限，导致设置面板中小于 500 的值静默失效，现已移除）。
 * 当存储的对话轮数超出阈值时，自动从会话开头执行 FIFO 裁剪，移除最老的多余对话轮次。**只删消息节点、不动 `tool_archives/` 归档文件**：归档的磁盘生命周期由 `tool_result_guard` 的滚动淘汰（每会话保留最新 N 个文件）专职负责，避免 `max_messages` 成为静默删文件的破坏性操作。
 * **智能对齐**：此策略确保物理保存下来的会话，其起始消息也总是以一个完整的 `User` 消息起始，从而绝对避免了历史反序列化对齐失败的问题。
* **Rust 实现策略**：
 * 在 `chat_session/write.rs` 的 `append_messages`（**存储写入边界**）中，扫描 `User` 消息索引并执行轮数对齐物理截断，确保物理存储也总是以 `User` 消息为开端：

 ```rust
 let max_turns = cfg.max_messages; // 0 = 不限制
 let mut user_indices = Vec::new;
 for (idx, msg) in session.messages.iter.enumerate {
 if msg.role == Some(NodeRole::User) {
 user_indices.push(idx);
 }
 }
 if max_turns > 0 && user_indices.len > max_turns {
 let start_idx = user_indices[user_indices.len - max_turns];
 session.messages.drain(0..start_idx); // 归档文件不在此处删除
 }
 ```

---

### 2. 单轮迭代边界（v2：不设工具轮数上限）

> **目标**：让自主 Agent 的多步决策由模型自己收束，主循环不设「跑了几轮就停」的旋钮。

* **现状规则**：
 * 一次用户 Prompt 触发一个请求；请求内部 `推理 -> 工具调用 -> 工具结果返回 -> 再推理` 的迭代在
 v2 运行器的环体里跑完（[`../loop/turn_runner.rs`](../symbio-plugin-loop/src/turn_runner.rs) 的 2e/2f），**主循环侧看不见第几轮工具**。
 * 界只有三条：单条响应内的工具调用数上限 `MAX_TOOL_CALLS_PER_RESPONSE = 32`
 （`crates/symbio-plugin-model/src/stream.rs`，超出的调用合成一条「本次调用未执行」的结果喂回模型，指引它
 把剩余调用放到下一轮）、用户中止、provider 断流。
 * 长度截断（`finish = Length`）后的自动续写有 `loop_rows::MAX_STAGE_ROUNDS = 3`
 （[`../loop/chat_loop/turn.rs`](../symbio-plugin-loop/src/chat_loop/turn.rs)），它数的是**续写次数**，不是工具轮数。
* **为什么这一层不设软上限**：主循环读不到工具轮数——工具在 v2 运行器的环体里一批批跑完
 （[`../loop/turn_runner.rs`](../symbio-plugin-loop/src/turn_runner.rs)，步骤序列与批次边界见
 [DE-102-core-loop](src/docs/102-core-loop.md "DE-102-core-loop") §4.1），本轮请求侧只有一个
 「发过几次请求」的计数（续写才让它 >1）。一个判不到真实轮数的
 上限等于**没有上限**，却会让人以为设过：它与「默认不限制」逐字等价，配错与没配长得一样，
 没有任何信号能区分。真要这道上限，位置在 2e（结果接进 `exchange`）与 2f（轮内注入）之间，
 那里有真实的 `tool_round`；裁决与逐符号名册见
 [D-074](../../docs/003-design/014-单环一表.md "DE-014-单环一表")。

---

### 3. 大模型自动语义压缩策略与对话轮数对齐 (`auto_compress` / `context_messages`)

> **目标**：解决 LLM Token 绝对窗口溢出问题，通过“宏观压缩”在维持长距离记忆连贯性的同时极大降低 Token 话费。

* **具体规则**：
 * **对话轮数感知滑动窗口限制 (Dialogue Turn-Aware Sliding Window)**：
 * 每次组装上下文时，如果未指定具体的消息条数限制，系统会自动依据 `context_messages` (默认 **6** 轮对话) 作为滑动窗口。
 * **智能对齐**：为了避免工具调用等中间过程物理截断导致 context_messages 切碎了上一轮的 User 消息（导致 Agent 在对齐时把整段历史吞掉），加载过程会**从后往前定位**第 `context_messages` 个 `User` 消息。
 * 系统将从该 `User` 消息起点开始，返回**完整且未遭打碎的最近 $N$ 轮历史对话**。
 * **动态语义压缩**：当滑动窗口内的对话历史 Token 预估值超过大模型上下文限制的 **70%** 时，且 `auto_compress` 开启，系统将自动对老旧历史进行 LLM 语义合并。
* **Rust 实现策略**：
 * **Turn 对齐获取 (`crates/symbio-plugin-session/src/chat_session/read.rs`)**：
 * `ChatSession::get_context_messages` 中先做三层清理（过滤 `Failed` 消息、剔除孤儿节点、content 归一兜底），再以 `context_messages` (可通过参数显式覆盖) 调用 `sliding_window` 实现 User 消息轮次级对齐：

 ```rust
 // 从后往前定位第 context_limit 个 User 消息，以其作为起点裁剪历史
 let mut user_indices = Vec::new;
 for (idx, msg) in messages.iter.enumerate {
 if msg.role == Some(NodeRole::User) {
 user_indices.push(idx);
 }
 }
 if user_indices.len <= max_turns {
 return messages.to_vec;
 }
 let start_idx = user_indices[user_indices.len - max_turns];
 messages[start_idx..].to_vec
 ```

 * **语义合并 (`crates/symbio-plugin-loop/src/context.rs`)**：
 * 每次大模型执行 Turn 之前，`prepare_compression`（配合 `should_start_compression` 的 Token 估计检测）会检测 Token 估计值。如果超标，它会将历史会话（保留最近 30% 活跃明细）打包发送给一个背景 LLM，并使用特殊的 System Prompt 提炼出高密度的 **`<state_snapshot>` XML 状态快照**：

 ```xml
 <state_snapshot>
 <completed_items>已成功重构 Session 核心，并在 symbio 中消灭了硬编码。</completed_items>
 <knowledge_discovered>Model 插件和 Agent 插件解耦，跨插件只能经 VDFS 地址交互。</knowledge_discovered>
 <open_issues>前端 UI 还需适配会话配置的新字段。</open_issues>
 </state_snapshot>
 ```

 * 该快照以一条高密度的 User 消息替代被压缩的老旧历史，完美承接之前的所有意图与已知知识，将上下文开销瞬间降到最低。

---

### 4. 存储期历史工具链物理裁剪策略 (`prune_historical_tool_calls` × `context_messages`)

> **目标**：本地 Session 存储只保留“对话骨架”（User/Assistant 文本），把已沉淀进历史深处的庞大工具执行足迹从**物理存储**中移除，防止会话文件无限膨胀；最近几轮的工具明细完整保留，供加载层与请求视图层继续使用。

* **具体规则**：
 * 当 Session 持久化保存消息时，自动以**最近 `context_messages`（默认 6）个 User 消息**作为分水岭（`keep_turns` 与加载层轮次窗口同源）。
 * 分水岭之前的历史中，`Tool`（工具执行结果）、`ToolCall`（工具意图）与 `Reasoning`（推理过程）消息**直接物理删除**，被删 ToolCall 的请求/响应 Text 子节点一并清除。
 * 分水岭之前不再拥有任何保留子节点、且文本内容为空的 Assistant 容器消息一并拔除。
 * 被删除消息若关联 `.txt` 存档文件，存档文件同步物理删除，杜绝磁盘文件泄露。
 * **只保留完整的 User / Assistant 文本对话**，本地存储长期维持在极简规模。
* **Rust 实现策略**：
 * **物理清理 (`crates/symbio-plugin-session/src/chat_session/write.rs` 内的 `prune_historical_tool_calls`，保存消息时调用；该函数只被 session 插件消费，定义在 session 插件内)**：

 ```rust
 // keep_turns = 配置的 context_messages（默认 6）：
 // 定位倒数第 keep_turns 个 User 消息 limit_idx 作为分水岭
 let limit_idx = user_indices[user_indices.len - keep_turns];
 // 1. 扫描 messages[..limit_idx]，收集 Tool / ToolCall(msg_type) / Reasoning 消息
 // 加入 to_remove，并异步删除其关联的 .txt 存档文件
 // 2. 被剪除 ToolCall 的直接子节点（请求/响应 Text）一并加入 to_remove
 // 3. 无保留子节点且文本为空的 Assistant 容器一并加入 to_remove
 // 4. 物理截断：
 messages.retain(|msg| !to_remove.contains(&msg.id));
 ```

---

### 5. 请求级注入段（`build_request_sections`：长期记忆 / 任务调度 / 委派者真源）

> **目标**：把**本轮才成立**的读视图送进请求——长期记忆召回、就绪任务集、委派判定。它们不是会话历史的一部分，因此不落库、每请求现算。
>
> **边界**：注入段只**追加**，不改写历史。发给模型的对话体是事实网格的纯投影（[D-048a](README.md "crates/symbio-plugin-session/README")）；请求视图级的内容淡化 / 轮次淡化 / 分层滑窗骨架化三层已按 [D-078](README.md "crates/symbio-plugin-session/README") 退役——那三层改写的是历史正文，与「请求体 = 投影」互斥。

* **段的来源是端口，不是本模块的固定列表**（DE-025 D-215 / D-220）：`symbio_plugin_api::context::ContextProvider` 由能力包注册、`inventory` 编译期收集，`loop` 按节奏收集、按 owner 序拼接。本仓当前提交三条（同以 `owner() == "loop"` 提交，**提交序 = 同节奏内的段序**）：`RecallProvider` 产 `slots::RECALL`、`ReadySetProvider` 产 `slots::READYSET`（两者轮首节奏）、`DelegateProvider` 产 `slots::DELEGATE`（请求级节奏）。加一类每轮上下文 = 加一条 provider，不改 `loop`。
* **具体规则**（唯一入口 `build_request_sections`，段各自独立、非空才插）：
 * **长期记忆召回段 (Recall Context)**：`run_recall` 在**本轮第一次请求**读一次跨会话召回视图（`memory_events::recall_view`，`off` 档与无目录的临时会话不读），转成端口形状（`RecallFacts`）交给 provider；`RecallProvider` 渲染出段落（`memory_events::prompt_section`，抬头 `【长期记忆】`），非空时产出 `meta.kind = "recall_context"` 的节点。**置顶不置尾**：置尾会把「最后一条 user 消息」从用户的问题换成记忆——mock 场景匹配与轮次窗口都读错；**不进系统提示词**：那里有「唯一真源 = 注册段」的纪律（本 README 第 1 节）。e2e `t29` 同时钉住「记忆段在包里」与「`messages.json` 里一个字都不多」。
 * **任务调度段 (Ready Set)**：`ReadySetProvider` 读本轮事实切片 + 控制面给的容量（`task_cap`）调 `task_events::prompt_section`（**只读**开档、不创建事实源，`off` 档不读），抬头 `【任务调度】`，只列**依赖已全部完成**的候选；非空时产出 `meta.kind = "readyset"`。**候选集空 ⇒ 整段不出现**（给模型一段空调度只会诱导它瞎猜）。e2e `t31` 同时钉住「段落在包里」与「`messages.json` 里一个字都不多」。
 * **委派者真源段 (Delegate Context)**：`DelegateProvider`（请求级节奏，`meta.kind = "delegate_context"`）把本轮**委派判定**（命中了哪条判据 / 为什么没启动子会话）+ 能力目录 + worker 进展渲染成一段。输入是 `LiveInputs { tools, delegate, workers }`：工具清单是**收集成形后**的那份（轮首取不齐），委派结论是控制面 Q1 判完的三态（`NotJudged` / `Start { reason }` / `Idle`）。provider 不读配置、不重判；渲染函数仍是 [`../loop/chat_loop/delegate.rs`](../symbio-plugin-loop/src/chat_loop/delegate.rs) 的 `delegate_section`。
 * **段的消费形态**：`chat_loop::request_view_prefix` 顺序取出视图里**全部**带 `meta.kind` 的段（不按下标——缺一段时下标全部左移；也不按名字查表——那会与 `slots` 形成第二处拼写），拼成一条 `system` 前缀消息交给运行器。**视图次序 = 装配次序 = 模型读到的次序**，而装配次序只与 owner 字典序有关（见 [D-047](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")）。
 * **水位治理不在本层**：压缩只有一个入口 `auto_compress_process`（70% 阈值自动触发，加上用户点「失败压缩节点重试」的 `force` 路径），它改写的是存储态上下文（写成 `session.consolidated` 一格事实），不是请求视图。
* **Rust 实现策略**：
 * 唯一入口为 [`../loop/context/view.rs`](../symbio-plugin-loop/src/context/view.rs) 的 `build_request_sections`——收的是「段身份 + 正文」的数组，**不决定有哪几段**；由 [`../loop/chat_loop/inputs.rs`](../symbio-plugin-loop/src/chat_loop/inputs.rs) 的 `prepare_request_sections` 调用：

 ```rust
 pub fn build_request_sections(sections: &[(&'static str, String)]) -> Vec<ChatNode> {
 sections
 .iter()
 .map(|(slot, text)| ChatNode {
 meta: Some(section_meta(slot)), // {"kind": slot} —— 拼写只有这一处
 content: Some(MessageContent::Text(text.clone())),
 ..Default::default()
 })
 .collect
 }
 ```

 * 调用点参数（[`../loop/chat_loop/inputs.rs`](../symbio-plugin-loop/src/chat_loop/inputs.rs)）：轮首段取 `TurnState::context_sections`（`run_recall` 里经 `context_sections(&request, Cadence::Turn)` 收集一次，`request_round == 0` 口径；**段数由 provider 决定**，宿主不假设几段）；请求级段在工具收集完成后经 `context_sections(&request, Cadence::Request)` 收集，两批按「轮首段在前、请求级段在后」拼成一个数组。

---

## 四、 会话记忆 (Session Memory)

会话记忆是**本会话自己钉住的约定与结论**：轮次一多，早期的结论会被压缩、裁掉乃至淘汰出上下文；会话记忆的价值恰恰在于**不随上下文水位消失**。

```text
{homedir}/session/<会话 id>/MEMORY.md 记忆本体（与 session.json / messages.json 同目录）
<根>/session/<会话 id>/MEMORY.md 可编辑地址（模型与用户共用）
```

它与转写（`messages.json`）的分工：转写是**流水**（说过什么），会话记忆是**从这个会话里提炼出来的、不许被压缩掉的那几条**。它不是「对话摘要」（那是压缩快照的活），也不是「跨会话的经验」（那该写进工作区或智能体记忆）。

### 会话这一层在「几层记忆」里的位置

**层数、各层的地址与所有者不在本文**——那张表由 `providers/memory/mod.rs` 的模块文档 own（复述必然漂移）。本文只 own 会话这一层的个性：物理落位 `{会话目录}/MEMORY.md`、地址 `<根>/session/<id>/MEMORY.md`、注入者是本插件。

各层形态相同、**共用 `providers/memory` 一份实现**——两道闸门（写侧拒绝 / 读侧截断）与「谁能读写它，谁负责注入它」的归属原则及理由见该模块文档注释；本插件只提供这一层的「落位、地址、标题、空内容提示、两道闸门开多大」（`memory_max_bytes` / `memory_inject_max_bytes`）。

本插件**不读** `{workdir}/AGENTS.md`——那份归 memory 插件的工作区腿：同一份文件被两个插件注入 = 同一内容进两次上下文。

### 与「事件记忆」的分工（两层，不是一个东西的两份）

本节讲的是**文件记忆**（`MEMORY.md`，钉住的约定，可编辑、走系统提示词注册通道）。
S5 的**事件记忆**是另一层：`memory_events.rs` 把每轮用户发言编码成 `memory.encoded`
事件写进**本会话的 `v2-events.wal`**，跨会话召回后**置顶注入请求视图**（第 6 节第五步），
活记忆到 4 条过 `consolidate::accept` 才合并——不另开文件、不经系统提示词、
不进 VDFS。分工：文件记忆 = 会话自己的约定（人可读写）；事件记忆 = 可检索、
可巩固、可排除式遗忘的**情景经验**（机器记账）。三段各自的判定方与四条口径见
[`memory_events.rs`](../symbio-plugin-loop/src/memory_events.rs) 模块文档。

### 系统提示词：只有一个注册通道

`Registry::put`（提示词段行 `CapabilityRow::prompt`）是**唯一**注册入口：注册的每一段都按注册顺序送达模型，**不存在「按优先级链取一个」的竞争**。`resolve_system_prompt` 的结果 = **请求显式段（若有）** + **全部注册段**，全空时才用兜底常量。

「取一个」这个需求本身站不住：人格的**选择**发生在**注册期**（model 插件已按 `PROVIDER_ID` 解析出唯一生效 provider 后才注册），消费侧再按 key 选一次是空动作。

会话侧经这个通道贡献**一段**：

| 注册名 | 内容 | 形态 |
|---|---|---|
| `session-memory` | 本会话的 `MEMORY.md`（**会话记忆**） | 可读写：有地址、两道闸门 |

> 智能体自身的 `AGENTS.md`（两个作用域）归 `agent` 插件，不经本通道——见 [`../agent/README.md`](../symbio-plugin-agent/README.md "crates/symbio-plugin-agent/README")。

### VDFS 侧

会话记忆挂在**会话节点之下**，与 `消息` / `子会话` / `工作目录` 并列——它本来就是会话的一部分，不另开一条寻址：

```text
<根>/session/<id>/MEMORY.md rw 记忆（文件；写受闸门约束）
```

- `list` 与 `stat` **共用共享实现产出的同一份形状**，两条链路不会分叉；
- 写入经 `vdfs_notify_change` 投递变更（路径 `<id>/MEMORY.md`，与列表地址同源）；
- **不可删除**（`delete` 恒 `Forbidden`）：删除即丢失本会话的长期约定，而抹掉之后没有东西能把它找回来。要清空就写入空内容——那是一次可读、可审、可撤销的显式动作。

### 作用域闸门

`ctx[SESSION_ID]` 缺失 / 为空 → **什么都不注入**。收集期拿不到会话 id 的广播（例如设置页的选项收集）不该凭空造一份记忆出来。闸门在 `SessionPlugin::store_with` 一处收口。

---

## 五、 策略对比与总结 (Summary Matrix)

| 策略维度 | 核心控制参数 | 执行时机 | 动作目标 | 底层实现文件 |
| :--- | :--- | :--- | :--- | :--- |
| **存储级轮数裁剪** | `max_messages` | `append_messages` 保存时 | FIFO 截断超出的最老对话轮（0 = 不限制），不动归档文件 | `crates/symbio-plugin-session/src/chat_session/write.rs` |
| **单轮迭代边界** | （无配置项：v2 不设工具轮数上限） | v2 运行器环体（[`../loop/turn_runner.rs`](../symbio-plugin-loop/src/turn_runner.rs) 2e/2f） | 跑多少批工具由模型自己收束；唯一的数量界是**单条响应**内 `MAX_TOOL_CALLS_PER_RESPONSE = 32`（超出的调用合成「本次调用未执行」喂回）。主循环侧读不到工具轮数，因此那一层的软上限不存在（裁决见 [D-074](../../docs/003-design/014-单环一表.md "DE-014-单环一表")） | `crates/symbio-plugin-model/src/stream.rs` |
| **加载轮次对齐 + 宏观语义快照合并** | `context_messages` / `auto_compress` | `get_context_messages` 加载时 / `prepare_compression` 请求前 | 三层清理后按最近 6 个 User 消息对齐截取完整轮次；70% Token 溢出时用 XML 状态快照合并（保留最近 30%） | `crates/symbio-plugin-session/src/chat_session/read.rs`<br>`crates/symbio-plugin-loop/src/context.rs` |
| **存储期历史工具链物理裁剪** | `context_messages`（分水岭）<br>`prune_tool_history`（开关） | `append_messages` 保存时 | 物理删除分水岭之前（默认最近 `context_messages` 轮内保留）的 Tool / ToolCall / Reasoning 及其子节点；**只删节点不删归档文件**（归档由 L0 `tool_result_guard` 滚动回收）；内存临时会话跳过此裁剪 | `crates/symbio-plugin-session/src/chat_session/write.rs`（`prune_historical_tool_calls`） |
| **请求级注入段** | （无配置项：三段各自判空） | `build_request_sections` 每轮请求前 | 长期记忆召回段（**置顶**，`meta.kind = recall_context`）→ 任务调度段（就绪任务集，`meta.kind = readyset`，记忆段仍占 `index 0`）→ 委派者真源段（`meta.kind = delegate_context`，落 `index 2`）；三段只追加、不改写历史，不落库幂等 | [`../loop/context/view.rs`](../symbio-plugin-loop/src/context/view.rs) |

通过这套**五维协同策略**，Symbio 构建了"存储层（写入时 L0 工具守卫 + 保存时 `max_messages` FIFO / `prune_historical_tool_calls` 生命周期裁剪 + 语义快照落库，内容节点恒为完整原文）→ 加载层（`context_messages` 轮次窗口对齐）→ 请求层（`build_request_sections` 三段注入，只追加不改写）→ 语义压缩层（`auto_compress` 语义合并）"的上下文治理链路，实现了高保真度的会话还原与清爽的本地持久化。请求体是事实网格的纯投影（[D-048a](README.md "crates/symbio-plugin-session/README")）——模型看到的每条历史都能在网格里找到出处，`prompt_fidelity` 每轮复核这一点。

---

## 六、 工具结果字段约定（跨插件契约）

> **目标**：让「工具返回任意 JSON → 压成一段给模型看的文本」这一**必要适配**不再隐式——加一个工具不必读 `extract_result` 源码才知道自己该返回哪个字段。

工具可以返回任意 JSON（`shell` 给 `output`、`vdfs_read` 给 `VdfsContent`、`ask_user` 给 `content` + `prompt`、`todo_write` / `content_search` 同理），而模型只消费**一段文本**。这层「任意形状 → 文本」的压缩是**必要的**，不是失误；失误在于它曾**没有写下来**，加一个工具就得读源码才知道写哪个字段名。

**唯一消费方**：[`tool_executor::extract_result`](../symbio-plugin-loop/src/tools/tool_executor.rs)（字段名有常量 `RESULT_CONTENT = "content"` / `RESULT_OUTPUT = "output"`，判定顺序由 [`tool_executor.test.rs`](../symbio-plugin-loop/src/tools/tool_executor.test.rs) 钉死——改顺序 = 改契约）。判定顺序（**先命中者胜**，而非「存在即取」）：

| 序号 | 判据 | 结果 |
|---|---|---|
| 1 | `content` 是字符串 | 它本身 |
| 2 | `output` 是字符串 | 它本身 |
| 3 | `success` 是布尔 | `true` → 整包 JSON；`false` → `Error: {error}` |
| 4 | 顶层就是字符串 | 它本身 |
| 5 | 以上都不是 | 整包 JSON |

**生产方 → 字段**一览（写哪个字段由各工具自己的文档注释声明，与消费方解耦——这是**自觉保留**的张力，不是疏漏）：

| 工具 | 返回字段 | 备注 |
|---|---|---|
| `shell`（命令类） | `output` | 命令 stdout/stderr 合并文本 |
| `content_search` | `output` | 搜索结果文本 |
| `vdfs_read` | `content`（`VdfsContent`） | 文件内容；「不给 id」的 `{"content":"..."}` 亦合法 |
| `ask_user` | `content` + `success:false` + `failure_kind` + `prompt` | 编排层凭 `failure_kind=NEEDS_INTERACTION` 收口为「等待用户动作」并构造 `user_prompt` 节点（结果节点唯一写入者在 `tool_executor`） |
| `todo_write` | `content` | 清单状态文本 |

> **控制流不得建立在这套字段上**：需要「本轮结束于等待用户动作」这类判断时，用 `failure_kind` 约定字段（见上表 `ask_user`），不要靠猜 JSON 形状——`extract_result` 只负责「取出可读文本」，不承载控制语义。

## 关键决策

> 本节是**仍然约束现行代码的取舍**：每条一句决策、一句理由。已退场或主题已消失的决策不在其中（`git log` 是历史）。

### D-003 Session 作为编排入口

早期 `agent` 插件独占会话编排，耦合严重。 `session` 是唯一编排入口（核心循环 `chat_loop`），`agent` 只负责「人格」。 关注点分离；同一 Session 可绑定不同 Agent；支持无 Agent 的纯工具模式。 Session 需要收集全树工具；Agent 插件适配为被动角色。

### D-018 压缩失败不得裁剪历史——失败必须是可见、可重试、可持久化的状态

上下文压缩（L2 语义压缩）失败时，历史上有一条「本地机械兜底」：若预判 LLM 摘要请求必然超限，就按 token 预算机械砍掉早期消息、插入截断说明头，然后回报成功。而这条路径的触发时机恰恰是「压缩反复失败的终点」⇒ 压缩失败的最终代价由历史买单，用户看到的却是「已压缩上下文」的成功态节点——没有原因、没有重试入口、历史凭空变短、后续内容与截断前失联。 压缩失败一律不改动历史。

### D-041 会话的响应性由编排层调度 + 无副作用对话服务承担

新增 `chat` 插件，`chat/reply` 输入 「用户输入 + 已投影上下文 + 运行现状快照 + intent」，输出 `Answered` / `Escalate` / `Report`。 `session` 保留唯一编排者与唯一转写写入者；它在三个调用点（轮首 triage / 轮边界 triage / 进度 tick）调 chat，并写回返回文本。 抽干整队、合并成一条用户消息、在轮边界折进当前轮。

### D-048a v2 的 prompt 形状必须回到结构化消息（不把工具结果拍平成纯文本）

D-048 把「`full` 让工具结果从 `role=tool` 降级成 user 消息里的纯文本」记为「未验证的假设」。复核后判定：这不是假设，是明确的退步——本条把它钉成决策。 v2 的 prompt 必须是结构化消息数组，保留 `role`（`user` / `assistant` / `tool`）与 `tool_call_id`，不得把整段对话渲染成一条 user 消息的纯文本。 工具结果的承载方式是协议，不是排版。

### D-049 WAL 是会话唯一事实源，SessionStore 降为缓存——轨 S 的起点

D-048 把出厂档位翻到 `full` 后，写侧换成 WAL 原生记账，`SessionStore` 不再是真源、降为缓存：删掉缓存后会话仍能开且历史完整，是换源的第一条判据。

### D-051 v1 执行档与 V2Mode 旋钮退役——v2 是唯一执行路径

`V2Mode` 是 `off` / `full` 两档的执行旋钮，`off` 档走 v1 执行路径。 它不是中性的「v1 模式」——它静默关掉四样东西（长期记忆召回 / 就绪任务集 / 技能路由 / 轮次事实入格），且 v1 分支与 v2 运行器是同一行为面的两份实现。

### D-052 事实的边界是「发生过什么」：思考与半截产出入格——真源切换的前置

事实网格记的是「发生过什么」。有两处产出被按「完成态」筛掉了： 思考不入格。收束格载荷只有正文与模型（`chat.assistant.final` = `text` + `model`），模型的推理过程只活在转写节点里，网格里查不到。 半截产出不入格。轮次被中断（用户按停止）或收束于等待用户时，落格纪律写的是 「不落任何收束格」，于是模型已经产出的半截正文、半截思考连同终止的原因与 时刻一起消失。

### D-053 消息的变更即事实：编辑 / 删除入格，压缩走快照—— 的 CRUD 前置

 要把消息真源从 `messages.json` 换到 `v2-events.wal`。 换源的第一步判据（删掉缓存后会话仍能开且历史完整）已经绿了，但主读路径整体切到重建 之前有一个硬阻塞：只有「追加」进了 WAL，其余对消息列表的变更全都没进。

### D-054 消息列表的变更收成闭集原语 + 顺序反转 + 类型强制——b3 的纪律与强制

[D-053](README.md "crates/symbio-plugin-session/README") 把「编辑 / 删除 / 压缩」 逐个补进事实网格。补到 S2.7-b3 前置时出现一个信号：每遇到一个新场景，就发现「这里也没有事实」 ——恢复路的删子树、abort 的改父节点、压缩记录节点、`partial` 节点、写侧裁剪……一处处补， 像在给一个漏水的桶贴补丁。用户的判断是对的：这样一直打补丁，很可能彻底破坏方案的正确性。 但「WAL 只追加不修改」不是元凶。

### D-057 prompt 形状基线钉「装配决定的结构」，规模只作观测

[D-048a](README.md "crates/symbio-plugin-session/README") 判定「把工具结果拍平成纯文本是退步」，依据是协议契约与 训练形态的论证；它同时留下一句没被兑现的话——本仓没有任何度量能回答 「这样对模型是变好还是变坏」。

### D-062 事实完整性收口：actor 在写入方按真实来源派生，ts 兜底在 WalStore 唯一写入口（MemoryStore 不兜底）

事实源里的每条事件必须完整回答谁（actor）、何时（ts）、发生了什么。这条不变量对 seq 成立（收口到 [`Store`](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 唯一写入口），对 actor 与 ts 没有对应约束——`d781a6c7` 号会话的实测后果：363 条助手与工具产出被记成 `actor: "user"`（写入方写死 `PRINCIPAL_USER`），548 个事件 `ts` 缺失或为 0，时间轴无法重建。修法：actor 在写入方按真实来源派生，`ts` 兜底收进 `WalStore` 唯一写入口（`MemoryStore` 不兜底）。

### D-072 `context_compact` 工具与 55% 水位提醒一并退役；压缩只剩一个入口

[DE-014 §5](../../docs/003-design/014-单环一表.md "DE-014-单环一表") 步骤 5 要删 v1 外壳的 `close_turn` 死分支。

### D-078 请求视图三层淡化退役：请求体是事实网格的纯投影，注入段只追加不改写

这条判定的起点是一句很轻的判断——「v2 请求里的历史部分不经过 `build_request_view` 的三层裁剪」。核完前提 发现这句话只说对了一半，而错的那一半决定了处方： 三层确实在算。

### D-085 压缩取代的是消息列表，不是轮次生命周期——恢复锚读完整史，归档刀口不动

快照的取代边界是消息列表，不是轮次生命周期。 快照回答的是「给模型看的上下文 从哪开始」；轮次事实（`turn × opened` / `turn × closed`）是 C4 的配对依据，不被 压缩改写。C4（`invariants::unresolved_turns`）读的本来就是完整史——归档后它仍会报 该轮未收束。

### D-111 消息读数由交付出口装配：压缩与记忆的事实行离开前端

口径的 owner 是拿着数的那一方。`context/pipeline.rs` 的文档注释里写着水位口径 （「内容水位，不含请求级固定开销 ⇒ 与上限相除是下限」），前端那份是第二处复述—— 后端改口径，前端不会红，只会静静显示旧标签（这正是 `present/reading.rs` 模块文档立的理由）。 `title`/`icon`/`actions` 走的就是「交付前按形状现算」， 本条只是把同一手法用于第四类呈现数据；

### D-137 上下文水位挂在轮开头的骨架节点：一次测量、出口排版、前端零口径

信任面要显示「还剩多少上下文」，而后端从来没有**实时水位**这个东西——估算只在压缩判定里算一次，算完即丢。于是缺口有两条假修法：每帧重估（把估算成本请进读路径，而估算不会因为多算几次变准），或前端自己乘阈值系数（阈值就有了第二个 owner，而它住在 `loop/context.rs`）。

裁决按**事实的粒度**走：水位是**请求级事实**，一轮一次。

- **测量点**：轮开头（`chat_loop/inputs.rs` ③′）算一次，随本轮的**骨架节点** `meta.context` 下发——骨架已经在 `emit_streaming_start` 里带 `turn`，两个请求级事实同属一个载体，故不新增 run_op、不新增 core 投影、不改 `ChatNode` 顶层字段。
- **会话级**：`SessionSummary::of`（save 时的投影）扫尾部**最近一条**带水位的节点，落进 `session.json`。不给 `vdfs/stat` 加专用回读——那是热路径，而清单本来就是会话的唯一读入口。
- **口径与措辞分家**（同 [D-111](README.md "crates/symbio-plugin-session/README")）：估算与阈值系数归 `loop/context.rs`，「数字怎么说」归 `stats_reading::context_reading_of`。阈值由 `compression_threshold_tokens` **同一个函数**同时喂触发判定与出口读数——两处各写一遍的系数必然漂，漂了表现为「界面上还没到刻度，后端已经动手整理」。
- **产出的是既有方言，不是自造形状**：本函数返回 `ReadingSection`（行 + 可选 `ReadingBar` 几何），条的 `level` 与参考线 `marks` 是产者**除好的 0..1 比率**。这里**不发一套自造的业务键**（`{ used_ratio, compress_ratio }` 那种形状）——业务键会把「已用」「压缩线」写进渲染器，换一个产者（成本、记忆）就得改前端，那正是 `present/reading` 模块文档立起来要挡的事。`bar` 是**方言里的通用几何槽**（不含任何主题词），且 `ReadingBar` 无方法、不是 trait：core 的三格形状（CS-001）不受影响。
- **不给「压缩」入口**：[D-072](README.md "crates/symbio-plugin-session/README") 之后手动重试只由 failed 压缩节点下发，一条健康的水位线后面没有任何可点的东西；按钮不能承诺它没有的能力。这条线的职责只有一句——让你看见**后端会在这里接手**。
- **缺数少一行，绝不补 0**：「水位已降到 0」是最坏的谎报（失败路径本就没有 `after_tokens`）；没有分母就没有 `bar`（渲染器因此整条不出现，而不是画一条宽度 0 的条）；过上限时文本仍报实测数，只有条画满。同一条诚实形状也用在失败上：方言载荷排不出来就**少这一格属性**，交付出口不为此炸进程——「形状被改坏」是 `stats_reading.test.rs` / `nodes.test.rs` 的断言，不是运行时的一次 panic。

代价随行：显示的始终是**本轮请求开头**那一刻的估算（`hint` 里就这么写），不是随着流式输出实时爬升的水位——一轮之内的增长看不见，而它本来就只在该轮结束时才成为事实。
