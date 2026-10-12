# 结构化 vs 拍平：真实模型上的对照读数

> ⚠️ **生成物，不手改。** 唯一写者是 `node e2e/ab/run.mjs`。
> 判据口径归 `e2e/ab/tasks.mjs` 与 `e2e/ab/flatten.mjs`，
> **结论与边界**归 [`docs/task/T-048.md`](../task/T-048.md)（本文件不写结论）。

生成于 2026-10-12T05:48:01.497Z · 模型 `MiMo-V2.6-Flash`（provider `AMD-MiMo-V2.6-Flash`）· 题 22 道 × 两臂 × 2 次
温度 0（两臂同值，实验只认「两臂一样」不认「出厂默认」）· max_tokens 4096 · 两臂的信封差别只有 `tools` / `tool_choice` 是否存在
上下文来源：真实链路捕获（场景「full-主线装配-工具轮-对话面关闭」，captured 2026-10-12T05:20:54.344Z）· system 前缀 2 条 · tools 21 个

## 汇总（判据列）

| 判据 | 臂 A 结构化（出厂） | 臂 B 拍平 |
|---|---|---|
| select：选对工具（`tool_name_hit`） | 24/28 | 24/27 |
| select：参数是合法 JSON（`args_json_ok`） | 28/28 | 26/26 |
| select：必填键齐全（`required_keys_hit`） | 26/28 | 25/26 |
| select：调了不存在的工具名（越少越好） | 0/28 | 0/27 |
| control：不该调用却调用（越少越好） | 0/6 | 0/6 |
| attribution：结果↔调用对应得回来 | 4/10 | 4/10 |
| 拍平侧：文本协议被遵守（臂 A 不适用） | — | 20/26 |
| 拍平侧：正文漏出模型内部调用格式（越少越好） | — | 6/43 |

## 观测列（**不进判据**，与 ADR-057 同一取舍）

| | 臂 A | 臂 B |
|---|---|---|
| prompt token 合计 | 258854 | 213856 |
| 请求体字节合计 | 609352 | 614456 |
| 墙钟合计（ms） | 169768 | 407155 |
| provider 自报账单（usage.cost，合计） | 0.001804 | 0.002673 |
| 未收束的题数（已从判据分母剔除） | 0 | 1 |
| 请求出错的题数 | 0 | 0 |

**样本健康度**：两臂合计 1 道题未收束、0 道请求出错；重复之间可比对 43 组，其中 2 组换一次时刻就跑出了不同结论。「未收束」不计入上面任何一条判据的分母——本机服务坏过一整段时会把回答刷成 `<unused32>` 复读，把它读成「模型没做到」就是拿环境噪声当读数。

## 逐题（同一道题两臂并排；`×k` 是第 k 次重复）

