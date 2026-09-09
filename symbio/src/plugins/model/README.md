# Model 插件

无状态单轮 LLM 网关。Phase E 定型后，model 只做一件事：**接收一次调用，把消息/工具传给某个 Provider，把产出流回给调用方**——不执行工具、不维护会话、不感知编排循环。

## 职责边界

- **Provider 注册**：启动时通过 `traverse` 向 `CAPABILITY_MANAGER` 注册 `ModelProviderEntry`（provider_id → 配置的模型条目），供 session 的 `chat_loop` 按需取用。
- **协议适配**：内置 4 个协议适配器——`openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api`，统一转换为内部 `model_chat::Request/Response` 事件流（文本、思考、工具调用）。
- **单轮执行**：`execute_turn` 即单轮"发消息→收流"的完整闭环，不含重试、裁剪、压缩等编排逻辑（这些归 session，见 `session/README.md` 六大策略）。
- **配置存取**：providers CRUD 与引擎参数（API Key、Base URL 等）的 `config get/set/schema`。

## 明确不做（已迁出/从未承担）

- ~~工具执行与审批流分发~~ → 工具注册由各工具插件 traverse 提供，执行由 session 的 tool_executor 编排
- ~~通过 `session/append` 回写消息~~ → session 自己持久化，model 对 session 零依赖
- ~~会话循环 / 上下文裁剪 / 压缩~~ → 会话引擎整体位于 session（详见 `docs/archive/implementation-logs/model-session-refactor.md` 的 Phase E 记录）

## 路由

| Path | 说明 |
|------|------|
| `entities/*` | 统一实体协议（Provider 实体的 create/get/update/delete/query，见 `docs/design/entity-management-mechanism.md`） |
| `config/get` / `config/set` / `config/schema` | 引擎参数配置 |
| `status` | 引擎/Provider 状态 |
| `chat_sync` | 同步单轮调用（当前返回 NotImplemented，预留接口） |

## 关联

- 上游消费者：`session`（chat_loop 直连）
- 协议适配层代码：`mod.rs` / `providers/`
- 历史改造记录：`docs/archive/implementation-logs/model-session-refactor.md`
