# ADR 正文 · 模型与协议

> **文档类型：阐述** — 本文件是 [DECISIONS.md](../DECISIONS.md) 的**正文分册**之一，
> 只记录**为什么这样设计**（状态 / 决策 / 理由 / 被否决的方案 / 后果与不变量）。
> 索引与域分配规则在 [DECISIONS.md](../DECISIONS.md)；「现在是什么」看 [CURRENT.md](../CURRENT.md)。

**本册范围**：多协议适配 / TLS / 本地嵌入 / SSE 解析。

---

## ADR-004: 多协议 LLM 适配

**状态**：已接受

**决策**：`model` 内置 4 套协议适配器——`openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api`。

**理由**：供应商无关（同一套代码对接多家）；协议演进（OpenAI 从 Chat 到 Responses）；各家工具调用格式不同。

**后果**：`model` 代码量较大；新增协议 = 新增适配器。`model` **无自有路由**——它是无状态单轮网关 `execute_turn`，由 `session` 在循环内直连调用（见 ADR-021）。

---

## ADR-013: TLS 后端 = 平台原生栈（`native-tls`），不做纯 Rust 密码学

**状态**：已接受

**背景**：HTTP 出口（LLM 适配、MCP http transport、web 插件、telegram）统一走 `reqwest`，其 TLS 原由 rustls 提供，而 rustls 的两个官方密码学后端**都是 C**：默认 `aws-lc-rs`（`aws-lc-sys` 是整份 BoringSSL 分支，~69 MB C 源码，且**无条件**依赖 `cmake`）与备选 `ring`（8.3 MB C）。纯 Rust 的替代 `rustls-rustcrypto` 上游自述 **DO NOT USE IN PRODUCTION** ⇒「纯 Rust + 生产可用」这条路今天不存在。

**决策**：HTTP 出口的 TLS 后端改为**平台原生栈**（`reqwest` 的 `native-tls`）：Windows → SChannel（纯 Rust FFI 绑定、零 C 源）、macOS → Security.framework、Linux → 系统 OpenSSL。**明确接受**由此引入的**平台分支**。

**理由**：选它而非 `ring`，唯一理由是它能把目标平台上的 C 编译**真正降到零**，并连带移除 `cmake` 这个构建期前置工具；它**几乎不新增依赖族**；语义上证书信任本就来自系统库（原路线的 `rustls-platform-verifier` 做的是同一件事），属于**换实现而非换语义**，且 `symbio/src` 未使用任何 rustls 专属 `reqwest` 配置项。

**后果**：
- Linux 构建前置：`libssl-dev` + `pkg-config`（Debian/Ubuntu）或 `openssl-devel`（RHEL/Fedora），已记入 [CONTRIBUTING.md](../../CONTRIBUTING.md) §1。
- **跨平台行为从此随 OS 变**：TLS 版本上限 / 密码套件 / 错误文案都不同 ⇒ 一类「CI 绿本地红」的排查成本上升。这是**明确接受的代价**。
- 将来若要 HTTP/3 或后量子（ML-KEM），需回到 rustls 并重新接受 C 依赖——本决策不阻断该回退，只是要重付一次代价。

---

## ADR-016: 本地嵌入改用 `ort`（ONNX Runtime）——推翻 ADR-014 的性能前提，并接受它当初拒绝的代价

**状态**：已接受

**背景**：ADR-014 选 `tract-onnx` 而弃 ORT，理由有三（零 C/C++ 编译、不引入 ~391 MB ORT 预编译产物、不交付 `DirectML.dll`）；它对性能的判断是「嵌入场景是离线索引 + 单条查询，秒级可接受」。**这句话对「单条查询」成立，对「建索引」完全不成立**：实测 tract 每 token ≈ 22.5 ms 且纯线性（release 只比 debug 快 16% ⇒ 是**结构性**慢，非未调好），本仓全量建索引 ≈ 6 小时，而工具执行有 `TOOL_EXEC_HARD_TIMEOUT_SECS = 600` 的硬超时 ⇒ **语义检索永远拿不到结果**，用户看到的是「一直回复中，永远没有响应」。

**决策**：
1. **推理后端换 `ort`**（ONNX Runtime 的 Rust 绑定），**其余一律不动**：模型仍是内嵌的 int8 `bge-small-zh-v1.5`，分词仍用 `tokenizers` + `fancy-regex`，后处理仍是 `encode(text, true)` + CLS 池化 + L2 归一化。
2. **不恢复 `fastembed`**：它的 C 编译链主犯是硬编码的 `tokenizers/onig`（⇒ `onig_sys`），不是 ORT；直接依赖 `ort` 就能拿到同样的推理速度，而不必把 `onig_sys` 请回来。
3. **接受 ADR-014 当初拒绝的代价**，因为「不可用」比「贵」更糟。

