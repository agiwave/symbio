---
kind: task
id: T-147-ChatNode协议改名与字段瘦身
title: ChatMessage → ChatNode 一次原子改——Turn 退役、role 删除、tool_call_id 沉 meta、principal 升级为结构体
status: doing
created: 2026-10-16
track: 会话与输入
deps: [T-146-主体可路由地址]
prio: now
parent: T-145-前端机制化批
where: symbio/src/symbio_core/schemas/session/ · symbio/src/plugins/session/ · symbio/src/plugins/agent/ · cli/src/client.rs · cli/src/render.rs · tauri/src/schemas/ · scripts/protocol-mirror-audit.mjs
exit: `chat_message.rs` → `chat_node.rs`；`ChatMessage` → `ChatNode`；`MessageType` → `NodeKind`（五值闭集：Body/Notice/Skeleton/Interaction/Typing）；`MessageRole` 从协议删除；`tool_call_id`/`name` 从顶层移除（沉 meta 或用 title）；`ChatMessage.principal: Option<String>` → `Option<Principal>`（`{kind: PrincipalKind, id: String, title: Option<String>}` 三字段）；历史 `messages.json` 一次性读侧别名（`"agent:main"` → `{kind:Agent,id:"session/main"}`，`"user"` → `{kind:Person,id:"person/local"}`）；`ResumeAction::RetryTurn` → `RetrySubtree`；前端 `chat_message.ts` → `chat_node.ts` 同步、`principal?: string` → `principal?: { kind, id, title? }`；100+ 单测一次改绿；`protocol-mirror-audit` 指向新文件名与新 principal 结构；一次 diff 内落地，不分批
---

## 内容

决策依据：[ADR-070 Skeleton 取代 Turn](../010-adr/070.md "ADR-070") · [ADR-069 主体可路由地址](../010-adr/069.md "ADR-069")
父设计：[PL-013 §3](../004-plan/013-前端机制化.md "PL-013-前端机制化")

**协议层最大一次原子改**。Turn→Skeleton 的语义解绑是整批的核心；[T-148]~[T-152] 全部依赖它。
「一次改完、一次改绿」不是洁癖——分两批改意味着中间态协议两侧不一致，`protocol-mirror-audit` 会红一片。

principal 从字符串升到结构体一并做：[T-146] 已把值域迁到可路由地址 `<ns>/<name>`，本批把 kind
与 title 从「靠约定读 id 前缀 / 靠 meta」升为「协议字段」。历史 `messages.json` 里旧字符串
`"agent:main"` / `"user"` 靠读侧别名一次到位，不保留双形态。

字段瘦身清单见 ADR-070 §决策 2 的表；Principal 三字段与读侧别名规则见 ADR-069 §决策 6。

## 处理过程

按「协议两侧同时改绿」的原子约束分五档推进——每档前后端一起动、审计一起绿、一次原子提交：

