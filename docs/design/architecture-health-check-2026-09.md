# 架构体检报告（2026-09-19，HEAD = 3c20c72）

> 触发背景：`3c20c72`（统一 VDFS 发现到 `Plugin::get_vfs_provider`）揭示了一个**同源缺陷族**——
> 「同一件事存在**两条并行通道**，其中一条绕过中心化机制」。本报告按该线索对全仓做一次系统排查，
> 逐条给出**可复现证据**（文件:行 / 命令输出 / 进程内探针实测）。
>
> 本文件是**一次性审计记录**，不是规范；结论落地后按约定自行归档或删除。

## 0. 基线与方法

跑过的判定型检查（2026-09-19，工作树干净）：

| 检查 | 结果 |
|---|---|
| `cargo test --lib`（gate 阶段 1） | 793 passed |
| `node scripts/gate.mjs --only=frontend` | 4 / 4 通过 |
| `node scripts/gate.mjs --only=docs,facts` | **14 / 15，facts 阶段失败（见 F-3）** |
| `mechanism-audit` / `plugin-entry-audit` / `grep-audit` / `style-audit` / `doc-link-audit` / `test-layout-audit` / `dead-code-audit` / `protocol-mirror-audit` / `schema-audit` | 通过（schema-audit 为报告型） |

方法：①读本次提交及其前两次提交的 diff；②按「并行通道」模式做横向检索（注册 vs 查询、两份清单、
跨栈镜像、生成器 vs 生成物）；③对最可疑的一处写了**进程内探针**（临时测试，跑完已删）取得实测证据。

## 1. 发现清单（按严重度）

### F-1【P0·正确性】子智能体挂载点穿越只做了一半：`read` / `write` 绕过挂载点

**现象**：`3c20c72` 让 `RelPath::Agent`（首层）与 `RelPath::File`（子路径）穿过 `agent/<id>`
挂载点委托给子 composite——但只落地在 **`list`（vdfs.rs:304/340）、`stat`（:395）、`delete`（:603）**；
**`read`（:425-431）与 `write`（:550-557）仍直读/直写物理 `AgentDirStore`**，`mkdir` / `move_item`
未实现（默认 `NotImplemented`）。同一 provider 同时服务前端链路与 LLM 工具链（`vdfs_*` 取的是同一个
系统根），因此两条链路同病。

**实测证据**（临时探针，子智能体 `reviewer`，父会话 WORKDIR 内放 `AGENTS.md`）：

```
[PROBE] list(reviewer)                = ["session","model","agent","skill","mcp","setting"]
[PROBE] list(reviewer/work)           = Ok([... path: "<VDFS_ROOT>/reviewer/work/AGENTS.md", size: 21 ...])
[PROBE] stat(reviewer/work)           = Ok(VdfsNode { name:"work", hidden:true ... })
[PROBE] read(reviewer/work/AGENTS.md) = Err(NotFound("读取失败：...agent\reviewer\work\AGENTS.md ..."))
[PROBE] write(reviewer/setting/PROBE.txt) = Ok(created:true)
[PROBE] physical exists agent/reviewer/setting/PROBE.txt = true
```

即：**列得出、读不到**；写入**报成功却落到裸 agent 目录**（污染智能体包，且子 composite 的
`setting` provider 完全没参与）。这与 `3c20c72` 修的是同一根因的另一半——「按操作逐个补」导致
只补到会被测试覆盖的那几个操作。

**影响**：子智能体页（前端）与 LLM 的 `vdfs_read` / `vdfs_write` / `vdfs_edit` 对 `agent/<id>/…`
行为错位；`agent/<id>/session`、`agent/<id>/model` 这类**只存在于虚拟视图**的路径必然失败，
而 `agent/<id>/skill/...` 因物理目录恰好同名而「碰巧能读」——最坏的一种不一致（时好时坏）。

**建议**：把「穿过挂载点」收口为**单点判定**（一处 `sub_vfs` 判定 + 九个操作统一走它），
并从「操作枚举」改为「路径前缀判定」；补一条覆盖九个操作的回归测试（现有只有 `list` 一例）。

### F-2【P0·机制失效】`register_vdfs_provider` 已成**只写不读**的死通道，但仍被实现、被断言、被用来生成事实表

**现象**：系统链路改走 `Plugin::get_vfs_provider` 后，`CapabilityVisitor` 上的
`register_vdfs_provider` / `list_vdfs_providers` / `get_vdfs_provider(name)`
（`symbio_core/capability.rs:267-283`、`tools.rs:110-130`）**在生产链路上无任何消费者**：

- 生产者仍齐全：11 个资源插件在 `traverse` 里照旧注册（gateway / local / mcp / model / session /
  setting / skill / telegram / web / work / agent）。
