# Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书)

Session 插件是 Symbio 架构中的**会话持久化与编排中心**，也是**唯一的会话编排入口**：加载历史、**解析**系统提示词（收集各插件经 `register_system_prompt` 注册的段，本层不自己拼）、经 CapabilityVisitor 汇集工具、直接获取 model 插件注册的唯一生效 `ModelProvider`（core 纯 trait，`Arc<dyn ModelProvider>` 单一契约；协议适配细节内化于 model 插件）并在进程内驱动会话循环；Model 插件是**无状态 LLM 网关**（按上下文注册 Provider + 协议适配），对 session 零依赖。

本文档将系统性地阐述 Symbio 的会话保存、内容压缩、工具迭代限制以及发送过滤策略，说明其具体规则、参数配置及 Rust 底层实现策略。

> **深度设计与专题文档**（核心循环收口、模块分工、历史审计、前端性能、心跳、VDFS 会话消息映射、级联选项……）
> 见 [`docs/`](./docs/README.md)——本文只写**现行机制与配置面**，`docs/` 写**为什么这样设计**。

---

## 一、 系统架构数据流图 (Architecture & Data Flow)

下列架构图展示了一个完整的“用户请求 -> Session (编排入口) -> 进程内会话循环 (视图组装/推理/工具) -> 数据库持久化”的生命周期：

```mermaid
flowchart TD
    User([1. 用户发起 Chat 请求]) --> SessionChat[Session 插件 - chat 路由]
    SessionChat --> SaveUser[2. 保存/追加用户消息]
    SaveUser --> LoadHistory[3. 加载历史上下文<br>get_context_messages 轮次窗口对齐]
    LoadHistory --> BuildPrompt[4. 解析系统提示词<br>收集各插件注册的段<br>人格/记忆/指令]

    subgraph Session 会话编排层（唯一编排入口）
        BuildPrompt --> AggTools[5. 工具汇集<br>CAPABILITY_VISITOR 注册表]
        AggTools --> Resolve[6. 获取生效 ModelProvider<br>get_model_provider 单次解析]
        Resolve --> Spawn[7. 进程内 spawn 会话循环<br>RATE_LIMITER 限流跟随请求方]
        Spawn --> BuildView[8. 构建请求视图<br>build_request_view<br>内容淡化/工具淡化/骨架化/水位提醒]
        BuildView --> LLMCall[9. ModelProvider 推理调用<br>execute_turn 直连无状态 LLM 网关]
        LLMCall --> ToolAction[10. 执行工具链]
        ToolAction --> LoopCheck{11. 迭代完成?}
        LoopCheck -- No --> BuildView
    end

    LoopCheck -- Yes --> EndTurn[12. 助理回答定格]
    EndTurn --> SaveFinal[13. 将助理与工具结果回写 Session]
    SaveFinal --> SQLite[(会话数据库 / SQLite)]
```

---

## 二、 核心参数与配置 Schema (Configuration Details)

会话与压缩系统的全部表现都由会话配置参数控制。配置落在**本插件自己的目录**：

```text
{homedir}/session/PLUGIN.yml     本插件的配置（可在 <根>/session/PLUGIN.yml 上编辑）
```

字段真源是本插件的 `config.rs::SessionConfig`（**定义由配置的拥有者产出**：设置页的表单定义从 `SessionConfig::default()` 读出，不写第二份字面量，避免「面板显示值与实际行为漂移」）。

以下是完整配置示例：

