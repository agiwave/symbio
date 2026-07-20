# Symbio Agent Issues 跟踪

| 项目 | 内容 |
|------|------|
| 跟踪时间 | 2026-06-16 |
| 最后更新 | 2026-07-06 (I-050 项目级文档清理 + I-045 真正完成) |
| 范围 | `symbio/src/plugins/agent` 核心代码 |
| 配套 | 修复历史见 [CHANGELOG.md](./CHANGELOG.md)；执行计划见 [PLAN.md](./PLAN.md) |

> **ISSUES.md 只记录当前代码中未修复的问题**。已修复项全部迁入 [CHANGELOG.md](./CHANGELOG.md)。

---

## 摘要

| 维度 | 当前评估 |
|------|----------|
| 核心代码 | 🟢 稳定 |
| 架构 | 🟢 模块职责清晰：capabilities（能力层）、core（核心抽象）、store（存储层）、handlers（请求处理）|
| 目录结构 | 🟢 v36 优化后：消除碎片文件、修正职责归属、修正命名 |
| 类型安全 | 🟢 I-014 已完成 |
| 可观测性 | 🟢 I-015 已完成（10 个 tracing span）|
| 性能 | 🟢 I-026 已完成（graph.rs 分页加载）|
| 测试 | 🟢 234 个单元测试通过 |
| Clippy | 🟢 0 warnings |
| Token 效率 | 🟢 I-027 ~ I-038 已修复（节省约 1200-1800 tokens/请求）|
| 文档一致性 | 🟢 I-039 ~ I-049 已修复 |

---

## 已修复问题（v47）

> v47 的 12 个问题（I-050 ~ I-061）已全部修复，详细记录见 [CHANGELOG.md](./CHANGELOG.md) v47 段。
> v49 已修复（**I-065 系统提示词三层目标**：动态构建 + LLM 自我管理 + 限制与最大化），见 CHANGELOG v49。
> v50 已修复（**I-066 启用 memory.reflect**），见 CHANGELOG v50。
> v54 已修复（**I-070 预算状态段落地 + 候选池健康报告 + 价值密度评分**），见 CHANGELOG v54。
> v48 同样已修复（I-062 反思 / I-063 设计哲学 / I-064 name 渲染），见 CHANGELOG v48。
> 2026-07-06 已修复（**I-071 项目级文档系统性清理**），见 CHANGELOG 2026-07-06 段。

### I-050: CHANGELOG/ARCHITECTURE 与实际代码严重脱节

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `docs/explanation/ARCHITECTURE.md`, `docs/CHANGELOG.md`, `capabilities/cognition.rs`（注释） |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

CHANGELOG.md v40-v46 描述"26 个 op 跨 5 个认知域（memory/reason/learn/plan/metacognition）"，但实际代码只有 4 个 op（save/delete/retrieve/graph_query），全部在 memory 域。具体脱节点：
- `ARCHITECTURE.md` 列出 `capabilities/ops/memory/*.rs`、`ops/learn/*.rs` 等多文件，实际只存在 `capabilities/ops/memory/{save,delete,retrieve,graph_query,mod}.rs`
- `CHANGELOG.md` v40-v45 多次引用 `capabilities/ops/reason/mod.rs`、`capabilities/ops/plan/generate.rs` 等不存在的路径
- `cognition.rs:23` 注释 `## 操作列表（10 个，全部在 memory 域）` 与实际 4 个不符
- `cognition.rs:262` doc comment 提到 `memory.update`、`memory.save` 等 10 个 op，实际只 4 个

**影响**：
- LLM 通过 docs 看到的工具能力与实际不符
- 开发者按文档找文件找不到
- "26 个 op / 5 个域"是 v40 拆除后未落地的目标态

**修复方案**：
- ARCHITECTURE.md 反映 4 个 op 的真实状态，移除对未实现 op 的描述
- CHANGELOG.md 标注"v40-v46 描述为目标态，实际落地为 4 个 op（memory 域）"
- cognition.rs 注释和 doc comment 改为 4 个 op

> **现状补注（后续演进）**：本问题修复后，memory 域操作又经 v51/v52 增至 **5 个**（`save` / `retrieve` / `graph_query` / `reflect` / `consolidate`），并废除了 `delete`（改由 `save {confidence:0}` 软删除）。当前权威能力清单见 [ARCHITECTURE.md §3.4](./ARCHITECTURE.md) 与 [agent/README.md §3.2](../../README.md)。

---

### I-051: examples 引用不存在的 op

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `capabilities/cognition.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

`AgentCognitionTool::meta()` 的 `examples` 字段第 4 个示例为 `operation=memory.update, target_id=某个知识ID, updates={...}`，但 `memory.update` op 不存在（I-050）。同时第 3 个示例使用 `relation='causes'` 作为顶层参数，但 `graph_query` op 实际参数是 `graph_operation='neighbors'` + `node_id`，无 `relation` 顶层参数。

**影响**：LLM 按 examples 调用 `memory.update` → "未知操作" 错误，浪费一次往返；按 `graph_query` 示例调用也会参数错误。

**修复方案**：重写 examples 为 4 类典型场景，每条都使用实际存在的 op 和正确的参数结构。

---

### I-052: tool schema 不暴露 op 参数结构

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `capabilities/cognition.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

