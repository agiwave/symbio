# Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书)

Session 插件是 Symbio 架构中的**会话持久化与编排中心**。Phase E-② 重构后，它是**唯一的会话编排入口**：加载历史、组装系统提示词、经 CapabilityManager 汇集工具、解析 Model Provider 并在进程内驱动会话循环；Model 插件退居**无状态 LLM 网关**（provider 注册表 + 协议适配），对 session 零依赖。

本文档将系统性地阐述 Symbio 的会话保存、内容压缩、工具迭代限制以及发送过滤策略，说明其具体规则、参数配置及 Rust 底层实现策略。

---

## 一、 系统架构数据流图 (Architecture & Data Flow)

下列架构图展示了一个完整的“用户请求 -> Session (编排入口) -> 进程内会话循环 (视图组装/推理/工具) -> 数据库持久化”的生命周期：

```mermaid
flowchart TD
    User([1. 用户发起 Chat 请求]) --> SessionChat[Session 插件 - chat 路由]
    SessionChat --> SaveUser[2. 保存/追加用户消息]
    SaveUser --> LoadHistory[3. 加载历史上下文<br>get_context_messages 轮次窗口对齐]
    LoadHistory --> BuildPrompt[4. 构建系统提示词<br>prompt.rs 人格/心智流形注入]

    subgraph Session 会话编排层（唯一编排入口）
        BuildPrompt --> AggTools[5. 工具汇集<br>CAPABILITY_MANAGER 注册表]
        AggTools --> Resolve[6. Provider entry 解析<br>精确 id → is_default → 首个注册]
        Resolve --> Spawn[7. 进程内 spawn 会话循环<br>RATE_LIMITER 限流跟随请求方]
        Spawn --> BuildView[8. 构建请求视图<br>build_request_view<br>内容淡化/工具淡化/骨架化/水位提醒]
        BuildView --> LLMCall[9. protocol 推理调用<br>直连无状态 LLM 网关]
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

会话与压缩系统的全部表现都由全局会话配置参数控制。以下是 `symbio.toml` (或动态 SessionConfig) 中关于会话与记忆的完整配置示例：

```yaml
session:
  # 存储目录：固定为 <homedir>/plugins/session/，从 HomedirRegistry 派生，
  # 跟随系统目录 (homedir) 切换，不再作为配置项。
  store_kind: file                         # 存储后端类型: file (文件目录) 或 sqlite (SQLite 数据库)
  # 注：会话默认不绑定智能体（纯工具模式）。智能体选择属于前端会话级偏好，
  # 由 session.metadata.agent_id 按会话记录，后端不提供 default_agent。
  
  # 1. 存储级策略
  max_messages: 100              # 单会话本地保存的最大对话轮数限制 (以 User 消息计数；代码内强制 .max(500) 兜底，即实际生效下限为 500)
  
  # 2. 单轮迭代策略
  max_tool_rounds: 65535         # 单轮对话中工具迭代轮数上限 (65535 = 实质无上限；仅当显式调低时作为软上限，达到后提示并退出，不静默熔断)
  
  # 3. 微观内容截断策略
  compress_line_threshold: 200   # 单条内容节点（正文/思考）的行数阈值，超过该行数（或 token 超 2048）在请求视图中淡化（存储恒为完整原文）
  compress_keep_recent: 3        # 内容节点淡化时豁免的最近内容节点数量（B1 保护窗口；末条消息恒受保护）
  
  # 4. 宏观语义压缩策略
  auto_compress: true            # 是否启用基于 Token 溢出 (70% 触发) 的大模型自我语义合并
  context_messages: 6            # 发送给 AI 的对话上下文滑动窗口轮数 (以 User 消息轮次对齐截取；也是存储期 prune 的保留分水岭)
  
  # 5. 工具滑动窗口策略
  tool_context_window: 15        # LLM 推理上下文中保留完整明细的最近工具调用数量 (超出者骨架化为带一行摘要的占位符；声明 context_retention 保留策略的工具其最新调用豁免此窗口)
  
  # 6. 水位提醒与主动压缩
  enable_compact_tool: false     # 是否启用 55% Token 水位提醒 (nudge) 注入与 context_compact 主动压缩工具 (默认关闭，须手动开启；关闭后仅保留 70% 自动压缩兜底)
