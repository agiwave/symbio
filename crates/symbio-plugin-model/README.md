---
kind: design
id: crates/symbio-plugin-model/README
title: Model 插件
status: active
created: 2026-07-20
impl: done
---

# Model 插件

无状态单轮 LLM 网关。model 只做一件事：**接收一次调用，把消息/工具传给某个 Provider，把产出流回给调用方**——不执行工具、不维护会话、不感知编排循环。

## 职责边界

- **Provider 注册**：启动时通过 `traverse` 向 `CAPABILITY_VISITOR` 注册**唯一生效的核心 `ModelProvider`**（trait object，由 `bound_provider::BoundProvider` 实现——持久化配置 `ModelProviderConfig` + 协议适配器绑定）。解析链：会话上下文选定的 provider_id > 默认 Provider > 首个启用的 Provider。
- **协议适配**：协议契约 `ModelProtocol`（钩子收 `&ModelProviderConfig`）**插件私有**，不对外导出；内置 4 个实现——`openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api`，统一转换为内部 `model_chat::Request/Response` 事件流（文本、思考、工具调用）。`resolve_protocol_id` 别名表与 `MODEL_PROTOCOL_*` 注册常量同样内化于 `protocols/`——**未识别的取值返回 `None`，不兜底成 `openai_chat`**：写错的协议在配置校验 / 能力收集期报出来，而不是等运行期发错请求体。
- **单轮执行**：`ModelProvider::round_trip` 的实现在这里，即单轮"发消息→收流"的完整闭环，不含重试、裁剪、压缩等编排逻辑（这些归会话引擎：持久化与生命周期在 [`session`](../symbio-plugin-session/README.md "crates/symbio-plugin-session/README")，主循环在 [`loop`](../symbio-plugin-loop/README.md "crates/symbio-plugin-loop/README")）。
- **配置存取**：providers CRUD 与引擎参数（API Key、Base URL 等）都在 `<根>/model` 上——条目读写走 `vdfs/read` / `vdfs/write`，字段定义随节点 `schema` 下发。

## 明确不做

- 工具执行与审批流分发：工具注册由各工具插件 traverse 提供，执行由 `crates/symbio-plugin-loop` 的 tool_executor 编排
- 通过落库接口回写消息：转写由 session 持久化，model 对 session 零依赖
- 会话循环 / 上下文裁剪 / 压缩：主循环整体位于 `crates/symbio-plugin-loop`，模型只收到「这一刻要什么」

## 路由

**无**（权威登记见 [路由参考](../../docs/004-ref/003-ROUTES.md "RE-003-ROUTES") §Model 插件）。model 的全部对外能力
都在 VDFS 上（`<根>/model`）。持久化分两处：跨条目状态（`default_provider_id`）写自己的
`crates/symbio-plugin-model/PLUGIN.yml`，Provider 明细是资源、各存各的 `provider.json`。

模型 provider **不设插件路由**：`<根>/model` 挂载点由本插件自己的 `impl VdfsProvider`
提供——落盘走 `providers` 出口的 `SingleFileVdfs`（实现住 `providers/vdfs_service/`；
一个条目 = 一份 `provider.json`，
条目内部不外露），清单走 `MemoryVdfs`（内存镜像：启动时从磁盘灌入、写盘成功后回灌）。
字段定义随 VDFS 节点 `schema` 下发，连通性自检走节点动作 `vdfs/action { action: "test" }`。

## 关联

- 上游消费者：`loop`（`chat_loop` 直连；session 经驱动器间接使用）
- 协议适配层代码：`protocols/`

## 关键决策

