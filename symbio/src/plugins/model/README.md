# Model 插件

无状态单轮 LLM 网关。model 只做一件事：**接收一次调用，把消息/工具传给某个 Provider，把产出流回给调用方**——不执行工具、不维护会话、不感知编排循环。

## 职责边界

- **Provider 注册**：启动时通过 `traverse` 向 `CAPABILITY_VISITOR` 注册**唯一生效的核心 `ModelProvider`**（trait object，由 `bound_provider::BoundProvider` 实现——持久化配置 `ModelProviderConfig` + 协议适配器绑定）。解析链：会话上下文选定的 provider_id > 默认 Provider > 首个启用的 Provider。
- **协议适配**：协议契约 `ModelProtocol`（钩子收 `&ModelProviderConfig`）**插件私有**，不对外导出；内置 4 个实现——`openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api`，统一转换为内部 `model_chat::Request/Response` 事件流（文本、思考、工具调用）。`resolve_protocol_id` 别名表与 `MODEL_PROTOCOL_*` 注册常量同样内化于 `protocols/`。
- **单轮执行**：`execute_turn` 即单轮"发消息→收流"的完整闭环，不含重试、裁剪、压缩等编排逻辑（这些归 session，见 `session/README.md` 六大策略）。
- **配置存取**：providers CRUD 与引擎参数（API Key、Base URL 等）的 `config get/set`。

## 明确不做

- 工具执行与审批流分发：工具注册由各工具插件 traverse 提供，执行由 session 的 tool_executor 编排
- 通过 `session/append` 回写消息：session 自己持久化，model 对 session 零依赖
- 会话循环 / 上下文裁剪 / 压缩：会话引擎整体位于 session

## 路由

| Path | 说明 |
|------|------|
| `config/get` / `config/set` | 引擎参数配置（字段定义随 `.vdfs/model` 节点 `schema` 下发） |

模型 provider **不设插件路由**：`.vdfs/model` 挂载点由本插件自己的
`impl VdfsProvider` 提供——落盘走 `providers::vdfs_service::SingleFileVdfs`
（一个条目 = 一份 `provider.json`，条目内部不外露），清单走 `MemoryVdfs`
（内存镜像：启动时从磁盘灌入、写盘成功后回灌）。原 `entities/*` / `config/schema` /
`status` / `chat_sync` 路由均已下线——字段定义改由 `detail_definition` 随 VDFS 节点
`schema` 下发，连通性自检改由节点动作 `vdfs/action { action: "test" }`。

## 关联

- 上游消费者：`session`（chat_loop 直连）
- 协议适配层代码：`protocols/`
