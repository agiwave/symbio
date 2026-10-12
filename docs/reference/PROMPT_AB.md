# 结构化 vs 拍平：真实模型上的对照读数

> ⚠️ **生成物，不手改。** 唯一写者是 `node e2e/ab/run.mjs`。
> 判据口径归 `e2e/ab/tasks.mjs` 与 `e2e/ab/flatten.mjs`，
> **结论与边界**归 [`docs/registry/task/S-end③.md`](../registry/task/S-end③.md)（本文件不写结论）。

生成于 2026-10-12T05:11:15.611Z · 模型 `MiMo-V2.6-Flash`（provider `AMD-MiMo-V2.6-Flash`）· 题 22 道 × 两臂 × 2 次
温度 0（两臂同值，实验只认「两臂一样」不认「出厂默认」）· max_tokens 4096 · 两臂的信封差别只有 `tools` / `tool_choice` 是否存在
上下文来源：真实链路捕获（场景「full-主线装配-工具轮-对话面关闭」，captured 2026-10-12T04:28:14.815Z）· system 前缀 2 条 · tools 21 个

## 汇总（判据列）

| 判据 | 臂 A 结构化（出厂） | 臂 B 拍平 |
|---|---|---|
| select：选对工具（`tool_name_hit`） | 16/18 | 9/17 |
| select：参数是合法 JSON（`args_json_ok`） | 18/18 | 9/9 |
| select：必填键齐全（`required_keys_hit`） | 17/18 | 9/9 |
| select：调了不存在的工具名（越少越好） | 0/18 | 0/17 |
| control：不该调用却调用（越少越好） | 0/3 | 0/3 |
| attribution：结果↔调用对应得回来 | 2/5 | 2/5 |
| 拍平侧：文本协议被遵守（臂 A 不适用） | — | 9/9 |

## 观测列（**不进判据**，与 ADR-057 同一取舍）

| | 臂 A | 臂 B |
|---|---|---|
| prompt token 合计 | 152828 | 121401 |
| 请求体字节合计 | 359396 | 348767 |
| 墙钟合计（ms） | 85390 | 103346 |
| provider 自报账单（usage.cost，合计） | 0.001054 | 0.000855 |
| 未收束的题数（已从判据分母剔除） | 0 | 0 |
| 请求出错的题数 | 18 | 19 |

**样本健康度**：两臂合计 0 道题未收束、37 道请求出错；重复之间可比对 8 组，其中 0 组换一次时刻就跑出了不同结论。「未收束」不计入上面任何一条判据的分母——本机服务坏过一整段时会把回答刷成 `<unused32>` 复读，把它读成「模型没做到」就是拿环境噪声当读数。

## 逐题（同一道题两臂并排；`×k` 是第 k 次重复）

