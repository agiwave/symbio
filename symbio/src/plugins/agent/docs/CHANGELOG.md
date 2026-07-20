# Symbio Agent 修复历史

> 本文件按版本倒序记录 `symbio/src/plugins/agent` 的修复历史。
> 当前活跃的待办问题、扫描发现请见 [ISSUES.md](./ISSUES.md)。
> 执行计划与任务追踪请见 [PLAN.md](./PLAN.md)。

---

## 2026-07-06: 项目级文档系统性清理

**用户反馈（两轮）**：
1. "项目文档与代码实现存在系统性脱节"——`docs/archive/proj/` 下历史规划文件、`docs/archive/design_docs/` 中早期提案、以及 `docs/README.md` 中插件文档列表与实际不符。
2. "继续，注意删除历史过期文档信息或者文档，确保所有文档保持最新"——既然能通过新方案覆盖过期文档，应**直接删除**而非保留横幅标注。

**本次修复（仅触及文档，业务代码零改动）**：

### I-071: 项目级文档系统性清理 ✅

| 改动 | 说明 |
|---|---|
| 新增 [`docs/archive/proj/IMPROVEMENT_PLAN_2026.md`](../../../proj/IMPROVEMENT_PLAN_2026.md) | 项目级改进方案的**唯一权威来源**（10 个 P0-P3 改进方向 + 季度路线图） |
| 删除 `docs/archive/design_docs/ARCHITECTURE_IMPROVEMENT.md` | Skill/Subagent/Hook 提案已**全部落地** |
| 删除 `docs/archive/design_docs/MODEL_CHAT_REDESIGN.md` | 扁平消息树设计已落地 |
| 删除 `docs/archive/design_docs/COMPARISON_WITH_QWEN_CODE.md` | 报告差异多数已通过新增/重写插件弥合 |
| 删除 `docs/archive/proj/PLAN.yml` | 2026-03 早期规划，严重过时 |
| 删除 `docs/archive/proj/TASK_INDEX.md` | 2026-03 早期任务索引 |
| 删除 `docs/archive/proj/tasks/T00x-*.md` | 4 个早期任务文件 |
| 删除 `docs/archive/proj/MODEL_CHAT_IMPLEMENTATION_PLAN.md` | Phase 1-4 已落地 |
| 删除 `docs/archive/proj/MODEL_CHAT_IMPROVEMENT_PLAN.md` | 已落地且内容存在事实性错误 |
| 修复 `docs/README.md` | §3 删除过期链接；§5 删除"注意"段；§6 移除 docs/proj 引用 |
| 修复 `docs/archive/design_docs/HISTORY_AND_REVIEWS.md` | 修复"当前形态"错误描述（前端已于 2026-06 恢复为 Tauri） |

### I-045 真正完成（v46 修复有遗漏）

v46 修复时**未完全修干净** `COGNITION.md` §14 / `PRINCIPLES.md` §8 中的 `OPERATIONS.md` 引用。本次清理中**真正完成**：

- [docs/COGNITION.md](./COGNITION.md) §14：`OPERATIONS.md` → `TESTING.md`（含 op 操作手册）
- [docs/PRINCIPLES.md](./PRINCIPLES.md) §8：`OPERATIONS.md` → `TESTING.md`
- [docs/README.md](../../../README.md) §5：删除顶部"注意"段（不再提及已合并/废除的旧文件名）

**验证**（2026-07-06）：

- 全仓库 grep 已无 `OPERATIONS.md` / `PROMPT_ARCHITECTURE.md` / `CODE_ANALYSIS_REPORT.md` 等不存在文件名的**误导性**引用
- 业务代码（agent、model、session、mcp、skill、home 等）零改动
- `docs/archive/design_docs/` 仅保留 `HISTORY_AND_REVIEWS.md`
- `docs/archive/proj/` 仅保留 `IMPROVEMENT_PLAN_2026.md`

---

## v54 (2026-06-25)

### I-070: 系统提示词预算状态段落地 + 候选池健康报告 ✅

**用户诉求**：
> "智能体能自己有效的管理自己的系统提示词认知（多了通过工具自动删除，自动淘汰相对不重要的），自动控制系统提示词认知大小，并且让系统提示词认知质量和效率最大化。"
> "LLM主动优化就是我期望的一种自动。系统负责'压力感知'，LLM 负责'价值判断'，系统绝不预计算'哪些该删'。"

