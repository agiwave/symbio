# Model / Session 插件分工改造设计（文档驱动）

> 状态：Phase A/B/C/D 已完成（cargo check 零警告 + 全量测试通过；model 中 `plugins::session`/`SESSION_OPEN`/`SESSION_COMPRESS` 引用零命中），Phase E 待实施
> 基线：2026-09-08 代码实测（本文件的现状描述均来自当日源码核验，非推测）
> 约束（用户原话）：**1. 务必不要破坏当前架构；2. 项目要以文档驱动开发。**
> 验证基线：`cargo check -p symbio` 零警告 + `cargo test -p symbio --lib` 模块过滤全绿（一律从 `d:\Bing\symbio\symbio` 运行）。

---

## 1. 目标（用户需求）

1. **Model/Session 插件分工重构**：
   - `Session` = 会话引擎：会话循环（loop）、工具编排、持久化锚点、上下文策略归一、压缩、resume —— **有状态**。
   - `Model` = 无状态 LLM 网关：provider 注册表、协议适配、SSE → 标准化事件流 —— **无状态**。
   - 依赖方向：`model → session` **零依赖**；`session → model` 单向依赖。
2. **symbio_core 定义核心 `ModelProvider` 接口**：提供不同协议的大语言模型请求和解析封装（类似现有 `ModelProtocol`），使协议实现的核心契约上移到 core。
3. **扩充 `CapabilityManager`**：除注册 `Capability` 外，还可注册 `ModelProvider` 与系统提示词 —— AI 对话能力纳入与工具完全相同的"统一注册收集机制"；会话时通过现有 traverse 机制**一次性**收集工具 + 模型服务 + 系统提示词。

## 2. 现状核实（含对分析稿的三处修正）

单 crate（`symbio`）多模块结构；reqwest/tokio 等均为顶层依赖，core 与插件共享。跨插件"依赖"是**模块路径依赖**（`use crate::plugins::session::…`），非 crate 依赖。

### 2.1 与此前分析稿不符的三处（以实测为准）

| 分析稿说法 | 实测事实 |
|---|---|
| `protocol.rs` 定义 ModelProtocol | [protocol.rs](../src/plugins/model/protocols/protocol.rs) 只是 5 行重导出 shim；trait 真身在 [protocols/mod.rs](../src/plugins/model/protocols/mod.rs) |
| persist_messages 在 message_builder.rs | 实际在 [chat_loop.rs](../src/plugins/model/chat_loop.rs)（last_saved 锚点共 7 个调用点） |
| 入口 `handle_chat_message` / `LoopContext` | 实际入口是 session 侧 `handle_chat_send_oneoff`（orchestrator.rs）；循环上下文是 `SessionContext` + `TurnOutput` |

### 2.2 model → session 依赖清单（全部）

1. **1 处显式 use**：[compression.rs](../src/plugins/model/compression.rs) 调 `crate::plugins::session::context::apply_layered_sliding_window`（纯函数）。
2. **2 条路径路由**：
   - `SESSION_OPEN`：[chat_loop.rs `open_chat_session`](../src/plugins/model/chat_loop.rs) fork ctx 路由到 session 取 `ChatSessionHandle`；
   - `SESSION_COMPRESS`：model 侧路由 session 执行压缩。
3. session → model：零编译依赖（仅 `MODEL_CHAT = "model/chat"` 路径常量，常量本身在 core）。

### 2.3 已在 core 的关键事实（降低迁移成本）

- `PluginChannel`（transport.rs）、`ChatSession`/`ChatSessionHandle`（chat_session.rs）、`ModelConfig`（schemas/model/model_config.rs）、`ModelProviderConfig`/`ModelProvidersConfig`（schemas/model/model_providers.rs）、`CapabilityMeta`/`ToolContextRetention`（capability.rs）、`MODEL_PROTOCOL_*` 常量。
- `submit_object_creator!(id, ctor, dyn Trait)` 按 `TypeId::of::<dyn Trait>()` 注册（creator.rs:124），注册点与查询点同步换 trait 名即可，无隐藏耦合。
- `DefaultToolManager` 是 `CapabilityManager` 的**唯一**实现（tools.rs:40）——扩充 trait 方法零破坏。
- `ModelPlugin::traverse` 当前是空壳（plugin.rs:824-831，注释"Model 插件目前不直接暴露工具"）——Phase B 的天然挂载点。
- traverse 注册样板（local/plugin.rs:295-316）：`ctx.get(PATH)` 判 `TRAVERSE_AVAILABLE_TOOLS` → `ctx.get(CAPABILITY_MANAGER)` → 逐个 register。

## 3. 目标架构与边界契约