```

---

## 三、 六大核心策略的具体规则与 Rust 实现机制 (Implementation Strategies)

### 1. 单会话对话轮数上限存储策略 (`max_messages`)

> **目标**：防止单个会话历史由于对话轮数过多而无限膨胀，导致磁盘溢出或加载序列化变慢。

* **具体规则**：
  * 对每个 Active Session 物理存储的历史记录实施**最大保存对话轮数限制**。
  * `max_messages` 表示最大保存的对话轮数，每一轮以一个 `User` 消息起始，默认及安全下限为 `500` 轮。
  * 当存储的对话轮数超出阈值时，自动从会话开头执行 FIFO 裁剪，移除最老的多余对话轮次，且在裁剪时会自动检查并物理清理对应的本地工具结果存档文件，防止磁盘文件泄露。
  * **智能对齐**：此策略确保物理保存下来的会话，其起始消息也总是以一个完整的 `User` 消息起始，从而绝对避免了历史反序列化对齐失败的问题。
* **Rust 实现策略**：
  * 在 `SessionPlugin::invoke_append` 中，扫描 `User` 消息索引并执行轮数对齐物理截断，确保物理存储也总是以 `User` 消息为开端：

    ```rust
    let max_turns = self.config.read().await.max_messages.max(500);
    let mut user_indices = Vec::new();
    for (idx, msg) in session.messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }
    if user_indices.len() > max_turns {
        let start_idx = user_indices[user_indices.len() - max_turns];
        // 异步检查并物理删除 start_idx 之前老旧存档文件以防止磁盘泄露，然后执行截断：
        session.messages.drain(0..start_idx);
    }
    ```

---

### 2. 单轮会话工具调用迭代上限策略 (`max_tool_rounds`)

> **目标**：为自主 Agent 的多步决策提供可选的"软安全网"，防止极端场景下大模型失误、死锁或命令失败导致的"天价账单死循环 (Runaway Loop)"。默认**不设上限**，把控制权交给模型自主决策。

* **具体规则**：
  * 在用户提交单次 Prompt 后，系统在后台开启一个自主决策（Loop）链。
  * 大模型在一个循环中可以做 `推理 -> 工具调用 -> 工具结果返回 -> 再推理 -> 再工具调用` 的迭代。
  * **默认无上限**：`max_tool_rounds` 默认 `65535`，同时请求层 `max_tool_rounds: None` 也表示无上限，正常对话不会被打断。
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
  * Session 编排层（`orchestrator.rs`）默认将配置值透传为 `Some(session_cfg.max_tool_rounds)`，因此配置默认 `65535` 即为"实质无上限"。

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
  * 在 `plugins/session/compression.rs` 的 `fade_aged_content_nodes` 中实现，由 `build_request_view` 作为第一步调用：

    ```rust
    let mut view = messages.to_vec();
    // ① 内容节点淡化：B1 保护窗口之外的超大节点做视图级淡化（不落盘、幂等）
    fade_aged_content_nodes(&mut view, content_keep_recent, line_threshold);
    ```

  * 旧机制（写入时物理脱水 + `.txt` 存档 + `meta.archive_path` 骨架落库）已废除：不再为每条大消息生成存档文件，深历史信息由第 4 节的 L2 语义快照承接；`meta.archive_path` 现仅由 L0 工具结果守卫写入（指向 `tool_archives/`，配对清理见 `append_messages`/`prune_historical_tool_calls`/`replace_messages`——后者在 L2 语义压缩或紧急截断整体重写消息列表时，删除"旧列表引用且新列表不再引用"的孤儿存档）。

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
      <knowledge_discovered>Model 插件和 Agent 插件解耦，跨插件必须路由 session/config/get。</knowledge_discovered>
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
  * **物理清理 (`plugins/session/chat_session.rs` 内的 `prune_historical_tool_calls`，保存消息时调用；体检备注 audit-5：原独立文件 `context.rs` 因唯一消费者就是 chat_session，已并入)**：

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
  * **第二步：老旧工具结果淡化 (Tool Fade)**。单轮请求的工具迭代轮数超过 `FADE_ACTIVATE_ROUNDS`（**40** 轮）后激活；最近 `FADE_KEEP_RECENT_TURNS`（**12**）轮之外的工具执行结果，在 **2048 Token 预算**内做头部/尾部摘要压缩并打上 `tool_result_faded` 标记；**绝不改动 Assistant 消息**，推理链文本始终完整。
  * **第三步：工具明细骨架化 (Layered Sliding Window)**。全局窗口 `tool_context_window`（默认 **15**，按 ToolCall 个数计数）约束**未声明保留策略**（`All`）的工具；从**当轮**工具列表实时解析每个工具的能力声明（`context_retention`：`LastOnly` / `LastN(n)`，按工具短名——名称最后一个 `/` 之后的部分——匹配），**声明了策略的工具其最近 N 次调用即使滚出全局窗口也完整保留**（LastOnly=最新 1 次），策略保留优先于全局窗口——否则 todo 清单等"只有最新一次有意义"的工具会在长会话中丢失最新状态，引发大模型重复写入。窗口外的调用参数与结果替换为占位文案并**保留一行摘要**（失败结果：错误类型 + 工具名 + 首行原因；成功结果：首个非空行摘要，单条摘要上限 48 Token；无法提取摘要时退回纯占位文案），**骨架化的 ToolCall 参数保留定位锚点回声**（`path`/`command`/`url`/`pattern` 等关键参数截断回显于占位符，约 10 Token/条——否则"读过某文件第 N 行"这类摘要因缺失文件路径而无法回溯），**只骨架化、不删除，且 ToolCall↔Tool 配对与 parent_id 完整保留**，不会造成大模型逻辑断联。
  * **第四步：Token 水位提醒 (Nudge)**。当上下文 Token 估计值达到模型上下文限制的 **55%**（`CONTEXT_NUDGE_THRESHOLD`）且 `enable_compact_tool` 开启时，向视图末尾追加一条 `meta.kind = "context_nudge"` 的 User 提醒，引导大模型主动调用 `context_compact` 工具；**请求级注入、每个请求最多一次**，不落库、不占用轮次窗口的 User 计数，也不会在前端以用户消息形式出现。
* **Rust 实现策略**：
  * 唯一入口为 `plugins/session/compression.rs` 的 `build_request_view`，由 `plugins/session/chat_loop.rs` 主循环**每轮请求构建前**调用（骨架化实现 `apply_layered_sliding_window` 位于 `plugins/session/context_window.rs`——体检备注 audit-5：Phase C 曾归 `symbio_core`，E-② 后仅本插件消费，随 Phase sink 下沉回 session；原 `plugins/session/context.rs` 已删除，其 `prune_historical_tool_calls` 并入 `chat_session.rs`）：

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

  * 调用点参数（`chat_loop.rs`，每轮请求前实时计算）：`window = req.tool_context_window.unwrap_or(15)`；`retention` 由 `tools.iter().filter_map(...)` 从当轮工具的能力声明动态解析（`rsplit('/')` 取短名、过滤 `All`）；`fade_active = tool_rounds > FADE_ACTIVATE_ROUNDS`；`fade_keep_turns = FADE_KEEP_RECENT_TURNS`；`inject_nudge` 由 55% 水位检测（`should_emit_context_nudge`）与请求级门控共同决定。

---

## 四、 策略对比与总结 (Summary Matrix)

| 策略维度 | 核心控制参数 | 执行时机 | 动作目标 | 底层实现文件 |
| :--- | :--- | :--- | :--- | :--- |
| **存储级轮数裁剪** | `max_messages` | `invoke_append` 保存时 | FIFO 截断超出的最老对话轮（代码强制下限 500），同步清理关联存档 | `plugins/session/chat_session.rs` |
| **单轮工具软上限** | `max_tool_rounds` | `run_chat_loop` | 默认 65535 无上限；显式调低时达到上限提示后正常退出（非熔断），支持续跑 | `plugins/session/chat_loop.rs` |
| **超大内容节点淡化** | `compress_line_threshold`<br>`compress_keep_recent` | `build_request_view` 每轮请求前 | B1 保护窗口（末条 + 最近 3 个内容节点）之外的超大正文/思考按行数（>200）或 token（>2048）头尾摘要淡化；存储恒为完整原文，不落库幂等 | `plugins/session/compression.rs` |
| **加载轮次对齐 + 宏观语义快照合并** | `context_messages` / `auto_compress` | `get_context_messages` 加载时 / `prepare_compression` 请求前 | 三层清理后按最近 6 个 User 消息对齐截取完整轮次；70% Token 溢出时用 XML 状态快照合并（保留最近 30%） | `plugins/session/chat_session.rs`<br>`plugins/session/compression.rs` |
| **存储期历史工具链物理裁剪** | `context_messages`（分水岭） | `invoke_append` 保存时 | 物理删除最近 6 轮分水岭之前的 Tool / ToolCall / Reasoning 及其子节点与存档 | `plugins/session/chat_session.rs`（`prune_historical_tool_calls`） |
| **请求视图层动态剪裁** | `tool_context_window` + fade/nudge | `build_request_view` 每轮请求前 | ① 内容节点淡化（每轮无条件）→ ② >40 轮激活老旧工具结果淡化（保留最近 12 轮）→ ③ 窗口（15）外工具明细骨架化为带一行摘要的占位符（配对保留，声明 `context_retention` 的工具最新 N 次豁免窗口）→ ④ 55% 水位提醒；全部不落库幂等 | `plugins/session/compression.rs`<br>`plugins/session/context_window.rs` |

通过这套精心设计的**六维协同策略**，Symbio 构建了"存储层（写入时 L0 工具守卫 + 保存时 `max_messages` FIFO / `prune_historical_tool_calls` 生命周期裁剪 + L2 语义快照落库，内容节点恒为完整原文）→ 加载层（`context_messages` 轮次窗口对齐）→ 请求视图层（`build_request_view` 内容淡化/工具淡化/骨架化/水位提醒，不落库幂等）→ 语义压缩层（`auto_compress` 语义合并）"四层递进的上下文治理链路（"何时落库"判据见第 3 节决策表），实现了高保真度的会话还原、高度清爽的本地数据持久化，并在大模型面前维持了极低 Token 开销与绝对安全的行为控制屏障。