```yaml
session:
  # 存储目录：固定为 <homedir>/session/，取宿主层的资源类别根
  # （vdfs_service::entry::category_dir），跟随系统目录 (homedir) 切换，不作为配置项暴露。
  # 注：会话只有一种落盘布局，没有可切换的"存储后端"——原 `store_kind` 配置项
  # 已删除（理由见 store/mod.rs 模块头）。临时会话（`_t_` 前缀 / 空 id）不落盘，
  # 由构造点选择，与用户配置无关。
  # 注：会话默认不绑定智能体（纯工具模式）。智能体选择属于前端会话级偏好，
  # 由 session.metadata.agent_id 按会话记录，后端不提供 default_agent。
  
  # 1. 存储级策略
  max_messages: 100              # 单会话本地保存的最大对话轮数限制 (以 User 消息计数；0 = 不限制，默认 100)
  
  # 2. 单轮迭代策略
  max_tool_rounds: 0             # 单轮对话中工具迭代轮数软上限 (0 = 不限制，默认；仅当显式调低时作为软上限，达到后提示并退出，不静默熔断)
  
  # 3. 微观内容截断策略
  compress_line_threshold: 200   # 单条内容节点（正文/思考）的行数阈值，超过该行数（或 token 超 2048）在请求视图中淡化（存储恒为完整原文）
  compress_keep_recent: 3        # 内容节点淡化时豁免的最近内容节点数量（B1 保护窗口；末条消息恒受保护）
  
  # 4. 宏观语义压缩策略
  auto_compress: true            # 是否启用基于 Token 溢出 (70% 触发) 的大模型自我语义合并
  context_messages: 6            # 发送给 AI 的对话上下文滑动窗口轮数 (以 User 消息轮次对齐截取；也是存储期 prune 的保留分水岭)
  
  # 5. 工具滑动窗口策略
  tool_context_window: 15        # LLM 推理上下文中保留完整明细的最近工具调用数量 (超出者骨架化为带语义摘要与取回指引的占位符（规模 + 条目名/首行 + 定位锚点 + `Re-run <tool>` 指引，见 `context_window.rs`）；声明 context_retention 保留策略的工具其最新调用豁免此窗口；0 = 关闭骨架化)
  fade_activate_rounds: 40       # 单轮工具迭代超过该轮数后，激活"老旧工具结果淡化"（请求视图层）
  fade_keep_recent_turns: 12     # 工具淡化时保留完整结果最近轮数（窗口外的更旧结果做头尾摘要）
  prune_tool_history: true       # 是否在落库时物理裁剪 context_messages 分水岭之前的历史工具链（Tool/ToolCall/Reasoning 及其子节点；只删节点不删归档；内存临时会话自动跳过）
  
  # 6. 水位提醒与主动压缩
  enable_compact_tool: false     # 是否启用 55% Token 水位提醒 (nudge) 注入与 context_compact 主动压缩工具 (默认关闭，须手动开启；关闭后仅保留 70% 自动压缩兜底)
  
  # 7. 会话记忆（<会话目录>/AGENTS.md，可编辑地址 <根>/session/<id>/AGENTS.md）
  memory_max_bytes: 16384        # 单次**写入**的字节上限，超出直接拒绝（不截断、不部分写入）
  memory_inject_max_bytes: 4096  # 每轮**注入**系统提示词的字节上限，超出部分截断并指路 vdfs_read 取全文
```

---

## 三、 六大核心策略的具体规则与 Rust 实现机制 (Implementation Strategies)

### 1. 单会话对话轮数上限存储策略 (`max_messages`)

> **目标**：防止单个会话历史由于对话轮数过多而无限膨胀，导致磁盘溢出或加载序列化变慢。

* **具体规则**：
  * 对每个 Active Session 物理存储的历史记录实施**最大保存对话轮数限制**。
  * `max_messages` 表示最大保存的对话轮数，每一轮以一个 `User` 消息起始；默认 `100`，`0` 表示不限制（历史上此处曾有 `.max(500)` 硬下限，导致设置面板中小于 500 的值静默失效，现已移除）。
  * 当存储的对话轮数超出阈值时，自动从会话开头执行 FIFO 裁剪，移除最老的多余对话轮次。**只删消息节点、不动 `tool_archives/` 归档文件**：归档的磁盘生命周期由 `tool_result_guard` 的滚动淘汰（每会话保留最新 N 个文件）专职负责，避免 `max_messages` 成为静默删文件的破坏性操作。
  * **智能对齐**：此策略确保物理保存下来的会话，其起始消息也总是以一个完整的 `User` 消息起始，从而绝对避免了历史反序列化对齐失败的问题。
* **Rust 实现策略**：
  * 在 `SessionPlugin::invoke_append` 中，扫描 `User` 消息索引并执行轮数对齐物理截断，确保物理存储也总是以 `User` 消息为开端：

    ```rust
    let max_turns = cfg.max_messages; // 0 = 不限制
    let mut user_indices = Vec::new();
    for (idx, msg) in session.messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }
    if max_turns > 0 && user_indices.len() > max_turns {
        let start_idx = user_indices[user_indices.len() - max_turns];
        session.messages.drain(0..start_idx); // 归档文件不在此处删除
    }
    ```