```
symbio_core（契约层）
├── model_provider.rs        [新] trait ModelProvider + ProtocolEvent/FinishReason/Usage
│                                 + resolve_protocol_id + spawn_orchestrator
├── capability.rs            [扩] CapabilityManager 增 provider/系统提示词注册收集
├── chat_pipeline.rs         [扩] collect_capabilities 语义 = 工具+模型服务+系统提示词
├── context_window.rs        [新] apply_layered_sliding_window（纯函数，Phase C 迁入）
└── keys.rs                  [扩] SESSION_HANDLE 键（Phase D）

plugins/model（无状态 LLM 网关，最终形态）
├── plugin.rs                provider 注册表路由 + chat 单轮网关入口
├── protocols/*              四协议实现（实现 core::ModelProvider）
├── context.rs               HTTP/SSE → TurnOutput（单请求维度）
└── message_builder.rs / turn_processor.rs

plugins/session（有状态会话引擎，最终形态）
├── chat_loop.rs             [迁入 Phase E] 主循环 + last_saved 锚点 + persist
├── compression.rs           [迁入 Phase E] 压缩编排
├── context.rs               上下文策略归一（窗口/水位/骨架化）
└── orchestrator.rs          会话编排 / resume / 审批流
```

**边界契约三条**：
1. **session 是持久化唯一写入方**：`last_saved` 锚点对账、`append_messages`/`replace_messages` 调用全部归 session 侧；model 只经 `ChatSessionHandle` 做增量 append（Phase D 起 handle 由 ctx 交付）。
2. **schema 与纯函数定 core**：跨插件共享的请求/响应 schema、上下文策略纯函数（滑窗/骨架化）一律放 core，插件互不 `use` 对方模块路径。
3. **model 产出标准化事件流**：`ModelProvider::handle_chat_stream` 输入 `ModelConfig + system_prompt + messages + tools`，输出 `ProtocolEvent` 流；对会话状态机（审批/压缩/锚点）零认知。

## 4. Phase A：symbio_core 定义核心 `ModelProvider` 接口

**内容**：将 [protocols/mod.rs](../src/plugins/model/protocols/mod.rs) 中的 `trait ModelProtocol` + `FinishReason` + `Usage` + `ProtocolEvent` + `resolve_protocol_id` + `spawn_orchestrator` **整体迁入** 新模块 `symbio_core/model_provider.rs`，trait 更名 **`ModelProvider`**（用户指定名称）。方法签名逐字保持：

```rust
#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn get_api_url(&self, config: &ModelConfig) -> String;
    fn get_headers(&self, config: &ModelConfig) -> HeaderMap;
    fn prepare_request(&self, config: &ModelConfig, system_prompt: &str,
        messages: &[ChatMessage], tools: &[CapabilityMeta]) -> Value;
    fn get_validation_input(&self) -> Value { serde_json::json!({ "ping": true }) }
    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent>;
    async fn handle_chat_stream(&self, config: &ModelConfig,
        parent: &Option<Arc<dyn Plugin>>, ctx: Arc<dyn InvokeRequest>)
        -> InvokeResponse<PluginPayload>;
}
```

**兼容策略**：
- `plugins/model/protocols/mod.rs` 降级为**re-export shim**（保留既有模块路径可用）：`pub use crate::symbio_core::model_provider::*;`
- **`spawn_orchestrator` 不迁**：它调用 `super::context::ChatOrchestrator` 与 `super::chat_loop::run_chat_loop`（model 插件内部构件，Phase E 随 loop 迁往 session），core 不得反向依赖插件。留驻 `protocols/mod.rs` shim 中，import 改指 core 类型。
- 四个协议实现文件仅改 `impl ModelProtocol for` → `impl ModelProvider for` 与 import 行；`submit_object_creator!(…, dyn ModelProvider)` 与 `create_object::<dyn ModelProvider>`（plugin.rs 两处）同步更名——TypeId 注册/查询三点一致。
- 无新增依赖：reqwest `HeaderMap`、`Plugin`/`PluginChannel`/`InvokeRequest`/`PluginPayload` 均已在 core 可达。

**验证**：cargo check 零警告；model 模块测试全绿（协议 SSE 解析测试不改一字）。

## 5. Phase B：扩充 CapabilityManager（ModelProvider + 系统提示词统一收集）

**core 侧**（capability.rs + tools.rs）：

```rust
/// 模型服务注册条目（AI 对话能力 = 与工具同机制的"能力"）
pub struct ModelProviderEntry {
    pub provider_id: String,      // 与 ModelProviderConfig.id 一致
    pub protocol_id: String,      // MODEL_PROTOCOL_* 常量值
    pub description: String,
    pub system_prompt: Option<String>,   // provider 级默认系统提示词
    pub provider: Arc<dyn ModelProvider>,
}

pub trait CapabilityManager: … {
    // …既有 5 方法不动…
    async fn register_model_provider(&self, entry: ModelProviderEntry);
    async fn list_model_providers(&self) -> Vec<ModelProviderEntry>;   // 收集目录
    async fn get_model_provider(&self, provider_id: &str) -> Option<Arc<dyn ModelProvider>>;
    async fn register_system_prompt(&self, name: &str, prompt: String);
    async fn list_system_prompts(&self) -> Vec<(String, String)>;      // (name, prompt) 保序
}
```