- **147a**（additive）：`Principal` 结构体 + `PrincipalKind` 枚举 + `from_legacy` 读侧别名 helper；未接消费方。→ commit `910c4214`
- **147b**（wire 翻转）：`ChatMessage.principal: Option<String>` → `Option<Principal>`；`Principal` 自定义 `Deserialize` 同吃字符串与对象两种 wire 形态；写侧 `attributed` 构造 `Principal`；读侧 `visible_for` / `actor_and_ts` 通过 `.id` 消费；TS 加 `Principal` 接口 + `normalizePrincipal` / `principalId` / `principalKind` 三个 helper 与后端 `from_legacy` 逐字一致。
- **147c-i**（机械改名）：`chat_message.rs` → `chat_node.rs`；`ChatMessage` → `ChatNode`；`MessageType` → `NodeKind`；`MessageStatus` → `NodeStatus`；`MessageRole` → `NodeRole`；前端 `chat_message.ts` → `chat_node.ts` 同步。枚举**未**收五值——六值变体名与 wire 词都保留。
- **147c-ii-prep**（判据 helper，本批）：`NodeShape` 目标五值 + `legacy_shape_to_style_tag` 六→五映射 + `CHAT_STYLE_TAG_*` 词表 + `ChatNode::shape()` / `style_tag()` 判据入口 + 谓词 `is_run` / `is_tool_call` / `is_reasoning` / `is_text` / `is_compression` / `is_interaction`——谓词**签名**即未来形态判据入口，五值收敛那一档消费方代码不动、改点收在构造侧。四条单测（映射表、谓词互斥、shape 折叠、style_tag 从 meta 读）钉住基线。
- **147c-ii**（六→五收敛，延后）：`NodeKind` 变体收 Body/Notice/Skeleton/Interaction/Typing；写侧把 `NodeKind::Turn` 构造改成 `Skeleton + meta.style_tag="run"`，其余五档同理；前端 TS 同步。**规模**：~300 Rust 站点 + 250+ TS 站点，一次原子改。**已试手并回退一次**（本会话内），失败原因不是规模——是**纯文本 rename 会引入语义 bug**：
  - 构造侧：`NodeKind::Text` 与 `NodeKind::Reasoning` 都折成 `Body`，但后者要求 `meta.style_tag="reasoning"`；`Turn` 与 `ToolCall` 都折成 `Skeleton`，分别要求 tag `"run"` / `"tool"`；`Compression` → `Notice + tag="compression"`。**rename 不注入 tag = wire 语义丢**。
  - 读侧：`match m.msg_type { Some(Text) => A, Some(Reasoning) => B, ... }` 折成 `Some(Body)` 一个臂——A/B 分支**在同一 shape 里**，必须改判 `style_tag()`。当前谓词 `is_text` / `is_reasoning` 已就位可作替换点，但每处 match 都要**人工判**是哪个语义。
  - 兼容侧：`message_of_node` 折 `legacy_shape_to_style_tag`（本会话已起草），把旧 VDFS attrs 词读成 `kind + 注入 style_tag 到 meta`；新写方**已带** `meta.style_tag` 时不覆盖。
  - **下一轮开工次序**：先做读侧谓词化（`is_text` / `is_reasoning` 等 6 谓词覆盖全部判据点，把 `msg_type == Some(NodeKind::X)` 类比较与 `match msg_type` 类分支全替换）——这一步可独立提交、wire 不变；再做构造侧（`ChatNode::with_skeleton(tag)` / `with_body(tag)` / `with_compression_notice()` / `with_interaction()` helper 已在 c-ii-prep 就位，替换 `msg_type: Some(NodeKind::X)` 字面构造）；最后 enum 收五值 + 前端 TS 同步 + `legacy_shape_to_style_tag` 读侧兼容。**四档都能独立提交、审计一路绿**。