---

### 2. 单轮会话工具调用迭代上限策略 (`max_tool_rounds`)

> **目标**：为自主 Agent 的多步决策提供可选的"软安全网"，防止极端场景下大模型失误、死锁或命令失败导致的"天价账单死循环 (Runaway Loop)"。默认**不设上限**，把控制权交给模型自主决策。

* **具体规则**：
  * 在用户提交单次 Prompt 后，系统在后台开启一个自主决策（Loop）链。
  * 大模型在一个循环中可以做 `推理 -> 工具调用 -> 工具结果返回 -> 再推理 -> 再工具调用` 的迭代。
  * **默认无上限**：`max_tool_rounds` 默认 `0`（`0` 即显式『不限制』，不再用 65535 这类魔法数表达同一意图），经 `SessionConfig::model_chat_max_tool_rounds()` 翻译为请求层 `max_tool_rounds: None`，正常对话不会被打断。
  * **显式软上限**：仅当调用方**主动**设置该参数（`Some(max)`）时，迭代达到上限后不再继续调用工具，而是向前端发送明确提示后正常持久化退出（非静默熔断）：

    > 已达到本轮工具调用上限（{max}）。如需继续，请再次发送消息。

  * 用户再次发送消息即开启新一轮请求，历史上下文照常加载，天然支持"超限续跑"。
* **Rust 实现策略**：
  * 在 `ChatOrchestrator::run_chat_loop` 的主编排环中，每轮开始检查软上限（仅显式设置时生效）：

    ```rust
    // 默认（request 未显式给出）=「无上限」；仅在调用方**显式**设置时才作为软上限并给出提示。
    let configured_max_tool_rounds = req.max_tool_rounds;
    // ... 主循环内：
    if let Some(max) = configured_max_tool_rounds {
        if tool_rounds >= max {
            // 向前端发送明确提示后持久化退出（非静默熔断）
            let error = format!("已达到本轮工具调用上限（{max}）。如需继续，请再次发送消息。");
            // ... 发送 Error 事件 → persist_messages → fire_stop_hook → return Ok(())
        }
    }
    ```
  * Session 编排层（`orchestrator.rs`）通过 `session_cfg.model_chat_max_tool_rounds()` 下发：配置为 `0` 时传 `None`（无限轮次），`> 0` 时传 `Some(n)`（软上限）。此前无条件 `Some(...)` 的写法会让 `0` 变成"0 轮即熔断"的错误行为，已修正。

---

### 3. 微观内容节点淡化策略 (`compress_line_threshold` × `compress_keep_recent`)

> **目标**：保护 LLM 发送上下文免受超大日志、超长代码文件等大文本的直接拖累。**内容节点存储完整原文，淡化只发生在发给大模型之前的请求视图层**——追溯能力由会话存储本身保证，淡化参数调整/回滚即时生效。

> **设计原则——"何时落库"分层决策表**（全系统上下文治理的统一判据）：
>
> | 变换 | 性质 | 时机 | 落库？ | 理由 |
> |---|---|---|---|---|
> | L0 工具结果守卫 | 机械截断 | **写入时** | ✅ 预算内文本 + 全文存档 + 取回协议 | 入口守卫：防超大文本进入存储与请求包 |
> | 内容节点淡化（本节） | 机械截断 | 请求视图每轮 | ❌ | 参数/位置的纯函数，重算毫秒级；**落库零 token 收益**，却需存档/取回/配对清理整套机制 |
> | 工具淡化 / 骨架化 / 水位提醒 | 机械变换 | 请求视图每轮 | ❌ | 依赖轮次位置，需随配置即时生效 |
> | L2 语义快照 | LLM 摘要 | 水位触发 | ✅ | 不可逆、需智能、重放昂贵（需再调一次 LLM） |
> | prune / FIFO 裁剪 | 物理删除 | 保存时 | ✅ | 生命周期管理，控制存储体积 |
>
> 分界线：**纯机械、可重放的变换 → 视图层每轮计算（存储 = 真实历史）；需智能且不可逆的变换 → 落库固化；影响存储体积的 → 写入时守卫**。
>
> *性能注*：视图层每轮对全历史重跑淡化确为重复计算，但均为单遍字符串扫描（毫秒级，对比 LLM 推理延迟可忽略）。若未来 profiling 证实占比过高，预案为**内存级淡化缓存**（键 = `(msg_id, 内容 hash, 配置指纹)`，会话生命周期内有效，命中后跳过重算，调参经指纹自动失效）——明确排除"淡化结果落库"方案（零 token 收益，见上表）。