I-027 修复移除了 `format_schema_params` 嵌入到 op description 的逻辑后，`build_schema()` 生成的 `operation` 描述只列了简短一行（`memory.save: 保存认知单元。参数 items 为 CU 数组...`）。LLM 第一次调用 `agent_cognition` 时：
- 看到的 schema 只有 `operation` 一个 required 字段
- 不知道各 op 的具体参数（如 `memory.save` 实际是 `items` 数组 + 顶层简写）
- 只能靠试错，错误时 `check_required_params` 才返回提示

**影响**：LLM 首次调用成功率低，需要 1-2 次试错才能正确调用。

**修复方案**：
- op description 改为"场景 → 操作"双段式（场景描述 + 参数示例）
- examples 字段给出 4 个真实可用的调用范例
- 移除 dead code `format_schema_params`

---

### I-053: `format_schema_params` 函数是 dead code

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/cognition.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

`AgentCognitionTool::format_schema_params()` 函数（约 40 行）在 I-027 修复后已不再被 `build_schema()` 调用，但函数定义和单元测试 `format_schema_params_helper` 仍保留。`check_required_params` 内部仍调用 `format_schema_params`，但使用方式不当（仅在缺参数时报错信息中显示参数说明，LLM 收到的是英文拼接而非结构化）。

**影响**：
- 维护成本：函数保留但无 schema 文档价值
- 误导读者：函数命名暗示"暴露给 LLM"，实际只在错误路径使用

**修复方案**：
- 移除 `format_schema_params` 函数定义
- 移除相关测试 `format_schema_params_helper`
- `check_required_params` 改为只输出"缺失参数名 + 该 op 完整 description（包含参数示例）"

---

### I-054: system_prompt 引导语语法错乱且表述模糊

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `handlers/system_prompt.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

`system_prompt::build` 的引导语（`build` 函数末尾处）：
```
你的拥有大量认知单元，剧本认知能力的强人工智能体，下面的重要认知单元（可修改），可以用于指导你的行为。
```

存在 3 处问题：
1. **语法错误**："你的拥有" → 错别字（应为"你拥有"）；"剧本认知能力" → 错别字（应为"具备"或"具有"）
2. **表述模糊**："可修改" 未说明修改范围、修改权限、修改入口
3. **结构混乱**：括号位置在"重要认知单元"之后、句末之前，破坏阅读

**影响**：LLM 看到不自然的引导语可能困惑；"可修改"会让 LLM 误以为可以自由修改任何 CU。

**修复方案**：
- 修正语法错误
- 替换"可修改"为清晰说明"通过 `memory.save` 更新 `level=msg` 或非 core 的 `level=sys` 单元；`level=core` 永久不可变"
- 重新组织语句结构

---

### I-055: 三层上下文（system / user.prompt）语义边界不清

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `handlers/system_prompt.rs`, `handlers/chat.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

LLM 接收到的提示词由两部分组成：
- `system_prompt`：通过 `system_prompt::build()` 注入到 system message（身份 + 系统级 CU 列表）
- `user.message.prompt`：通过 `ContextBuilder::build()` 注入到 user message 前缀（`<active_memory>` + `<context>` + `<task_context>`）

但 prompt 中没有任何说明告知 LLM 这些标签的语义。LLM 看到：
- `## 你的身份` — 这是谁
- `## 系统级认知单元` — 这是我的知识
- `<active_memory>...</active_memory>` — 这是什么？
- `<task_context>...</task_context>` — 跟"系统级 CU"有什么区别？
- `<context>当前时间 ...</context>` — 为什么在用户消息里？

**影响**：LLM 困惑于上下文标签的来源和用途，可能忽略或误用。

**修复方案**：
- system_prompt 开头加"元说明段"，说明以下各部分语义
- 明确"系统级 CU（静态）"vs"临时上下文（动态）"边界
- 临时上下文标签从 `<active_memory>`/`<task_context>`/`<context>` 改为更语义化的命名（如 `<current_context>`），并在 system_prompt 中说明

---

### I-056: system_prompt 按学术分类（is_a）分组对 LLM 不友好

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `handlers/system_prompt.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

`render_cognitive_units` 按 `is_a` 第一个类型分组（`fact`/`rule`/`skill`/`strategy`/`judgment`/`experience`/`intuition`/`emotion`）。这是**学术分类**，LLM 看到"### fact（共 50 项）"不知道这些事实是关于什么的、什么时候用。

**影响**：
- LLM 不知道每组 CU 的"用途"
- 标题抽象（`fact` / `intuition`）不如"业务场景"直观
- 大数量 CU 时（fact 50 项）需要 LLM 自己浏览，无法快速定位

**修复方案**：
- 改为按业务场景分组：`[身份] / [行为规则] / [专业技能] / [思维策略] / [判断准则] / [经验教训] / [其他知识]`
- 优先级映射：rule→[行为规则]、skill→[专业技能]、strategy→[思维策略]、judgment→[判断准则]、experience→[经验教训]、fact→[其他知识]
- 分组标题用 display_name（如"事实"→"知识库"）

---

### I-057: system_prompt 渲染过于详细

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟢 低 |
| 影响范围 | `handlers/system_prompt.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