**明确接受的代价**：
- **C/C++ 编译仍然为零**（`ort-sys` 的 ORT 是 build 期下载的预编译二进制，`cargo tree -i cc / -i ring / -i onig_sys` 三者皆空）。
- **~359 MB 预编译缓存回到 `%LOCALAPPDATA%\ort.pyke.io`**（Windows x64 拿到 DirectML flavour；`directml` 虽是可关 feature，pyke 预编译本身已含 DML EP，用 features 关不掉）。
- **`DirectML.dll` 是静态导入**，非延迟加载。Win10 1903+ / Win11 由系统提供，目标平台不必随包；**跨到更老的 Windows 则需要随包**。
- **离线构建会静默退化**：拉不到预编译 flavour 时 `ort-sys` 不报错、只是「不链接」，直到链接期才以 `undefined symbol: OrtGetApiBase` 失败 ⇒ CI 需要能联网，或经 `ORT_LIB_LOCATION` 指向本地副本。

**数值一致性（换引擎不该换语义）**：`ort` 与 `fastembed` 用的是同一个引擎，实测一万余次对照的余弦量级差异属 int8 量化噪声（两个引擎各自自一致性 ≥ 0.999999），不是质量退步。

**顺带确立的两条索引侧决策**：
- **索引落盘 + 按 `mtime` 增量重建**（`{workdir}/.symbio/cache/codebase-index.bin`）。此前 `INDEX_CACHE` 是纯进程内、**零失效机制**，agent 会拿一份不含自己刚写的代码的索引去搜且完全无从察觉。已知边界：指纹 = `mtime` + 字节数，**刻意保留 `mtime` 的写入检测不到**，`rebuild=true` 是兜底。
- **生成物不进索引**：按名字排除锁文件 / 压缩产物 + 按行数上限排除生成的数据文件——它们的词表全是短 token，对任何查询都有中等相似度，会**挤占 top-k**。

---

## ADR-022: SSE 增量解析——**契约在 core，字段名在协议层**