* **具体规则**（`build_request_view` 四步之首，每轮无条件执行）：
  * **内容节点** = User/Assistant 消息的正文与思考（Reasoning）文本；工具调用参数与工具结果分别由骨架化层与工具淡化层管辖，不在此处理。
  * **B1 保护窗口**：最后一条消息恒受保护，另有最近 `compress_keep_recent`（默认 `3`）个内容节点豁免——活跃对话永远 100% 完整。
  * 窗口之外的内容节点，当**行数超过 `compress_line_threshold`（默认 `200`）或 token 超过 2048** 时执行视图级淡化：
    * 行数路径：保留头尾各 `compress_line_threshold / 4` 行，中段替换为 `[... 已省略 N 行中段内容 ...]`；
    * Token 路径（单行超长 JSON/URL/base64 绕过行数检测时）：按 2048 token 预算做头 60% / 尾 40% 切分，占位说明"该早期内容已在请求视图中淡化以控制上下文长度，完整原文保留于会话存储"。
  * 淡化仅在视图内存中生效（节点标记 `meta.content_faded`），**不写存档文件、不回写存储**；视图每轮从存储重建，天然幂等。
* **Rust 实现策略**：
  * 在 `plugins/session/compression.rs` 的 `fade_aged_content_nodes` 中实现，由 `build_request_view` 作为第一步调用（完整调用代码见第 6 节）。内容淡化不产生任何存档文件；`meta.archive_path` 只由 L0 工具结果守卫写入（指向 `tool_archives/`），深历史信息由第 4 节的 L2 语义快照承接。存档配对清理见 `append_messages`/`prune_historical_tool_calls`/`replace_messages`——后者在 L2 语义压缩或紧急截断整体重写消息列表时，删除"旧列表引用且新列表不再引用"的孤儿存档。

---

### 4. 大模型自动语义压缩策略与对话轮数对齐 (`auto_compress` / `context_messages`)

> **目标**：解决 LLM Token 绝对窗口溢出问题，通过“宏观压缩”在维持长距离记忆连贯性的同时极大降低 Token 话费。

* **具体规则**：
  * **对话轮数感知滑动窗口限制 (Dialogue Turn-Aware Sliding Window)**：
    * 每次组装上下文时，如果未指定具体的消息条数限制，系统会自动依据 `context_messages` (默认 **6** 轮对话) 作为滑动窗口。
    * **智能对齐**：为了避免工具调用等中间过程物理截断导致 context_messages 切碎了上一轮的 User 消息（导致 Agent 在对齐时把整段历史吞掉），加载过程会**从后往前定位**第 `context_messages` 个 `User` 消息。
    * 系统将从该 `User` 消息起点开始，返回**完整且未遭打碎的最近 $N$ 轮历史对话**。
  * **动态语义压缩**：当滑动窗口内的对话历史 Token 预估值超过大模型上下文限制的 **70%** 时，且 `auto_compress` 开启，系统将自动对老旧历史进行 LLM 语义合并。
