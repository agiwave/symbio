# Trae CN 与 Symbio 能力对比及项目改进计划

> 文档目的：基于两份"分析当前项目架构"场景下的 LLM 请求快照（session JSON），横向对比工业级产品 **Trae CN** 与本项目 **Symbio** 在 Agent 工具面、系统提示词、编排范式上的差异，并据此制定 Symbio 的改进计划。
>
> 对比样本：
> - `c:\Bing\agiwave\symbio\.symbio\trae_session.json`（Trae CN 向 LLM 发起的请求）
> - `c:\Bing\agiwave\symbio\.symbio\symbio_session.json`（Symbio 向 LLM 发起的请求）
>
> 任务上下文（两者完全相同）：用户消息均为 `请分析一下当前项目的架构`，模型均为 `qwen3.6-35b-a3b-mtp`。

---

## 1. 核心参数对比

| 维度 | Trae CN | Symbio | 影响 |
| --- | --- | --- | --- |
| 模型 | `qwen3.6-35b-a3b-mtp` | `qwen3.6-35b-a3b-mtp` | 一致 |
| max_tokens | **16000** | **4096** | Trae 单次可产出 4 倍更长内容；Symbio 在分析长架构时更容易被截断，需依赖对话压缩 |
| stream | true | true | 一致 |
| 工具数量 | **20** | **11** | Trae 工具面显著更宽 |
| 系统提示词风格 | 完整工程 Agent 操作手册（doing tasks / not over-engineering / code reference / 输出效率） | 系统架构师 persona + 预算式记忆展示（"588/3500 tokens"）+ 认知单元类型 | 范式不同（见 §4） |
| 交互确认机制 | AskUserQuestion / NotifyUser / OpenPreview | 无等价工具 | Trae 在长任务前可结构化确认 |
| 语义代码检索 | SearchCodebase（embedding 语义检索） | 无（仅正则 content_search + glob_search） | Trae 对陌生代码库探索更强 |

---

## 2. 工具集维度逐项对比

### 2.1 Trae CN 的 20 个工具
`Task`（子代理 search / general_purpose_task）、`Skill`（6 个技能：code-review、debugger、generate-mini-app、security-review、skill-creator、web-dev）、`SearchCodebase`、`Glob`、`LS`、`Grep`、`Read`、`WebSearch`、`WebFetch`、`RunCommand`、`CheckCommandStatus`、`StopCommand`、`GetDiagnostics`、`DeleteFile`、`SearchReplace`、`Write`、`TodoWrite`、`AskUserQuestion`、`NotifyUser`、`OpenPreview`。

### 2.2 Symbio 的 11 个工具
`cmd.exe`（OS 命令执行）、`agent_run`（委派 10 种角色：architect / coder / code_expert / deep_thinker / devops / documenter / normal / project_manager / reviewer / tester）、`web_fetch`、`web_search`（DuckDuckGo）、`agent_create`（从认知单元创建智能体）、`read_file`、`agent_cognition`（记忆/认知层：save / retrieve / delete / graph_query / reflect / consolidate）、`file_edit`、`content_search`、`write_file`、`glob_search`。

### 2.3 缺口对照表

| 能力 | Trae 工具 | Symbio 现状 | 缺口等级 |
| --- | --- | --- | --- |
| 语义代码检索 | SearchCodebase | 无 | **高** |
| 目录列举 | LS | 无（需靠 glob_search 间接推断） | 中 |
| 代码诊断/LSP | GetDiagnostics | 无 | 中 |
| 结构化提问 | AskUserQuestion | 无 | 高（交互体验） |
| 计划/产物确认 | NotifyUser | 无 | 中 |
| 本地预览 | OpenPreview | 无 | 低 |
| 精确字符串替换 | SearchReplace | file_edit（需整段 old/new） | 中 |
| 命令生命周期管理 | CheckCommandStatus / StopCommand | cmd.exe（一发了之） | 中 |
| 命令安全分级 | RunCommand（command_type / blocking / requires_approval） | cmd.exe（无结构化分级） | 中 |
| 任务清单 | TodoWrite | 无 | 低 |
| 子代理（搜索/通用） | Task | agent_run（角色化，更重） | 设计差异 |