| 题（臂 A vs 臂 B） | A 调用 | A hit | A 参数 | A 关联 | A 答 | B 调用 | B hit | B 参数 | B 关联 | B 答 | B 协议 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sel-list-dir ×1（select） | vdfs_list | ✓ | ✓/✓ | – | — | vdfs_list | ✓ | ✓/✓ | – | `{"tool": "vdfs_list", "arguments": {"pat` | ✓ |
| sel-tree ×1（select） | vdfs_tree | ✓ | ✓/✓ | – | — | vdfs_tree | ✓ | ✓/✓ | – | `<tool_call><function=vdfs_tree><paramete` | ✗ |
| sel-read-file ×1（select） | vdfs_read | ✓ | ✓/✓ | – | — | vdfs_read | ✓ | ✓/✓ | – | `<tool_call><function=vdfs_read><paramete` | ✗ |
| sel-stat ×1（select） | vdfs_stat | ✓ | ✓/✓ | – | — | vdfs_stat | ✓ | ✓/✓ | – | `{"tool": "vdfs_stat", "arguments": {"pat` | ✓ |
| sel-glob ×1（select） | vdfs_search | ✓ | ✓/✓ | – | — | vdfs_search | ✓ | ✓/✓ | – | `{"tool": "vdfs_search", "arguments": {"p` | ✓ |
| sel-grep ×1（select） | content_search | ✓ | ✓/✓ | – | — | content_search | ✓ | ✓/✓ | – | `{"tool": "content_search", "arguments": ` | ✓ |
| sel-write ×1（select） | vdfs_write | ✓ | ✓/✓ | – | — | vdfs_write | ✓ | ✓/✓ | – | `{"tool": "vdfs_write", "arguments": {"pa` | ✓ |
| sel-edit ×1（select） | vdfs_read | ✗ | ✓/✗ | – | — | vdfs_read | ✗ | ✓/✗ | – | `<tool_call><function=vdfs_read><paramete` | ✗ |
| sel-delete ×1（select） | vdfs_stat | ✗ | ✓/✓ | – | — | vdfs_stat | ✗ | ✓/✓ | – | `<tool_call><function=vdfs_stat><paramete` | ✗ |
| sel-websearch ×1（select） | web_search+web_search | ✓ | ✓/✓ | – | — | web_search | ✓ | ✓/✓ | – | `{"tool": "web_search", "arguments": {"qu` | ✓ |
| sel-webfetch ×1（select） | web_fetch | ✓ | ✓/✓ | – | — | web_fetch | ✓ | ✓/✓ | – | `{"tool": "web_fetch", "arguments": {"url` | ✓ |
| sel-http ×1（select） | http_request | ✓ | ✓/✓ | – | — | http_request | ✓ | ✓/✓ | – | `{"tool": "http_request", "arguments": {"` | ✓ |
| sel-todo ×1（select） | todo_write | ✓ | ✓/✓ | – | — | todo_write | ✓ | ✓/✓ | – | `{"tool": "todo_write", "arguments": {"to` | ✓ |
| sel-agent ×1（select） | agent_run | ✓ | ✓/✓ | – | — | agent_run | ✓ | ✓/✓ | – | `{"tool": "agent_run", "arguments": {"pro` | ✓ |
| ctl-no-tool-math ×1（control） | — | – | –/– | – | `896` | — | – | –/– | – | `896` | – |
| ctl-no-tool-recall ×1（control） | — | – | –/– | – | `好的，我用自己的话解释一下这两者的区别： **【工作区记忆】**（`WORKSP` | — | – | –/– | – | `这两者都是给我的"便签本"，但作用域和寿命不一样： **工作区记忆**（`WOR` | – |
| ctl-no-tool-refuse ×1（control） | — | – | –/– | – | `明白` | — | – | –/– | – | `明白` | – |
| attr-out-of-order-same-name ×1（attribution） | — | – | –/– | ✗ | `plan` | — | – | –/– | ✗ | `plan` | – |
| attr-out-of-order-mirror ×1（attribution） | — | – | –/– | ✗ | `lib.rs` | — | – | –/– | ✗ | `lib.rs` | – |
| attr-three-way-permuted ×1（attribution） | — | – | –/– | ✓ | `Cargo.toml` | — | – | –/– | ✓ | `Cargo.toml` | – |
| attr-five-way-reversed ×1（attribution） | — | – | –/– | ✗ | `200` | — | – | –/– | ✗ | `200` | – |
| attr-pick-one-field ×1（attribution） | — | – | –/– | ✓ | `2048` | — | – | –/– | ✓ | `2048` | – |
| sel-list-dir ×2（select） | vdfs_list | ✓ | ✓/✓ | – | — | vdfs_list | ✓ | ✓/✓ | – | `{"tool": "vdfs_list", "arguments": {"pat` | ✓ |
| sel-tree ×2（select） | vdfs_tree | ✓ | ✓/✓ | – | — | — | ✗ | –/– | – | `<tool_call><vdfs_tree>{"path": "docs", "` | – |
| sel-read-file ×2（select） | vdfs_read | ✓ | ✓/✓ | – | — | vdfs_read | ✓ | ✓/✓ | – | `<tool_call><function=vdfs_read><paramete` | ✗ |
| sel-stat ×2（select） | vdfs_stat | ✓ | ✓/✓ | – | — | vdfs_stat | ✓ | ✓/✓ | – | `{"tool": "vdfs_stat", "arguments": {"pat` | ✓ |
| sel-glob ×2（select） | vdfs_search | ✓ | ✓/✓ | – | — | vdfs_search | ✓ | ✓/✓ | – | `{"tool": "vdfs_search", "arguments": {"p` | ✓ |
| sel-grep ×2（select） | content_search | ✓ | ✓/✓ | – | — | content_search | ✓ | ✓/✓ | – | `{"tool": "content_search", "arguments": ` | ✓ |
| sel-write ×2（select） | vdfs_write | ✓ | ✓/✓ | – | — | vdfs_write | ✓ | ✓/✓ | – | `{"tool": "vdfs_write", "arguments": {"pa` | ✓ |
| sel-edit ×2（select） | vdfs_read | ✗ | ✓/✗ | – | — | — | – | –/– | – | （未收束） | – |
| sel-delete ×2（select） | vdfs_stat | ✗ | ✓/✓ | – | — | vdfs_delete | ✓ | ✓/✓ | – | `{"tool": "vdfs_delete", "arguments": {"p` | ✓ |
| sel-websearch ×2（select） | web_search+web_search | ✓ | ✓/✓ | – | — | web_search | ✓ | ✓/✓ | – | `{"tool": "web_search", "arguments": {"qu` | ✓ |
| sel-webfetch ×2（select） | web_fetch | ✓ | ✓/✓ | – | — | web_fetch | ✓ | ✓/✓ | – | `<tool_call><function=web_fetch><paramete` | ✗ |
| sel-http ×2（select） | http_request | ✓ | ✓/✓ | – | — | http_request | ✓ | ✓/✓ | – | `{"tool": "http_request", "arguments": {"` | ✓ |
| sel-todo ×2（select） | todo_write | ✓ | ✓/✓ | – | — | todo_write | ✓ | ✓/✓ | – | `{"tool": "todo_write", "arguments": {"to` | ✓ |
| sel-agent ×2（select） | agent_run | ✓ | ✓/✓ | – | — | agent_run | ✓ | ✓/✓ | – | `{"tool": "agent_run", "arguments": {"pro` | ✓ |
| ctl-no-tool-math ×2（control） | — | – | –/– | – | `896` | — | – | –/– | – | `896` | – |
| ctl-no-tool-recall ×2（control） | — | – | –/– | – | `好的，我用自己的话解释一下这两者的区别： **【工作区记忆】**（`WORKSP` | — | – | –/– | – | `好的，不调工具，我直接说说我自己的理解： **工作区记忆（WORKSPACE.m` | – |
| ctl-no-tool-refuse ×2（control） | — | – | –/– | – | `明白` | — | – | –/– | – | `明白` | – |
| attr-out-of-order-same-name ×2（attribution） | — | – | –/– | ✗ | `plan` | — | – | –/– | ✗ | `plan` | – |
| attr-out-of-order-mirror ×2（attribution） | — | – | –/– | ✗ | `lib.rs` | — | – | –/– | ✗ | `lib.rs` | – |
| attr-three-way-permuted ×2（attribution） | — | – | –/– | ✓ | `Cargo.toml` | — | – | –/– | ✓ | `Cargo.toml` | – |
| attr-five-way-reversed ×2（attribution） | — | – | –/– | ✗ | `200` | — | – | –/– | ✗ | `200` | – |
| attr-pick-one-field ×2（attribution） | — | – | –/– | ✓ | `2048` | — | – | –/– | ✓ | `2048` | – |

`类`：`select` = 该调哪个工具；`control` = 不该调用；`attribution` = 结果与调用的对应关系。
`hit` 后的 ⚠ = 调了工具面上不存在的名字。`参数` 列是 `合法JSON/必填键齐`。`—` = 未调用/未作答，`–` = 该题不适用。

重复之间的判据翻转：sel-tree（B：true / false）、sel-delete（B：false / true）——同一道题同一臂换个时刻跑，结论会变，读数请按「43 组可比对里有 2 组翻过」来看，不要当逐题定论。

## 逐题原文（臂各自实际送出与收到的东西）

### sel-list-dir（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2614ms · prompt 5847tok · 13674B
  - 送出：`vdfs_list({"path": "src"})`
- **B 拍平**：finish=stop · 2405ms · prompt 4822tok · 13842B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
  - 正文：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
### sel-tree（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2458ms · prompt 5853tok · 13681B
  - 送出：`vdfs_tree({"depth": 2, "path": "docs"})`
- **B 拍平**：finish=stop · 2734ms · prompt 4828tok · 13849B · 调用从「native-markup」读回
  - 送出：`<|tool_call|><function=vdfs_tree><parameter=depth>2</parameter><parameter=path>docs</parameter></function>`
  - 正文：`<tool_call><function=vdfs_tree><parameter=depth>2</parameter><parameter=path>docs</parameter></function></tool_call>`
### sel-read-file（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2041ms · prompt 5844tok · 13674B
  - 送出：`vdfs_read({"path": "Cargo.toml"})`
- **B 拍平**：finish=stop · 2295ms · prompt 4819tok · 13842B · 调用从「native-markup」读回
  - 送出：`<|tool_call|><function=vdfs_read><parameter=path>Cargo.toml</parameter></function>`
  - 正文：`<tool_call><function=vdfs_read><parameter=path>Cargo.toml</parameter></function></tool_call>`
### sel-stat（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2844ms · prompt 5857tok · 13691B
  - 送出：`vdfs_stat({"path": "README.md"})`
- **B 拍平**：finish=stop · 2976ms · prompt 4832tok · 13859B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
  - 正文：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
### sel-glob（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2916ms · prompt 5851tok · 13679B
  - 送出：`vdfs_search({"pattern": "**/*.rs"})`
- **B 拍平**：finish=stop · 2635ms · prompt 4826tok · 13847B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_search", "arguments": {"pattern": "**/*.rs"}}`
  - 正文：`{"tool": "vdfs_search", "arguments": {"pattern": "**/*.rs"}}`