* **Rust 实现策略**：
  * **Turn 对齐获取 (`plugins/session/chat_session.rs`)**：
    * `ChatSession::get_context_messages` 中先做三层清理（过滤 `Failed` 消息、剔除孤儿节点、content 归一兜底），再以 `context_messages` (可通过参数显式覆盖) 调用 `sliding_window` 实现 User 消息轮次级对齐：

    ```rust
    // 从后往前定位第 context_limit 个 User 消息，以其作为起点裁剪历史
    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }
    if user_indices.len() <= max_turns {
        return messages.to_vec();
    }
    let start_idx = user_indices[user_indices.len() - max_turns];
    messages[start_idx..].to_vec()
    ```

  * **语义合并 (`plugins/session/compression.rs`)**：
    * 每次大模型执行 Turn 之前，`prepare_compression`（配合 `should_start_compression` 的 Token 估计检测）会检测 Token 估计值。如果超标，它会将历史会话（保留最近 30% 活跃明细）打包发送给一个背景 LLM，并使用特殊的 System Prompt 提炼出高密度的 **`<state_snapshot>` XML 状态快照**：

    ```xml
    <state_snapshot>
      <completed_items>已成功重构 Session 核心，并在 symbio 中消灭了硬编码。</completed_items>
      <knowledge_discovered>Model 插件和 Agent 插件解耦，跨插件只能经 VDFS 地址交互。</knowledge_discovered>
      <open_issues>前端 UI 还需适配并提交 tool_context_window 新字段。</open_issues>
    </state_snapshot>
    ```

  * 该快照以一条高密度的 User 消息替代被压缩的老旧历史，完美承接之前的所有意图与已知知识，将上下文开销瞬间降到最低。

---

### 5. 存储期历史工具链物理裁剪策略 (`prune_historical_tool_calls` × `context_messages`)

> **目标**：本地 Session 存储只保留“对话骨架”（User/Assistant 文本），把已沉淀进历史深处的庞大工具执行足迹从**物理存储**中移除，防止会话文件无限膨胀；最近几轮的工具明细完整保留，供加载层与请求视图层继续使用。

* **具体规则**：
  * 当 Session 持久化保存消息时，自动以**最近 `context_messages`（默认 6）个 User 消息**作为分水岭（`keep_turns` 与加载层轮次窗口同源）。
  * 分水岭之前的历史中，`Tool`（工具执行结果）、`ToolCall`（工具意图）与 `Reasoning`（推理过程）消息**直接物理删除**，被删 ToolCall 的请求/响应 Text 子节点一并清除。
  * 分水岭之前不再拥有任何保留子节点、且文本内容为空的 Assistant 容器消息一并拔除。
  * 被删除消息若关联 `.txt` 存档文件，存档文件同步物理删除，杜绝磁盘文件泄露。
  * **只保留完整的 User / Assistant 文本对话**，本地存储长期维持在极简规模。
* **Rust 实现策略**：
  * **物理清理 (`plugins/session/chat_session.rs` 内的 `prune_historical_tool_calls`，保存消息时调用；该函数只被 session 插件消费，定义在 session 插件内)**：

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

### 6. 请求视图层动态剪裁策略 (`build_request_view`：内容淡化 / 工具淡化 / 骨架化 / 水位提醒)

> **目标**：在不改动物理存储的前提下，为每一次大模型 API 请求动态组装"最利于推理"的消息视图。视图**每轮请求都从存储态历史重新重建**，四项剪裁全部**不落库、天然幂等**——存储保持完整历史，`last_saved` 持久化锚点与切片不会错位。

