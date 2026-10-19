---
kind: arch
id: symbio/src/plugins/session/README
title: Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书)
status: active
created: 2026-07-20
---

# Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书)

Session 插件是 Symbio 架构中的**会话持久化与编排中心**，也是**唯一的会话编排入口**：加载历史、**解析**系统提示词（收集各插件经提示词段行注册的段，本层不自己拼）、经 `Registry`（收集口，put/get/list 三方法）汇集工具、直接获取 model 插件注册的唯一生效 `ModelProvider`（core 纯 trait，`Arc<dyn ModelProvider>` 单一契约；协议适配细节内化于 model 插件）并在进程内驱动会话循环；Model 插件是**无状态 LLM 网关**（按上下文注册 Provider + 协议适配），对 session 零依赖。

本文档将系统性地阐述 Symbio 的会话保存、内容压缩、工具迭代限制以及发送过滤策略，说明其具体规则、参数配置及 Rust 底层实现策略。

> **深度设计与专题文档**（核心循环收口、模块分工、历史审计、前端性能、VDFS 会话消息映射、级联选项……）
> 见 [`docs/`](docs/README.md "symbio/src/plugins/session/docs/README")——本文只写**现行机制与配置面**，`docs/` 写**为什么这样设计**。

---

## 一、 系统架构数据流图 (Architecture & Data Flow)

下列架构图展示了一个完整的“用户请求 -> Session (编排入口) -> 进程内会话循环 (视图组装/推理/工具) -> 磁盘持久化（会话目录）”的生命周期：

```mermaid
flowchart TD
    User([1. 用户消息：chat/send 或写 <sid>/inbox]) --> SessionChat[Session 插件 - chat 路由]
    SessionChat --> SaveUser[2. 入队收件箱 → 空间自己消费<br>空闲时取队首 → 保存/追加用户消息]
    SaveUser --> LoadHistory[3. 加载历史上下文<br>get_context_messages 轮次窗口对齐]
    LoadHistory --> BuildPrompt[4. 解析系统提示词<br>收集各插件注册的段<br>人格/记忆/指令]

    subgraph Session 会话编排层（唯一编排入口）
        BuildPrompt --> AggTools[5. 工具汇集<br>CAPABILITY_VISITOR 注册表]
        AggTools --> Resolve[6. 获取生效 ModelProvider<br>capability_model_active 单次解析]
        Resolve --> Spawn[7. 进程内 spawn 会话循环<br>RATE_LIMITER 限流跟随请求方]
        Spawn --> BuildView[8. 构建请求视图<br>build_request_sections<br>记忆/调度/委派三段注入]
        BuildView --> LLMCall[9. ModelProvider 推理调用<br>execute_turn 直连无状态 LLM 网关]
        LLMCall --> ToolAction[10. 执行工具链]
        ToolAction --> LoopCheck{11. 迭代完成?}
        LoopCheck -- No --> BuildView
    end

    LoopCheck -- Yes --> EndTurn[12. 助理回答定格]
    EndTurn --> SaveFinal[13. 将助理与工具结果回写 Session]
    SaveFinal --> Disk[(会话目录<br>&lt;homedir&gt;/session/&lt;id&gt;/<br>session.json + messages.json)]
```

---

## 二、 核心参数与配置 Schema (Configuration Details)

会话与压缩系统的全部表现都由会话配置参数控制。配置落在**本插件自己的目录**：

```text
{homedir}/session/PLUGIN.yml     本插件的配置（可在 <根>/session/PLUGIN.yml 上编辑）
```

字段真源是本插件的 `config.rs::SessionConfig`（**定义由配置的拥有者产出**：设置页的表单定义从 `SessionConfig::default()` 读出，不写第二份字面量，避免「面板显示值与实际行为漂移」）。