**状态**：已接受。**本 ADR 决策 1 与决策 6 中的「位置」条款已被 [ADR-034](#adr-034-sse-行解析契约随流循环迁入-model-插件) 取代**——契约现已与流循环同处 `plugins/model/`；决策 2–5（两个方法、每行只问一次、UTF-8 对齐、字段名留协议层、`ModelProtocol` 以其为父 trait）全部不变。

**背景**：为了首字延迟，`parse_sse_stream` 在换行到达前会先尝试从半截 JSON 里挤出正文。这件事原先由 **core 内置的启发式解析器**代劳（在整行里搜五个硬编码字段名），三个后果：**加协议要改 core**；**两条路径两套转义**（core 的 `unescape_partial` 与协议解析器的 `serde_json` 对 `\uXXXX` 处理不同 ⇒ 按前缀截断会**吃字**，最隐蔽——不报错，只是偶尔少一个字）；**每块重扫整行** ⇒ 单行极长时 O(行长²)。

**决策**：
1. **契约拆成两个方法**（`symbio_core::sse`）：`parse_line`（完整行）与 `open_partial_line(head) -> Option<Box<dyn PartialLineExtractor>>` + `PartialLineExtractor::push(bytes, out)`（只吃新增字节）。`open_partial_line` **默认返回 `None`**，即「本行不做增量提取」——这是合法且正确的降级（代价只是首字延迟变大），因此不实现增量的协议一行代码都不用写。
2. **每行只问一次**：答 `None` 就记下、本行不再重试。
3. **UTF-8 边界对齐写进契约**（`utf8_chunk`）：被切断的多字节字符**不消费**，留给下一次 `push`——SSE 分块由 TCP 决定，一个中文字符横跨两块是常态。
4. **字段名与转义规则全部留在协议层**：四个协议共用一个逐字节推进的 JSON 结构扫描器，协议只实现三个钩子（`begin_string` / `text` / `scalar`），用**键路径全等**判定「这个位置算什么」。
5. **`ModelProtocol` 以 `SseLineParser` 为父 trait**；`BoundProvider::execute_turn` 直接把协议实例交给 `parse_sse_stream`，core 与协议之间不再有闭包中转。
6. **core 只负责**：按 `\n` 切行 → `parse_line` → 按前缀截断去重 → 尾巴交给提取器。core **不再认识任何协议字段名**。

**理由**：**「增量文本恰好是完整行文本的前缀」这条不变量，必须由同一套转义规则保证**——只有协议层同时掌握「字段在哪」与「怎么解码」；**降级必须是免费的**；**判定用全等而非包含**：错配的代价是把别处的文本当增量吐出去，不匹配的代价只是退回「等换行」——**宁可漏，不可错**。

**后果与风险**：扫描器必须**自己实现 JSON 字符串解码**并与 `serde_json` 对齐，这是本决策的**主要风险点**，靠两层测试压住（直接比对 `serde_json` 的解码结果 + 四个协议各一条「逐字节切分喂进去，增量拼出的文本必须等于完整行解析出的文本」的不变量测试）；对非法 JSON 转义比 `serde_json` 宽松，分歧只在非法输入上出现；下标未知就放弃本次增量（用错下标会把参数接到别的工具调用上，比「等换行」糟糕得多）。

---

## ADR-034: SSE 行解析契约随流循环**迁入 `model` 插件**

**状态**：已接受。**取代 [ADR-022](#adr-022-sse-增量解析契约在-core字段名在协议层) 决策 1 / 决策 6 中的「位置」条款**；决策 2 里「`turn` 装帧 / 消息构造家族」的**位置条款**已被 [ADR-038](./core.md#adr-038-帧与消息构造家族按依赖方数量下沉) 取代（形状不变）。

**背景**：[ADR-022](#adr-022-sse-增量解析契约在-core字段名在协议层) 把 SSE 增量解析拆成两个方法（`parse_line` / `open_partial_line`），并把契约放在 core——当时的理由是「**core 负责按 `\n` 切行**，协议层负责『这一行是什么』」，即 core 是两侧共同可见的中立地。

此后按行切分的循环 `parse_sse_stream` 作为「实现细节而非契约」下沉到 `plugins/model/stream.rs`（内核瘦身批次）。契约的**唯一消费方**随之离开 core，而实现方（四个协议适配器）本来就在 model。于是：

- `SseLineParser` / `SsePartialLineExtractor` 的**实现与消费全在 model 一个模块内**；
- `utf8_chunk`（`pub(crate)`）只有 `stream.rs` 一个调用点；
- `ModelProtocolEvent`（协议事件方言）的生产方与消费方同样都在 model。

按 [ADR-023](./core.md#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层) 的准入判据（**依赖方数量**：只被一个模块依赖的内容一律下沉回该模块），它们不应留在 core。ADR-022 决策 6 的「core 只负责：按 `\n` 切行 → …」在循环迁走的那一刻就已与代码不符，只是没人回头改——本条同时修掉这处漂移。

**决策**：
1. `SseLineParser` / `SsePartialLineExtractor` / `utf8_chunk` 迁入 `plugins/model/protocols/sse.rs`；`ModelProtocolEvent` 迁入 `plugins/model/protocols/mod.rs`（紧邻 `ModelProtocol`——它是该 trait `parse_line` 的返回类型）。
2. `symbio_core::llm` 只剩 `model_provider`（`ModelProvider` / `ModelFinishReason` / `ModelUsage`）与 `turn`（`TurnOutput` 与帧 / 消息构造家族）——即**只有 session 与 model 两侧共用**的符号。
3. ADR-022 的**形状**决策全部不变（见上方状态行）。

**理由**：ADR-022 真正要保住的是「**core 不再认识任何协议字段名**」这条**负面约束**——在本决策下它**更强**（连协议抽象都不在 core 了）。位置本身不是目的：契约与实现方、消费方同处一个模块时，「谁实现、谁消费」一屏读完，改签名不会漏掉某个远处的调用点。

**被否决的方案**：
- **保留在 core 并登记为「预留契约」**：ADR-023 明确否定「先上提、等消费者」——`SseLineParser` 今天既没有第二个实现方，也没有第二个消费方，而「将来可能有」是不可证伪的理由。
- **把 `parse_sse_stream` 移回 core**：那是往 core 搬实现（HTTP 重试机器与流循环），与 ADR-023 的方向相反。
- **只改文档、不动代码**：会留下「文档说契约在 core、代码里 core 侧无人用」的持续漂移，正是 [ADR-012](./vdfs.md#adr-012-读侧成本是设计约束现在是什么必须有一张可核对的表) 要消灭的东西。

**后果与不变量**：
- 新增协议只需动 `plugins/model/`，core 不受影响。
- `symbio_core::llm::sse` 子模块消失；`symbio_core` 不再导出 `SseLineParser` / `SsePartialLineExtractor` / `ModelProtocolEvent` / `utf8_chunk`。
- 若将来出现**第二个** SSE 消费者（例如另一类流式 provider），按 ADR-023 再上提到 core——判据不变，上提成本是一次编译期可检的搬迁。

---