`render_cu_line` 输出格式：
```
- **{name}**: {description (≤200字符)} (id: `{id}`)
```

每个 CU 都包含 `id` 后缀。但 LLM 实际使用 `id` 的场景仅限"通过 `memory.update` 引用回写"，而 `memory.update` 已不存在。`memory.save` 顶层简写也支持不传 id。

**影响**：
- 每条 CU 多出 `(id: \`xxx\`)` 约 15 字符
- 100 条 CU 累计 1500 字符 ≈ 375 tokens 浪费
- LLM 看到 `id` 可能误以为每次调用都需要传

**修复方案**：
- `id` 仅在"操作提示"中提及（如"如需引用此知识，使用工具调用"）
- `render_cu_line` 简化为 `- {name}: {description}`

---

### I-058: GraphQueryOp 的子操作说明是内嵌多行字符串

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/ops/memory/graph_query.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

`GraphQueryOp::meta()` 的 description 用 `\`n  ` 拼接多行：
```
"图结构查询与遍历推理。通过 graph_operation 参数指定子操作：\n  \
- neighbors: 获取邻居节点（需要 node_id）\n  \
- path: 查找两点间路径（需要 source_id, target_id）\n  \
..."
```

被嵌入到 `build_schema()` 的 op 列表后，LLM 看到的是一长串带 `\n  ` 转义的原始字符串，难以解析。

**影响**：
- LLM 看到的多行文本不规整
- 6 个子操作的参数表混在 description 中，认知负担重

**修复方案**：
- description 顶部一句话说明"图操作 6 个子操作"
- 每个子操作作为独立 op（`graph_query.neighbors`、`graph_query.path` 等），通过统一注册机制自动加入 schema
- LLM 通过 operation 字符串路由，无需看 description 解析

---

### I-059: system_prompt 与 tool description 内容重叠

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `handlers/system_prompt.rs`, `capabilities/cognition.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

system_prompt 和 `agent_cognition` tool description 都在说"管理认知单元"，但角度不同：
- system_prompt："你的拥有大量认知单元，下面的重要认知单元（可修改）..."
- tool description："管理认知单元（记忆、知识、事实）..."

LLM 看到两段相似的描述，认知负担加倍。

**影响**：
- Token 浪费
- LLM 可能困惑于两段描述的边界

**修复方案**：
- system_prompt 聚焦"系统级 CU 是什么"（身份 + 静态知识列表）
- tool description 聚焦"agent_cognition 工具能做什么"（4 个 op 的场景 + 参数）
- 两者通过不同的"视角"区分：system_prompt 是"我的知识库"，tool 是"管理我的知识库的工具"

---

### I-060: 缺"工作流示例"，LLM 不知道 op 之间的组合关系

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/cognition.rs` |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

LLM 看到 4 个 op 的 description，但不知道它们如何组合使用。常见工作流：
1. **检索 → 更新**：先 `memory.retrieve` 找到 id，再 `memory.save` 顶层带 id 局部更新
2. **语义检索 → 图探索**：`memory.retrieve(filter:{semantic:...})` 拿到 id，再 `memory.graph_query(node_id:id)`
3. **批量更新**：`memory.retrieve(filter:{is_a:'rule'})` 拿到一批 id，循环 `memory.save` 更新每条

这些工作流没有显式说明，LLM 只能逐步试错。

**影响**：LLM 单独使用每个 op 效率低，复杂任务成功率低。

**修复方案**：在 system_prompt 末尾或 tool description 末尾加"工作流段"，列出 3-4 个典型工作流。

---