### sel-grep（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2568ms · prompt 5853tok · 13685B
  - 送出：`content_search({"pattern": "fn main"})`
- **B 拍平**：finish=stop · 3269ms · prompt 4828tok · 13853B · 调用从「text-protocol」读回
  - 送出：`{"tool": "content_search", "arguments": {"pattern": "fn main"}}`
  - 正文：`{"tool": "content_search", "arguments": {"pattern": "fn main"}}`
### sel-write（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2556ms · prompt 5847tok · 13689B
  - 送出：`vdfs_write({"path": "notes/greeting.txt", "text": "hello"})`
- **B 拍平**：finish=stop · 2810ms · prompt 4822tok · 13857B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_write", "arguments": {"path": "notes/greeting.txt", "text": "hello"}}`
  - 正文：`{"tool": "vdfs_write", "arguments": {"path": "notes/greeting.txt", "text": "hello"}}`
### sel-edit（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 3502ms · prompt 5855tok · 13690B
  - 送出：`vdfs_read({"path": "src/main.rs"})`
- **B 拍平**：finish=stop · 4481ms · prompt 4830tok · 13858B · 调用从「native-markup」读回
  - 送出：`<|tool_call|><function=vdfs_read><parameter=path>src/main.rs</parameter></function>`
  - 正文：`<tool_call><function=vdfs_read><parameter=path>src/main.rs</parameter></function></tool_call>`
