# Model 插件

LLM 核心封装插件。负责与各主流大模型 API 通讯、流式响应编排以及工具调用的底层执行。

## 功能特性

- **纯粹推理**：不持有会话状态，完全依赖外部（如 Session 插件）提供上下文。
- **协议适配**：支持 OpenAI, Anthropic, Gemini 等主流协议。
- **工具执行**：集成了审批流逻辑的工具分发器。
- **流式编排**：统一处理文本流、思考流 (Reasoning) 和工具调用流。

## 核心接口

| Path | 说明 |
|------|------|
| `chat` | **[Connection]** 接收上下文并返回推理流 |
| `providers/list` | 列出已配置 Provider |
| `providers/get` | 获取单个 Provider 详情 |
| `providers/set` | 新增/更新 Provider |
| `providers/delete` | 删除 Provider |
| `providers/set_default` | 设置默认 Provider |
| `status` | 查询引擎/Provider 状态 |
| `config/get` / `config/set` / `config/schema` | 配置 API Key、Base URL 等引擎参数 |

## 协作机制

1. **显式上下文接收**：Model 插件通过 `chat` 路由接收包含完整 `system_prompt`, `messages`, `tools` 以及编排参数的 `model_chat::Request`。
2. **职责分离**：它不再主动向 Session 请求数据，而是作为一个"热插拔"的推理引擎工作。
3. **结果回写**：推理完成后，通过 `session/append` 将产生的 Assistant 消息与工具执行结果持久化到存储中。
4. **工具分发**：当模型请求工具时，Model 插件通过根插件寻找对应的具体工具实现（如 `tools/shell`）。