### I-061: cognition.rs 注释声明 10 个 op，实际 4 个

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/cognition.rs`（注释） |
| 发现时间 | 2026-06-22 |
| 状态 | ✅ 已修复 |

**问题描述**：

`cognition.rs:23` 注释：
```rust
//! ## 操作列表（10 个，全部在 memory 域）
//!
//! - `memory.*`: save, save_batch, save_working, retrieve, delete, list, count, graph_query, update, stats
```

实际只有 4 个 op（save/delete/retrieve/graph_query），注释提到的 `save_batch`/`save_working`/`list`/`count`/`update`/`stats` 都不存在。

**影响**：读者按注释找文件或操作时困惑。

**修复方案**：注释改为 4 个 op 实际列表。

---

### I-071: 项目级文档系统性清理（用户两轮反馈）

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `docs/` 全树；`docs/archive/proj/IMPROVEMENT_PLAN_2026.md`；agent/docs/ISSUES.md（自身） |
| 发现时间 | 2026-07-06 |
| 状态 | ✅ 已修复 |

**问题描述**：

用户两轮反馈：
1. "项目文档与代码实现存在系统性脱节"——`docs/archive/proj/` 下历史规划文件、`docs/archive/design_docs/` 中早期提案、以及 `docs/README.md` 中插件文档列表与实际不符。
2. "继续，注意删除历史过期文档信息或者文档，确保所有文档保持最新"——既然能通过新方案覆盖过期文档，应**直接删除**而非保留横幅标注。

**根因**：
- 多份早期提案文档（ARCHITECTURE_IMPROVEMENT.md / MODEL_CHAT_REDESIGN.md / COMPARISON_WITH_QWEN_CODE.md）已 100% 由代码实现或被新文档覆盖，但因"历史价值"被保留，导致对当前代码产生**持续性误导**。
- 2026-03 的 `PLAN.yml` / `TASK_INDEX.md` / `tasks/T00x-*.md` 严重过时（任务清单中 T002/T003/T012/T013 是 Tauri 时代项）。
- `docs/README.md` §5 "注意"段、`docs/COGNITION.md` §14、`docs/PRINCIPLES.md` §8 中仍残留对不存在文件的引用（`OPERATIONS.md` / `PROMPT_ARCHITECTURE.md` / `CODE_ANALYSIS_REPORT.md`）。
- `docs/archive/design_docs/HISTORY_AND_REVIEWS.md` 中"当前形态"错误描述为"纯 Rust 核心库 + E2E CLI"，但前端已于 2026-06 恢复为 Tauri。

**修复方案**（详见 `docs/CHANGELOG.md` 2026-07-06 段）：

1. **新建 `docs/archive/proj/IMPROVEMENT_PLAN_2026.md`** 作为项目级改进方案的**唯一权威来源**（10 个 P0-P3 改进方向 + 季度路线图）。
2. **删除** 11 个过期历史文档（详见 CHANGELOG 表格）。
3. **修复** `docs/README.md` 索引、`HISTORY_AND_REVIEWS.md` 当前形态描述、`IMPROVEMENT_PLAN_2026.md` 中的引用。
4. **真正完成** I-045（删除 OPERATIONS.md 引用残留）。

**验证**（2026-07-06）：

- ✅ `docs/archive/design_docs/` 仅保留 `HISTORY_AND_REVIEWS.md`
- ✅ `docs/archive/proj/` 仅保留 `IMPROVEMENT_PLAN_2026.md`
- ✅ 全仓库 grep 无 `OPERATIONS.md` / `PROMPT_ARCHITECTURE.md` / `CODE_ANALYSIS_REPORT.md` / `ARCHITECTURE_IMPROVEMENT.md` / `COMPARISON_WITH_QWEN_CODE.md` / `MODEL_CHAT_REDESIGN.md` / `MODEL_CHAT_IMPLEMENTATION_PLAN.md` / `MODEL_CHAT_IMPROVEMENT_PLAN.md` / `PLAN.yml` / `TASK_INDEX.md` / `tasks/T00x-*.md` 引用残留（CHANGELOG.md 和 IMPROVEMENT_PLAN_2026.md 中作为历史记录除外）
- ✅ 业务代码（agent、model、session、mcp、skill、home 等）零改动——本次清理只触及文档

---

## 已修复问题（v46）

### I-045: 文档引用不存在的 OPERATIONS.md

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `docs/COGNITION.md`, `docs/PRINCIPLES.md`, `docs/README.md` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复（**2026-07-06 真正完成**） |

**问题描述**：

COGNITION.md 和 PRINCIPLES.md 的定位说明中都引用了"操作流程（见 OPERATIONS.md）"，但 `docs/` 目录中不存在 OPERATIONS.md 文件。docs/README.md §5 顶部也有指向"已合并/废除"的 `PROMPT_ARCHITECTURE.md` / `OPERATIONS.md` / `CODE_ANALYSIS_REPORT.md` 引用。

**影响**：读者按指引找不到对应文档；即使说明是"已合并/废除"，对读者仍是噪音。

**修复方案**（两阶段）：

- **第一阶段（v46 末）**：移除对 `OPERATIONS.md` 的引用，操作流程相关内容已内嵌在各 op 文件的 doc comment 和 TESTING.md 中。**但未完全修干净**：COGNITION.md §14、PRINCIPLES.md §8 仍残留 `[**OPERATIONS.md**](./OPERATIONS.md)` 链接（v46 漏改）。
- **第二阶段（2026-07-06 项目级文档清理）**：
  - 真正删除 `docs/COGNITION.md` §14 中的 `OPERATIONS.md` 引用，替换为 `TESTING.md`（含 op 操作手册）
  - 真正删除 `docs/PRINCIPLES.md` §8 中的 `OPERATIONS.md` 引用，替换为 `TESTING.md`
  - 删除 `docs/README.md` §5 顶部"注意"段（不再提及已合并/废除的旧文件名）
  - 在 `docs/archive/proj/IMPROVEMENT_PLAN_2026.md` 中明确归档此问题

**最终状态**（2026-07-06 验证）：grep 全仓库已无 `OPERATIONS.md` 引用残留。

---

### I-046: TESTING.md §2.3 测试计数过时

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `docs/TESTING.md` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

TESTING.md §2.3 仍显示原始的 6 个存储层测试结果，而 §2.4 已更新为 295 个。§2.3 与 §2.4 数据不一致，读者可能困惑。

**影响**：对测试覆盖范围产生误解。

**修复方案**：将 §2.3 标记为"存储层基础测试（历史存档）"，明确其仅展示存储后端测试子集。

---

### I-047: CognitionContext.agent_dir 标记 #[allow(dead_code)]

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `core/traits.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`CognitionContext.agent_dir` 字段被 `#[allow(dead_code)]` 标记，说明该字段当前未被读取。虽然 store 层广泛使用 `agent_dir` 参数，但 `CognitionContext` 中的这个字段从未被访问。