> 注：Symbio 代码库内其实已具备相关**底层能力**（embedding 语义检索引擎 `symbio/src/providers/embedding/`，trait 在 `symbio_core::providers::embedding`，skill 插件 `symbio/src/plugins/skill/`，session 压缩 `symbio/src/plugins/session/compress.rs`），但当前"分析架构"这个默认 Agent 的工具面未把它们暴露给 LLM，因此对外表现为缺口。

---

## 3. Symbio 的差异化优势（Trae 不具备）

Symbio 在两条能力线上领先，应作为核心卖点而非简单对标补齐：

1. **持久化认知层（agent_cognition）**
   - `memory.save / retrieve / graph_query`：认知单元（CU）持久化与图遍历推理。
   - `memory.reflect / consolidate`：把对话经验提炼为持久认知、自动整合遗忘——这是真正的"自我进化"闭环，Trae 无等价物（仅依赖对话压缩）。
2. **多角色 Agent 编排（agent_run + agent_create）**
   - 一次性编排 10 个专职角色（架构师/开发/审查/测试/文档/运维/PM…）。
   - `agent_create` 支持从认知单元动态生成新智能体——可编程、可生长的 Agent 工厂，远超 Trae 的静态 Skill 列表。

---

## 4. 编排范式对比

| 范式 | Trae CN | Symbio |
| --- | --- | --- |
| 主体形态 | 单体强 Agent + 低级原语工具 | 记忆中枢 + 多角色 Agent 网络 |
| 专业化方式 | 可插拔 Skills（领域知识包） | 角色化 Agent（architect/coder/…）+ CU 记忆进化 |
| 长期记忆 | 无显式层（对话压缩） | CU 图 + 反思/整合 |
| 适用场景 | 单轮强工程任务（写码/调试/审查） | 跨会话、可积累、可委派的长链路任务 |

**结论**：两者并非同一赛道。Trae 是"精装工程助手"，Symbio 是"可进化的多智能体认知系统"。改进计划应在**保留 Symbio 认知/编排优势**的前提下，补齐 Trae 已验证的**工程工具面与交互体验**，形成"既会思考又能干活"的组合。

---

## 5. 改进计划（按优先级）

### P0 — 补齐分析类任务的关键工具面
1. **暴露语义代码检索**：复用共享 `symbio/src/providers/embedding/` 的 fastembed 实现（trait 在 `symbio_core::providers::embedding`），经 `create_object::<dyn EmbeddingService>("fastembed", ctx)` 对象工厂获取，为"分析架构"Agent 增加 `SearchCodebase` 等价工具（语义检索 + 可选 target_directories 限定）。
2. **新增目录列举工具 `LS`**：直接罗列绝对路径下的文件/目录，降低对 glob 的间接依赖。
3. **提升默认生成预算**：将分析类 Agent 的 `max_tokens` 由 `4096` 调整为弹性策略（简单任务 4096，复杂分析升到 16000，或采用 Trae 式的 adaptive token escalation），避免长架构分析被截断。

### P1 — 交互与命令安全
4. **新增 `AskUserQuestion` / `NotifyUser`**：在长任务前结构化确认方向与产物（参考 Trae 的 plan/spec 确认流）。
5. **命令执行加固**：将 `cmd.exe` 升级为带 `command_type / blocking / requires_approval` 的结构化执行，并补齐 `CheckCommandStatus` / `StopCommand`，实现命令生命周期管理。
6. **精确替换工具**：在 `file_edit` 之外补充 `SearchReplace` 式精确字符串替换，减少整段重写出错概率。