- **147c-ii-s1**（读侧谓词化，本批）：symbio/src **生产代码**的 `msg_type` 直接判据全量换成 6 谓词——21 文件；`None | Some(Text)` 类历史兼容写成 `msg_type.is_none() || is_text()`；`provider_adapter::DeltaBridge` 的流归属表脱离 `NodeKind`（私有 `TextStream{Body,Reasoning}` 二值枚举）；`message_builder` / `nodes::message_text` / `transcript::snapshot_entries` 的 `match msg_type` 分支改谓词 if/else 链。CLI `render.rs` 与 `v2_exec` 的 `UiNodeKind` 映射**不在本档**（按裸 `NodeKind` 值分派，谓词无入口；enum 收五值时编译期响亮，随构造侧一起迁）。wire 与测试断言不变，1693 单测全绿。
- **147c-ii-s2**（双形态共存档，本批）：`NodeShape` 合入 `NodeKind`——枚举扩到**十一变体**（旧六 + 目标五 `Body`/`Notice`/`Skeleton`/`Interaction`/`Typing`），`legacy_shape_to_style_tag` 返回 `NodeKind`；6 谓词**同时认两种形态**（`is_run` = `Turn` ∨ `Skeleton`+tag`run`，`is_text` = `Text` ∨ `Body` 非 reasoning tag，其余同构），构造侧链式 helper `ChatNode::with_style_tag(tag)` 落地。前端同批：`NODE_KINDS` 扩十一词（`protocol-mirror-audit` C 组**集合相等**，两侧必须同步）、`CHAT_STYLE_TAG_*` 镜像词表、`messageTypes.foldNodeKind` 把新形态折叠回旧词（与后端谓词同一判据，折叠点唯一在 `messageTypeOf`）、`NODE_KIND_LABELS`/`TYPE_PRESENTATION` 补五格（`typing` 走 fallback 渲染器，专属组件留给 [T-151]）。Rust 1694 测 + 前端 870 测 + typecheck + 镜像审计全绿。**wire 不变**：后端写方仍全旧词，构造侧迁移（s3）逐文件换形态时双形态天然兼容。
- **147c-ii-s3**（构造点迁移，本批）：symbio/src **生产代码**全部 `msg_type` 构造点换新形态——正文 `Body`、推理 `Body`+tag`reasoning`、运行骨架 `Skeleton`+tag`run`、工具骨架 `Skeleton`+tag`tool`、压缩 `Notice`+tag`compression`、交互 `Interaction`；覆盖落库 builder（`message_build`）、流式帧（`model/stream`、`v2_exec` 的 `UiNodeKind::shape` 改为 tag 注入器——变体名是本地流选择器不是 wire 词）、`provider_adapter` 工具轮、chat_loop io/state、rebuild、tool_executor 失败骨架、nodes/supplements/resume/prompt/compose/pipeline/view/heartbeat/subagent/shell 等。CLI `render.rs` 同批加 `Surface` 本地折叠（is_text/is_reasoning/is_tool_call 判据，含 `msg_type.is_none()` 回正文）——**否则 `NodeKind::Text` 匹配臂在 wire 翻成 `Body` 后静默丢正文**。旧六值仅存测试 fixture（读侧兼容用例），删除留给 s4 由编译期枚举剩余站点。wire 实际新词：body/skeleton/notice/interaction + style_tag。1694+11 测、typecheck、vitest 870、镜像审计 0 错全绿。
- **147d**（字段瘦身，拆三小批）：`role` / `tool_call_id` / `name` 三个顶层字段退役。规模盘点（Explore 交叉核验，仅 ChatNode-scoped，剔除 provider `NativeMessage.role` / `VdfsNode.name` / 工具能力 `spec.name`）：
  - **147d-i `tool_call_id` 沉 meta**（~11 站点）：Rust 非测试 4（`transcript.rs:238-239` / `plugin/nodes.rs:619` / `v2_tools.rs:264` / `message_build.rs:234`）+ 测试 3（`turn_runner.test.rs:447,1259` / `v2_exec.test.rs:816`）；TS 3（`sessionTranscriptSync.ts:257` / `sessions.ts:400` / `chat_node.ts:371` 声明）；e2e 1（`t44` 字段快照）。前端**无决策消费**——纯 copy-through。**最安全的一档**，可独立提交。
  - **147d-ii `name` → `principal.title`**（~46 站点，含兼容 shim）：Rust ~15 非测试（`message_builder.rs:240,269` / `nodes.rs:558,616` / `context.rs:201` / `window.rs:356` / `transcript.rs:192,235,236` / `projection/transcript.rs:225` / `resume.rs:585` / `provider_adapter.rs:339`）+ ~20 测试 fixture；TS ~5-8（`messageTypes.agentNameOf` 工具名显示）；e2e ~3。工具名是**活跃显示字段**——需 `meta.tool_name` / `principal.title` 双读 shim。
  - **147d-iii `role` 删除**（~378 站点，最大）：Rust 237（非测试 93 / 测试 144）、TS/Vue 106（非测试 27 / 测试 79）、e2e 35。**不是纯删除**——每个读点都是分派谓词（`m.role == Some(User)` / `role === CHAT_ROLE_USER`），要改写为 `principal` 前缀判据；只在 `principal=None` 遗留位需要过渡 shim。真实推进要等 [T-147c-ii] 落地。
- **147e**（动词改名，本批）：`ResumeAction::RetryTurn` → `RetrySubtree`；wire 词 `retry_turn` → `retry_subtree`；后端函数 `process_retry_turn` → `process_retry_subtree`；前端常量 `RESUME_ACTION_RETRY_TURN` → `RESUME_ACTION_RETRY_SUBTREE`；跨栈 25 站点一次改绿。语义泛化——靶子从「Failed Turn」变成「任一骨架节点」，为 [T-147c-ii] 五值收敛铺路（`Turn` 变体在 wire 上消失时 `RetrySubtree` 不再依赖那个名字）。