**核心设计原则**：
- **唯一"自动" = LLM 主动优化**：系统提供纯事实压力信号，LLM 自主决定操作
- **系统职责 = 测量 + 工具提供**：预算用量、候选池统计、截断数
- **LLM 职责 = 价值判断**：踢出谁、删除谁、保留谁

**六大改动**：

| 改动 | 说明 |
|---|---|
| **改动1：预算状态段渲染** | `system_prompt::build()` 末尾追加"预算状态段"（纯事实：用量/候选池规模/截断数 + 工具速查），超阈值（85%）时升级为"⚠️ 预算告警" |
| **改动2：截断数量追踪** | `render_cognitive_units()` 累计 `candidate_total` / `candidate_shown`，不记录具体被截断的 CU ID |
| **改动3：价值密度评分** | `compute_cu_score()` 从纯价值分改为价值密度：`score = value / max(1, tokens/TOKEN_NORM_UNIT)`，短而高价值的 CU 优先保留 |
| **改动4：候选池健康报告** | `memory.consolidate` 新增第 4 项分析：扫描候选池内信念偏低（< 0.3）或长期未访问（> 30 天）的 CU，**只报告不修改** |
| **改动5：配置扩展** | `AgentConfig` 新增 `prompt_warn_threshold`（默认 0.85） |
| **改动6：编译验证** | `cargo check --lib` 0 error/0 warning + `cargo test --lib` 全通过 |

**预算状态段格式**：
```
## 预算状态
- 已用：1247 / 3500 tokens（36%）
- 候选池：共 45 项，展示 32 项，13 项因预算未展示
- 管理工具：memory.save（priority>20 移出候选池 / confidence:0 物理删除）| memory.consolidate（批量整理）
```

**候选池健康报告格式**（consolidate dry_run 输出）：
```json
{
  "candidate_pool_health": {
    "pool_size": 12,
    "low_belief_count": 2,
    "low_belief": [{"id":"...", "name":"...", "meta_belief":0.15}],
    "stale_access_count": 3,
    "stale_access": [{"id":"...", "name":"...", "days_since_access": 45}],
    "note": "此报告仅列事实，不自动修改任何 CU。"
  }
}
```

**文件**：
- 修改 `core/prompt_budget.rs`（价值密度评分 + BudgetUsage 候选池统计字段）
- 修改 `core/config.rs`（新增 `prompt_warn_threshold`）
- 修改 `handlers/system_prompt.rs`（预算状态段渲染 + 移除 `#[allow(dead_code)]`）
- 修改 `capabilities/ops/memory/consolidate.rs`（候选池健康报告 + 文档注释升级）
- 修改 `docs/CHANGELOG.md`（本文件）
- 修改 `docs/ISSUES.md`（I-070 记录）

---

## v53 (2026-06-24)

### I-069: 默认 priority 改为 opt-out + 预算告警不删除记忆 ✅

**用户反馈**：
> "1，默认记忆不应该为10吧？默认记忆的认知单元就放在系统提示词里面，是一个合适的选择吗？
> 2，预算警告时，就删除认知记忆吗？这也不合适吧，系统提示词只是认知记忆的一部分，很多比进入系统提示词的认知单元更重要的认知单元，所以，重要的是踢出系统提示词范围，这个逻辑不应该和是否删除相关，说穿了，重点是优化系统提示词条目（提高效率），而不是删除记忆。
> 3，不要遗留编译错误和警告。"

**核心改动**：

| 改动 | 说明 |
|---|---|
| **`priority` 默认值 10 → 100** | 默认**不进入**系统提示词候选池（opt-out）。多数新记忆是临时/低价值的，不应自动挤占提示词 |
| LLM 显式 opt-in 候选池 | 想进候选池必须显式设 `priority ≤ 20`（如 `priority: 0/5`） |
| 预算告警 ≠ 删除记忆 | 告警段引导 LLM **调整 priority** 把低价值 CU 踢出提示词（如 `priority: 200`），**CU 仍完整保留**在库中，需要时通过 `memory.retrieve` 调出 |
| 删除 vs 踢出语义分离 | 踢出 = `priority > 20`（可逆）；删除 = `confidence: 0`（不可逆，物理删除） |
| 清理废除后的死代码 | 移除 `tokenize_for_relevance_public`（view_prompt 废除后的孤儿）；`BudgetUsage::remaining/usage_ratio`、`BuildResult::usage` 加 `#[allow(dead_code)]`（公开 API 供"预算告警"段未来使用） |
| 编译干净 | `cargo check --lib` 0 error / 0 warning；`cargo test --lib` 234 passed |