### sel-delete（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 11032ms · prompt 5845tok · 13673B
  - 送出：`vdfs_stat({"path": "tmp/old.txt"})`
- **B 拍平**：finish=stop · 6251ms · prompt 4820tok · 13841B · 调用从「native-markup」读回
  - 送出：`<|tool_call|><function=vdfs_stat><parameter=path>tmp/old.txt</parameter></function>`
  - 正文：`<tool_call><function=vdfs_stat><parameter=path>tmp/old.txt</parameter></function></tool_call>`
### sel-websearch（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 4141ms · prompt 5845tok · 13683B
  - 送出：`web_search({"lr": "zh-CN", "max_results": 10, "query": "Rust async 生命周期 教程"}) + web_search({"max_results": 10, "query": "Rust async lifetime tutorial"})`
- **B 拍平**：finish=stop · 3047ms · prompt 4820tok · 13851B · 调用从「text-protocol」读回
  - 送出：`{"tool": "web_search", "arguments": {"query": "rust async 生命周期 教程", "max_results": 10}}`
  - 正文：`{"tool": "web_search", "arguments": {"query": "rust async 生命周期 教程", "max_results": 10}}`
### sel-webfetch（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 2717ms · prompt 5848tok · 13687B
  - 送出：`web_fetch({"url": "https://example.com"})`
- **B 拍平**：finish=stop · 2190ms · prompt 4823tok · 13855B · 调用从「text-protocol」读回
  - 送出：`{"tool": "web_fetch", "arguments": {"url": "https://example.com"}}`
  - 正文：`{"tool": "web_fetch", "arguments": {"url": "https://example.com"}}`
### sel-http（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 3147ms · prompt 5851tok · 13706B
  - 送出：`http_request({"method": "GET", "url": "https://api.example.com/data"})`
- **B 拍平**：finish=stop · 2868ms · prompt 4826tok · 13874B · 调用从「text-protocol」读回
  - 送出：`{"tool": "http_request", "arguments": {"method": "GET", "url": "https://api.example.com/data"}}`
  - 正文：`{"tool": "http_request", "arguments": {"method": "GET", "url": "https://api.example.com/data"}}`
### sel-todo（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 4729ms · prompt 5856tok · 13683B
  - 送出：`todo_write({"todos": [{"content": "分析架构", "priority": "high", "status": "pending"}, {"content": "拆分模块", "priority": "high", "status": "pending"}, {"content": "补测试", "priority": "medium", "status": "pending"}]})`
- **B 拍平**：finish=stop · 6667ms · prompt 4831tok · 13851B · 调用从「text-protocol」读回
  - 送出：`{"tool": "todo_write", "arguments": {"todos": [{"content": "分析架构", "priority": "high", "status": "pending"}, {"content": "拆分模块", "priority": "high", "status": "pending"}, {"content": "补测试", "priority": "high", "status": "pending"}]}}`
  - 正文：`{"tool": "todo_write", "arguments": {"todos": [{"content": "分析架构", "priority": "high", "status": "pending"}, {"content": "拆分模块", "priority": "high", "status": "`