- 消费者只剩两处**测试**：`plugins/session/plugin.test.rs:197`、`plugins/work/plugin.test.rs:60,114`
  ——它们断言的是一个不再影响任何用户可见行为的通道（守卫已「空转」）。
- `agent/host/scope.rs:146-150` 仍为这条死通道做前缀包装（`agent/<id>/<name>`）。
- **`scripts/gen-current-facts.mjs:336-338` 仍从 `register_vdfs_provider(...)` 提取「VDFS 挂载点」列**
  → 权威事实表的该列由死通道生成；`docs/CURRENT.md` 的读表须知（脚本 :628-629）也仍以它为定义处。

**影响**：这正是本次事故的同款隐患——**一条看起来权威、实际不生效的通道**。下一位改动者若只改
`register_vdfs_provider`（文档还在教他这么做），将看不到任何效果，且所有守卫全绿。

**建议**：二选一，不要留中间态——① 删除该通道（连同 `scope.rs` 的包装与两处空转断言，
并把生成器改为从 `get_vfs_provider` / 实例表挂载名提取）；② 明确保留并写清「仅 LLM 工具用的
按名查询」，且为其补一个**真实消费者**与守卫。

### F-3【P0·门禁】`gen-current-facts --check` 在 HEAD 上必红，且**无法通过重生成修好**

**现象**：`docs/CURRENT.md` 与生成器已互相矛盾：

- 生成器 §4 是**手写行**（`scripts/gen-current-facts.mjs:691`）：
  `| Agent bundle | bundle 目录 … | BundleStore 自管 … |`
  ——`f5ba1b3d` 已把该概念整体改名为 agent 目录 / `AgentDirStore`，但**只改了生成物，没改生成器**
  （`git show f5ba1b3d --stat` = 仅 `docs/CURRENT.md` 1 行）。
- 因此**重生成**会把正确措辞**退回**旧词（本报告作者已实测：重生成后 diff 出现
  `+Agent bundle … BundleStore`），而现行文件若保留正确措辞，`--check` 又永远不相等。
- 叠加第二重真漂移：`3c20c72` 未重生成，代码行数已变（`symbio\src` 49280 → 49466 行，
  测试 18785 → 18817）。

**证据**：

```
$ node scripts/gen-current-facts.mjs --check      → exit 1（docs/CURRENT.md 与代码漂移）
$ node scripts/gate.mjs --only=docs,facts         → 14 / 15，失败项 = gen-current-facts.mjs --check
```

而 CI 正在跑这一阶段（`.github/workflows/ci.yml:174`：`node scripts/gate.mjs --only=docs,facts`），
即**当前 HEAD 会红 CI**。附带一处小缺陷：gate 汇总同时打印 `✗ gen-current-facts.mjs --check`
与 `✓ gen-current-facts --check` 两行（同一项的成败被打印两次），容易让人误判为「通过」。

**建议**：① 把 §4 的手写行改为从代码提取（或至少与真源同处一处常量，消除「手改生成物」的诱因）；
② 重生成 CURRENT.md 并提交，使 CI 恢复绿；③ 顺手修 gate 的重复汇报行。

### F-4【P1·真相源】父子两份插件清单：文档宣称「派生、改一处即一致」，代码是两份手写数组，且内容与三处文档冲突

- `symbio_core/keys.rs:180-195`（`SYSTEM_AGENT_PLUGINS`，14 项）与 `:222-236`（`SUB_AGENT_PLUGINS`，13 项）
  **是两条独立字面量数组**，并非「超集派生」；文档却称「直接复用其超集，改一处即父子一致」
  （keys.rs:201、home/plugin.rs:43-47）——**没有任何守卫**校验二者关系
  （`SUB_AGENT_PLUGINS` 全仓只被 `agent/host/plugin.rs:270` 使用）。
- 更直接的错误：三处文档说子树不含 `model`
  （keys.rs:204-213「只差两个系统级单槽 `model` / `vdfs`」、agent/host/plugin.rs:26 与 :103、
  `plugins/agent/README.md:41`「`model` / `vfs` 不在其中」），**但清单里有 `model`**。
- **实测**：`list("reviewer")` 的返回里**确实有 `model`**（见 F-1 探针输出）。
  而该实例的模型注册被 `SubAgentVisitor::register_model_provider`（scope.rs:123-130）丢弃、
  会话模型取自系统单槽（`session/orchestrator/consume.rs:117` 的 `manager.get_model_provider()`）
  ⇒ 子智能体页上的 `model` 入口是**能配但不生效**的入口。

**建议**：决定 `model` 去留（建议按文档去掉，或按代码改文档 + 明确「子树 model 仅供子树会话」），
并把两份清单改为**真正的派生**（`SYSTEM = SUB ∪ {model, vdfs}`）+ 一条断言测试。

### F-5【P1·守卫缺口】跨栈镜像只守了 5 项，其余「第二份真相」无人看守

