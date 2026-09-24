# VDFS 体系体检报告（2026-09-24，HEAD = 09ddf8b）

> 触发背景：对 VDFS 体系（`plugins/vdfs/*` + `providers/vdfs_service/*` + `symbio_core/vdfs_provider.rs`）
> 做一次系统评估，重点核对「变更通知契约」（`VdfsChange` 载荷）、物理层安全边界、`with_id` 双份、
> `category_dir` 兼容路径，以及安全边界测试强度。逐条给出**可复现证据**（文件:行 / 命令输出）。
>
> 本文件是**一次性审计记录**，不是规范；结论落地后按约定自行归档或删除。

## 0. 基线与方法

- HEAD：`09ddf8b`（2026-09-24 21:54，refactor(vdfs): 地址从节点挪到条目，VdfsItem 拆出）。
- 方法：①读 `docs/DECISIONS.md` 的 ADR-011 / ADR-025 全文；②读 `vdfs_provider.rs` 的 `VdfsChange`
  定义（第一手核实，不凭快照）；③读 `physical.rs` 的 `FsPolicy` 与 `path_allowed`；④读
  `vdfs_service/entry.rs` 与 `single_file.rs` 的 `category_dir` / `for_category` 注释；⑤核对
  `with_id` 在 model / mcp 两处的位置与 ADR-011 依据。

> **方法论注**：本次评估发现，上一轮凭印象把 `category_dir` 判为「迁移残留（技术债）」是**误判**——
> 读注释后发现它是有明确职责的兼容路径。教训：**评估结论必须逐条核实代码注释与 ADR，不凭印象**。

## 1. 变更通知契约（`VdfsChange`）—— 现行事实

**权威定义**（`symbio_core/vdfs_provider.rs:1003-1035`，ADR-025 之后）：

```rust
pub struct VdfsChange {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,   // 业务载荷（ChatMessage / VdfsNode）；缺失 = 无载荷（回读收敛）
}
impl VdfsChange {
    pub fn bare(path) -> Self { data: None }                       // 无载荷：回读收敛
    pub fn with_data(path, data) -> Self { data: Some(...) }       // 带载荷
}
```

**演进脉络（已核实）**：

| 时点 | 决策 | 出处 |
|---|---|---|
| S16–S19 | 消息挂 VDFS 变更频道（`appended`+`delta`、`truncated`、`renamed`+`to`、`node`/`content`） | ADR-025 背景 |
| S23–S25 + 批次 E/G | 拆掉载荷，`VdfsChange` 收窄为 `{path, change}` 三取值，消息改走 `session/stream` | ADR-011 批次 G 补充（DECISIONS.md:287-292） |
| **ADR-025（2026-09-23）** | **推翻「无载荷」**：加回 `data: Option<Value>` 载荷字段（承载 `ChatMessage`/`VdfsNode`）；`delta` 是 `ChatMessage` 帧内字段（尾部追加、零回读），`VdfsChange` 载荷字段名是 `data` 不是 `delta` | ADR-025（DECISIONS.md:1477-1566）+ vdfs_provider.rs:1003 |

**要点**：
- 取值集合 = `created` / `updated` / `deleted`（无操作枚举，形状收敛到 `{path, data?}`，见 vdfs_provider.rs:978-984）。
- `data` 缺失 = 「变了但不带载荷」，消费端按需回读（资源域删除是唯一表达，回读 `NotFound` 即删除）。
- 判据：**一个载荷字段必须有生产性生产者**（vdfs_provider.rs:970）。
- 会话域：`Transcript::publish` → `with_data(ChatMessage)`，前端 `sessionTranscriptSync.ts` 增量刷新
  （delta 追加、48ms 合帧、零回读、deltaGen 代际防作废），**非防抖重拉**；e2e T9（t9-ws-stream.mjs）
  + 单测 `sessionTranscriptSync.spec.ts`（14 用例）覆盖。
- 资源域：vdfs_service 三实现 `notify_change` → `bare` 无载荷，回读收敛，仅限资源清单类 UI。

## 2. 物理层安全边界 —— 客观事实与待决策项

**权威定义**（`plugins/vdfs/physical.rs:60-104`）：

