# 会话 / 转写的前端性能设计

> 目标：解决「会话列表显示效率很低」。
> 取向：**靠机制，不造特例** —— 判定标准是「只有会话能用吗？」是，就是特例，不做。

---

## 1. 问题：现在的成本模型

| # | 热点 | 代价 |
|---|---|---|
| H1 | 每个 token 复制整张消息表 | O(n) / token |
| H2 | 根节点判定 `visible.find(...)` | O(n²) |
| H3 | 每次读都排序 | O(n log n) |
| H4 | 对消息树 `watch(deep)`（树里有 `parent` 回指，是环） | 每 token 深遍历 |
| H5 | 节点身份在 token 间churn | 整树重建 |
| H6 | 折叠体用 `v-show` | 收起态仍在 DOM、仍跑 `marked` |
| H7 | 模板与 computed 各算一次 `highlight()` | 双倍 |
| H8 | `marked()` / `highlight()` 无缓存 | 重复渲染 |
| H9 | 无虚拟滚动 | DOM 规模 = 消息数 |
| H10 | 每 token 强制同步布局（读 `scrollHeight`） | 布局抖动 |
| H11 | 整份 `fetchTranscript` | 打开即读全部历史 |

**后端侧**：`vdfs/list <根>/session` 曾经要对每个会话读出并解析**含全部消息的**
`session.json` —— 清单成本 = O(所有会话的全部历史)（见 §11）。

---

## 2. 设计原则

1. **让列表只给结构**，正文按需读。
2. **窗口父闭合**：返回的集合里任一项的 parent 要么为空、要么也在集合里。
3. **折叠即轻量**（数据层也是）：只保留一行预览，展开才取详情，收起即弃。
4. **取向靠机制**：判定标准「只有会话能用吗？」。

---

## 3. 目标一：会话清单有界（默认最新 100）

- `vdfs/list` 可选带 `limit` / `before`；provider 不认则降级全量。
- **不新增专用端点**：走既有调用级参数袋（`VDFS_PARAM_LIMIT` / `VDFS_PARAM_BEFORE`），
  **不改 `VdfsProvider::list` 的签名**，其余 provider 零改动。
- 「100」是**名义值**（对会话清单＝会话条数；对转写＝根节点数，见 §4）。
- ⚠️ 封顶会让更早的条目在侧栏不可见，因此必须与「加载更老」（`before` 游标）配套。
  存储拆分（§11）之后后端成本已不再是瓶颈，封顶的收益主要在前端渲染，
  是否启用需权衡。

---

## 4. 目标二：转写窗口化（滚动到顶加载更早）

### 4.1 窗口单位 = 根节点（Turn 页），不是消息条数

```text
<根>/session/<id>/消息            ← 列表（seq 升序，含在途消息）
<根>/session/<id>/消息/<mid>      ← 列表项（正文按需 read）

一页 = 一段连续的根节点 + 它们的全部子孙
       ┌─ 根 A ─┬ 子 a1 ── 孙 a1x
       │        └ 子 a2
       ├─ 根 B ─── 子 b1
       └─ 根 C   （用户消息，叶子）
```

「根」= `parent_id` 为空，**或** `parent_id` 指向本页之外。前端据此建树，
**永远看不到悬空父节点** —— 这就是原则 2。

> **「最多 N 条」里的 N 是名义值。** `limit` 数的是**根节点**，不是消息条数：
> 一页 = N 个根 + 它们的**全部**子孙，所以实际返回条数 ≥ N。
> 这么定就是为了**不拆开 Turn** —— 一个 Turn（根 + 全部子孙）要么整棵在页里、
> 要么整棵不在，消费方永远不会拿到「半个 Turn」，会话的历史结构因此保持完整。
> 单测 `transcript_window_keeps_turns_whole` 锁死这条不变量。

### 4.2 协议

```text
vdfs/list { path: "<id>/消息", limit: 30, before: "<id>/消息/<mid>" }
```

- `before` = **已持有的最老列表项的地址**（游标是地址，不是页码）；返回它**之前**的一页。
- 缺省 `limit` / `before` → 全量（今天的语义）。
- **窗口父闭合是 provider 的责任**：`limit` 只是「根节点条数」的目标，
  provider 必须把这一段的子孙一并带上。