* **具体规则**（唯一入口 `build_request_view`，严格按序四步执行）：
  * **第一步：超大内容节点淡化 (Content Fade)**。B1 保护窗口（最后一条消息 + 最近 `compress_keep_recent` 个内容节点）之外的超大正文/思考节点，按行数阈值 `compress_line_threshold` 或 2048 Token 预算做头尾摘要淡化（详见第 3 节）；每轮无条件执行，视图级不落库。
  * **第二步：老旧工具结果淡化 (Tool Fade)**。单轮请求的工具迭代轮数超过 `fade_activate_rounds`（默认 **40** 轮，会话配置项）后激活；最近 `fade_keep_recent_turns`（默认 **12** 轮，会话配置项）之外的工具执行结果，在 **2048 Token 预算**内做头部/尾部摘要压缩并打上 `tool_result_faded` 标记；**绝不改动 Assistant 消息**，推理链文本始终完整。两个阈值经 `ChatSession` trait 由会话实现提供（持久会话读配置，临时会话读默认），`chat_loop` 不再持有硬编码常量（审计 R3）。
  * **第三步：工具明细骨架化 (Layered Sliding Window)**。全局窗口 `tool_context_window`（默认 **15**，按 ToolCall 个数计数）约束**未声明保留策略**（`All`）的工具；从**当轮**工具列表实时解析每个工具的能力声明（`context_retention`：`LastOnly` / `LastN(n)`，按工具短名——名称最后一个 `/` 之后的部分——匹配），**声明了策略的工具其最近 N 次调用即使滚出全局窗口也完整保留**（LastOnly=最新 1 次），策略保留优先于全局窗口——否则 todo 清单等"只有最新一次有意义"的工具会在长会话中丢失最新状态，引发大模型重复写入。窗口外的调用参数与结果替换为占位文案并**保留语义摘要**（失败结果：错误类型 + 工具名 + 首行原因；成功结果：JSON 列表给规模 + 前若干条目名、文本给首个有内容行，单条摘要上限 64 Token，且摘要收尾不受预算截断；无法提取摘要时退回纯占位文案），占位符另附**取回指引**（`Re-run <tool> to get the full output.`——核对成本由此变成一次可预期的工具调用），**骨架化的 ToolCall 参数保留定位锚点回声**（`path`/`command`/`url`/`pattern` 等关键参数截断回显于占位符，约 10 Token/条——否则"读过某文件第 N 行"这类摘要因缺失文件路径而无法回溯），**只骨架化、不删除，且 ToolCall↔Tool 配对与 parent_id 完整保留**，不会造成大模型逻辑断联。
  * **第四步：Token 水位提醒 (Nudge)**。当上下文 Token 估计值达到模型上下文限制的 **55%**（`CONTEXT_NUDGE_THRESHOLD`）且 `enable_compact_tool` 开启时，向视图末尾追加一条 `meta.kind = "context_nudge"` 的 User 提醒，引导大模型主动调用 `context_compact` 工具；**请求级注入、每个请求最多一次**，不落库、不占用轮次窗口的 User 计数，也不会在前端以用户消息形式出现。
* **Rust 实现策略**：
  * 唯一入口为 `plugins/session/compression.rs` 的 `build_request_view`，由 `plugins/session/chat_loop.rs` 主循环**每轮请求构建前**调用（骨架化实现 `apply_layered_sliding_window` 位于 `plugins/session/context_window.rs`；归属：`context_window.rs` 与 `chat_session.rs` 均为 session 插件私有，core 不依赖它们）：

    ```rust
    pub fn build_request_view(
        messages: &[ChatMessage],
        window: usize,
        retention: &std::collections::HashMap<String, crate::symbio_core::ToolContextRetention>,
        fade_active: bool,
        fade_keep_turns: usize,
        content_keep_recent: usize,
        line_threshold: usize,
        inject_nudge: bool,
    ) -> Vec<ChatMessage> {
        let mut view = messages.to_vec();
        // ① 内容节点淡化：每轮无条件执行（B1 保护窗口之外的超大正文/思考）
        fade_aged_content_nodes(&mut view, content_keep_recent, line_threshold);
        if fade_active {
            // ② 老旧工具结果淡化：FADE_ACTIVATE_ROUNDS 门控
            fade_aged_tool_results(&mut view, fade_keep_turns);
        }
        if window > 0 && !retention.is_empty() {
            // ③ 工具明细骨架化
            view = super::context_window::apply_layered_sliding_window(
                &view,
                window,
                retention,
            );
        }
        if inject_nudge {
            view.push(ChatMessage {
                id: uuid::Uuid::new_v4().to_string(),
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text(CONTEXT_NUDGE_TEXT.to_string())),
                status: Some(MessageStatus::Completed),
                meta: Some(serde_json::json!({ "kind": "context_nudge" })),
                ..Default::default()
            });
        }
        view
    }
    ```

  * 调用点参数（`chat_loop.rs`，每轮请求前实时计算）：`window = req.tool_context_window.unwrap_or(cfg_defaults.tool_context_window)`（`window == 0` = 关闭工具明细骨架化，与配置文档一致）；`retention` 由 `tools.iter().filter_map(...)` 从当轮工具的能力声明动态解析（`rsplit('/')` 取短名、过滤 `All`）；`fade_active = tool_rounds > fade_activate_rounds`、`fade_keep_recent_turns` 均取自会话配置（经 `ChatSession` trait，`chat_loop` 不再持有硬编码常量）；`inject_nudge` 由 55% 水位检测（`should_emit_context_nudge`）与请求级门控共同决定。

---

## 四、 会话记忆 (Session Memory)

会话记忆是**本会话自己钉住的约定与结论**：轮次一多，早期的结论会被压缩、淡化乃至淘汰出上下文；会话记忆的价值恰恰在于**不随上下文水位消失**。