> 本节收录口径见 [DE-017 §6](../../docs/003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")。

### D-004 多协议 LLM 适配（4 套适配器）

`model` 内置 4 套协议适配器——`openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api`。 供应商无关（同一套代码对接多家）；协议演进（OpenAI 从 Chat 到 Responses）；各家工具调用格式不同。 `model` 代码量较大；新增协议 = 新增适配器。

### D-013 TLS 后端 = 平台原生栈（native-tls），不做纯 Rust 密码学

HTTP 出口（LLM 适配、MCP http transport、web 插件、telegram）统一走 `reqwest`，其 TLS 原由 rustls 提供，而 rustls 的两个官方密码学后端默认 `aws-lc-rs`（`aws-lc-sys` 是整份 BoringSSL 分支，~69 MB C 源码，且无条件依赖 `cmake`）与备选 `ring`（8.3 MB C）。

### D-016 本地嵌入改用 ort（ONNX Runtime）——推翻 DE-002 的性能前提

DE-002 选 `tract-onnx` 而弃 ORT，理由有三（零 C/C++ 编译、不引入 ~391 MB ORT 预编译产物、不交付 `DirectML.dll`）；它对性能的判断是「嵌入场景是离线索引 + 单条查询，秒级可接受」。

### D-048 出厂档位改 full——v2 链路是出厂路径，bridge 退役

`V2Mode` 的 `#[default]` 由 `Bridge` 改为 `Full`，并退役 `bridge` 档—— 转写路径（`record` / `record_to_wal` / `authorize_close` / `first_user_utterance` / `last_assistant_text` / `V2Closure`）与只服务于「跳过它」的 `TurnState::v2_executed` 一并删除；`full` 成为出厂档，不再保留一条绕开 v2 的路径。随后 D-051 把旋钮本身也退役。

### D-096 快速档裁 (ii) 一次受限模型调用，ClassifyOnly 令牌闸门显式退役——「不得生成」的强制力搬到返回类型与工具集判据上

快速档 = (ii) 一次受限的模型调用（不是 (i) 纯规则分类器）， `ClassifyOnly` / `TokenIssuer::issue_fast` / `CanClassify` / `CanGenerate` 从 `symbio-kernel` 契约面收掉， `RuleOnly` / `FullModel` 两档令牌与 `LlmAdapter::generate(&FullModel)` 不动。

### D-106 模型消费端口归一：内部请求共享 LlmAdapter，未识别协议不静默兜底

`LlmAdapter` + `TokenIssuer`。`classify/decide.rs` 与 `compose/wording.rs` 改经 `ProviderLlmAdapter::for_turn` + `generate_turn(&FullModel, …)`；内部请求的形态一项不少——工具集 `&[]`、出口 `SilentDeltas`、system prompt 就地构造、中止走调用方给的共享信号。

### D-506 流式用量必须显式索取：OpenAI Chat 的 `stream_options` 与两种「在总数之外」的 token

**背景**。真实会话压测暴露出 `session/stats.usage` 恒为 0：OpenAI Chat 的流式 `usage`
只在最后一个 chunk 出现，且必须由请求显式开启 `stream_options.include_usage`。不发这个
字段 ⇒ `context.request_tokens` 缺席 ⇒ compact 的水位判据读不到数 ⇒ 压缩静默不发生；
这不是「少一个统计」，而是判据空转。同一轮排查还发现两份协议把用量拆在总额之外：
Anthropic 的 `input_tokens` 不含 `cache_read_input_tokens` / `cache_creation_input_tokens`，
Gemini 的 `candidatesTokenCount` 不含 `thoughtsTokenCount`。

**裁决**。

- `openai_chat` 的请求固定发 `stream_options: { include_usage: true }`。它是 OpenAI
  规范字段，兼容端点要么实现、要么忽略；不按 provider 名字猜能力。
- Anthropic 的 `cache_*` 折进 `ModelUsage.input`；Gemini 的 `thoughtsTokenCount` 折进
  `output`——两者都是上下文窗口 / 计费口径的一部分，留在外面就是少算。
- `ModelUsage` 不新增缓存分桶：`input` 的语义是「总 prompt token」，价格仍按单档输入价
  折算，缓存折扣在账面上是上界。需要分档计价时另开决策，不在协议适配层猜。

**判据**。LM Studio 实跑 `/v1/chat/completions` / `/v1/responses` / `/v1/messages`，
`session/stats` 的用量列均有读数；Anthropic 观测 `input_tokens=11` +
`cache_read_input_tokens=7` ⇒ 读面 18；Gemini 单测覆盖 `5 + 3 = 8`。

**不做什么**。不为兼容性先加 provider 开关（先按规范发，遇到真实拒绝再加）；不在本条做
缓存分档计价；不把「估算 token」冒充 provider 实测用量。

> 归属：改动全在本插件的协议适配层与私有 `ModelUsage`，不触及共享事件格位 / 授权行 /
> 可见域 / 公共数据面。

### D-507 一次性问答必须以用户回合收尾；空流按瞬时故障重采样

**背景**。真实压测里压缩偶发拿不到摘要。抓到的上游响应是
`200 + finish=stop + 1 个 completion token`，零正文、零推理、零工具调用。摘要 wire
从事实投影后以 **Agent 终答**结尾（实测 `[Person 26, Agent 174]`），模型把「已经答完的
对话」当成无需再答，直接 EOS；原样重发连提示缓存都打不破，4 次里 4 次仍空。

**裁决**。

- `model.assist` 在 wire **以 Agent 收尾**时补一条用户侧收束指令
  （`请按 system 指令给出最终答复。`）。它只是本请求的提示，不落事实、不改世界语义；
  不关 reasoning，也不把「思考」从请求里拿掉。
- `parse_sse_stream` 在无正文 / 推理 / 工具调用且 `finish≠length` 时返回**瞬时**
  `StreamFailure`，交给既有流重试轴（3 次）；每次重试把请求体里的 `temperature`
  上调 0.2（封顶 1.0），用**重新采样**而不是逐字节原样重发去打掉退化样本 / 缓存命中。
  `finish=length` 的零内容是确定性截断，仍标非瞬时。
- 有 reasoning 的收束不算空流：既有的「思考即正文」逻辑继续生效。

**判据**。LM Studio 的 `lfm2.5-24b-a2b-apex` 在 5k 窗口实跑：修复前 0/4 落
`session.consolidated`，补用户收束后 4/4；`[assist]` 的 wire 形状从
`[Person, Agent]` 变为 `[Person, Agent, Person]`。

**不做什么**。不靠关闭 reasoning 或换非思考模型来绕；不把空流静默当成功；不无限重试；
不在重试里改消息内容或工具表。

> 归属：只改本插件发给上游 LLM 的 wire 尾部与流解析算法，「不落事实、不改世界语义」，
> 不触及任何共享契约。