这条与既有先例一致：`vdfs/tree` 早就有 `depth` + `limit`。

### 4.3 前端

- 只持有 `[windowStart, newest]` 的节点集。
- 滚动到顶 → 取上一页 → **前插**；前插后补偿滚动锚点（`scrollHeight` 差值回填 `scrollTop`）。
- 到达最老一页（返回条数 < `limit` 或无 `before`）→ 显示「已到最早」。

---

## 5. 目标三：折叠即轻量

- 折叠体改用 **`v-if`**（不是 `v-show`）：收起态不进 DOM、不跑 `marked`、子孙不挂载。
- 数据层：折叠节点只留**一行预览**，展开才 `read` 详情，收起即弃。
- 解析缓存（§5.4）：`marked` / `highlight` 结果按内容键缓存（LRU），
  超长（正在生长）的内容不进缓存。

---

## 6. 协议增量

| 项 | 做法 |
|---|---|
| 有界 `list` | `VdfsPathRequest` 加可选 `limit` / `before` → 宿主注入调用级参数 → provider 自取 |
| 转写树化 | 复合节点（Turn / ToolCall）→ 目录；内容节点 → 文件（**推迟**，见 §8） |
| 删除归一 | 删掉会话专属的 `chat/delete_message` 写路径（**推迟**） |

---

## 7. 前端改造清单（按热点）

| 热点 | 改动 |
|---|---|
| H2 | 根节点判定用 `Set`，O(n²) → O(n) |
| H4 | 引入 `transcriptVersion` 计数器 + 消息表**单一写入口** `commitMessages`；消费方改为浅监听 |
| H6 | 折叠体 `v-show` → `v-if` |
| H7 | `highlight()` 收进 computed，避免与模板各算一次 |
| H8 | `renderMarkdownCached`（LRU，按内容键；超长不缓存） |
| H10 | 滚动合并到每帧一次（rAF） |

---

## 8. 分期与验收

| 阶段 | 内容 | 状态 |
|---|---|---|
| 0 | 前端止血（H2 / H4 / H6 / H7 / H8 / H10），**零协议改动** | ✅ |
| 1 | 存储拆分：`session.json`（元数据+投影）/ `messages.json`（消息），清单只读前者（§11） | ✅ |
| 2 | 有界 `list`：后端 `limit`+`before`；**中栏前端**（首屏一页 + 滚到底续页） | ✅ |
| 2.5 | 转写树化（复合节点成目录） | 推迟 |
| 3 | 删除归一 | 推迟 |

另：`<根>/session` 清单**只列会话**——插件配置文件不再并列其中（可达性不变，
进设置菜单走 ConfigurableVisitor 通道）。

**当前基线**：`cargo test --lib` **528 用例**（原 518）；前端 vitest **18 文件 / 146 用例**。

### 8.1 中栏前端的分页口径（`useVdfs`）

分页做在**机制层**（`composables/useVdfs.ts`），不是会话专供：任何目录都只取
最新 `VDFS_PAGE_SIZE`（100，名义值）条，滚到距底 200px 自动续页，也留一个
显式的「加载更早」按钮。左栏导航**不分页**（项本来就少）。

三个容易踩的点：

1. **`hasMore` 是启发值，不是协议字段**。满页 ⇒ 可能还有。为什么不让后端回
   `has_more`：那要每个 provider 都多算一次；而「多翻一次空页」的代价只是一次
   请求。**宁可多问一次，也不给机制加字段。**
2. **追加时按 `path` 去重**。不认窗口参数的 provider 每次都返回同一整页 ——
   去重后新项为 0，于是 `hasMore` 收敛为 `false`。一次空转换来正确性，
   且**永远不会产生重复项**（比 `has_more` 更稳）。
3. ⚠️ **有界之后不能凭「不在这页里」判「它没了」**。`refresh()` 原本会在选中项
   不在列表里时清理选中态；分页后深链直开一个旧会话就会落空。现在只有
   `!hasMore`（确信已拿全）时才清理。

**门禁**：`cargo check --tests`、`cargo clippy --workspace --all-targets -- -D warnings`、
`cargo test --lib`、前端 `vue-tsc` + vitest、两个审计脚本。