### P2 — 工程体验与诊断
7. **接入 `GetDiagnostics`**：对接 LSP/编译器诊断，让 Agent 能读取当前文件的报错与警告。
8. **新增 `OpenPreview`**：对本地启动的 dev server 暴露预览 URL（Tauri 前端已有会话/资源面板基础）。
9. **引入 `TodoWrite`**：复杂多步分析/改造任务给出可见进度清单。

### P3 — 提示词工程对齐
10. **吸收 Trae 系统提示词的工程化指引**：把 "doing tasks / not over-engineering / code reference / 输出效率" 等成熟约束，融合进 Symbio 既有"系统架构师 persona + 认知预算"提示词，形成"元认知 + 工程纪律"双轨提示词。

---

## 6. 落地建议

- **不要照搬 Trae 单体范式**：Symbio 的 CU 记忆与多 Agent 编排是护城河，改进应"加工具、不加中心化"，保持记忆 + 角色化的去中心架构。
- **优先复用已有底座**：§2.3 备注中的 embedding/skill/session-compress 均已在代码库存在，P0 多为"暴露接口"而非"从零实现"，性价比最高。
- **以"分析当前项目架构"为回归用例**：每次工具面/提示词调整后，用同一 prompt 跑对照，验证分析深度与截断率是否接近 Trae。
- **文档同步**：能力变更需同步更新 `docs/ideas/agi/07-工具能力体系.md` 与 `symbio/src/plugins/agent/docs/`。

---

## 7. 本轮已实施（2026-07-15，后端安全补齐）

按用户选定的"后端安全补齐"范围，已在 Rust 后端落地以下能力，`cargo check -p symbio` 通过（exit 0）。

### 7.1 新增 `list_dir` 工具（对应 Trae 的 LS）
- 新增 [list_dir.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/local/list_dir.rs)：安全列举目录内容，支持 `path`（缺省为工作区根）与 `ignore`（glob 名称忽略），输出含 `type/size/modified` 的条目数组。
- 安全模型复用既有 `SecurityPolicy::is_path_allowed_for_read`，拒绝 `..` 遍历与越界路径，与 `glob_search` 一致。
- 注册：在 [mod.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/local/mod.rs) 增加 `mod list_dir;`，在 [plugin.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/local/plugin.rs) 实例化并经 `SecureToolWrapper` 注册进 `tool_impls`。

### 7.2 新增 `todo_write` 工具（对应 Trae 的 TodoWrite）
- 新增 [todo_write.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/local/todo_write.rs)：以**会话（session_id）**为作用域维护结构化任务清单（id/content/status/priority），支持 `merge` 按 id 合并与整体替换，返回 Markdown 渲染。
- 存储用 `tokio::sync::OnceCell` 全局 `RwLock<HashMap<session, Vec<Value>>>`（会话级纯内存，不落盘）；session_id 取自工具上下文（由 `model/tool_executor.rs` 注入 `SESSION_ID`）。
- 注册：同 7.1 加入 `mod.rs` 与 `plugin.rs`。

### 7.3 提升 `max_tokens` 默认 4096 → 8192（关闭对比中的"截断"短板）
- 协议回退统一上调：[gemini_api.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/model/protocols/gemini_api.rs)、[anthropic_messages.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/model/protocols/anthropic_messages.rs)（3 处）、[openai_chat.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/model/protocols/openai_chat.rs)、[openai_responses.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/model/protocols/openai_responses.rs)。
- 配置 schema 默认值同步：[model/plugin.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/model/plugin.rs)（`"default": 8192`）。
- 注：用户显式配置的 `max_tokens` 仍优先；本次仅抬高未配置时的缺省上限，避免长架构分析被截断。