- `DefaultToolManager` 增两组并行存储：`providers: RwLock<IndexMap<String, ModelProviderEntry>>`、`system_prompts: RwLock<IndexMap<String, String>>`（保留注册顺序）。
- `collect_capabilities` 机制**零改动**（同一 `traverse(TRAVERSE_AVAILABLE_TOOLS)` 广播、同一 `CAPABILITY_MANAGER` 键、同一错误桶）；仅头注释语义扩充为"工具 + 模型服务 + 系统提示词"三合一收集。

**model 插件侧（贡献者）**：`ModelPlugin::traverse` 空壳填实——命中 `TRAVERSE_AVAILABLE_TOOLS` 时，遍历 `self.providers` 配置，对每个 provider 经工厂 `create_object::<dyn ModelProvider>(protocol_id)` 实例化并 `register_model_provider`；provider 配置的 `system_prompt` 一并注册；工厂失败按软故障记日志（不进致命错误桶）。

**消费侧（优先取收集结果，缺失回退现状）**：
1. **provider 解析**（plugin.rs `handle_chat_session_internal`）：先 `ctx.get(CAPABILITY_MANAGER) → get_model_provider(provider_id)`；未命中回退现有 `ModelProvidersConfig::resolve()` + `create_object` 路径——**保证任何场景行为不回退**。
2. **系统提示词**（chat_loop.rs 既有链路 `request.system_prompt` > provider config `system_prompt` 之上追加第三级）：两者皆空时取收集到的 `"default"` 系统提示词。既有两级优先级一字不动。

**验证**：新增 tools.rs 单测（provider/prompt 注册往返 + 保序）；model/session 模块测试全绿。

## 6. Phase C：消除 model→session 显式依赖

`apply_layered_sliding_window`（原 [session/context.rs:99](../src/plugins/session/context.rs)）是纯函数，依赖 `ChatMessage`/`MessageType`/`ToolContextRetention` 全在 core → **已迁入 [symbio_core/context_window.rs](../src/symbio_core/context_window.rs)**（5 个滑窗单测随函数迁入；session 内已无消费者，故不保留转发重导出，维持零警告基线）；model/compression.rs 已改调 `crate::symbio_core::apply_layered_sliding_window`，session/context.rs 仅存 `prune_historical_tool_calls`。（Phase sink 后该函数再次下沉至 `plugins/session/context_window.rs`——E-② 后唯一消费者回到 session，见 10.1）

## 7. Phase D：消除 model→session 路由依赖

1. **SESSION_OPEN → ctx 交付**：keys.rs 新增 `SESSION_HANDLE: SymbioKey<Value = Arc<ChatSessionHandle>>`。session 编排器发起 `model/chat` 前把会话句柄 set 进 ctx；model `open_chat_session` 改为**优先读 ctx**，缺失时回退 legacy 路由（过渡期双轨）；验证后移除 session 侧 SESSION_OPEN 分支。
2. **SESSION_COMPRESS → 纯函数下沉**：压缩变换（骨架化/裁剪）下沉 core 纯函数；model 直接调用 + 经 `SessionContext.session`（ChatSessionHandle）自行 `replace_messages` 持久化；session 侧仅保留存储写入实现。
3. **完成判据**：`grep -rn "plugins::session\|SESSION_OPEN\|SESSION_COMPRESS" src/plugins/model/` **零命中**；session → model 仅剩 `MODEL_CHAT` 常量（core）。

## 8. Phase E：会话引擎迁移（接口先行）

> 接口设计基于 2026-09-08 源码核验：`MODEL_CHAT` 全库路由点唯一（session/orchestrator.rs），agent 插件不路由 model/chat → 迁移可原子切换，无需双轨。

### 8.1 目标拓扑

```text
现状：session run_chat_loop_task ──route model/chat──▶ model run_chat_loop（多轮循环+SessionContext）
目标：session run_chat_loop（搬迁版多轮循环）──每轮进程内直调──▶ provider.execute_turn（单轮网关）
```

- **model 单轮网关** = 现turn_processor.send_request 逻辑收编为 core `ModelProvider::execute_turn`：一请求（config+system+messages+tools）→ SSE 执行 + `StreamEvent` 帧实时下发 + 返回单轮产物 `TurnOutput`。对轮次循环/工具编排/持久化锚点零认知。
- **session 会话引擎** = 搬迁的 run_chat_loop：turn 循环、工具执行、resume、压缩、last_saved 锚点、审批/中止状态机。会话句柄经 `open_session_handle` 本地直取（不再绕 ctx）。
- 事件形态不变：帧仍是 core `session_chat_response::StreamEvent`，前端零改动。