---

## 9. 不做什么 / 取舍

- **不做虚拟滚动**（先）：窗口化 + 折叠后 DOM 规模已被界定；若仍不够，只在根级做。
- **不做前端持久索引 / 本地数据库**：分页由 provider 计算，保持「一个协议、一处注册」。
- **不新增保留段 / 平行项**。

---

## 10. 一句话总结

> 让**列表**只给结构，让**窗口**父闭合，让**正文**按需读、收起即弃，
> 让**身份**在 token 之间保持稳定。四个动作都不需要新协议——只需要给 `list`
> 一个可选的窗口，外加把「Turn 没有正文」这件事实在 provider 里承认下来。

---

## 11. 存储拆分：元数据与消息分文件（清单慢的根因）

### 11.1 问题

`Session { id, messages, created_at, updated_at, metadata }` —— 消息**内联**在
`session.json` 里。于是列一次清单 = 读 + 解析**所有会话的全部历史**；
有界窗口只减少「读几个文件」，**不减少每个文件的大小**，治不了这个。

### 11.2 方案：一个目录两个文件

```text
<根>/<safe_id>/session.json     ← 元数据 + 清单投影（小，与聊多久无关）
<根>/<safe_id>/messages.json    ← 消息（大，只有打开会话才读）
```

- `save` 写两个文件（各原子写：tmp + rename），**先消息、后元数据**（先数据后索引）；
- `load` 读两个文件；
- `list` **只读 `session.json`** —— 这是全部收益的来源。

### 11.3 关键：清单投影必须落盘（**以及必须兜底**）

「清单不读消息」的前提是：清单需要的字段不能来自消息。而 `session_node`
原本正是从消息算的：

| 字段 | 原来源 | 拆分后 |
|---|---|---|
| 标题 | `display_title()` → `metadata.title` 否则 `derive_session_title(&messages)` | 落盘 `title` |
| `message_count` | `s.messages.len()` | 落盘 `message_count` |
| `description` | `derive_session_summary(&messages)` | 落盘 `summary` |
| `meta_tags` | `session_meta_tags(s)` | 落盘 `meta_tags` |

它们是**投影**（可重算），不是第二份真相；计算入口唯一：`SessionSummary::of()`。

⚠️ **翻过的车（务必保留的教训）**：投影字段是后加的 `#[serde(default)]`，
**存量文件里没有**，于是清单读到空标题 → 前端回落节点 `name` = 会话 id
→ **侧栏一夜之间全变成短 guid**。补救是两条，缺一不可：

1. **读时兜底**：`SessionMetaFile::into_summary()` 发现 `title` 为空时，
   用**内联消息**就地补算投影（`display_title()` 恒返回非空，所以空标题＝投影缺失）；
2. **迁移时补算**：`split_inline_messages()` 搬消息的同时把投影写进元数据，
   而不是只清空内联副本。

### 11.4 兼容与迁移

- **读兼容**：`messages.json` 缺失 → 回落 `session.json` 里的内联 `messages`；
- **写即迁移**：任何会话被保存一次就自动拆开；
- **一次性迁移 / 修复** `migrate_split_messages()`：幂等，两个方向
  （旧布局 → 拆；已拆但投影为空 → 用消息文件重算）。

⚠️ **数据不可逆**：会话数据目录不在版本控制里（`.gitignore` 第 5 行 `.symbio/`）。
**改磁盘格式前必须先备份**，并同时准备反向迁移。

### 11.5 影响面

| 文件 | 改动 |
|---|---|
| `store/mod.rs` | 两个文件形状 + 读写 + 投影 + 兜底 + 迁移 |
| `types.rs` | `SessionSummary`；`derive_session_summary` / `session_meta_tags` 移入 |
| `plugin.rs` | `session_node` / `nodes_of_sessions` 改收 `&SessionSummary` |
| `heartbeat.rs` / 崩溃清理 | 仍走**全量** `list_sessions()`（它们确实要读消息） |

**不动**：内存中 `Session.messages` 语义、VDFS 对外地址、前端协议。
拆的是**磁盘布局**，不是契约。