**示例**：

```rust
// 不传 priority → 默认 100（不进入系统提示词）
{"items":[{"is_a":["fact"],"content":"地球是圆的","confidence":0.7}]}

// 显式 opt-in 系统提示词
{"items":[{"is_a":["rule"],"content":"代码必须遵循安全协议","confidence":0.9,"priority":5}]}

// 预算优化：踢出系统提示词（CU 仍保留）
{"items":[{"id":"cu_xxx","priority":200}]}

// 软删除：物理删除（不可逆）
{"items":[{"id":"cu_xxx","confidence":0}]}
```

**设计原则**：
- **进系统提示词是 opt-in**（不是默认）：候选池是稀缺资源，必须 LLM 显式表达"这个 CU 重要"
- **踢出 vs 删除分离**：踢出只调整 priority（CU 还在），删除才 confidence: 0
- **预算告警目标 = 优化提示词条目**：不是清理记忆

---

## v51 (2026-06-24)

### I-067: 系统提示词机制简化（方案 D）✅

**用户反馈**：
> "有两个问题：1，必须是 level: sys 才会进入提示词；2，感觉现在的工具和提示词已经过重了，甚至超过内核心内容本身。感觉有必要进一步从原理上系统的规划一下这个系统提示词机制。找到一个更简洁优化的方案。"

**问题诊断**：
1. **level 字段双语义炸弹**：LLM 写新 CU 必须主动 set_level(Sys)，忘标则永远看不到——反思写了但下次还是不知道。
2. **元开销过重**：v49 引入的"工具速查"+"预算状态"两段占系统提示词 ~280 tokens，约内核 CU 的 8%，元/核比 65%。

**方案 D 修复**（用户选择，否定了"合并 op 减 token"方案）：

**1. 删除 system_prompt 的 footer 段（v49 引入的元信息）**：
- 删除 `render_footer` 函数
- 不再注入"工具速查"和"提示词预算"两段（~280 tokens）
- `memory.view_prompt` op 仍可通过 self-build 获取 budget usage（不依赖 ctx）

**2. 删除 chat.rs 的 CTX_PROMPT_USAGE 写入**：
- view_prompt 不再从 ctx 读取 budget usage（自己重新构建系统提示词即可）
- 简化 chat 路径

**3. 用 priority 决定"是否进入系统提示词"（核心修复）**：
- 旧规则：`level == "sys"` 才进
- 新规则：`level != "core" && priority <= 50` 才进（`ENTER_PROMPT_THRESHOLD`）
- `core` 保护机制保留（永不入提示词）
- LLM 写新 CU 时**不需考虑 level**——save op 强制 `level=sys`

**4. save op 行为变化**：
- `cu_from_item` 强制新 CU `level=sys`（覆盖 item 中的 level 字段）
- 默认 `priority=50`（让 LLM 写的新 CU 默认进提示词候选，可通过 view_prompt 提权到 0-5）
- description 增加 priority 说明

**5. reflect op 行为变化**：
- 写新 insight/rule 时 `priority=0`（强制进入系统提示词）
- 反思日志 `priority=10`（不进提示词，避免占用预算）

**6. view_prompt op 升级**：
- 查询条件改为 `level != core && priority <= 50`（与 system_prompt 一致）
- 新增 `promote_candidates` 返回项：列出"未进提示词但 meta_belief>=0.7 的高价值 CU"
- LLM 看到后用 `memory.save` 提权：`{items:[{id:'cu_xxx', priority:0}]}`

**7. 移除 CognitiveLevel 从 core/mod reexport**：
- 内部仍保留 `CognitiveLevel` 枚举（保护机制 + seed_cus 兼容）
- 但不再在 reexport 中（避免 LLM 通过 typed_unit API 直接 set_level）