### 8.2 接口签名（core，E-① 落点）

```rust
// symbio_core/model_provider.rs —— ModelProvider 增设单轮网关方法
// （统一实现，四个协议无需各自重复：内部 = prepare_request + HTTP/SSE 执行机器 +
//   parse_response_line → StreamEvent 帧下发 + 事件累积）
async fn execute_turn(
    &self,
    config: &ModelConfig,
    system_prompt: &str,
    messages: &[ChatMessage],
    tools: &[CapabilityMeta],
    channel: &mut PluginChannel,      // 复用现有帧协议（StreamEvent JSON）
    abort_flag: &Arc<AtomicBool>,
) -> Result<TurnOutput, PluginError>;
```

```rust
// symbio_core —— 单轮产物（自 model/context.rs 迁入；字段逐字保持）
pub struct TurnOutput { text, reasoning, response_id, tool_accumulator,
                        response_text_child_id, reasoning_child_id, finish, usage }
pub struct ToolCallAccumulator { /* 自 model/tool_call.rs 迁入 */ }
```

```rust
// symbio_core/capability.rs —— ModelProviderEntry 增配置字段
// （collect_capabilities 每次会话重新 traverse 注册 → config 恒新鲜；
//   session 经 get_model_provider 一次取齐 provider 实例 + 配置，无需路由 model）
pub struct ModelProviderEntry { ..., pub config: ModelConfig }
```

- 配套迁 core：`model/protocol.rs` 的 SSE 执行机器（`execute_post_with_abort`/`parse_sse_stream`/`PostResult`，仅依赖 reqwest+core 类型）；TurnProcessor 内 ProtocolEvent→StreamEvent 的转换/累积逻辑（收编进 execute_turn 统一实现）。
- `Abort`/`Approval` 等控制帧协议（`{"type":"abort"}`）保持不变——session 循环消费。

### 8.3 模块迁移清单