- `FsPolicy`：`workspace_only: bool`（默认 **false**）+ `allowed_roots` + `forbidden_paths` 黑名单。
- `Default`（:70-85）：`workspace_only = false`，黑名单仅 6 条：`/etc` `/root` `/usr` `~/.ssh` `~/.gnupg` `~/.aws`。
- `path_allowed`（:90-104）：`workspace_only=false` 时，只要不在黑名单且无 `..` 段，**任何绝对路径都可读**。

**观察**：默认全盘可读、仅 6 条黑名单兜底，**安全面偏大**。这是 VDFS「虚拟动态文件系统」设计哲学
（让 LLM 可访问工作区外文件）的**有意产物**，但值得显式确认是否该收紧（如默认 `workspace_only=true`，
或把黑名单做成可配置）。

**待决策**：见 §5 决策项 1。

## 3. `with_id` 双份 —— ADR-011 有意为之

- `plugins/model/plugin.rs:506` 与 `plugins/mcp/plugin.rs:545` 各持一份 `with_id`（manifest 补齐 id）。
- 依据：`docs/DECISIONS.md:332`（ADR-011 后果）——「manifest 补齐 id」这条不变量不再有唯一实现，
  `model` / `mcp` 各持一份 `with_id`。
- 观察：双份是**有意**的（不引入共享抽象），但**没有测试锁等价**——若两处逻辑漂移，无守卫发现。
  **建议**：补一条断言测试锁两处 `with_id` 行为等价（输入同一 manifest，输出一致）。

## 4. `category_dir` / `for_category` —— 非残留，是有意的兼容路径

**纠正上一轮误判**：

- `vdfs_service/entry.rs:34` `category_dir`：注释明确「**生产代码不应调用它**——插件一律用父插件经
  `PLUGIN_DIR` 告知的目录……当前仅剩两处合法使用：① 读旧版历史落位的数据迁移（model 的旧分类 `ai`）；
  ② 测试构造」。
- `vdfs_service/single_file.rs:40` `SingleFileVdfs::for_category`：注释明确「**仅迁移 / 兼容旧落位使用**……
  当前唯一使用者是 model 插件迁移旧分类 `ai` 的那段代码」。

**结论**：`category_dir` / `for_category` 是**有意保留的、服务于数据迁移的兼容路径**，不是残留/技术债。
上一轮评估凭印象判为「迁移残留」是**误判**（教训：评估必须读注释核实）。

## 5. 待决策项（需人确认）

1. **物理层默认安全面**：`workspace_only=false` 全盘可读是否该收紧（默认 true / 黑名单可配置）？
   这是 VDFS 设计哲学的有意产物，但安全面偏大，需确认取舍。
2. **`with_id` 双份锁等价测试**：是否值得补一条断言测试，防两处逻辑漂移？
3. **安全边界测试强度**：`physical.test.rs` 现有 `workspace_only_confines_absolute_paths` 等用例，
   是否需补充「黑名单命中」「`..` 逃逸」「allowed_roots 边界」等更全的用例？

## 6. 评分（供参考，非规范）

| 维度 | 评分 | 依据 |
|---|---|---|
| 架构 | ★★★★★ | 单一契约 dispatch、单一地址门面 fs.rs、单向分层、克制 |
| 文档一致性 | ★★★★★ | CURRENT.md 自动生成 + CI 门禁（60-facts.mjs） |
| 安全 | ★★★★☆ | 默认全盘可读、仅黑名单兜底（安全面偏大，待决策） |
| 重复度 | ★★★★☆ | with_id 双份（ADR-011 有意，缺锁等价测试） |
| 可维护性 | ★★★★★ | 三实现共用 entry.rs 原语，注释详实 |

## 7. 本报告自身的元教训（写给未来的自我）

1. **评估结论必须逐条核实**（读代码注释 + ADR 全文），不凭印象/快照。本次 `category_dir` 误判即教训。
2. **带 ADR 编号的断言尤其危险**：ADR-011 批次 G 说「无载荷」是**历史时点**，ADR-025 已推翻——
   引用 ADR 时必须确认**该 ADR 的当前状态**（是否已被后续 ADR 推翻）。
3. **评估结论要沉淀到 `docs/archive/`**（项目既有惯例），不只留会话记忆——压缩后新会话才能看到。