**影响**：代码意图不清晰，读者不知道是遗漏还是预留。

**修复方案**：添加注释说明该字段的用途（为未来扩展预留，如相对路径解析），或将 `#[allow(dead_code)]` 替换为显式注释。

---

### I-048: From\<StoreError\> 丢失语义，is_not_found() 依赖字符串匹配

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `core/error.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

1. `From<StoreError> for AgentError` 将所有 StoreError 变体统一包装为 `AgentError::Storage(String)`，丢失了 `AlreadyExists`/`NotFound`/`InvalidInput` 的语义。
2. `is_not_found()` 不得不用字符串匹配（`m.to_lowercase().contains("not found")`）来找回这些语义，脆弱且低效。
3. `From<AgentError> for PluginError` 也依赖字符串匹配（`msg.contains("AlreadyExists")`）。

**影响**：错误处理依赖字符串模式匹配，新增错误变体时容易遗漏更新。

**修复方案**：
1. 在 `AgentError` 中添加 `NotFound(String)` 和 `AlreadyExists(String)` 变体
2. 更新 `From<StoreError>` 直接映射到对应变体
3. 更新 `is_not_found()` 匹配变体而非字符串
4. 更新 `From<AgentError> for PluginError` 使用变体匹配

---

### I-049: AgentChatTool 流式转发逻辑过于庞大

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/chat.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`AgentChatTool::execute` 方法中 `tokio::spawn` 内的流式转发逻辑约 80 行，处理 `StreamEvent::Update`/`Error`/`Status`、`WaitingUserAction` 审批、文本累积等。这段逻辑内嵌在 `execute` 方法中，增加了函数复杂度。

**影响**：可读性差，难以单独测试流式转发逻辑。

**修复方案**：将 `tokio::spawn` 内的异步块提取为独立的 `stream_relay` 函数。

---

## 已修复问题（v44）

### I-039: README.md 路由表与实际 handler 不一致 ✅

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `README.md` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

README.md §3.2 和 §3.3 列出了以下路由，但 `handlers/mod.rs` 的路由表中不存在：
- `agent/save` — handlers 中无此路由
- `agent/get_context` — handlers 中无此路由
- `agent/store` — 已被 `agent_cognition(memory.save)` 替代
- `agent/query` — 已被 `agent_cognition(memory.retrieve)` 替代

实际路由表（`handlers/mod.rs`）只有：`list`、`get`、`chat`、`config/get`、`config/set`。

**影响**：用户按 README 调用会得到 "Unknown path" 错误。

**修复方案**：更新 README.md 路由表，反映实际 API。说明旧路由已迁移到 `agent_cognition` 统一工具中。

---

### I-040: CHANGELOG.md v43 中引用已不存在的 engine/ 路径

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `docs/CHANGELOG.md` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

v43 变更文件列表中引用了 `engine/conversation.rs` 和 `engine/seed_cus.jsonl`，但当前目录结构中已无 `engine/` 子目录。这些文件已迁移到：
- `engine/conversation.rs` → `handlers/chat.rs`（ContextBuilder 已内聚到 chat handler）
- `engine/seed_cus.jsonl` → `store/mindscape/seed_cus.jsonl`

**影响**：读者按链接找不到文件。

**修复方案**：更新 v43 变更文件列表中的路径。早期版本的 `engine/` 引用保持不变（记录历史事实）。

---

### I-041: TESTING.md 测试计数与 PLAN.md 不同步

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `docs/TESTING.md` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

TESTING.md §2.3 展示 "6 passed"（仅存储层测试），但 PLAN.md 声称 "218 个单元测试"。TESTING.md 缺少其他 212 个测试的汇总信息。

**影响**：读者对测试覆盖范围产生误解。

**修复方案**：更新 TESTING.md，补充完整测试统计。

---

### I-042: COGNITION.md CognitiveUnit 定义与代码不符

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟢 低 |
| 影响范围 | `docs/COGNITION.md` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

COGNITION.md §1.1 写道 `pub type CognitiveUnit = CognitiveUnit;`（自引用定义），但实际代码中是 `pub struct CognitiveUnit { data: Map<String, Value> }`。

**影响**：读者对类型定义产生困惑。

**修复方案**：更新为实际的 struct 定义。

---

### I-043: memory.save op 未设置 is_a 关系

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `capabilities/ops/memory/save.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`memory.save` op 中使用 `kind` 字段（语义标签，如 "semantic"/"general"）来标记记忆类型，但没有设置 `is_a` 关系字段。这导致：
1. 保存的记忆没有 `is_a` 关系，`FilterExpr::IsA` 查询无法命中
2. `CognitiveUnitValidator` 无法验证其类型合法性
3. 系统提示词中认知索引按 `is_a` 分组，这些记忆不会出现在索引中