```text
{homedir}/session/<会话 id>/AGENTS.md     记忆本体（与 session.json / messages.json 同目录）
<根>/session/<会话 id>/AGENTS.md                 可编辑地址（模型与用户共用）
```

它与转写（`messages.json`）的分工：转写是**流水**（说过什么），会话记忆是**从这个会话里提炼出来的、不许被压缩掉的那几条**。它不是「对话摘要」（那是压缩快照的活），也不是「跨会话的经验」（那该写进工作区或智能体记忆）。

### 三层记忆是同一件事的三个作用域

| 层 | 所有者插件 | 物理落位 | VDFS 地址 |
|---|---|---|---|
| 工作区 | work | `{workdir}/AGENTS.md` | `<根>/work/AGENTS.md` |
| **会话** | **session** | **`{会话目录}/AGENTS.md`** | **`<根>/session/<id>/AGENTS.md`** |
| 智能体 | agent | `{bundle 目录}/AGENTS.md` | `<根>/agent/<id>/AGENTS.md` |

三层形态完全一样（一个 UTF-8 文本文件 + 两道容量闸门 + 一行头信息的提示词片段 + 一个 VDFS 读写节点），因此**共用一份内核实现**（`symbio_core::memory`）。各插件只提供「个性」：落位、地址、标题、空内容提示、两道闸门开多大。

### 两道闸门（三层统一）

| 闸门 | 位置 | 行为 |
|---|---|---|
| 写入（`memory_max_bytes`） | `MemoryFile::write` | **拒绝**（不截断、不部分写入） |
| 注入（`memory_inject_max_bytes`） | `MemoryFile::inject` | **截断** + 明确告知地址 |

写侧拒绝的理由：截断会让模型「以为写进去了、其实丢了一半」——静默丢内容是最坏的一类失败；拒绝把判断权交回模型（精简后重写）。读侧截断的理由：记忆文件可以比注入预算大，超出部分靠模型按地址 `vdfs_read` 读取。**两者取值不同才有意义**。

### 归属：一个作用域只有一个所有者

**谁能读写它，谁负责注入它。** 这条原则消灭了两类问题：

- **重叠**：`{workdir}/AGENTS.md` 曾被本插件（当「工作区指令」进 `req.system_prompt` 基座，只读、无地址）与 work 插件（当「工作区记忆」进注册段，可读写、有地址）**各注入一次**，同一内容进两次上下文；
- **模糊**：同一个文件被一处叫「指令」、一处叫「记忆」，用户看不出该往哪写。

副作用一并修掉：走 `req.system_prompt` 会**顶掉**模型插件注册的人格（旧优先级链的第一位是显式值）。改为走注册通道后，指令与人格**并列共存**。

**本插件不再读 `{workdir}/AGENTS.md`**——那是 work 的工作区记忆。

### 系统提示词：只有一个注册通道

`CapabilityVisitor::register_system_prompt` 是**唯一**注册入口：注册的每一段都按注册顺序送达模型，**不存在「按优先级链取一个」的竞争**。`resolve_system_prompt` 的结果 = **请求显式段（若有）** + **全部注册段**，全空时才用兜底常量。

「取一个」这个需求本身站不住：人格的**选择**发生在**注册期**（model 插件已按 `PROVIDER_ID` 解析出唯一生效 provider 后才注册），消费侧再按 key 选一次是空动作。

会话侧经这个通道贡献**一段**：

| 注册名 | 内容 | 形态 |
|---|---|---|
| `session-memory` | 本会话的 `AGENTS.md`（**会话记忆**） | 可读写：有地址、两道闸门 |

⚠️ 这里**曾经还有一段** `session-global-instructions`（`{homedir}/AGENTS.md`，只读、无地址、无容量）。
那是一次越界：会话不是那个文件的所有者——它既不给地址、也不限容，模型改不动它，
读一遍注入只是把「智能体自身的指令」这件事临时挂在会话上。现已归 `agent` 插件
（见 `plugins/agent/host/instruction.rs`）：同一份文件现在有地址（`<根>/agent/AGENTS.md`）、
有两道闸门、可编辑，于是它**进了内核**（`symbio_core::memory`）。
判据「内核收『模型能自己改的东西』」因此划的是**形态**，不是名字听起来像不像指令。