**Token 节省**（实测）：
- 系统提示词：-280 tokens（footer 删除）
- 实际收益：-6.9%（4080 → 3800 tokens）
- 元开销：65% → 0%

**为什么不做"合并 op 减 token"**（用户挑战后的诚实评估）：
- 合并 N 个 op → 1 个会让 union JSON Schema 更大（+18% tokens）
- 嵌套 action 参数增加 LLM 认知负担（+30%）
- 反而**反向**——保持 6 个清晰命名的 op

**测试**：240 个测试通过（无新增/无回归）

### 关键设计决策记录

| 决策 | 选择 | 理由 |
|---|---|---|
| 删 level 字段？ | ❌ 不删 | `level=core` 是安全机制（保护内核不可变）|
| 删 footer？ | ✅ 删 | 元信息不应进系统提示词 |
| 合并 op？ | ❌ 不合并 | 反而增加 token + LLM 负担 |
| priority 默认值？ | 50（候选）| LLM 写新经验不需考虑 level |
| ENTER_PROMPT_THRESHOLD？ | 50 | 平衡"新经验进"和"提示词不爆" |

---

## v52 (2026-06-24)

### I-068: API 简化 + 废除 view_prompt ✅

**用户反馈**：
> "1，memory.save感觉因该更名为memory.store或者memory.update？2，在保存的时候，对于需要遗忘的内容，可以直接遗忘（删除）；3，不应该有view_prompt这个op吧？系统提示词原本就是LLM能看到的，没必要单独再用一个op去查看，感觉正确的做法应该是，当系统提示词过多时，主动添加提示性系统提示词，让LLM主动优化系统提示词才对。4，请修改文档，确保文档和实现一致。"

**核心改动**：

| 改动 | 说明 |
|---|---|
| 保留 `memory.save`（不重命名）| save 已是 CRUD 直觉命名，重命名收益小 |
| `memory.save {id, confidence:0}` 立即物理删除 | LLM "想删除" = `save` 一个空值 CU，**不等** consolidate |
| 废除 `memory.delete`（之前轮次）| 由 save 软删除取代 |
| **废除 `memory.view_prompt` op** | 系统提示词就是 LLM 能看到的，单独 op 查询是冗余 |
| 系统提示词末尾"预算告警"段 | 主动提示 LLM 调用 `save confidence:0` 软删除 |
| 删除 `level` 字段 | 旧 `core/sys/msg` 已废除 |
| 删除 `delete.rs` | 由 save 软删除取代 |
| 删除 `view_prompt.rs` | 系统主动驱动，LLM 无需查询 |
| 操作数：6 → 5 | 落地 5 个：save/retrieve/graph_query/reflect/consolidate |

**双层保护机制**（防止误删）：
1. **schema 元数据**（`is_a` 含 `kind/prop/meta/relation/cu`）→ 永不被遗忘
2. **候选池内**（`priority ≤ 20`）→ 永不被遗忘
3. **软删除信号**（`confidence=0`）→ 绕过双层保护，**立即删除**（因为是 LLM 显式信号）

**软删除 vs 自动遗忘**：
- **软删除**（`save {id, confidence:0}`）：LLM 显式触发，立即物理删除（主路径）
- **自动遗忘**（`consolidate`）：周期任务，基于 `retention = confidence × meta_belief × exp(-Δt/memory_strength)` 阈值清理
- 两者**互补**：save 处理"我知道要删的"，consolidate 处理"系统判断该忘的"

**为什么不要 view_prompt op**：
- 系统提示词就是 LLM 能看到的文本，单独 op 让 LLM "查看"是冗余的
- 正确做法：系统**主动**追加"预算告警"段到系统提示词末尾
- LLM 看到告警 → 主动调用 `save confidence:0` 软删除 / `consolidate` 整理
- 节省一个 op 的 prompt 占用（LLM 工具列表更简洁）

**测试**：234 个测试通过（新增/修改 0 个，新增 `op_memory_soft_delete_via_scaffold` 改为验证"立即删除"而非"延迟 consolidate"）