### sel-agent（select）· 第 1 次
- **A 结构化**：finish=tool_calls · 12130ms · prompt 5860tok · 13692B
  - 送出：`agent_run({"prompt": "请调研当前工作目录下这个仓库的「压缩策略」（compression strategy），并写一份小结。具体要求：\n\n1. 检索仓库中与压缩相关的实现与配置，例如：gzip/brotli/deflate/zstd 等压缩算法的使用、HTTP 压缩中间件、构建产物压缩、归档/打包（zip/tar/gz）、数据序列化压缩、数据库或消息队列的压缩选项等。\n2. 关注点包括：在哪里启用、压缩算法与级别、触发条件（如阈值、Content-Type、编码协商）、涉及的关键文件与代码位置（附文件路径和行号）、以及默认开关状态。\n3. 如果仓库中确实没有压缩相`
- **B 拍平**：finish=stop · 11434ms · prompt 4835tok · 13860B · 调用从「text-protocol」读回
  - 送出：`{"tool": "agent_run", "arguments": {"prompt": "调研本仓库的压缩策略并写一份小结：请检索仓库中与「压缩」（如 compression、gzip、deflate、zstd、brotli、zip、archive、打包、瘦身等关键词）相关的代码与配置，梳理出本仓库使用了哪些压缩/打包策略、在哪些环节（构建、传输、存储、日志、会话记忆等）生效、相关实现位置（文件路径与关键函数），并输出一份结构化的小结报告（含结论与依据）。"}}`
  - 正文：`{"tool": "agent_run", "arguments": {"prompt": "调研本仓库的压缩策略并写一份小结：请检索仓库中与「压缩」（如 compression、gzip、deflate、zstd、brotli、zip、archive、打包、瘦身等关键词）相关的代码与配置，梳理出本仓库使用了哪些压缩/`
### ctl-no-tool-math（control）· 第 1 次
- **A 结构化**：finish=stop · 2111ms · prompt 5857tok · 13681B
  - 正文：`896`
- **B 拍平**：finish=stop · 3249ms · prompt 4832tok · 13849B · 调用从「prose」读回
  - 正文：`896`
### ctl-no-tool-recall（control）· 第 1 次
- **A 结构化**：finish=stop · 14247ms · prompt 5867tok · 13707B
  - 正文：`好的，我用自己的话解释一下这两者的区别： **【工作区记忆】**（`WORKSPACE.md`） 可以理解为这个工作区的"公共白板"或"团队共享笔记"。它对整个工作区里的所有会话、所有智能体都生效，写进去的内容是长期有效的"全局约定"——比如项目偏好、通用结论、环境特点（例如"这个项目统一用 pnpm"、"测试前必须先`
- **B 拍平**：finish=stop · 15581ms · prompt 4842tok · 13875B · 调用从「prose」读回
  - 正文：`这两者都是给我的"便签本"，但作用域和寿命不一样： **工作区记忆**（`WORKSPACE.md`）——是给**这个工作目录/项目**长期用的备忘。它跨会话存在：今天记下的东西，明天新开一个会话还在。适合放那些"只要在这个项目里干活就一直成立"的内容，比如项目约定、我的偏好、环境事实、既定结论。它是整个工作区共享的，`
### ctl-no-tool-refuse（control）· 第 1 次
- **A 结构化**：finish=stop · 1538ms · prompt 5854tok · 13682B
  - 正文：`明白`
- **B 拍平**：finish=stop · 1649ms · prompt 4829tok · 13850B · 调用从「prose」读回
  - 正文：`明白`
### attr-out-of-order-same-name（attribution）· 第 1 次
- 测的是：同名调用 + 乱序结果 + 正文不回显路径 ⇒ 判据测的就是 tool_call_id 这一层关联
- 候选条数 2 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 2 分之一
- **A 结构化**：finish=stop · 6256ms · prompt 5965tok · 14294B
  - 正文：`plan`
- **B 拍平**：finish=stop · 2345ms · prompt 4948tok · 14276B · 调用从「prose」读回
  - 正文：`plan`
### attr-out-of-order-mirror（attribution）· 第 1 次
- 测的是：同一上下文换问另一那次调用 ⇒ 与上一题配对，测的是「是否按到达顺序归因」而不是碰运气
- 候选条数 2 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 2 分之一
- **A 结构化**：finish=stop · 2620ms · prompt 5965tok · 14295B
  - 正文：`lib.rs`