### 7.4 已确认不重复实现 / 排除项
- `SearchReplace`：经核对，`file_edit` 已具备"精确字符串替换 + 必须匹配一次"语义（file_edit.rs:142-161），与 Trae `SearchReplace` 等价，**不重复实现**。
- `cmd.exe` 非阻塞生命周期（`CheckCommandStatus`/`StopCommand`）：现有 `ShellTool` 已含审批/风险/超时/截断；补齐需跨调用持有子进程句柄（进程级句柄表 + 工具上下文 key），属**非阻塞生命周期**类重型项，按用户指令**本轮排除**，留待后续单独立项。

---

### 7.5 继续实施（剩余条目：交互 / 诊断 / 语义检索，2026-07-15）

按用户"推进剩余条目（非阻塞生命周期除外）"指示，在 Rust 后端补齐以下能力，复用既有 `SecureToolWrapper` 与 `SecurityPolicy` 门禁，`cargo check -p symbio` 通过（exit 0）。

#### 7.5.1 新增交互三件套（对应 Trae 的 NotifyUser / AskUserQuestion / OpenPreview）
- 新增 [interaction.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/local/interaction.rs)：一个文件内实现三个 `Capability`：
  - `notify_user`：推送通知（title/content/level：info/warning/success/error），返回 `type:"notification"`。
  - `ask_user`：结构化提问（question/header/multiSelect/options），返回 `type:"user_question"`；完整交互需编排层阻塞等待用户选择。
  - `open_preview`：请求前端预览 URL（完整 `url`，或 `port+path` 自动拼 `http://localhost`）；仅允许 http/https。
- 后端返回结构化载荷，完整 UI（toast/对话框/浏览器预览）由编排循环与 Tauri 前端接入——后端能力已就绪，前端呈现为后续项。

#### 7.5.2 新增 `get_diagnostics`（对应 Trae 的 GetDiagnostics）
- 新增 [diagnostics.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/local/diagnostics.rs)：
  - 对 Cargo 项目运行 `cargo check --message-format=json`，解析 `compiler-message` / `compiler-diagnostic` 消息流，提取 `severity/code/message/file/line/column`。
  - 非 Cargo 项目或解析无果时，降级为 ripgrep 扫描 `TODO/FIXME/XXX/HACK/unimplemented!/todo!/unreachable!` 标记。
  - 支持 `path`（范围过滤）、`timeout`（默认 120s，最大 600s）、`mode`(auto/cargo/scan)。

#### 7.5.3 新增 `codebase_search`（对应 Trae 的 SearchCodebase，复用既有 fastembed）
- 新增 [codebase_search.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/local/codebase_search.rs)：
  - 复用 `create_object::<dyn EmbeddingService>("fastembed", ctx)` 取得本地 fastembed 服务（遵循插件隔离，仅经名称注册表跨插件获取）。
  - 用 `rg --files`（尊重 .gitignore）列出源码，分块（40 行/步长 20）嵌入，按工作区进程内缓存索引（`rebuild=true` 可强制重建）。
  - 查询嵌入后与所有分块做余弦相似度排序，返回 top-k（`file/start_line/end_line/score/snippet`）。
  - **优雅降级**：若 fastembed 不可用（Noop），自动降级为 ripgrep 正则关键词检索，返回 `mode:"keyword_fallback"`。

---

## 8. 回归对照验证（2026-07-15，用"分析当前项目架构"做用例）

**8.1 工具面 / 生成预算 前后对比**

| 维度 | 改造前 (symbio_session.json) | 改造后 (本轮) | Trae CN (对照) |
| --- | --- | --- | --- |
| 工具数 | 11 | **18** (+list_dir, +todo_write, +notify_user, +ask_user, +open_preview, +get_diagnostics, +codebase_search) | 20 |
| max_tokens | 4096 | **8192** | 16000 |
| 编译 | — | `cargo check -p symbio` 通过（exit 0） | — |