**影响**：通过 `memory.save` 保存的记忆在类型过滤和认知索引中不可见。

**修复方案**：在 `execute` 方法中，根据 `kind` 参数设置 `is_a` 关系。如果 `kind` 是有效的认知类型（如 "fact"/"rule"/"experience"），设置为 `is_a`；否则默认设为 `["fact"]`。

---

### I-044: handlers 和 ops 层缺少独立单元测试

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `handlers/`, `capabilities/ops/` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

当前 218 个测试主要覆盖存储层和核心类型，以下层缺少独立测试：
- `handlers/chat.rs`（最复杂的 handler，含 ContextBuilder）— 0 个测试
- 26 个 `capabilities/ops/` 操作 — 0 个独立测试
- `manager/create_agent.rs` — 0 个测试

**影响**：核心业务逻辑的回归保护不足。

**修复方案**：为 ops 层添加单元测试，使用 `MemoryStorage` 作为 mock store。优先覆盖最常用的 ops（memory.save, memory.retrieve, learn.extract 等）。

---

## 已修复问题（v37）

### I-027: build_schema() 参数信息冗余导致 token 浪费 ✅

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `capabilities/cognition.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`AgentCognitionTool::build_schema()` 在生成 `operation` 字段描述时，将每个 op 的参数信息格式化为 `content(string*)` 格式追加到 description 末尾。但这些参数信息已通过每个 op 的 `input_schema`（JSON Schema）传递给 LLM，造成重复。

当前格式（27 个 op）：
```
- memory.save: 保存单条记忆。范例：content='今天学习了Rust', type=semantic, confidence=0.8 参数：content(string*), type(string), confidence(number), id(string), tags(array), related(array)
```

**影响**：
- 单次请求额外消耗约 1000-1500 tokens
- LLM 的 function calling 能力已能从 JSON Schema 理解参数，description 中的参数信息冗余

**修复方案**：
修改 `build_schema()` 中的格式化逻辑，移除 `参数：{params_str}` 部分：
```rust
// 当前
op_descriptions.push(format!("- {}: {} 参数：{}", name, m.description, params_str));
// 修复后
op_descriptions.push(format!("- {}: {}", name, m.description));
```

可删除 `format_schema_params` 方法（如果不再被其他地方使用）。

---

### I-028: ✅ 系统提示词引用不存在的工具名

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `engine/conversation.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