- **B 拍平**：finish=stop · 4670ms · prompt 4948tok · 14277B · 调用从「prose」读回
  - 正文：`lib.rs`
### attr-three-way-permuted（attribution）· 第 1 次
- 测的是：三次同名调用的关联面——一次对不上还能猜，三次全对上的概率才说明关联是不是真在读
- 候选条数 3 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 3 分之一
- **A 结构化**：finish=stop · 2065ms · prompt 5993tok · 14455B
  - 正文：`Cargo.toml`
- **B 拍平**：finish=stop · 1522ms · prompt 4978tok · 14373B · 调用从「prose」读回
  - 正文：`Cargo.toml`
### attr-five-way-reversed（attribution）· 第 1 次
- 测的是：候选到 5 条且完全逆序 ⇒ 猜的基线降到 1/5，而「按到达顺序」的启发式在这里必然指错
- 候选条数 5 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 5 分之一
- **A 结构化**：finish=stop · 1479ms · prompt 6066tok · 14871B
  - 正文：`200`
- **B 拍平**：finish=stop · 6197ms · prompt 5060tok · 14607B · 调用从「prose」读回
  - 正文：`200`
### attr-pick-one-field（attribution）· 第 1 次
- 测的是：结果是一段 JSON 文本，问其中一个字段 ⇒ 测「读回结果内容」而不是「读回关联」（单调用，无猜的空间）
- **A 结构化**：finish=stop · 1862ms · prompt 5948tok · 14104B
  - 正文：`2048`
- **B 拍平**：finish=stop · 1890ms · prompt 4929tok · 14182B · 调用从「prose」读回
  - 正文：`2048`
### sel-list-dir（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2459ms · prompt 5847tok · 13674B
  - 送出：`vdfs_list({"path": "src"})`
- **B 拍平**：finish=stop · 2612ms · prompt 4822tok · 13842B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
  - 正文：`{"tool": "vdfs_list", "arguments": {"path": "src"}}`
### sel-tree（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 3359ms · prompt 5853tok · 13681B
  - 送出：`vdfs_tree({"depth": 2, "path": "docs"})`
- **B 拍平**：finish=stop · 3792ms · prompt 4828tok · 13849B · 调用从「prose」读回
  - 正文：`<tool_call><vdfs_tree>{"path": "docs", "depth": 2}</function></tool_call>`
### sel-read-file（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 1935ms · prompt 5844tok · 13674B
  - 送出：`vdfs_read({"path": "Cargo.toml"})`
- **B 拍平**：finish=stop · 2592ms · prompt 4819tok · 13842B · 调用从「native-markup」读回
  - 送出：`<|tool_call|><function=vdfs_read><parameter=path>Cargo.toml</parameter></function>`
  - 正文：`<tool_call><function=vdfs_read><parameter=path>Cargo.toml</parameter></function></tool_call>`
### sel-stat（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2968ms · prompt 5857tok · 13691B
  - 送出：`vdfs_stat({"path": "README.md"})`
- **B 拍平**：finish=stop · 2891ms · prompt 4832tok · 13859B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
  - 正文：`{"tool": "vdfs_stat", "arguments": {"path": "README.md"}}`
### sel-glob（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2416ms · prompt 5851tok · 13679B
  - 送出：`vdfs_search({"pattern": "**/*.rs"})`
- **B 拍平**：finish=stop · 3620ms · prompt 4826tok · 13847B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_search", "arguments": {"pattern": "**/*.rs"}}`
  - 正文：`{"tool": "vdfs_search", "arguments": {"pattern": "**/*.rs"}}`
### sel-grep（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2266ms · prompt 5853tok · 13685B
  - 送出：`content_search({"output_mode": "content", "pattern": "fn main"})`
- **B 拍平**：finish=stop · 5671ms · prompt 4828tok · 13853B · 调用从「text-protocol」读回
  - 送出：`{"tool": "content_search", "arguments": {"pattern": "fn main"}}`
  - 正文：`{"tool": "content_search", "arguments": {"pattern": "fn main"}}`
### sel-write（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 3453ms · prompt 5847tok · 13689B
  - 送出：`vdfs_write({"path": "notes/greeting.txt", "text": "hello"})`
- **B 拍平**：finish=stop · 2986ms · prompt 4822tok · 13857B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_write", "arguments": {"path": "notes/greeting.txt", "text": "hello"}}`
  - 正文：`{"tool": "vdfs_write", "arguments": {"path": "notes/greeting.txt", "text": "hello"}}`
### sel-edit（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 4106ms · prompt 5855tok · 13690B
  - 送出：`vdfs_read({"path": "src/main.rs"})`