`scripts/protocol-mirror-audit.mjs:55-96` 只守 3 对 `kind/ext` + 2 条缺席检查。但前端
`tauri/src/schemas/vdfs.ts:18-35` 另持有 **10 个 VDFS 操作名的字面量副本**（后端 `VDFS_OPS` 共 14 个，
`:56-71`，计数由 `protocol.rs:302` 锁死）、7 个 `status`、多个 `ext` 与 `action` id 的副本。
`plugin-entry-audit.mjs:780-782` 明说「取值只取 Rust 侧常量」，即**不比对前端**；
`mechanism-audit` 的 M-007 只管「前端内部定义权唯一」。
⇒ 后端重命名 `vdfs/list` 或某个 status 时，前端会静默失效而**所有守卫仍绿**。

**建议**：把 `VDFS_OPS` 清单与 `VDFS_STATUS_*` / `VDFS_EXT_*` 纳入 `protocol-mirror-audit` 的镜像对
（前端已是「一个常量一行」，成本很低）。

### F-6【P2·文档漂移】本次提交只更新了 3 份文档，其余仍按旧模型叙述

- `symbio/src/plugins/work/README.md:14` 与 `work/mod.rs:14`：仍把挂载点写成
  `CapabilityVisitor::register_vdfs_provider`（死通道）。
- `docs/design/vdfs.md:540-543`：称 `RelPath::File`（子路径）**均**经 `sub_agent(id).get_vfs_provider()`
  委托——与 F-1 实测不符（只有 3/6 个操作如此）。
- `symbio/src/plugins/agent/host/plugin.rs:293`：注释称「子树里没有 `agent` 实例」，而常量与实测
  （`list("reviewer")` 含 `agent`）都相反。
- `scripts/gate.mjs:94` 注释仍在说 `BundleStore` 的 prompt/skill/mcp 分类扫描。

### F-7【P2·清理项】死导出与吞错告警

- `node scripts/schema-audit.mjs`：后端 `SchemaResponse`、`OPTION_PICK_FILE`（已承认保留）、
  前端 `vdfs.ts` 的 `VdfsAccess` / `VdfsActionFile` / `VdfsValidationError` / `SessionRoute` /
  `OUTCOME_*`、`message_prompt.ts` 4 个类型等，均为**无人引用的死导出**。
- `node scripts/grep-audit.mjs`：27 条 `let _ = ….await` 吞错 WARN（多数是有意为之，例如
  gateway 写回失败、shell 杀进程）；但 `home/plugin.rs:142,548` 的 `flush()`、
  `session/chat_loop/compress.rs:199` 的 `fire_hook` 值得逐个确认是否该记日志。

## 2. 根因：一个模式，三种表现

三次同类问题都长在「**并行通道**」这一根上：

1. **广播 vs 查询**（`3c20c72` 已修）：系统链路借 `traverse` 广播收集 → 绕过 `root_hidden` 过滤。
2. **修一半留一半**（F-1）：新增查询通道后，**按操作逐个迁移**，漏掉的 `read`/`write` 仍走物理路径。
3. **新旧并存成死通道**（F-2）：旧广播通道**保留不删**，于是「实现、测试、生成器、文档」四方仍指着它。

推论：**只靠「改对代码」不能防复发**——需要机械守卫盯住「通道是否有生产消费者」「父子清单是否同源」
「生成器是否含手写事实」「跨栈镜像是否逐字相等」。现有 9 个审计脚本覆盖了「路由 / 词表 / 地址 /
死码 / 布局」，但**没有一条能发现上面四类**。

## 3. 建议的新增守卫（按性价比排序）

| id | 判据 | 能拦住 |
|---|---|---|
| G-1 | 子挂载点「九操作一致穿越」测试（`agent/<id>/<子目录>` 逐一 list/stat/read/write/delete/mkdir/move/action/watch） | F-1 及后续任何「漏迁移」 |
| G-2 | trait 注册方法必须有**生产**消费点（`register_*` 与 `list_*`/`get_*` 成对；仅测试引用视为无消费者） | F-2、以后任何死通道 |
| G-3 | 生成器**不得包含手写事实行**（除脚注外，所有事实行须含模板变量或来自代码提取） | F-3 的「手改生成物」诱因 |
| G-4 | 父子清单同源断言（`SYSTEM == SUB ∪ {…}` 或改派生） | F-4 |
| G-5 | 跨栈镜像扩到 `VDFS_OPS` / `VDFS_STATUS_*` / `VDFS_EXT_*` | F-5 |

## 4. 需要人决策的两处

1. **`model` 是否留在子树**：文档说去掉、代码留着、实测可见且不生效（F-4）。
2. **死通道去留**：删（推荐，与 `store_kind` 三后端那次处理一致）还是补真实消费者（F-2）。