[conversation.rs L159](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/agent/engine/conversation.rs#L159) 中认知索引的使用指引写道：

```
以下是你的已知认知索引。需要详情时，使用 agent_query 检索。
```

但工具列表中不存在 `agent_query`，正确的是 `agent_cognition` 配合 `memory.retrieve` 操作。

**影响**：
- LLM 可能尝试调用不存在的 `agent_query` 工具
- 导致工具调用失败，用户体验差

**修复方案**：
```rust
// 当前
"以下是你的已知认知索引。需要详情时，使用 agent_query 检索。\n\n"
// 修复后
"以下是你的已知认知索引。需要详情时，使用 agent_cognition(memory.retrieve) 检索。\n\n"
```

---

### I-029: ✅ reason.graph_traverse 与 memory.graph_query 功能重叠

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/ops/reason/graph_traverse.rs`, `capabilities/ops/memory/graph_query.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

两个操作功能重叠：
- `reason.graph_traverse`：支持 neighbors/path/subgraph（3 个子操作）
- `memory.graph_query`：支持 get_neighbors/find_path/get_subgraph/infer_relations/query_by_type/find_bridge_nodes（6 个子操作）

两者调用相同的底层函数（`get_neighbors`, `find_path`, `get_subgraph`），`graph_traverse` 是 `graph_query` 的严格子集。

**影响**：
- LLM 需要理解两个工具的区别，增加认知负担
- 工具定义冗余，浪费约 50-80 tokens

**修复方案**：

方案 A（推荐）：删除 `reason.graph_traverse`，保留 `memory.graph_query`
- 在 `memory.graph_query` 的 description 中注明"图遍历推理"
- 减少 1 个工具定义

方案 B：保留两者但明确区分
- `reason.graph_traverse`：专注于推理场景（自动推断关系）
- `memory.graph_query`：专注于精确查询

---

### I-030: ✅ learn.learn_episode 与 learn.extract 语义重叠

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/ops/learn/learn_episode.rs`, `capabilities/ops/learn/extract.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

两者都"从对话中提取知识"，description 不够区分：
- `learn.extract`：`"从对话历史中提取知识。范例：..."`
- `learn.learn_episode`：`"学习情节：从对话中提取并存储经验。范例：..."`

LLM 很难判断何时用哪个。

**影响**：
- LLM 可能错误选择工具
- 增加不必要的认知负担

**修复方案**：

修改 description 使差异更明显：
```rust
// learn.extract
"从对话中按句子拆分提取知识点并逐条存储。适用于：结构化知识、事实、规则。"

// learn.learn_episode
"将整段对话作为一个经验单元存储（不拆分）。适用于：完整交互过程、上下文场景。"
```

---

### I-031: ✅ learn.update 缺少使用范例

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `capabilities/ops/learn/update.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`learn.update` 是最常用的操作之一（改名、改描述），但 description 中没有 JSON 范例：

当前：
```
"更新身份或知识的字段（名字、描述、内容等）。target_id 是目标单元 ID（身份为 identity），updates 是要修改的字段键值对。"
```

对比其他 op：
- `memory.save`：有范例 `content='今天学习了Rust', type=semantic, confidence=0.8`
- `memory.delete`：有范例 `memory_id='abc123'`

**影响**：
- LLM 需要从 schema 推断参数格式，可能出错
- 实际使用中（如 session.json 中的"改名"场景），LLM 需要额外推理

**修复方案**：
```rust
// 修复后
"更新身份或知识的字段。范例：target_id='identity', updates={name:'李四'} 或 target_id='abc123', updates={description:'新描述'}"
```

---

### I-032: ✅ memory.graph_query 子操作说明不清晰

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/ops/memory/graph_query.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`memory.graph_query` 定义了 10 个参数（graph_operation, node_id, max_hops, limit, source_id, target_id, max_depth, relation_types, infer, type_name），但不同子操作只用其中部分参数。

当前 description 只有一个简单范例：
```
"图结构查询。范例：graph_operation=get_neighbors, node_id='node1', max_hops=2"
```

LLM 需要理解"哪些参数对应哪些子操作"，当前没有说明。

**影响**：
- LLM 可能传递错误的参数组合
- 增加调试难度

**修复方案**：
```rust
// 修复后
"图结构查询。子操作及参数：\n\
- get_neighbors(node_id*, max_hops, limit): 获取邻居节点\n\
- find_path(source_id*, target_id*, max_depth): 查找路径\n\
- get_subgraph(node_id*, max_depth, limit): 获取子图\n\
- infer_relations(node_id*, limit): 推断隐含关系\n\
- query_by_type(node_type*, limit): 按类型查询\n\
- find_bridge_nodes(limit): 查找桥接节点"
```

---
## 已修复问题（v37 补充修复）

### I-034: 种子数据中引用旧工具名 ✅

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `manager/expert_aus.jsonl`, `manager/normal_aus.jsonl`, `manager/thinker_aus.jsonl` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

三个智能体配置文件中的规则和策略引用了已不存在的旧工具名：
- `agent_memory` → 应为 `agent_cognition(memory.retrieve)`
- `agent_learn` → 应为 `agent_cognition(learn.extract)`

**修复**：更新所有 jsonl 文件中的工具名引用。

---

### I-035: memory.graph_query 子操作缩进格式 ✅

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/ops/memory/graph_query.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

子操作没有缩进，看起来像是独立的操作而不是 memory.graph_query 的子操作。

**修复**：调整字符串格式，使用 2 空格缩进。

---

### I-037: get_prop_cu_snapshot 未加载 kind 类型的 CU ✅

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `engine/conversation.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`get_prop_cu_snapshot` 函数查询的是 `is_a` 含有 `prop` 的 CU，但 `kind` 类型的 CU 的 `is_a` 字段是 `["kind"]`，不包含 `prop`。


**修复**：修改查询条件，同时查询 `prop` 和 `kind` 类型的 CU：
```rust
let filter = FilterExpr::Or(vec![
    FilterExpr::is_a("prop"),
    FilterExpr::is_a("kind"),
]);
```

---

### I-038: graph_query 描述中暴露内部实现函数名 ✅

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `capabilities/ops/memory/graph_query.rs` |
| 发现时间 | 2026-06-19 |
| 状态 | ✅ 已修复 |

**问题描述**：

`memory.graph_query` 的 description 和 input_schema 中暴露了内部实现函数名：
- `get_neighbors`, `find_path`, `get_subgraph`, `infer_relations`, `query_by_type`, `find_bridge_nodes`

这些是内部实现函数名，不应该暴露给 LLM。

**修复**：

1. 修改 description，使用语义化的子操作名称：
   - `get_neighbors` → `neighbors`
   - `find_path` → `path`
   - `get_subgraph` → `subgraph`
   - `infer_relations` → `infer`
   - `query_by_type` → `by_type`
   - `find_bridge_nodes` → `bridges`

2. 修改 `execute` 方法中的 `match` 分支，使用语义化的名称

3. 修改 input_schema 中的参数描述，移除内部函数名

---

## v46 末期修复（2026-06-20）

> 早于主 v47 段（2026-06-22）的修复批次；为避免与 v47 主段 I-051~I-055 冲突，本段编号改为 V-051~V-055。

### V-051: ARCHITECTURE.md 操作数量严重失实

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `docs/explanation/ARCHITECTURE.md`, `docs/PLAN.md` |
| 发现时间 | 2026-06-20 |
| 状态 | ✅ 已修复 |

**问题描述**：文档声称 ops/ 目录有 26 个操作（memory 8 + reason 1 + learn 8 + plan 6 + metacognition 3），实际仅存在 5 个 memory 操作。`learn/`、`plan/`、`metacognition/`、`reason/` 目录完全不存在。PLAN.md 也标注"25 操作"。

**修复**：更新 ARCHITECTURE.md 架构图和 PLAN.md 能力表，如实反映 5 个 memory 操作。

---

### V-052: RelationPropRegistry 注册机制与认知体系矛盾

| 项目 | 内容 |
|------|------|
| 严重程度 | 🔴 高 |
| 影响范围 | `core/prop_registry.rs`, `capabilities/ops/memory/save.rs`, `handlers/system_prompt.rs`, `store/mindscape/scaffold.rs` |
| 发现时间 | 2026-06-20 |
| 状态 | ✅ 已修复 |

**问题描述**：`RelationPropRegistry` 是一个独立的注册机制，维护一个 `HashSet<String>` 来记录"哪些属性名是关系"。但根据 COGNITION.md §4.5，prop 是否是关系完全由 prop CU 自身的数据决定（`is_a` 含 `relation` + `prop_value_is_a` ∈ {cu, cu[]}），不需要额外的注册表。

**修复**：
1. 在 `CognitiveUnit` 上添加 `is_relation_prop()` 方法，纯数据驱动判定
2. 在 `core/mod.rs` 添加 `query_relation_names(store)` 异步查询函数
3. 替换所有 `RelationPropRegistry` 使用方为直接查询 prop CU
4. 删除 `core/prop_registry.rs` 模块（约 360 行）

---

### V-053: cognitive_feedback_tests.rs 引用已删除模块

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟡 中 |
| 影响范围 | `store/mindscape/cognitive_feedback_tests.rs` |
| 发现时间 | 2026-06-20 |
| 状态 | ✅ 已修复 |

**问题描述**：该文件引用已删除的 `contradiction` 模块（`ContradictionJudge`/`StringMatchContradictionJudge`/`NegationDictionary`）、`deterministic_conflict_id`、`record_conflict_candidate`、`init_conflict_cache_from_store` 等，导致编译失败。

**修复**：删除该文件（测试的功能已全部移除）。

---

### V-054: PRINCIPLES.md 旧接口示例

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟢 低 |
| 影响范围 | `docs/PRINCIPLES.md` |
| 发现时间 | 2026-06-20 |
| 状态 | ✅ 已修复 |

**问题描述**：IAgentStore 示例仍用 `search(&self, query: &str, ...)` 旧接口。

**修复**：更新为 `semantic_search` + `query` 接口示例。

---

### V-055: Scaffold 中 list_active_conflicts 和 VersionedSnapshot 仅测试使用

| 项目 | 内容 |
|------|------|
| 严重程度 | 🟢 低 |
| 影响范围 | `store/mindscape/scaffold.rs` |
| 发现时间 | 2026-06-20 |
| 状态 | ✅ 已评估（保留，标注测试用途） |

**问题描述**：`list_active_conflicts` 和 `list_uncertain_units` 已在本轮清理中删除。但 `VersionedSnapshot`、`snapshot_cache`、`invalidate_snapshot_cache`、`build_validation_snapshot` 仅在测试中使用，无生产代码调用。

**建议**：保留（测试需要），但标注清晰的测试用途注释。

---

## 架构评估（2026-06-20）

### 整体评估

| 维度 | 评估 | 说明 |
|------|------|------|
| **代码质量** | 🟢 良好 | `cargo check` 0 warning，228 测试全通过 |
| **架构一致性** | 🟢 良好 | CognitiveUnit 为统一数据模型，AgentStore 为纯 CRUD 接口 |
| **文档准确性** | 🟢 已修复 | ARCHITECTURE/PLAN/PRINCIPLES 已同步更新 |
| **认知体系自洽** | 🟢 良好 | 关系/类型/层级均由 prop CU 数据驱动，无硬编码注册表 |
| **LLM 主导认知** | 🟢 达标 | 模块仅提供 CRUD + 数据管理，认知判断全部交给 LLM |

### 架构亮点

1. **数据驱动的关系机制**：`is_relation_prop()` 从 prop CU 的 `is_a`/`prop_value_is_a` 派生，运行时可自由扩展
2. **统一认知单元接口**：AgentStore 仅暴露 CognitiveUnit 方法，无 JSON 碎片接口
3. **体系化校验能力**：`validate_cu_with_context` 可查询 store 中的 prop CU 做完整校验
4. **简洁的 ops 注册**：`submit_cognition_op!` 宏 + `OpRegistry` 自注册机制

### 待改进项

| 优先级 | 改进项 | 复杂度 |
|--------|--------|--------|
| P2 | 将 `validate_cu_with_context` 集成到 save op 的写入流程 | 低 |
| P2 | 将 `list_uncertain_units` 提升为正式 op（`memory.low_confidence`） | 低 |
| P3 | 补充 memory.import/export/merge 等数据管理操作 | 中 |

所有已修复问题的详细历史见 [CHANGELOG.md](./CHANGELOG.md)。