- **B 拍平**：finish=length · 206632ms · prompt 4830tok · 13858B · 调用从「prose」读回
  - 正文：`<tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_call><tool_`
### sel-delete（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 4725ms · prompt 5845tok · 13673B
  - 送出：`vdfs_stat({"path": "tmp/old.txt"})`
- **B 拍平**：finish=stop · 8680ms · prompt 4820tok · 13841B · 调用从「text-protocol」读回
  - 送出：`{"tool": "vdfs_delete", "arguments": {"path": "tmp/old.txt"}}`
  - 正文：`{"tool": "vdfs_delete", "arguments": {"path": "tmp/old.txt"}}`
### sel-websearch（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 4150ms · prompt 5845tok · 13683B
  - 送出：`web_search({"lr": "zh-CN", "max_results": 10, "query": "rust async 生命周期 教程"}) + web_search({"max_results": 10, "query": "rust async lifetime tutorial"})`
- **B 拍平**：finish=stop · 3758ms · prompt 4820tok · 13851B · 调用从「text-protocol」读回
  - 送出：`{"tool": "web_search", "arguments": {"query": "rust async lifetime tutorial", "max_results": 10}}`
  - 正文：`{"tool": "web_search", "arguments": {"query": "rust async lifetime tutorial", "max_results": 10}}`
### sel-webfetch（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2403ms · prompt 5848tok · 13687B
  - 送出：`web_fetch({"url": "https://example.com"})`
- **B 拍平**：finish=stop · 2229ms · prompt 4823tok · 13855B · 调用从「native-markup」读回
  - 送出：`<|tool_call|><function=web_fetch><parameter=url>https://example.com</parameter></function>`
  - 正文：`<tool_call><function=web_fetch><parameter=url>https://example.com</parameter></function></tool_call>`
### sel-http（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 2786ms · prompt 5851tok · 13706B
  - 送出：`http_request({"method": "GET", "url": "https://api.example.com/data"})`
- **B 拍平**：finish=stop · 3290ms · prompt 4826tok · 13874B · 调用从「text-protocol」读回
  - 送出：`{"tool": "http_request", "arguments": {"url": "https://api.example.com/data", "method": "GET"}}`
  - 正文：`{"tool": "http_request", "arguments": {"url": "https://api.example.com/data", "method": "GET"}}`
### sel-todo（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 4795ms · prompt 5856tok · 13683B
  - 送出：`todo_write({"todos": [{"content": "分析架构", "id": "arch", "priority": "high", "status": "pending"}, {"content": "拆分模块", "id": "mod", "priority": "high", "status": "pending"}, {"content": "补测试", "id": "test", "priority": "medium", "status": "pending"}]})`
- **B 拍平**：finish=stop · 10571ms · prompt 4831tok · 13851B · 调用从「text-protocol」读回
  - 送出：`{"tool": "todo_write", "arguments": {"todos": [{"id": "task_1", "content": "分析架构", "priority": "high", "status": "in_progress"}, {"id": "task_2", "content": "拆分模块", "depends_on": ["task_1"], "priority": "high", "status": "pending"}, {"id": "task_3", "content": "补测试", "depends_on": ["task_2"], "prior`
  - 正文：`{"tool": "todo_write", "arguments": {"todos": [{"id": "task_1", "content": "分析架构", "priority": "high", "status": "in_progress"}, {"id": "task_2", "content": "拆分`
### sel-agent（select）· 第 2 次
- **A 结构化**：finish=tool_calls · 7521ms · prompt 5860tok · 13692B
  - 送出：`agent_run({"prompt": "请调研当前工作仓库的「压缩策略」（例如：代码/数据/资源中使用的压缩算法、gzip/zlib/lz4/deflate 等的使用位置、压缩参数配置、相关工具链或构建脚本中的压缩步骤等），并写一份小结。要求：\n1. 使用仓库内的检索工具（如 codebase_search、content_search、vdfs_search 等）定位相关代码与配置；\n2. 小结需包含：压缩策略的总体结论、关键实现位置（文件路径+简要说明）、使用的算法/库与参数、潜在的优化点或注意事项；\n3. 最终回复请给出完整的小结文本（Markdown 格式），不要只说\"已完`
- **B 拍平**：finish=stop · 21912ms · prompt 4835tok · 13860B · 调用从「text-protocol」读回
  - 送出：`{"tool": "agent_run", "arguments": {"prompt": "调研本仓库的压缩策略并写一份小结：请检索仓库中与压缩（compression/zip/gzip/zlib/deflate/brotli/lz 等）相关的代码、配置与文档，梳理当前使用的压缩方案、触发场景、参数与实现位置，最后输出一份结构化的小结（含关键文件路径与行号）。"}}`
  - 正文：`{"tool": "agent_run", "arguments": {"prompt": "调研本仓库的压缩策略并写一份小结：请检索仓库中与压缩（compression/zip/gzip/zlib/deflate/brotli/lz 等）相关的代码、配置与文档，梳理当前使用的压缩方案、触发场景、参数与实现位置，最后输`