| 去向 | 模块 | 说明 |
|---|---|---|
| → session | chat_loop、tool_executor、resume、compression、tool_call、tool_result_guard、turn 循环逻辑 | 会话状态机整体搬家；`super::` 内部互引保持 |
| → core | TurnOutput、ToolCallAccumulator、SSE 执行机器、execute_turn、消息构造家族（short_id/StreamChildIds/build_assistant_messages/build_tool_message——TurnOutput::into_messages 与 ToolCallAccumulator 的依赖闭包，孤儿规则要求随迁） | 单轮网关契约，插件互不 use 对方模块路径 |
| model 保留 | protocols/*（四协议实现）、types、message_builder（协议 flatten 用）、handlers、config 路由 | 无状态网关剩余物 |
| model 删除 | spawn_orchestrator、run_chat_loop、ChatOrchestrator、model/chat Session 通道路由 | grep 已证无第三方调用方 |

### 8.4 批次划分

- **E-① core 基建（纯增量，零行为变化）**：TurnOutput/ToolCallAccumulator/SSE 机器迁 core；ModelProvider::execute_turn 统一实现（model 侧 turn_processor 改调 core 版，行为等价）；ModelProviderEntry 加 config。cargo check 零警告 + 测试全绿即收。
- **E-② 原子切换**：session 搬入循环族模块（run_chat_loop_task 扩为完整会话引擎）；model 删 loop 族 + chat 路由收敛；同批完成否则编译不过（互引）。grep 判据：`grep -rn "run_chat_loop\|spawn_orchestrator" src/plugins/model/` 零命中。
- **E-③ 文档/注释清理**：model_chat.rs、chat_message.rs、chat_session.rs 头注释、session/README.md 中"模型插件 run_chat_loop"表述同步改写。

### 8.4.1 E-② 设计定型（2026-09-08 探查收官后固化）

**① trait 契约变更（core/model_provider.rs）**
- 删除 `handle_chat_stream`（四协议覆写全部是 `spawn_orchestrator` 一行委托；loop 族迁 session 后失义）与 `get_validation_input`（唯一消费者 validate_config）。
- 新增 `async fn ping(&self, config: &ModelConfig) -> Result<(), PluginError>`：三协议既有 `handle_ping`（openai_chat.rs:188 / gemini_api.rs:240 / anthropic_messages.rs:420）接入此方法；**openai_responses 补齐 ping 实现**——修复现状 validate_config 验证路径不对称（openai_responses 覆写无 ping 分支，`{"ping": true}` 会误入 run_chat_loop）。
- tools.rs MockProvider（测试桩）同步：删 handle_chat_stream 桩、增 ping 桩。

**② validate_config 改造（model/plugin.rs）**
- `create_object` → `protocol.ping(config)`：Ok → None（验证通过）；Err(e) → Some(e.to_string())。不再经 Session 帧循环判 idle/Error——语义更直接，行为等价（handle_ping 失败即返回 InternalError）。

**③ 限流归属（现状：rate_limiter.wait 仅在会话入口调一次，多轮循环不再限流）**
- `ProviderRateLimiter`（model/plugin.rs:33-87，纯结构零依赖）下沉 core（rate_limit.rs）+ 全局静态 `RATE_LIMITER`（语义等价：ModelPlugin 本就是进程内单例）。
- `ModelProviderEntry` 增 `rate_limit_ms: u64`（traverse 注册时从 model 本地 `ModelProviderConfig.rate_limit_ms` 携带；core `ModelConfig` 不加字段）。
- `CapabilityManager::get_model_provider` 改返回 `Option<ModelProviderEntry>`（session 一次取齐 provider 实例 + config + rate_limit_ms；原 `Option<Arc<dyn ModelProvider>>` 返回形态在删除 chat 路由后无消费者，直接改签名）。
- session 引擎入口调 `RATE_LIMITER.wait(&entry.provider_id, entry.rate_limit_ms)`——会话级节流语义逐字节保持。

**④ session 对接（session/orchestrator.rs run_chat_loop_task）**
- 保留 pair(4096) + tokio::spawn + Error 帧包装 + 1800s 消费循环**零改动**（前端帧时序不变）；仅把 `parent.route(chat_ctx)` 替换为进程内 `tokio::spawn(chat_loop::run_chat_loop(&orchestrator, ctx, plugin_chan))`。
- spawn 外层 match 保留 Error 帧包装语义（原 spawn_orchestrator 的 `Ok(Err(e))→Error 帧 + code` / `panic→INTERNAL_ERROR`）；host_tx_keepalive 技巧随跨插件通道消失而不再需要（进程内直调，host 端 tx 由编排器持有）。
- provider 解析：`ctx.get(CAPABILITY_MANAGER) → get_model_provider(provider_id)`（resolve_session_params 已设 PROVIDER_ID）；未命中 → Error 帧"未找到可用的 Model Provider，请先在设置中配置并启用至少一个"（与现状 ValidationError 文案一致）。
- `ChatOrchestrator::new(config, parent, protocol)` 由 session 编排器构造；engine ctx 载荷仍是 model_chat::Request（handle_chat_send_oneoff 已组装）。
- SESSION_HANDLE：session 编排器已 set，session 版 `open_chat_session` 直接读 ctx，legacy 路由回退分支删除。

**⑤ 模块安置图（最终）**
- → session：chat_loop（含 ChatOrchestrator/finalize_assistant_turn/persist_messages）、tool_executor、resume、tool_result_guard、compression（合并 context.rs 的 compress_temporary_messages）；mod.rs 注册全部新模块。
- 删除（model）：chat_loop.rs、tool_executor.rs、resume.rs、tool_result_guard.rs、compression.rs、context.rs、turn_processor.rs（64 行薄壳内联：chat_loop 直接调 `orchestrator.protocol.execute_turn` + `orchestrator.finalize_assistant_turn`）、protocol.rs / tool_call.rs（re-export shim 随循环族消亡）、spawn_orchestrator、route "chat" 分支、ProviderRateLimiter。
- model 保留：protocols/*（四协议 + ping）、types.rs（NativeMessage 协议层）、message_builder.rs（flatten_chat_messages 四协议唯一消费点 + 既有单测；build_tool_message/short_id 的 re-export 随消费者迁走后删除）、handlers.rs、detail.rs、plugin.rs（瘦身：provider 注册表路由 + CONFIG/entities/status + validate + traverse）。

**⑥ 搬迁文件引用改写规则**
- `super::compression` / `super::tool_executor` / `super::tool_result_guard` / `super::resume` → session 内同名模块（`super::` 语义不变）。
- `super::context::{compress_temporary_messages}` → `super::compression::compress_temporary_messages`；`ChatOrchestrator` → 同文件（session/chat_loop.rs）。
- `super::context::PostResult` / `super::protocol::execute_post_with_abort` / `super::message_builder::{short_id, build_tool_message}` / `super::tool_call::ToolCallInfo` → `crate::symbio_core::turn::*`。
- `crate::plugins::model::resume::…` → `crate::plugins::session::resume::…`。
- send_compression_request（compression.rs）改调 `protocol.execute_turn`（若内部仍走请求路径）。

### 8.5 风险与验证

| 风险 | 缓解 |
|---|---|
| 流式帧时序改变 | execute_turn 只收编 turn 内帧（Update/Abort），StreamingStart/Status 等轮间帧仍由 session 循环发——前端时序不变 |
| previous_response_id 续接（openai_responses） | PostResult::RetryWithoutContextId 机制随 SSE 机器迁 core，execute_turn 原样返回，重试决策仍归 session 循环 |
| 中止竞态 | abort_flag 语义不变；session 持 ai_control_tx 直控 flag（比跨插件帧轮询更简） |
| 迁移半途不可用 | E-① 纯增量先行验证；E-② 单批原子切换，cargo check 全量把关 |

## 9. 兼容性与风险

| 风险 | 缓解 |
|---|---|
| trait 迁移破坏工厂注册 | TypeId 三点（submit/create_object/impl）同批更名；cargo check 全量把关 |
| 收集机制改变现有会话行为 | 消费侧"收集优先、现状回退"；未收集到即走原路径，行为逐字节等价 |
| 系统提示词语义漂移 | 既有两级优先级不动，只追加第三级兜底；单测锁定 |
| Phase E 大迁移半途不可用 | 独立成批、接口先行；A-D 完成后 model 已零依赖 session，E 是纯搬家 |
| `SESSION_OPEN` 双轨期不一致 | 回退分支加日志；移除后以 grep 零命中为完成判据 |

## 10. 实施进度

- [x] 现状探查与事实核验
- [x] 本设计文档
- [x] Phase A：core `ModelProvider` 接口（含 7 处残余改名收尾 + `pub mod model_provider` 公开，cargo check 零警告）
- [x] Phase B：CapabilityManager 扩充 + traverse 注册 + 消费侧（provider 解析收集优先/工厂回退 + 系统提示词插入第二级 + tools.rs 3 个单测通过）
- [x] Phase C：滑窗纯函数迁 core（model 中 `plugins::session` 文本引用零命中）
- [x] Phase D：路由依赖消除（SESSION_HANDLE ctx 交付 + compress_messages trait 方法；model 中 `plugins::session`/`SESSION_OPEN`/`SESSION_COMPRESS` 零命中，session→model 仅剩 core `MODEL_CHAT` 常量）
- [x] Phase E-①：core 基建（symbio_core/turn.rs 收纳 SSE 机器 + TurnOutput/ToolCallAccumulator + 消息构造家族；ModelProvider::execute_turn 统一默认实现（parse_sse_stream 泛型化 `<P: ModelProvider + ?Sized>`），turn_processor 改薄委托，行为等价；ModelProviderEntry 加 config 字段（capability/tools/plugin 三处补齐）；model 侧 protocol/tool_call/message_builder/context 四文件 re-export shim，消费方零改动；cargo check 零警告 + cargo test --lib 226 通过 0 失败）
- [x] Phase E-②：原子切换——session 搬入循环族（orchestrator.rs run_chat_loop_task 改为进程内直连：CAPABILITY_MANAGER 解析 entry（精确 id → is_default → 首个注册，`ModelProviderEntry.is_default` 承载 resolve 回退语义）+ `RATE_LIMITER.wait` 限流跟随请求发起方 + `PluginChannel::pair(4096)` 进程内对接 + keepalive + 嵌套 spawn Error 帧包装，消费循环逻辑零改动）；model 删 loop 族 9 文件（chat_loop/compression/context/resume/tool_call/tool_executor/tool_result_guard/turn_processor/protocol 垫片）+ 删 `resolve_provider` + MODEL_CHAT 路由与 PATH 设置移除；`cargo check` 零警告 + `cargo test --lib` 228 通过 0 失败 + grep 判据 `run_chat_loop\|spawn_orchestrator` 在 src/plugins/model/ 零命中 + model 中 `plugins::session` 引用零命中
- [x] Phase E-③：文档/注释清理——`schemas/model/model_chat.rs`（provider_id 注释补充 session 侧解析回退链；resume 注释指向 `session/chat_loop.rs`）、`schemas/session/chat_message.rs`（ResumeRequest 注释同步 `session/resume.rs:process_resume` 落点）、`plugins/session/chat_session.rs`（compress_messages 消费方表述去 model 化）、`plugins/session/README.md`（架构总览与数据流图改写为"session 唯一编排入口 + 进程内直连无状态 LLM 网关"，正文与总结矩阵全部 `plugins/model/*` 路径同步为 `plugins/session/*`）；删除根目录 3 个 E-② 临时脚本（fix_e2c_plugin.ps1 / fix_e2d_orchestrator.ps1 / fix_e2d_corrupt.ps1）；`cargo check` 通过零警告。残留仅 README `<state_snapshot>` 示例 XML（虚构样例数据，非架构声明，保留）
- [x] Phase sink：core 单消费者定义下沉（判定依据见 10.1）——session 新增私有模块 `rate_limit.rs` / `context_window.rs` / `tokenizer.rs`，local 新增私有模块 `system.rs`（均携带 Phase sink 头注释），8 处消费方引用切换（orchestrator L286 限流、compression 三处 use + 滑窗调用、chat_loop 校准 + 用量上报、tool_result_guard use、shell / content_search 的 decode_output / validate_params），core 删除 4 文件与全部注册 / re-export；下沉暴露 6 处 dead_code（core 时代靠 `pub use` 导出豁免）：`ProviderRateLimiter::new` 为纯别名删除（测试改 `default()`），`run_command` 与 tokenizer 家族 4 项（`PER_MESSAGE_OVERHEAD` / `count_messages` / `count_tools` / `ratio` / `calibration_ratio`）标 `#[allow(dead_code)]` 留待 audit-session 处置；README L247/L263 失实表述同步修正（"context.rs 保留重导出"不实，改为指向 `plugins/session/context_window.rs`）；`cargo check` 零警告 + `cargo test --lib` 228 通过 0 失败
- [x] audit-session：session 插件内部体检（audit-1~5 全部收官，报告见 10.2）——audit-3 消灭 core/types.rs（ToolCall 下沉 model）；audit-4 删除 sink 遗留全部 5 处 `#[allow(dead_code)]` 项；audit-5 完成 compress.rs 更名 message_archive.rs、context.rs 并入 chat_session.rs、5 文件头注释、README 6 处失实修正
- [x] 回归：cargo check 零警告 + 全模块测试（`cargo test --lib` 227 通过 0 失败；audit-3 后 228→227 为删除死代码连带自测试的预期减量）

### 10.1 Phase sink 判定表（E-② 后 core 24 模块消费者分布定稿）

**下沉项**（唯一非测试消费者所在模块）：

| core 定义 | 去向 | 依据 |
|---|---|---|
| `rate_limit`（ProviderRateLimiter / RATE_LIMITER） | session | 唯一消费者 `session/orchestrator.rs` 发起 LLM 请求前节流 |
| `context_window`（apply_layered_sliding_window） | session | 唯一消费者 `session/compression.rs::build_request_view` |
| `tokenizer`（Tokenizer / CalibratedTokenizer / default_tokenizer / report_provider_usage） | session | 消费者全部在 session（compression / chat_loop / tool_result_guard） |
| `system`（decode_output / validate_params / run_command） | local | 消费者仅 `local/shell.rs` + `local/content_search.rs`（run_command 无消费者，随迁） |

**留 core 项**（多消费者或 core 基建自用，下沉会造反向依赖）：

| core 定义 | 依据 |
|---|---|
| turn 家族 | core model_provider `execute_turn` 自用 + session 消费 |
| chat_pipeline | agent/host + session 双消费 |
| schemas::model | core / model / session 三方共享 |
| DefaultToolManager | chat_pipeline 内部 + agent 测试 |
| homedir | storage_service + plugins/home 双消费 |
| ids / paths | 全局常量单一真相源（PLUGIN_* / SESSION_* 等被各插件广泛使用） |
| logger | `#[macro_export]` 宏被 core 自身（chat_pipeline / entities / model_provider / turn）+ 多插件使用；init_logger 由 init.rs 调用（宏经 crate 根导出，直接路径搜索漏报，须按宏名盘点） |
| transport | PluginChannel / PluginFrame 经 re-export 被 orchestrator / turn 消费 |
| providers / event_bus / entities | 多插件消费（storage_service / embedding / codebase_search / skill / mcp / model / agent / setting / session） |
| chat_session | core/keys.rs `SessionHandleKey` 引用 `ChatSessionHandle`，下沉将造 core→session 反向依赖 |
| keys / plugin / capability / model_provider / creator / error | core 基建 |
| types（SystemEvent / EventResult / BoxStream / ToolCall） | 已处置（audit-3）：前三者全仓库零消费删除；ToolCall 单插件消费下沉 model/types.rs，core/types.rs 消亡 |

### 10.2 session 插件内部体检报告（audit-session，2026-09-09）

**体检范围**：`plugins/session/` 21 个源文件全量盘点 + 交叉引用分析，处置三类问题——分工合理性 / 冗余重复 / 不清晰点。全程行为不变（纯移动 / 重命名 / 删除死代码 / 补注释），不破坏架构。

#### 10.2.1 分工结论：五层边界清晰，架构健康

| 层 | 文件 | 职责 |
|---|---|---|
| 入口 + 状态守卫 + Turn 收尾 | orchestrator.rs（1189 行） | `handle_chat_message` 主链路、`WorkingGuard`（Drop 守卫防异常泄漏 working 态）、`merge_message_patch`、`finalize_assistant_turn`、`persist_failure` |
| 主循环 + 压缩执行体 | chat_loop.rs（1492 行） | `run_chat_loop` 工具迭代主循环、`auto_compress_process` / `run_context_compact` / `send_compression_request` / `apply_message_level_compression`、`FallbackChatSession`、`save_transcript_archive` |
| 压缩策略层 | compression.rs（1049 行） | `build_request_view` 三步剪裁（fade/滑窗/nudge）、Token 估算族、`prepare_compression`、XML 快照三件套、`compress_temporary_messages` |
| 存储引擎 | chat_session.rs（629 行） | Persistent / Ephemeral 两实现、`sliding_window` / `prune_historical_tool_calls` / `backfill_timestamps` 等纯函数 |
| 工具分发执行 | tool_executor.rs（830 行） | `process_tool_calls_async` 并发分发、`emit_tool_update` / `extract_result` / `fire_hook` |

其余模块各司其职：handlers.rs（invoke 路由与 CRUD）、plugin.rs（插件装配与实体协议）、store.rs（SQLite 持久化）、active.rs（活跃会话状态机）、resume.rs（恢复会话）、heartbeat.rs / fs_watcher.rs（后台任务）。未发现循环依赖、越层调用或职责重叠——**分工合理性体检通过**。

#### 10.2.2 问题清单与处置记录

**audit-3：core/types.rs 死代码终审**

四个类型三种命运，判据是"core 层定义必须多插件消费"：

| 类型 | 消费面 | 处置 |
|---|---|---|
| SystemEvent / EventResult / BoxStream | 全仓库零消费（仅定义 + re-export） | 删除 |
| ToolCall | 全仓库唯一消费者是 model（message_builder 构造请求包 + NativeMessage 携带） | 下沉 model/types.rs（message_builder 的 `use super::types::*` glob 自动覆盖） |

core/types.rs 整文件消亡（core 20→19 文件）。下沉前完成三道保险检查：glob 导入覆盖 / tests 目录零引用 / schemas 目录无同名定义。

**audit-4：sink 阶段遗留的 5 处 `#[allow(dead_code)]` 项，全部删除**

| 项 | 判据 |
|---|---|
| `local/system.rs::run_command` | 从 core 时代起零消费者；shell.rs 命令执行有自身完整实现，此为从未启用的简化封装 |
| `tokenizer::PER_MESSAGE_OVERHEAD=7` + `count_messages` | 生产路径 compression.rs 本地定义 `=8`（消息结构开销两处定义且不一致，本身是坏味道）；7 版本仅自测试消费。聚合语义归 compression.rs 的 estimate_* 系列 |
| `tokenizer::count_tools` | 零消费（连测试都没有） |
| `tokenizer::ratio()` + `calibration_ratio()` | 注释称"供诊断日志展示"但无任何日志调用；内部 ratio 字段保留（feedback/count 仍用） |

Tokenizer trait 收敛为单方法 `count()`——纯文本计量归 tokenizer，消息/工具级聚合语义归 compression.rs，消除两处重复定义的土壤。

**audit-5：可读性修复（全部行为不变）**

| 问题 | 处置 |
|---|---|
| compress.rs 与 compression.rs 命名易混（前者是单条消息物理脱水存档，后者是上下文语义压缩服务） | 更名 `message_archive.rs`，7 处 `super::compress::` 引用切换（chat_session 5 + handlers 2） |
| context.rs 的 `prune_historical_tool_calls` 唯一消费者是 chat_session | 函数并入 chat_session.rs 末尾，context.rs 删除；顺带发现并清除 git 历史中 apply_layered_sliding_window 死副本（正式版+测试已随 Phase sink 在 context_window.rs） |
| orchestrator / handlers / chat_session / types 四文件无头注释或定位不清 | 头注释补齐（职责清单 / invoke 集合定位 / 双实现说明 / 与 core-schemas 的类型边界） |
| session/README.md 6 处失实（compress.rs / context.rs 旧路径 + L247 "context.rs 保留重导出"的 sink 前旧表述） | 全部修正并附体检备注；正文残留的 3 处 compress.rs/context.rs 字样均为有意保留的历史说明 |

**回归记录**：每步处置后 `cargo check` 零警告；最终 `cargo test --lib` **227 通过 0 失败**（audit-3 删除零消费者 SystemEvent/EventResult 时连带其自测试，228→227 为预期减量，功能覆盖无损失）。

#### 10.2.3 遗留观察（不构成问题，记录备查）

- `EphemeralChatSession`（`_t_` 临时会话）不执行 prune 与存档清理，属临时会话短生命周期语义的合理取舍。
- `plugin.rs::cleanup_crashed_sessions` 将 Streaming 态置 Failed、WaitingUserAction 态保留，语义正确（后者是等待用户动作的合法暂停态）。
- compression.rs 本地 `PER_MESSAGE_OVERHEAD=8` 与生产路径一致，唯一真相源；tokenizer 回归纯文本计量后两模块职责正交。