| 题（臂 A vs 臂 B） | A 调用 | A hit | A 参数 | A 关联 | A 答 | B 调用 | B hit | B 参数 | B 关联 | B 答 | B 协议 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sel-list-dir ×1（select） | vdfs_list | ✓ | ✓/✓ | – | — | vdfs_list | ✓ | ✓/✓ | – | `{"tool": "vdfs_list", "arguments": {"pat` | ✓ |
| sel-tree ×1（select） | vdfs_tree | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><function=vdfs_tree><paramete` | – |
| sel-read-file ×1（select） | vdfs_read | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><function=vdfs_read><paramete` | – |
| sel-stat ×1（select） | vdfs_stat | ✓ | ✓/✓ | – | — | vdfs_stat | ✓ | ✓/✓ | – | `{"tool": "vdfs_stat", "arguments": {"pat` | ✓ |
| sel-glob ×1（select） | vdfs_search | ✓ | ✓/✓ | – | — | vdfs_search | ✓ | ✓/✓ | – | `{"tool": "vdfs_search", "arguments": {"p` | ✓ |
| sel-grep ×1（select） | content_search | ✓ | ✓/✓ | – | — | （出错） | – | –/– | – | — | – |
| sel-write ×1（select） | vdfs_write | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><function=vdfs_write><paramet` | – |
| sel-edit ×1（select） | vdfs_read | ✗ | ✓/✗ | – | — | — | ✗ | –/– | – | `<tool_call><function=vdfs_read><paramete` | – |
| sel-delete ×1（select） | vdfs_stat | ✗ | ✓/✓ | – | — | vdfs_delete | ✓ | ✓/✓ | – | `{"tool": "vdfs_delete", "arguments": {"p` | ✓ |
| sel-websearch ×1（select） | web_search+web_search | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><function=web_search><paramet` | – |
| sel-webfetch ×1（select） | web_fetch | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><function=web_fetch><paramete` | – |
| sel-http ×1（select） | http_request | ✓ | ✓/✓ | – | — | http_request | ✓ | ✓/✓ | – | `{"tool": "http_request", "arguments": {"` | ✓ |
| sel-todo ×1（select） | todo_write | ✓ | ✓/✓ | – | — | todo_write | ✓ | ✓/✓ | – | `{"tool": "todo_write", "arguments": {"to` | ✓ |
| sel-agent ×1（select） | agent_run | ✓ | ✓/✓ | – | — | agent_run | ✓ | ✓/✓ | – | `{"tool": "agent_run", "arguments": {"pro` | ✓ |
| ctl-no-tool-math ×1（control） | — | – | –/– | – | `896` | — | – | –/– | – | `896` | – |
| ctl-no-tool-recall ×1（control） | — | – | –/– | – | `好的，我用自己的话解释一下： **【工作区记忆】**（`WORKSPACE.md` | — | – | –/– | – | `两者都是给"我"（这个智能体）长期记东西用的备忘录文件，但作用域和用途不一样： ` | – |
| ctl-no-tool-refuse ×1（control） | — | – | –/– | – | `明白` | — | – | –/– | – | `明白` | – |
| attr-out-of-order-same-name ×1（attribution） | — | – | –/– | ✗ | `plan` | — | – | –/– | ✗ | `plan` | – |
| attr-out-of-order-mirror ×1（attribution） | — | – | –/– | ✗ | `lib.rs` | — | – | –/– | ✗ | `lib.rs` | – |
| attr-three-way-permuted ×1（attribution） | — | – | –/– | ✓ | `Cargo.toml` | — | – | –/– | ✓ | `Cargo.toml` | – |
| attr-five-way-reversed ×1（attribution） | — | – | –/– | ✗ | `200` | — | – | –/– | ✗ | `200` | – |
| attr-pick-one-field ×1（attribution） | — | – | –/– | ✓ | `2048` | — | – | –/– | ✓ | `2048` | – |
| sel-list-dir ×2（select） | vdfs_list | ✓ | ✓/✓ | – | — | vdfs_list | ✓ | ✓/✓ | – | `{"tool": "vdfs_list", "arguments": {"pat` | ✓ |
| sel-tree ×2（select） | vdfs_tree | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><function=vdfs_tree><paramete` | – |
| sel-read-file ×2（select） | vdfs_read | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><function=vdfs_read><paramete` | – |
| sel-stat ×2（select） | vdfs_stat | ✓ | ✓/✓ | – | — | vdfs_stat | ✓ | ✓/✓ | – | `{"tool": "vdfs_stat", "arguments": {"pat` | ✓ |
| sel-glob ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-grep ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-write ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-edit ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-delete ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-websearch ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-webfetch ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-http ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-todo ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| sel-agent ×2（select） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| ctl-no-tool-math ×2（control） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| ctl-no-tool-recall ×2（control） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| ctl-no-tool-refuse ×2（control） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| attr-out-of-order-same-name ×2（attribution） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| attr-out-of-order-mirror ×2（attribution） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| attr-three-way-permuted ×2（attribution） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| attr-five-way-reversed ×2（attribution） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |
| attr-pick-one-field ×2（attribution） | （出错） | – | –/– | – | — | （出错） | – | –/– | – | — | – |

`类`：`select` = 该调哪个工具；`control` = 不该调用；`attribution` = 结果与调用的对应关系。
`hit` 后的 ⚠ = 调了工具面上不存在的名字。`参数` 列是 `合法JSON/必填键齐`。`—` = 未调用/未作答，`–` = 该题不适用。

## 逐题原文（臂各自实际送出与收到的东西）

### sel-list-dir（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2415ms · prompt 5847tok · 13674B
  - 送出：`vdfs_list({"path": "src"})`
- **B 拍平**：finish=stop · 2433ms · prompt 4822tok · 13842B
  - 送出：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
  - 正文：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
### sel-tree（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2235ms · prompt 5853tok · 13681B
  - 送出：`vdfs_tree({"depth": 2, "path": "docs"})`
- **B 拍平**：finish=stop · 3029ms · prompt 4828tok · 13849B
  - 正文：`<tool_call><function=vdfs_tree><parameter=depth>2</parameter><parameter=path>docs</parameter></function></tool_call>`
### sel-read-file（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 1890ms · prompt 5844tok · 13674B
  - 送出：`vdfs_read({"path": "Cargo.toml"})`
- **B 拍平**：finish=stop · 2553ms · prompt 4819tok · 13842B
  - 正文：`<tool_call><function=vdfs_read><parameter=path>Cargo.toml</parameter></function></tool_call>`
### sel-stat（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 6770ms · prompt 5857tok · 13691B
  - 送出：`vdfs_stat({"path": "README.md"})`
- **B 拍平**：finish=stop · 3124ms · prompt 4832tok · 13859B
  - 送出：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
  - 正文：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
### sel-glob（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2329ms · prompt 5851tok · 13679B
  - 送出：`vdfs_search({"pattern": "**/*.rs"})`
- **B 拍平**：finish=stop · 3593ms · prompt 4826tok · 13847B
  - 送出：`{"tool": "vdfs_search", "arguments": {"pattern": "**/*.rs"}}`
  - 正文：`{"tool": "vdfs_search", "arguments": {"pattern": "**/*.rs"}}`
### sel-grep（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2898ms · prompt 5853tok · 13685B
  - 送出：`content_search({"pattern": "fn main"})`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"process_concurrency_rate_limit_exceeded"}}}`
### sel-write（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 3219ms · prompt 5847tok · 13689B
  - 送出：`vdfs_write({"path": "notes/greeting.txt", "text": "hello"})`
- **B 拍平**：finish=stop · 3302ms · prompt 4822tok · 13857B
  - 正文：`<tool_call><function=vdfs_write><parameter=path>notes/greeting.txt</parameter><parameter=text>hello</parameter></function></tool_call>`
### sel-edit（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 3152ms · prompt 5855tok · 13690B
  - 送出：`vdfs_read({"path": "src/main.rs"})`
- **B 拍平**：finish=stop · 3290ms · prompt 4830tok · 13858B
  - 正文：`<tool_call><function=vdfs_read><parameter=path>src/main.rs</parameter></function></tool_call>`
### sel-delete（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 5803ms · prompt 5845tok · 13673B
  - 送出：`vdfs_stat({"path": "tmp/old.txt"})`
- **B 拍平**：finish=stop · 4742ms · prompt 4820tok · 13841B
  - 送出：`{"tool": "vdfs_delete", "arguments": {"path": "tmp/old.txt"}}`
  - 正文：`{"tool": "vdfs_delete", "arguments": {"path": "tmp/old.txt"}}`
### sel-websearch（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 3102ms · prompt 5845tok · 13683B
  - 送出：`web_search({"max_results": 10, "query": "rust async 生命周期 教程"}) + web_search({"max_results": 10, "query": "rust async lifetime tutorial"})`
- **B 拍平**：finish=stop · 5488ms · prompt 4820tok · 13851B
  - 正文：`<tool_call><function=web_search><parameter=query>rust async 生命周期 教程</parameter><parameter=max_results>10</parameter></function></tool_call><tool_call><function=`
### sel-webfetch（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2302ms · prompt 5848tok · 13687B
  - 送出：`web_fetch({"url": "https://example.com"})`
- **B 拍平**：finish=stop · 2770ms · prompt 4823tok · 13855B
  - 正文：`<tool_call><function=web_fetch><parameter=url>https://example.com</parameter></function></tool_call>`
### sel-http（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 1958ms · prompt 5851tok · 13706B
  - 送出：`http_request({"method": "GET", "url": "https://api.example.com/data"})`
- **B 拍平**：finish=stop · 2871ms · prompt 4826tok · 13874B
  - 送出：`{"tool": "http_request", "arguments": {"url": "https://api.example.com/data", "method": "GET"}}`
  - 正文：`{"tool": "http_request", "arguments": {"url": "https://api.example.com/data", "method": "GET"}}`
### sel-todo（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 4327ms · prompt 5856tok · 13683B
  - 送出：`todo_write({"todos": [{"content": "分析架构", "id": "arch", "priority": "high", "status": "pending"}, {"content": "拆分模块", "id": "mod", "priority": "high", "status": "pending"}, {"content": "补测试", "id": "test", "priority": "medium", "status": "pending"}]})`
- **B 拍平**：finish=stop · 7125ms · prompt 4831tok · 13851B
  - 送出：`{"tool": "todo_write", "arguments": {"todos": [{"content": "分析架构", "priority": "high", "status": "pending"}, {"content": "拆分模块", "priority": "high", "status": "pending"}, {"content": "补测试", "priority": "medium", "status": "pending"}]}}`
  - 正文：`{"tool": "todo_write", "arguments": {"todos": [{"content": "分析架构", "priority": "high", "status": "pending"}, {"content": "拆分模块", "priority": "high", "status": "`
### sel-agent（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 7501ms · prompt 5860tok · 13692B
  - 送出：`agent_run({"prompt": "请调研本仓库的压缩策略（compression strategy），并写一份小结。要求：\n1. 全面检索仓库中与压缩相关的内容（如 gzip/zlib/deflate/brotli/lz4/zstd、compress、archive、打包等关键词），弄清仓库使用了哪些压缩算法/库、在哪些模块使用、配置与参数（压缩级别、阈值等）。\n2. 分析压缩策略的设计意图与适用场景（如网络传输、存储、备份等），如有性能/体积权衡也请指出。\n3. 最终输出一份结构化的小结（Markdown 格式），包含：概述、涉及的模块/文件、压缩方式与参数、使用场景、优缺点`
- **B 拍平**：finish=stop · 10225ms · prompt 4835tok · 13860B
  - 送出：`{"tool": "agent_run", "arguments": {"prompt": "调研本仓库的压缩策略并写一份小结：请检索仓库中与压缩（compression、zip/gzip/zlib/deflate、zstd、pack、archive 等）相关的代码、配置与文档，梳理出本仓库采用了哪些压缩/打包策略（含触发时机、算法、参数、适用场景），最后输出一份结构化的小结（要点式，附关键文件路径与行号）。"}}`
  - 正文：`{"tool": "agent_run", "arguments": {"prompt": "调研本仓库的压缩策略并写一份小结：请检索仓库中与压缩（compression、zip/gzip/zlib/deflate、zstd、pack、archive 等）相关的代码、配置与文档，梳理出本仓库采用了哪些压缩/打包策略（含`
### ctl-no-tool-math（control）· 第 1 次
- **A 结构化**：finish=stop · 2038ms · prompt 5857tok · 13681B
  - 正文：`896`
- **B 拍平**：finish=stop · 2632ms · prompt 4832tok · 13849B
  - 正文：`896`
### ctl-no-tool-recall（control）· 第 1 次
- **A 结构化**：finish=stop · 10215ms · prompt 5867tok · 13707B
  - 正文：`好的，我用自己的话解释一下： **【工作区记忆】**（`WORKSPACE.md`） 这是整个工作区的"公共备忘录"。它记录的是**长期有效、对所有会话都适用**的内容，比如项目约定、常用偏好、环境事实、通用结论等。因为它是全工作区共享的，所以无论开多少个新会话，这些信息都能被继承和复用。也正因为它"公共"，改写前需要`
- **B 拍平**：finish=stop · 22890ms · prompt 4842tok · 13875B
  - 正文：`两者都是给"我"（这个智能体）长期记东西用的备忘录文件，但作用域和用途不一样： **工作区记忆（`WORKSPACE.md`）** - 存的是**整个工作区长期有效**的约定：比如项目偏好、常用结论、环境事实（路径、工具链版本之类）。 - 它是**跨会话共享**的：这个会话写下的东西，以后任何会话都还能读到。 - 适合`
### ctl-no-tool-refuse（control）· 第 1 次
- **A 结构化**：finish=stop · 3191ms · prompt 5854tok · 13682B
  - 正文：`明白`
- **B 拍平**：finish=stop · 2006ms · prompt 4829tok · 13850B
  - 正文：`明白`
### attr-out-of-order-same-name（attribution）· 第 1 次
- 测的是：同名调用 + 乱序结果 + 正文不回显路径 ⇒ 判据测的就是 tool_call_id 这一层关联
- 候选条数 2 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 2 分之一
- **A 结构化**：finish=stop · 3789ms · prompt 5965tok · 14294B
  - 正文：`plan`
- **B 拍平**：finish=stop · 2081ms · prompt 4948tok · 14276B
  - 正文：`plan`
### attr-out-of-order-mirror（attribution）· 第 1 次
- 测的是：同一上下文换问另一那次调用 ⇒ 与上一题配对，测的是「是否按到达顺序归因」而不是碰运气
- 候选条数 2 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 2 分之一
- **A 结构化**：finish=stop · 3032ms · prompt 5965tok · 14295B
  - 正文：`lib.rs`
- **B 拍平**：finish=stop · 3696ms · prompt 4948tok · 14277B
  - 正文：`lib.rs`
### attr-three-way-permuted（attribution）· 第 1 次
- 测的是：三次同名调用的关联面——一次对不上还能猜，三次全对上的概率才说明关联是不是真在读
- 候选条数 3 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 3 分之一
- **A 结构化**：finish=stop · 2356ms · prompt 5993tok · 14455B
  - 正文：`Cargo.toml`
- **B 拍平**：finish=stop · 1642ms · prompt 4978tok · 14373B
  - 正文：`Cargo.toml`
### attr-five-way-reversed（attribution）· 第 1 次
- 测的是：候选到 5 条且完全逆序 ⇒ 猜的基线降到 1/5，而「按到达顺序」的启发式在这里必然指错
- 候选条数 5 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 5 分之一
- **A 结构化**：finish=stop · 1107ms · prompt 6066tok · 14871B
  - 正文：`200`
- **B 拍平**：finish=stop · 1750ms · prompt 5060tok · 14607B
  - 正文：`200`
### attr-pick-one-field（attribution）· 第 1 次
- 测的是：结果是一段 JSON 文本，问其中一个字段 ⇒ 测「读回结果内容」而不是「读回关联」（单调用，无猜的空间）
- **A 结构化**：finish=stop · 1156ms · prompt 5948tok · 14104B
  - 正文：`2048`
- **B 拍平**：finish=stop · 1875ms · prompt 4929tok · 14182B
  - 正文：`2048`
### sel-list-dir（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2470ms · prompt 5847tok · 13674B
  - 送出：`vdfs_list({"path": "src"})`
- **B 拍平**：finish=stop · 2287ms · prompt 4822tok · 13842B
  - 送出：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
  - 正文：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
### sel-tree（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2427ms · prompt 5853tok · 13681B
  - 送出：`vdfs_tree({"depth": 2, "path": "docs"})`
- **B 拍平**：finish=stop · 2793ms · prompt 4828tok · 13849B
  - 正文：`<tool_call><function=vdfs_tree><parameter=depth>2</parameter><parameter=path>docs</parameter></function></tool_call>`
### sel-read-file（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 1765ms · prompt 5844tok · 13674B
  - 送出：`vdfs_read({"path": "Cargo.toml"})`
- **B 拍平**：finish=stop · 2439ms · prompt 4819tok · 13842B
  - 正文：`<tool_call><function=vdfs_read><parameter=path>Cargo.toml</parameter></function></tool_call>`
### sel-stat（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 1943ms · prompt 5857tok · 13691B
  - 送出：`vdfs_stat({"path": "README.md"})`
- **B 拍平**：finish=stop · 2710ms · prompt 4832tok · 13859B
  - 送出：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
  - 正文：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
### sel-glob（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
### sel-grep（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
### sel-write（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
### sel-edit（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
### sel-delete（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"error":{"message":"Rate limit exceeded: maximum 20 requests per minute for this OneClick user. Please try again later.","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}`
### sel-websearch（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### sel-webfetch（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### sel-http（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### sel-todo（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### sel-agent（select）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### ctl-no-tool-math（control）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### ctl-no-tool-recall（control）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### ctl-no-tool-refuse（control）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### attr-out-of-order-same-name（attribution）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### attr-out-of-order-mirror（attribution）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### attr-three-way-permuted（attribution）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### attr-five-way-reversed（attribution）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
### attr-pick-one-field（attribution）· 第 2 次
- **A 结构化**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
- **B 拍平**：finish=– · –ms · prompt –tok · –B
  - 错误：`HTTP 429: {"detail":{"error":{"message":"Model API rate limit exceeded; please retry later","type":"rate_limit_error","code":"token_rate_limit_exceeded"}}}`