### ctl-no-tool-math（control）· 第 2 次
- **A 结构化**：finish=stop · 1741ms · prompt 5857tok · 13681B
  - 正文：`896`
- **B 拍平**：finish=stop · 2149ms · prompt 4832tok · 13849B · 调用从「prose」读回
  - 正文：`896`
### ctl-no-tool-recall（control）· 第 2 次
- **A 结构化**：finish=stop · 12064ms · prompt 5867tok · 13707B
  - 正文：`好的，我用自己的话解释一下这两者的区别： **【工作区记忆】**（`WORKSPACE.md`） 可以理解为这个"项目/工作区"的公共备忘录。它面向的是**所有在这个工作区里工作的会话和智能体**，里面写的东西是长期有效的——比如团队约定、项目偏好、环境事实、通用结论等。不管我以后开新会话，还是别的智能体接手，读到它就`
- **B 拍平**：finish=stop · 17121ms · prompt 4842tok · 13875B · 调用从「prose」读回
  - 正文：`好的，不调工具，我直接说说我自己的理解： **工作区记忆（WORKSPACE.md）** 它是整个工作区的"长期公共备忘录"。只要是在这个工作区里干活——不管是这次会话还是以后别的会话——大家共享同一份。适合放那些**跨会话长期有效**的东西，比如：这个项目的固定约定、我的使用偏好、反复确认过的结论、环境事实（比如"这`
### ctl-no-tool-refuse（control）· 第 2 次
- **A 结构化**：finish=stop · 1680ms · prompt 5854tok · 13682B
  - 正文：`明白`
- **B 拍平**：finish=stop · 2403ms · prompt 4829tok · 13850B · 调用从「prose」读回
  - 正文：`明白`
### attr-out-of-order-same-name（attribution）· 第 2 次
- 测的是：同名调用 + 乱序结果 + 正文不回显路径 ⇒ 判据测的就是 tool_call_id 这一层关联
- 候选条数 2 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 2 分之一
- **A 结构化**：finish=stop · 3423ms · prompt 5965tok · 14294B
  - 正文：`plan`
- **B 拍平**：finish=stop · 2036ms · prompt 4948tok · 14276B · 调用从「prose」读回
  - 正文：`plan`
### attr-out-of-order-mirror（attribution）· 第 2 次
- 测的是：同一上下文换问另一那次调用 ⇒ 与上一题配对，测的是「是否按到达顺序归因」而不是碰运气
- 候选条数 2 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 2 分之一
- **A 结构化**：finish=stop · 4579ms · prompt 5965tok · 14295B
  - 正文：`lib.rs`
- **B 拍平**：finish=stop · 3791ms · prompt 4948tok · 14277B · 调用从「prose」读回
  - 正文：`lib.rs`
### attr-three-way-permuted（attribution）· 第 2 次
- 测的是：三次同名调用的关联面——一次对不上还能猜，三次全对上的概率才说明关联是不是真在读
- 候选条数 3 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 3 分之一
- **A 结构化**：finish=stop · 1469ms · prompt 5993tok · 14455B
  - 正文：`Cargo.toml`
- **B 拍平**：finish=stop · 1552ms · prompt 4978tok · 14373B · 调用从「prose」读回
  - 正文：`Cargo.toml`
### attr-five-way-reversed（attribution）· 第 2 次
- 测的是：候选到 5 条且完全逆序 ⇒ 猜的基线降到 1/5，而「按到达顺序」的启发式在这里必然指错
- 候选条数 5 ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 5 分之一
- **A 结构化**：finish=stop · 2598ms · prompt 6066tok · 14871B
  - 正文：`200`
- **B 拍平**：finish=stop · 2180ms · prompt 5060tok · 14607B · 调用从「prose」读回
  - 正文：`200`
### attr-pick-one-field（attribution）· 第 2 次
- 测的是：结果是一段 JSON 文本，问其中一个字段 ⇒ 测「读回结果内容」而不是「读回关联」（单调用，无猜的空间）
- **A 结构化**：finish=stop · 1299ms · prompt 5948tok · 14104B
  - 正文：`2048`
- **B 拍平**：finish=stop · 1522ms · prompt 4929tok · 14182B · 调用从「prose」读回
  - 正文：`2048`