### VDFS 侧

会话记忆挂在**会话节点之下**，与 `消息` / `子会话` / `工作目录` 并列——它本来就是会话的一部分，不另开一条寻址：

```text
<根>/session/<id>/AGENTS.md     rw    记忆（文件；写受闸门约束）
```

- `list` 与 `stat` **共用内核产出的同一份形状**，两条链路不会分叉；
- 写入经 `notify_change` 投递变更（路径 `<id>/AGENTS.md`，与列表地址同源）；
- **不可删除**（`delete` 恒 `Forbidden`）：删除即丢失本会话的长期约定，而抹掉之后没有东西能把它找回来。要清空就写入空内容——那是一次可读、可审、可撤销的显式动作。

### 作用域闸门

`ctx[SESSION_ID]` 缺失 / 为空 → **什么都不注入**。收集期拿不到会话 id 的广播（例如设置页的选项收集）不该凭空造一份记忆出来。闸门在 `SessionPlugin::store_with` 一处收口。

---

## 五、 策略对比与总结 (Summary Matrix)

| 策略维度 | 核心控制参数 | 执行时机 | 动作目标 | 底层实现文件 |
| :--- | :--- | :--- | :--- | :--- |
| **存储级轮数裁剪** | `max_messages` | `append_messages` 保存时 | FIFO 截断超出的最老对话轮（0 = 不限制），不动归档文件 | `plugins/session/chat_session.rs` |
| **单轮工具软上限** | `max_tool_rounds` | `run_chat_loop` | 默认 0 = 无上限；显式调低时达到上限提示后正常退出（非熔断），支持续跑 | `plugins/session/chat_loop.rs` |
| **超大内容节点淡化** | `compress_line_threshold`<br>`compress_keep_recent` | `build_request_view` 每轮请求前 | B1 保护窗口（末条 + 最近 3 个内容节点）之外的超大正文/思考按行数（>200）或 token（>2048）头尾摘要淡化；存储恒为完整原文，不落库幂等 | `plugins/session/compression.rs` |
| **加载轮次对齐 + 宏观语义快照合并** | `context_messages` / `auto_compress` | `get_context_messages` 加载时 / `prepare_compression` 请求前 | 三层清理后按最近 6 个 User 消息对齐截取完整轮次；70% Token 溢出时用 XML 状态快照合并（保留最近 30%） | `plugins/session/chat_session.rs`<br>`plugins/session/compression.rs` |
| **存储期历史工具链物理裁剪** | `context_messages`（分水岭）<br>`prune_tool_history`（开关） | `append_messages` 保存时 | 物理删除分水岭之前（默认最近 `context_messages` 轮内保留）的 Tool / ToolCall / Reasoning 及其子节点；**只删节点不删归档文件**（归档由 L0 `tool_result_guard` 滚动回收）；内存临时会话跳过此裁剪 | `plugins/session/chat_session.rs`（`prune_historical_tool_calls`） |
| **请求视图层动态剪裁** | `tool_context_window` + fade/nudge | `build_request_view` 每轮请求前 | ① 内容节点淡化（每轮无条件）→ ② >40 轮激活老旧工具结果淡化（保留最近 12 轮）→ ③ 窗口（15）外工具明细骨架化为带语义摘要 + 取回指引的占位符（配对保留，声明 `context_retention` 的工具最新 N 次豁免窗口）→ ④ 55% 水位提醒；全部不落库幂等 | `plugins/session/compression.rs`<br>`plugins/session/context_window.rs` |

通过这套精心设计的**六维协同策略**，Symbio 构建了"存储层（写入时 L0 工具守卫 + 保存时 `max_messages` FIFO / `prune_historical_tool_calls` 生命周期裁剪 + L2 语义快照落库，内容节点恒为完整原文）→ 加载层（`context_messages` 轮次窗口对齐）→ 请求视图层（`build_request_view` 内容淡化/工具淡化/骨架化/水位提醒，不落库幂等）→ 语义压缩层（`auto_compress` 语义合并）"四层递进的上下文治理链路（"何时落库"判据见第 3 节决策表），实现了高保真度的会话还原、高度清爽的本地数据持久化，并在大模型面前维持了极低 Token 开销与绝对安全的行为控制屏障。