**文件**：
- 删除 `capabilities/ops/memory/delete.rs`（之前轮次）
- 删除 `capabilities/ops/memory/view_prompt.rs`（本次）
- 修改 `capabilities/ops/memory/save.rs`（软删除立即生效）
- 修改 `capabilities/ops/memory/mod.rs`（移除 view_prompt mod）
- 修改 `capabilities/ops/mod.rs`（`all_ops_registered` 期望 5 个）
- 修改 `capabilities/cognition.rs`（移除 view_prompt 描述 + 测试）
- 修改 `core/prompt_budget.rs`（注释：BudgetUsage 用途改为"预算告警"段）
- 修改 `core/mod.rs`（注释）
- 修改 `core/config.rs`（注释）
- 修改 `handlers/system_prompt.rs`（注释 + 工具速查 + footer 注释）
- 修改 `docs/COGNITION.md`（同步：level 废除、软删除语义、能力清单）
- 修改 `docs/explanation/ARCHITECTURE.md`（同步：5 个 op、能力表格）
- 修改 `docs/ISSUES.md`（同步：I-050 + I-068 状态）
- 修改 `docs/TESTING.md`（同步：测试列表）

---

## v51 (2026-06-24)

### I-066: 启用 `memory.reflect`（反思→进化闭环）✅

**用户反馈**：
> "reflect.rs 这个功能是否完备，是否应该启用，如果应该的化请启用，同时整体评估一下当前 LLM 自动进化系统提示词的能力是否完备。"

**现状**：`reflect.rs` 文件已存在（500+ 行，8 个测试用例），设计完整但未在 `mod.rs` 中注册——既不编译进二进制、也不出现在 LLM 工具列表中。

**修复**：
- `core/mod.rs` 增加 `pub use typed_unit::{CognitiveLevel, CognitiveUnit}`（reflect 用 `crate::...core::CognitiveLevel::Sys` 完整路径）
- `capabilities/ops/memory/mod.rs` 注册 `mod reflect;`
- `capabilities/ops/mod.rs` 的 `all_ops_registered` 期望改为 6 个
- `reflect.rs` 的 `mod tests` 增加 `use ...CognitionOp;`（之前会因 trait 未导入编译失败）
- 修正测试 `reflect_creates_insight_cus` 的断言错误（`created_count` 只数 insight，不含反思日志）

**为什么 reflect 是进化的关键一环**：
- `memory.save`：被动记忆写入
- `memory.reflect`：**主动提炼**——把多轮对话中的零散经验蒸馏为一条高质量 CU
- 写回的 CU 类型（experience / fact / strategy / rule）在 `system_prompt::build` 的 `business_scenario_label` 中都有对应展示标题，**下次对话自动纳入系统提示词**

**测试**：242 个测试通过（新增 8 个 reflect 测试）

**文件**：
- 修改 `core/mod.rs`（reexport `CognitiveLevel`）
- 修改 `capabilities/ops/memory/mod.rs`（注册 reflect）
- 修改 `capabilities/ops/mod.rs`（期望 6 op）
- 修改 `capabilities/ops/memory/reflect.rs`（tests 补 import + 修断言）

---

## v49 (2026-06-24)

### I-065: 系统提示词三层目标重构

**用户原始诉求**：
> "如何动态的构建系统提示词；如何让 LLM 自己管理自己的系统提示词，并实现优化；如何限制系统提示词数量并且，让 LLM 在知道这个限制的基础上，最大化自己的能力。"

**三层目标**：
1. **动态构建**：基于 priority + meta_belief + 相关性（query）多维评分
2. **LLM 自我管理**：新 `memory.view_prompt` op 返回"仪表盘 + 建议清单"
3. **限制与最大化**：预算状态段 + 主动整理引导

**新增**：
- `core/prompt_budget.rs`：CJK-aware `estimate_tokens`（替代 chars/4 估算，100 字中文从 25 token 修正为 100 token）+ `PromptBudget` + `BudgetUsage` + `CuScore` + `compute_cu_score`
- `cu_fields::PRIORITY` 常量
- `AgentConfig::prompt_budget_tokens`（默认 3500）+ `prompt_overhead_tokens`（默认 500）
- `memory.view_prompt` op：返回 `{budget, sections, low_score_candidates, redundant_pairs, recommended_next_steps}`