以下是**与本节机制直接相关的键**的示例（不是全表）：全字段真源见
`config.rs::SessionConfig`，配置面总表见 [`docs/006-ref/001-CONFIGURATION.md`](../../../../docs/006-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")。
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
  max_messages: 100              # 单会话本地保存的最大对话轮数限制 (以 User 消息计数；0 = 不限制，默认 100)
  
  # 2. 单轮迭代边界：v2 **没有**这一项配置（工具轮数不设上限，理由见三.2）
  
  # 3. 宏观语义压缩策略
  auto_compress: true            # 是否启用基于 Token 溢出 (70% 触发) 的大模型自我语义合并
  context_messages: 6            # 发送给 AI 的对话上下文滑动窗口轮数 (以 User 消息轮次对齐截取；也是存储期 prune 的保留分水岭)
  
  # 4. 存储期工具链裁剪
  prune_tool_history: true       # 是否在落库时物理裁剪 context_messages 分水岭之前的历史工具链（Tool/ToolCall/Reasoning 及其子节点；只删节点不删归档；内存临时会话自动跳过）
  
  # 5. 会话记忆（<会话目录>/MEMORY.md，可编辑地址 <根>/session/<id>/MEMORY.md）
  memory_max_bytes: 16384        # 单次**写入**的字节上限，超出直接拒绝（不截断、不部分写入）
  memory_inject_max_bytes: 4096  # 每轮**注入**系统提示词的字节上限，超出部分截断并指路 vdfs_read 取全文
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
    let mut user_indices = Vec::new();
    for (idx, msg) in session.messages.iter().enumerate() {
        if msg.role == Some(NodeRole::User) {
            user_indices.push(idx);
        }
    }
    if max_turns > 0 && user_indices.len() > max_turns {
        let start_idx = user_indices[user_indices.len() - max_turns];
        session.messages.drain(0..start_idx); // 归档文件不在此处删除
    }
    ```

---

### 2. 单轮迭代边界（v2：不设工具轮数上限）

> **目标**：让自主 Agent 的多步决策由模型自己收束，主循环不设「跑了几轮就停」的旋钮。

* **现状规则**：
  * 一次用户 Prompt 触发一个请求；请求内部 `推理 -> 工具调用 -> 工具结果返回 -> 再推理` 的迭代在
    v2 运行器的环体里跑完（`turn_runner.rs` 的 2e/2f），**主循环侧看不见第几轮工具**。
  * 界只有三条：单条响应内的工具调用数上限 `MAX_TOOL_CALLS_PER_RESPONSE = 32`
    （`plugins/model/stream.rs`，超出的调用合成一条「本次调用未执行」的结果喂回模型，指引它
    把剩余调用放到下一轮）、用户中止、provider 断流。
  * 长度截断（`finish = Length`）后的自动续写有 `MAX_CONTINUE_ROUNDS = 3`
    （`chat_loop/turn.rs`），它数的是**续写次数**，不是工具轮数。
* **为什么这一层不设软上限**：主循环读不到工具轮数——工具在 v2 运行器的环体里一批批跑完
  （`turn_runner.rs`，步骤序列与批次边界见
  [AR-015-core-loop](docs/015-core-loop.md "AR-015-core-loop") §4.1），本轮请求侧只有一个
  「发过几次请求」的计数（续写才让它 >1）。一个判不到真实轮数的
  上限等于**没有上限**，却会让人以为设过：它与「默认不限制」逐字等价，配错与没配长得一样，
  没有任何信号能区分。真要这道上限，位置在 2e（结果接进 `exchange`）与 2f（轮内注入）之间，
  那里有真实的 `tool_round`；裁决与逐符号名册见
  [ADR-074](../../../../docs/010-adr/074.md "ADR-074")。

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
  * **Turn 对齐获取 (`plugins/session/chat_session/read.rs`)**：
    * `ChatSession::get_context_messages` 中先做三层清理（过滤 `Failed` 消息、剔除孤儿节点、content 归一兜底），再以 `context_messages` (可通过参数显式覆盖) 调用 `sliding_window` 实现 User 消息轮次级对齐：

    ```rust
    // 从后往前定位第 context_limit 个 User 消息，以其作为起点裁剪历史
    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Some(NodeRole::User) {
            user_indices.push(idx);
        }
    }
    if user_indices.len() <= max_turns {
        return messages.to_vec();
    }
    let start_idx = user_indices[user_indices.len() - max_turns];
    messages[start_idx..].to_vec()
    ```

  * **语义合并 (`plugins/session/context.rs`)**：
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
  * **物理清理 (`plugins/session/chat_session/write.rs` 内的 `prune_historical_tool_calls`，保存消息时调用；该函数只被 session 插件消费，定义在 session 插件内)**：

    ```rust
    // keep_turns = 配置的 context_messages（默认 6）：
    // 定位倒数第 keep_turns 个 User 消息 limit_idx 作为分水岭
    let limit_idx = user_indices[user_indices.len() - keep_turns];
    // 1. 扫描 messages[..limit_idx]，收集 Tool / ToolCall(msg_type) / Reasoning 消息
    //    加入 to_remove，并异步删除其关联的 .txt 存档文件
    // 2. 被剪除 ToolCall 的直接子节点（请求/响应 Text）一并加入 to_remove
    // 3. 无保留子节点且文本为空的 Assistant 容器一并加入 to_remove
    // 4. 物理截断：
    messages.retain(|msg| !to_remove.contains(&msg.id));
    ```

---

### 5. 请求级注入段（`build_request_sections`：长期记忆 / 任务调度 / 委派者真源）

> **目标**：把三段**本轮才成立**的读视图送进请求——长期记忆召回、就绪任务集、委派判定。它们不是会话历史的一部分，因此不落库、每请求现算。
>
> **边界**：注入段只**追加**，不改写历史。发给模型的对话体是事实网格的纯投影（[ADR-048a](../../../../docs/010-adr/048a.md "ADR-048a")）；请求视图级的内容淡化 / 轮次淡化 / 分层滑窗骨架化三层已按 [ADR-078](../../../../docs/010-adr/078.md "ADR-078") 退役——那三层改写的是历史正文，与「请求体 = 投影」互斥。

* **具体规则**（唯一入口 `build_request_sections`，三段各自独立、非空才插）：
  * **长期记忆召回段 (Recall Context)**：调用方（`chat_loop/inputs.rs`）在**本轮第一次请求**读一次跨会话召回视图（`memory_events::recall_view`，`off` 档与无目录的临时会话不读），把排版好的段落（`memory_events::prompt_section`，抬头 `【长期记忆】`）传入；非空时产出一条 `meta.kind = "recall_context"` 的 User 消息。**置顶不置尾**：置尾会把「最后一条 user 消息」从用户的问题换成记忆——mock 场景匹配与轮次窗口都读错；**不进系统提示词**：那里有「唯一真源 = 注册段」的纪律（本 README 第 1 节）。e2e `t29` 同时钉住「记忆段在包里」与「`messages.json` 里一个字都不多」。
  * **任务调度段 (Ready Set)**：同一时机读一次本会话事实源的就绪任务集（`task_events::prompt_section`），抬头 `【任务调度】`，只列**依赖已全部完成**的候选；非空时产出 `meta.kind = "readyset"`。**不与记忆段争 index 0**：本段先插、记忆段随后再插一次 0 把它挤到 `index 1`（两段并存时记忆 0 / 调度 1，单独存在时各自在 0）。候选集空 ⇒ **整段不出现**（给模型一段空调度只会诱导它瞎猜）。e2e `t31` 同时钉住「段落在包里」与「`messages.json` 里一个字都不多」。
  * **委派者真源段 (Delegate Context)**：调用方（`chat_loop/delegate.rs::delegate_section_with`）把本轮**委派判定**（命中了哪条判据 / 为什么没启动子会话）渲染成段落；非空时产出 `meta.kind = "delegate_context"`，最终落在 **index 2**（记忆 0 / 调度 1 / 委派 2）。插入次序是判据的一部分：顺序反了三段的位置全部错位，而段位置是模型读到的上下文次序（见 [ADR-047](../../../../docs/010-adr/047.md)）。
  * **三段的消费形态**：`chat_loop::request_view_prefix` 按 `meta.kind`（不按下标——缺一段时下标全部左移）取出三段，按**记忆 → 调度 → 委派**拼成一条 `system` 前缀消息交给运行器。拼成一条是因为运行器的 prompt 入口只收一段前缀；标 `system` 而不是 `user`，是为了让「本轮用户说了什么」在消息层上没有歧义。
  * **水位治理不在本层**：压缩只有一个入口 `auto_compress_process`（70% 阈值自动触发，加上用户点「失败压缩节点重试」的 `force` 路径），它改写的是存储态上下文（写成 `session.consolidated` 一格事实），不是请求视图。
* **Rust 实现策略**：
  * 唯一入口为 `plugins/session/context/view.rs` 的 `build_request_sections`，由 `plugins/session/chat_loop/inputs.rs::prepare_turn_inputs` 调用：

    ```rust
    pub fn build_request_sections(
        recall_section: Option<&str>,
        ready_section: Option<&str>,
        delegate_section: Option<&str>,
    ) -> Vec<ChatNode> {
        let mut sections = Vec::new();
        // 插入次序：委派 → 调度 → 记忆。各插 0，最终读作 记忆 0 / 调度 1 / 委派 2。
        if let Some(t) = delegate_section { sections.insert(0, section("delegate_context", t)); }
        if let Some(t) = ready_section    { sections.insert(0, section("readyset", t)); }
        if let Some(t) = recall_section   { sections.insert(0, section("recall_context", t)); }
        sections
    }
    ```

  * 调用点参数（`chat_loop/inputs.rs`）：`recall_section` 由 `TurnState::recall_view`（只在 `request_round == 0` 取一次）现算排版，视图级不落库；`ready_section` 由 `TurnState::ready_section`（同一条 `request_round == 0` 口径）调 `task_events::prompt_section` 现算（**只读**开档、不创建事实源，`off` 档不读）；`delegate_section` 由 `chat_loop/delegate.rs::delegate_section_with` 就地判定。

---

## 四、 会话记忆 (Session Memory)

会话记忆是**本会话自己钉住的约定与结论**：轮次一多，早期的结论会被压缩、裁掉乃至淘汰出上下文；会话记忆的价值恰恰在于**不随上下文水位消失**。

```text
{homedir}/session/<会话 id>/MEMORY.md     记忆本体（与 session.json / messages.json 同目录）
<根>/session/<会话 id>/MEMORY.md                 可编辑地址（模型与用户共用）
```

它与转写（`messages.json`）的分工：转写是**流水**（说过什么），会话记忆是**从这个会话里提炼出来的、不许被压缩掉的那几条**。它不是「对话摘要」（那是压缩快照的活），也不是「跨会话的经验」（那该写进工作区或智能体记忆）。

### 三层记忆是同一件事的三个作用域

| 层 | 所有者插件 | 物理落位 | VDFS 地址 |
|---|---|---|---|
| 工作区 | work | `{workdir}/AGENTS.md` | `<根>/work/AGENTS.md` |
| **会话** | **session** | **`{会话目录}/MEMORY.md`** | **`<根>/session/<id>/MEMORY.md`** |
| 智能体 | agent | `{agent 目录}/AGENTS.md` | `<根>/agent/<id>/AGENTS.md` |

三层形态相同、**共用 `providers/memory` 一份实现**——两道闸门（写侧拒绝 / 读侧截断）与「谁能读写它，谁负责注入它」的归属原则及理由见该模块文档注释；本插件只提供会话这一层的「个性」：落位、地址、标题、空内容提示、两道闸门开多大（`memory_max_bytes` / `memory_inject_max_bytes`）。

**本插件不再读 `{workdir}/AGENTS.md`**——那是 work 的工作区记忆。

### 与「事件记忆」的分工（两层，不是一个东西的两份）

本节讲的是**文件记忆**（`MEMORY.md`，钉住的约定，可编辑、走系统提示词注册通道）。
S5 的**事件记忆**是另一层：`memory_events.rs` 把每轮用户发言编码成 `memory.encoded`
事件写进**本会话的 `v2-events.wal`**，跨会话召回后**置顶注入请求视图**（第 6 节第五步），
活记忆到 4 条过 `consolidate::accept` 才合并——不另开文件、不经系统提示词、
不进 VDFS。分工：文件记忆 = 会话自己的约定（人可读写）；事件记忆 = 可检索、
可巩固、可排除式遗忘的**情景经验**（机器记账）。三段各自的判定方与四条口径见
[`memory_events.rs`](../loop/memory_events.rs) 模块文档。

### 系统提示词：只有一个注册通道

`Registry::put`（提示词段行 `CapabilityRow::prompt`）是**唯一**注册入口：注册的每一段都按注册顺序送达模型，**不存在「按优先级链取一个」的竞争**。`resolve_system_prompt` 的结果 = **请求显式段（若有）** + **全部注册段**，全空时才用兜底常量。

「取一个」这个需求本身站不住：人格的**选择**发生在**注册期**（model 插件已按 `PROVIDER_ID` 解析出唯一生效 provider 后才注册），消费侧再按 key 选一次是空动作。

会话侧经这个通道贡献**一段**：

| 注册名 | 内容 | 形态 |
|---|---|---|
| `session-memory` | 本会话的 `MEMORY.md`（**会话记忆**） | 可读写：有地址、两道闸门 |

> 智能体自身的 `AGENTS.md`（两个作用域）归 `agent` 插件，不经本通道——见 [`../agent/README.md`](../agent/README.md "symbio/src/plugins/agent/README")。

### VDFS 侧

会话记忆挂在**会话节点之下**，与 `消息` / `子会话` / `工作目录` 并列——它本来就是会话的一部分，不另开一条寻址：

```text
<根>/session/<id>/MEMORY.md     rw    记忆（文件；写受闸门约束）
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
| **存储级轮数裁剪** | `max_messages` | `append_messages` 保存时 | FIFO 截断超出的最老对话轮（0 = 不限制），不动归档文件 | `plugins/session/chat_session/write.rs` |
| **单轮迭代边界** | （无配置项：v2 不设工具轮数上限） | v2 运行器环体（`turn_runner.rs` 2e/2f） | 跑多少批工具由模型自己收束；唯一的数量界是**单条响应**内 `MAX_TOOL_CALLS_PER_RESPONSE = 32`（超出的调用合成「本次调用未执行」喂回）。主循环侧读不到工具轮数，因此那一层的软上限不存在（裁决见 [ADR-074](../../../../docs/010-adr/074.md "ADR-074")） | `plugins/model/stream.rs` |
| **加载轮次对齐 + 宏观语义快照合并** | `context_messages` / `auto_compress` | `get_context_messages` 加载时 / `prepare_compression` 请求前 | 三层清理后按最近 6 个 User 消息对齐截取完整轮次；70% Token 溢出时用 XML 状态快照合并（保留最近 30%） | `plugins/session/chat_session/read.rs`<br>`plugins/session/context.rs` |
| **存储期历史工具链物理裁剪** | `context_messages`（分水岭）<br>`prune_tool_history`（开关） | `append_messages` 保存时 | 物理删除分水岭之前（默认最近 `context_messages` 轮内保留）的 Tool / ToolCall / Reasoning 及其子节点；**只删节点不删归档文件**（归档由 L0 `tool_result_guard` 滚动回收）；内存临时会话跳过此裁剪 | `plugins/session/chat_session/write.rs`（`prune_historical_tool_calls`） |
| **请求级注入段** | （无配置项：三段各自判空） | `build_request_sections` 每轮请求前 | 长期记忆召回段（**置顶**，`meta.kind = recall_context`）→ 任务调度段（就绪任务集，`meta.kind = readyset`，记忆段仍占 `index 0`）→ 委派者真源段（`meta.kind = delegate_context`，落 `index 2`）；三段只追加、不改写历史，不落库幂等 | `plugins/session/context/view.rs` |