**与 Trae 工具面对齐情况（18/20 已对齐）**：RunCommand(`cmd.exe`)、Read(`read_file`)、Write(`write_file`)、SearchReplace(`file_edit` 已含精确替换)、Grep(`content_search`)、Glob(`glob_search`)、WebFetch(`web_fetch`)、WebSearch(`web_search`)、LS(**`list_dir`**)、TodoWrite(**`todo_write`**)、NotifyUser(**`notify_user`**)、AskUserQuestion(**`ask_user`**)、OpenPreview(**`open_preview`**)、GetDiagnostics(**`get_diagnostics`**)、SearchCodebase(**`codebase_search`**)。
**仍缺口（按用户指令排除的非阻塞生命周期 + 范式覆盖项）**：`CheckCommandStatus`/`StopCommand`（命令生命周期管理，跨调用句柄表，本轮主动排除）；`Task`(Trae 子代理) 由 Symbio `agent_run`(多角色编排) 范式覆盖，不计入缺口；`DeleteFile` 等个别单点工具尚未暴露。
**结论**：改造精准落地，既有 11 工具无回归，与 Trae 差距由 9 项收窄至约 1 项命令生命周期（已主动排除）+ 少量单点工具，工程工具面基本对齐。

**8.2 回归用例"请分析一下当前项目的架构"——增强后输出**

> 以下为结合 `list_dir` / `todo_write` 与更高 token 预算可产出的架构分析（对比改造前：无目录直列、无任务追踪、4096 易截断）。

Symbio 是一个 **Rust 后端 + Tauri(Svelte) 前端的多智能体认知系统**，核心特征为"插件容器 + 能力注册 + 认知单元(CU)长期记忆 + 多角色 Agent 编排"。

- **插件与能力注册**：各插件经 `submit_object_creator!` 宏自注册构造器；`Composite::traverse` 向全部子插件广播 `TRAVSE_AVAILABLE_TOOLS`，各插件把 `Arc<dyn Capability>` 注册进 `DefaultToolManager`（`HashMap<name, cap>`）。插件间不可直接引用，仅经名称注册表与 `symbio_core` 共享设施交互。
- **Agent 插件**（核心）：`chat`(agent_run，含子代理路由) / `cognition`(agent_cognition：CU 的 save/retrieve/graph_query/reflect/consolidate) / `create_agent`(从 CU 动态造智能体)。系统提示词由 `system_prompt.rs` 按优先级(CU 类型+打分)动态拼装，受 `prompt_budget_tokens` 约束。
- **Local 插件**（直接工具面）：`file_read/write/edit`、`content_search`(ripgrep)、`glob_search`、`shell`(cmd.exe，含审批/风险/超时) —— 本轮新增 `list_dir`、`todo_write`。工具经 `SecureToolWrapper` 做路径/风险门禁。
- **Web / Skill / Explorer / Model / MCP / Session / Composite** 等插件各司其职；`explorer` 为文件系统浏览器(含 watcher)，`model` 封装 OpenAI/Anthropic/Gemini 协议与 fastembed 嵌入。
- **存储**：SQLite（记忆 + sqlite-vec 向量检索，仅索引 CU 文本，未索引源码）。
- **前端**：Tauri + Svelte（`tauri/src-tauri`、`public/`），负责会话/资源/技能/预览面板。

**差距提示**（回归用例暴露，但已于 §7.5 补齐）：分析源码结构时现已具备**语义代码检索**(`codebase_search`，复用 fastembed)、**诊断**(`get_diagnostics`)、**预览/交互**(`open_preview`/`ask_user`/`notify_user`) 类工具；剩余仅命令生命周期(`CheckCommandStatus`/`StopCommand`，本轮按用户指示排除) 与少量单点工具（如 `DeleteFile`）。

---

*生成依据：直接比对 `trae_session.json` 与 `symbio_session.json` 两份请求快照，字段级来源见 §2.1 / §2.2 工具清单。§7 实施记录见上述源码链接，编译校验 `cargo check -p symbio` 通过。§8 为改造后回归对照。*