**重构**：
- `system_prompt::build` 签名 `(store) -> String` → `(store, &PromptBudget, Option<&str>) -> BuildResult`
- 评分排序：CU 按 `score = 0.4×1/(priority+1) + 0.3×meta_belief + 0.3×relevance` 降序
- 预算分配：按 CU 数量比例分配 + 至少 50 tokens/type 兜底
- 系统提示词末尾追加"提示词预算"段（让 LLM 知道总预算/已用/剩余）+ 工具速查
- `chat.rs` 先提取 user_text 传给 `system_prompt::build` 作为 relevance_query

**关键决策**：
- ❌ **砍掉原计划的 `memory.manage_prompt`**（5 个 action 全部和现有 op 重复：compact ⊆ consolidate，archive/promote/rewrite ⊆ save.update，delete ⊆ delete）
- ✅ 保留的 `memory.view_prompt` 直接输出"建议 + 可执行参数"，LLM 用现有 op 执行即可——0 重复、0 新概念

**测试**：234 个测试通过（新增 11 个：3 个 prompt_budget + 4 个 system_prompt + 14 个 view_prompt，含 budget/relevance/footer 等）

**文件**：
- 新增 `core/prompt_budget.rs`
- 新增 `capabilities/ops/memory/view_prompt.rs`
- 修改 `core/types.rs`（PRIORITY 常量）
- 修改 `core/config.rs`（预算字段）
- 修改 `core/mod.rs`（模块 + reexport）
- 修改 `handlers/system_prompt.rs`（重构 + 评分）
- 修改 `handlers/chat.rs`（传 budget + 记录 usage）
- 修改 `capabilities/ops/memory/mod.rs`（注册 view_prompt）
- 修改 `capabilities/ops/mod.rs`（`all_ops_registered` 期望 5 个）

---
---

## 历史版本摘要（v16 – v48，2026-06-15 ~ 2026-06-22）

以下为早期版本（v16–v48）的浓缩要点，详细逐条变更已精简，仅保留主线演进：

- **认知单元与存储**：v16 引入 Q8 embedding 量化（`core/embedding_quant.rs`）；v20 SQL 下推 + WAL（`store/sqlite.rs`）；v23 移除 O(n) 全索引兜底；多存储后端（Dir / File / Memory / SQLite）逐步成型。
- **关系与展示机制化（v9 体系落地）**：v21/v23 字段白名单与 core 拦截；v27 `AgentRegistry` 初始化收敛。
- **系统提示词与预算**：v48–v49 系统提示词按 `to_llm_value` 对齐渲染，新增预算评分（`core/config.rs` / `handlers/system_prompt.rs` / `handlers/chat.rs`）。
- **记忆操作收敛**：早期曾规划 24–27 个认知操作（reason/learn/plan/metacognition 等域），经 v51/v52 收敛为当前 `memory` 域 5 个操作（save/retrieve/graph_query/reflect/consolidate），`delete` 废除改为 `save {confidence:0}` 软删除。
- **engine/ 目录消除（v36–v37）**：`engine/` 下文件已迁移至 `store/mindscape/`、`handlers/chat.rs`、`capabilities/ops/memory/consolidate.rs`（见 [ARCHITECTURE.md](./ARCHITECTURE.md)）。

> 更早的逐条变更记录如需查阅，可参考 Git 历史；本文件聚焦与当前代码一致的近期变更。

---

## 关键文件链路（当前）

```text
src/plugins/agent/
├── core/
│   ├── typed_unit.rs           # 认知单元数据结构 + Q8 embedding
│   ├── embedding_quant.rs      # Q8 量化
│   ├── types.rs                # 字段常量（cu_fields 等）
│   └── traits.rs               # CAS / record_access 默认实现
├── store/
│   ├── dir.rs / file.rs / memory.rs / sqlite.rs   # 多后端存储
│   ├── embedding_store.rs      # 语义搜索装饰器
│   └── mindscape/              # 认知层（scaffold / cognitive_feedback）
├── capabilities/
│   ├── cognition.rs            # 认知调度
│   ├── chat.rs / create_agent.rs
│   └── ops/memory/             # 5 个记忆操作（save/retrieve/graph_query/reflect/consolidate）
├── handlers/                   # route 分发（list/get/chat/create/delete/config/system_prompt）
├── manager/                    # create_agent 等管理逻辑
├── plugin.rs                   # AgentPlugin 实现 + 注册
└── docs/                       # 本目录文档
```