通过这套**五维协同策略**，Symbio 构建了"存储层（写入时 L0 工具守卫 + 保存时 `max_messages` FIFO / `prune_historical_tool_calls` 生命周期裁剪 + 语义快照落库，内容节点恒为完整原文）→ 加载层（`context_messages` 轮次窗口对齐）→ 请求层（`build_request_sections` 三段注入，只追加不改写）→ 语义压缩层（`auto_compress` 语义合并）"的上下文治理链路，实现了高保真度的会话还原与清爽的本地持久化。请求体是事实网格的纯投影（[ADR-048a](../../../../docs/010-adr/048a.md "ADR-048a")）——模型看到的每条历史都能在网格里找到出处，`prompt_fidelity` 每轮复核这一点。

---

## 六、 工具结果字段约定（跨插件契约）

> **目标**：让「工具返回任意 JSON → 压成一段给模型看的文本」这一**必要适配**不再隐式——加一个工具不必读 `extract_result` 源码才知道自己该返回哪个字段。

工具可以返回任意 JSON（`shell` 给 `output`、`vdfs_read` 给 `VdfsContent`、`ask_user` 给 `content` + `prompt`、`todo_write` / `content_search` 同理），而模型只消费**一段文本**。这层「任意形状 → 文本」的压缩是**必要的**，不是失误；失误在于它曾**没有写下来**，加一个工具就得读源码才知道写哪个字段名。

**唯一消费方**：[`tool_executor::extract_result`](../loop/tools/tool_executor.rs)（字段名有常量 `RESULT_CONTENT = "content"` / `RESULT_OUTPUT = "output"`，判定顺序由 [`tool_executor.test.rs`](../loop/tools/tool_executor.test.rs) 钉死——改顺序 = 改契约）。判定顺序（**先命中者胜**，而非「存在即取」）：

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
