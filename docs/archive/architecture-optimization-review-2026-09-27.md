# 架构优化空间复核：减规模 · 提机制（2026-09-27）

> **文档类型：审计** — 对「架构还能往哪收」的一次系统性勘测。
> 复核对象：`symbio/src`（55,851 行实现 / 24,815 行测试）、`tauri/src`（20,269 / 10,429）、
> `cli/src`（1,573）、`scripts/`（13,176，即机制层自身），共 145 个 `.mjs`/`.rs`/`.ts`/`.vue`
> 语料的抽样通读与量化。
> 判据不是「代码写得漂不漂亮」，而是**同一条语义被手写了多少遍**——
> 重复的每一遍都是未来一个必须同步修改的点。

## 复核方法

- 用 `wc -l` 与 `find` 逐目录量化，取规模分布而非抽样印象；
- 对每个「疑似重复」去代码里数**实例个数**（`grep -c`），不采信目录名与文件名；
- 对每条优化建议追一句「**能不能红**」——不能写进 `scripts/` 守卫的建议，
  按本仓既有判据（`guard-and-adr-health-check-2026-09.md`）降级为「建议」而非「机制」。

---

## 落地进度

| # | 优化点 | 状态 |
|---|---|---|
| 1 | 配置挂载点 provider 收敛 | **✅ 已落地（2026-09-27）**——见 §2.4 |
| 2 | 脚本层解析器收口 | **✅ 已落地（两刀，2026-09-27）**——见 §7.3 / §7.4 |
| 3 | `vdfs/tools` 表驱动 | **✅ 已落地（2026-09-27）**——见 §3.3 |
| 4 | 三种宿主接缝共享原语 | **❌ 调查后否决**（2026-09-27）——见 §5.3 |
| 5 | `session` 定向审计 | **✅ 已出报告**（三处皆不可提取，结案）——见 §6.1 |
| 6 | `Capability` 样板收敛 | **❌ 核实后否决**（真重复 3 处 / 6 行）——见 §4.1 / §4.2 |
| 7 | 前端热点 | **❌ 核实后否决**（目标已实现）——见 §8.2 |

**七条全部有了结论**：三项落地（#1–#3），四项否决（#4–#7）。

---

## 一、结论速览

| # | 优化点 | 类别 | 可收敛规模 | 可机制化 | 核实后 |
|---|---|---|---|---|---|
| 1 | 插件 `get_vfs_provider` / 配置挂载点 `dispatch` 样板 | 减规模 | ~1,200 行 | ✓ 强 | 实测 **4 处**（非 ≥6），净 -9 行但语义副本 4→1 |
| 2 | `vdfs/tools/*` 九个工具近同构（表驱动可替代） | 减规模 | ~700 行 | ✓ 强 | 实测 **-75 行**（9 收 7） |
| 3 | `Capability` 手写样板（22 处 meta+execute） | 减规模 | ~600 行 | ✓ 中 | **否决**：真重复仅 **3 处 / 6 行**（见 §4.2） |
| 4 | 三种宿主接缝（Tauri / CLI / gateway）无共享适配原语 | 提机制 | — | ✓ 强 | **否决**：实为 **-45 行**（3%） |
| 5 | `session` 单插件 16,802 行，内部仍有横向重复 | 减规模 | ~800 行 | 中 | **否决**：三处皆不可提取 |
| 6 | 门禁脚本自身 13,176 行，`gen-current-facts` 与审计器有共享抽取空间 | 提机制 | ~600 行 | ✓ 中 | 第一刀 **-279 行** + 新守卫测试 + 第二刀 **D-006** |
| 7 | 前端 `DetailForm.vue` + `sessions.ts` 等热点仍偏大 | 减规模 | ~500 行 | 中 | **否决**：目标**已实现**——`registry/formWidgets.ts` 已是表驱动（见 §8.2） |

> **「核实后」一列是本报告的态度**：初版估算在实测后**全部显著缩水或被否决**——
> #1 ≥6 处→4 处、#3 600 行→6 行、#4 500-700 行→45 行、#5 800 行→~0、
> #7 500 行→**目标早已实现**。
> 「可收敛规模」是**上界猜想**，不是承诺——只有核实过的那一列才算数。
> **七条里有五条被自己的调查否决**，这不是失败：报告的价值正是**在动手之前**
> 把它们否决掉。

**核心判断**：本仓的**机制层（`scripts/`）已经是全仓最先进的部分**——棘轮、事实生成、
镜像审计、行数守卫一应俱全。真正的优化空间不在「再补几条守卫」，而在
**把「插件各自手写同一套骨架」这件事本身变成机制**：现在守卫在**检查**重复，
但机制没有**消除**重复。这是本次复核唯一的主线。

**同时确立的逆判据**：#3–#7 的调查都是「先数实例 / 先出报告，再决定动不动手」，
结果**全部被否决**。这说明主线判据必须配对一条**反向**判据——
**「有没有真逻辑差异」不满足时，就不该抽象**，哪怕看起来很像重复。
§一 的「可收敛规模」列在这几次里**没有一次是对的**
（估 600 / 500-700 / 800 / 500 行，实测 6 / 45 / ~0 / 已实现）。

---

## 二、最大的单一机会：插件骨架手写重复

### 2.1 现象（实测）

| 重复项 | 实例数 | 说明 |
|---|---|---|
| `fn get_vfs_provider(self) -> Some(self)` | **10** | gateway / local / mcp / model / plugin_manager / session / skill / telegram / web / work |
| `impl VdfsProvider for <Plugin>` | **13** | 上述 10 个 + agent / work / composite |
| 配置挂载点型 `dispatch` 的 List/Stat/Read/Write 四臂 | **≥6** | web / model / skill / mcp / plugin_manager / work |
| `capability_announce_configurable` 调用 | 9 | 形状一致 |
| `submit_object_creator!` | 25 | 每个插件末尾一行 |

### 2.2 重复的**精确形状**

配置挂载点型插件的 `dispatch`，逐个读下来语义**逐字一致**（以 `web/plugin.rs:176` 为样本）：

```rust
match req {
    List  { .. }        => if path.is_empty() { 单节点列表 } else { not_found },
    Stat                => if path.is_empty() { 自身根 } else if path == PLUGIN_FILE { config_file.node() } else { not_found },
    Read                => if path == PLUGIN_FILE { config_file.read(&self.config) } else { not_found },
    Write { content }   => if path == PLUGIN_FILE { config_file.apply(&self.config, &content) } else { not_found },
    _                   => not_found("未知路径"),
}
```

差异点**只有两个**：① 挂载点标题字符串；② 配置结构体类型（`WebConfig` / `ModelConfig` …）。
`mcp` / `model` / `skill` 三处 dispatch 各 230-250 行，其中真正的**业务差异**
（动态子节点枚举）只占一小半，其余是上面这段骨架的展开。

### 2.3 建议：把一个「配置挂载点」抽成 core 的现成 provider

在 `symbio_core` 侧提供一个**泛型适配器**，插件只需交出两样东西：

```rust
// 插件侧只需声明——不再手写 dispatch
impl ConfigMount for WebPlugin {
    type Config = WebConfig;
    const TITLE: &str = "网络工具";
    fn config_file(&self) -> &PluginConfigFile;
    fn config_slot(&self) -> &RwLock<WebConfig>;
}
// 机制侧一次实现：
impl<T: ConfigMount> VdfsProvider for T { /* 唯一的 dispatch */ }
```

**收益**：删掉 ≥6 处近 100% 重复的 `dispatch`，约 **1,200 行**；且**新增插件从
「记得抄对五条分支」变成「实现一个 trait」**——这是机制能力的实质提升，不只是行数。
**代价**：需要 `PluginConfigFile` 已暴露 `at()` / `node()` / `read()` / `apply()`
（已确认，`dir.rs:291/659/671/681` 全部存在），无需新 API。

**可机制化**：`plugin-entry-audit.mjs`（已 1,033 行）可加一条判定
——「配置挂载点插件不得自写 `dispatch`」，违规即红。

### 2.4 落地记录（2026-09-27）

**做的**：

1. 在 `symbio_core/plugin/dir.rs` 新增 `PluginConfigMount` trait + 泛型 blanket impl
   （`impl<T: PluginConfigMount + ?Sized> VdfsProvider for T`）——四臂 dispatch 现在是
   **唯一一份**。唯一变化点被显式化成两个：`TITLE` 常量（挂载点标题）与
   `after_write()`（可选写后副作用，缺省空）。
   - 命名遵循 `symbio_core/README.md` §1.2 的域前缀表（`plugin` 域 = `Plugin` 前缀）；
   - 导入用根平铺（`symbio_core::VdfsProvider`），不深引 `vdfs::`（README §1.4）。
2. 迁移四个**纯**配置挂载点插件：`gateway`（`after_write` = 重启监听）、
   `local`（= 策略热更）、`telegram`、`web`（后两者无副作用）。
3. 新增守卫规则 **E-011**（`plugin-entry-audit.mjs`）：名单 `CONFIG_MOUNT_PLUGINS`
   内的插件不得出现 `impl … VdfsProvider for …`，违规即 ERROR ——**「差异只声明一次」
   从约定变成了可判定的机制**。带 4 条回归测试（命中 / 正确形态不误报 / 名单外不误报 /
   豁免与空豁免）。

**实测结果**：

| 项 | 变化 |
|---|---|
| gateway / local / telegram / web 实现行 | **-137 行**（各 -30 ~ -38） |
| `symbio_core` 实现行 | +128 行（机制本身的成本） |
| 净变化 | ≈ 持平（-9 行） |

**净行数几乎不变，但这是正确的交易**：121 行 `dispatch` 骨架从**四份**收敛成**一份**，
且新增插件的成本从「抄对五条分支」降为「实现四行 trait」。行数从来不是这项工作的
真正指标——**语义副本数**才是：4 → 1。

**范围修正（与 §2.3 预估的差异）**：§2.3 估「≥6 处」，实测**纯**配置挂载点只有 **4 处**
（gateway / local / telegram / web）。原以为的 `model` / `skill` / `mcp` / `work` /
`plugin_manager` **都不是**——它们的挂载根下有真实资源条（条目 CRUD、动态子节点、
订阅），硬套会让机制去猜插件语义。这正是不该做的（见 §十）。**判据是「挂载根下是否
只有一份配置」，不是「dispatch 像不像」**。

**棘轮**：`line-budget-audit.mjs` 已收紧 gateway(1190) / local(3460) / telegram(887) /
web(1028) 四模块基线，`symbio_core` 基线按机制新增上调至 8143——**棘轮只往下拧**。

---

## 三、第二个机会：`vdfs/tools/*` 表驱动化

### 3.1 现象

`vdfs/tools/` 九个工具文件共 934 行，其中 `read.rs`(104) / `stat.rs`(51) /
`write.rs`(67) / `delete.rs`(62) / `mkdir.rs`(58) / `search.rs`(82) / `tree.rs`(110)
七个的**结构完全相同**：

```rust
pub struct XTool { provider: Arc<ToolVdfs> }
impl Capability for XTool {
    fn meta(&self) -> CapabilityMeta { tool("vdfs_x", <描述>, <schema>, <示例>, None) }
    async fn execute(...) -> { ensure_required(...); provider.<op>(...).await; to_value(...) }
}
```

真实差异：**工具短名、描述文字、JSON schema、转发到 provider 的哪个方法**。
`tree` / `list` 有少量专属加工，`read` 有行号分页（值得保留为独立逻辑）。

### 3.2 建议

用一张**静态表**（`const TOOLS: &[ToolSpec]`）声明七个性状一致的工具，
由一个通用 `VdfsTool<const OP: Op>` 或闭包式 `ToolSpec { name, desc, schema, examples, call }`
在一处实现 `Capability`。`read` 因为带行号+分页语义，**保留为特例**。

**收益**：约 **700 行 → 约 250 行**，且新增一个 VDFS 操作时改一处表项而非新建文件。
**代价**：`schema` 是 `serde_json::Value`，可继续逐条字面量（schema 本就该显式）。

**可机制化**：`line-budget-audit` 已有 `symbio/src/plugins/vdfs` 基线上限
（3087），收敛后可**收紧基线**，棘轮自动锁住不回弹。

### 3.3 落地记录（2026-09-27）

新增 `symbio/src/plugins/vdfs/tools/spec.rs`，把七个同形工具收成一张表：

- [`VdfsTool`] 是所有表项**唯一**的 `Capability` 实现（`meta` 读表、`execute` 透传）；
- [`ToolSpec`] 承载六个差异点：`name` / `description` / `schema` / `examples` /
  `retention` / `call`；`call` 是**普通 `fn` 指针**（`(Arc<ToolVdfs>, Value, ctx) ->
  BoxFuture<…>`）——`async` 闭包不能进 `static`，故用指向 async 块的函数指针。
- 新增一个「转发 + 回执」型 VDFS 工具 = **加一条表项**，不再新建文件。

**边界（为什么不是九个全收）**：`vdfs_read`（行号 + 分页）与 `vdfs_list`
（ignore glob 过滤 + 目录优先排序）带**表装不下的呈现加工**；硬塞进表会逼出一个
「按工具名分支」的大 `execute`——那是把七个文件的样板换成一份带七个分支的函数，
**没有消除任何东西**。判据是「有没有表装不下的呈现逻辑」，不是「行数够不够少」。
两者保留为独立文件（`read.rs` / `list.rs`）。

**实测结果**：

| 项 | 变化 |
|---|---|
| `symbio/src/plugins/vdfs` 实现行 | 3087 → **3012**（-75） |
| 删除文件 | `delete.rs` / `edit.rs` / `mkdir.rs` / `search.rs` / `stat.rs` / `tree.rs` / `write.rs`（7 个） |
| `spec.rs` 新增 | 一张 7 项规格表 + 一份 `Capability` 实现 + 两个回执 helper |

工具名与顺序由既有 `tools_cover_all_ops` 测试锁死（`list, tree, stat, read, edit,
search, write, delete, mkdir`），重构后 59 项 vdfs 测试全绿、全仓 961 项 lib 测试全绿。

**棘轮**：`line-budget-audit.mjs` 已把 `symbio/src/plugins/vdfs` 基线收紧至 **3012**。

**与 §2 的差异**：§2 是「消除语义副本」并**新增一条可判定守卫**（E-011）；
§3 是「消除结构副本」，收益更直接（净减行数），但**没有**对应的新守卫——因为
「表驱动」难以在不误报的前提下判定（一个合理的特例工具长得就像违规）。
此处选择**不造守卫**：棘轮（行数基线）已足以锁住回弹，再加一条会误伤的守卫
等于逼出豁免注释（见 §十的判据）。

---

## 四、第三个机会：`Capability` 样板与「同形 trait」的收敛

22 处 `impl Capability` 中，`vdfs/tools` 占 9、`web` 占 3、`local` 占 5。
其中 `meta()` 的构造参数形状统一为
`(name, description, schema, examples, None)`——这已经是**事实上的表**，
只是散在 22 个 impl 里。

**建议**：与 §3 合并处理。`web` 三工具（`http_request` / `web_fetch` / `web_search`）
可共用一张表；`local` 的 `shell` 族因 OS 差异（Windows = `cmd`）需保留运行期解析
（`CURRENT.md` §2 已明确这一动态项）。

### 4.1 核实结论（2026-09-27）：**收益远小于估计，仅一处值得做**

上面「22 处 `impl Capability`」的计数是真的，但**「可减 ~600 行」是错的**。
以最像的 `web` 三工具为例，实测：

| 块 | 行数 | 可表驱动？ |
|---|---|---|
| `meta()` | 35 / 19 / 30 = **84 行** | **否**——三者 `category` 不同（`Network` / `Network` / `SystemOperation`）、schema 形状不同、examples 风格不同。这三样**本就该显式**（§3.2 已言明「schema 本就该显式」） |
| `execute()` 外壳 | 9 × 3 = **27 行** | **是**——三处逐字相同（`execute_inner(args).await` → `to_value`） |
| `execute_inner()`（真逻辑） | 其余 ~685 行 | **否**——`http_request` 的七方法分派、`web_fetch` 的字符边界安全截断、`web_search` 的 Tavily/Serper/DDG 三级回退，**互不相干** |

**关键发现：`execute()` 已经很薄了。** 三处都遵守同一约定——
`execute` 只做「转发 + 序列化」，真逻辑全在 `execute_inner`。也就是说
**`execute_inner` 本身就是那个已存在的共享形状**（每文件一处），
而剩下的 9 行外壳是**已经收敛过一次的产物**。

**可做的**：把外壳收成 core 的一个默认方法或宏（如
`Capability::execute_via_inner`），三处各减 2 行 → **约 -6 行**。

**§4.2 精确测量（2026-09-27，第二遍）**：用 `rust-scan.mjs` 的 `matchBrace` 把全仓
19 个 `async fn execute` 的函数体逐个切出（正则数不准，勿用），按形状分四类：

| 形状 | 处数 | 有效行合计 | 判读 |
|---|---|---|---|
| **A 纯外壳**（`let x = self.execute_inner(..).await?; Ok(to_value(&x)?)`） | **3** | **6** | **唯一真重复** |
| B 含 `execute_inner` 与 `to_value` | 4 | 76 | **非同一形状**——见下 |
| C 含 `to_value` | 2 | 105 | 自有逻辑 |
| D 自有逻辑 | 10 | 394 | 正经实现（含 `subagent` 237 行） |

**B 组（`local` 的 `codebase_search` / `content_search` / `todo_write`）不是同形**：
它们的 `execute()` 体都在做**真实的上下文提取**——前者从 `ctx[WORKDIR]` 取值并校验
非空（缺失即 `ValidationError`），后者从 `session` / `workdir` / `agent` 三个上下文值
**推导一个 key**（`format!("{}::{}", wd, aid)` 或 `session`）。三个工具传给
`execute_inner` 的**参数语义各不相同**（`&workdir` vs `&key`），这是真逻辑差异，
不是样板。↑ 这正是 §10.1 逆判据的又一实例。

**修正之前的数字**：「9 × 3 = 27 行」把**函数签名与参数行**算进了外壳；只数函数体
有效行是 `2 × 3 = 6 行`。6 行**低于 `cargo fmt` 的抖动幅度**（本仓实测 fmt 会让模块
行数 ±1~2 漂移），做它的收益在噪声之下。

**不做的理由（比做的理由更重要）**：
- **`local` 的 6 处根本不同形**：`SecureToolWrapper`（安全包装器）、
  `ShellTool`（OS 差异）、`CodebaseSearchTool`(823 行) / `ContentSearchTool`(396) /
  `AskUserTool` / `TodoWriteTool` 各有独立逻辑。原估把它们算进「可收敛」，
  实测只有 `execute` 外壳那一层是共通的——而那一层**只有 3 处、共 6 行**。
- **`web` 三工具真的该拆成三个文件**：它们分别是「HTTP 客户端」「网页抓取」
  「搜索引擎聚合」，业务逻辑无交集。合成一个表驱动文件只会把三块不相干的
  逻辑塞进同一文件——正是 §3/§5/§6 一致拒绝的那种假抽象。
- **`web` 的 `web_search.rs` 有 371 行、`http_request.rs` 有 308 行**，
  去掉 `meta` 与 `execute` 外壳后**仍是 300 行级的独立实现**。
  「可收敛 600 行」这个数字从来就站不住。

**结论**：本条**否决**（不是「可做但收益有限」）——收益 6 行低于噪声，且再等
更多实例的成本**已经付得起**（`matchBrace` 一跑就数清）。若将来 `execute` 外壳
样板显著增加（如 MCP 侧成批出现），用同一测量脚本重新数一遍再决定。

---

## 五、提机制：三种宿主接缝缺一个共享原语

### 5.1 现象

三种接入各自建树、各自接线，**没有任何共享的适配代码**：

| 接入 | 位置 | 规模 | 接线方式 |
|---|---|---|---|
| 桌面 | `tauri/src-tauri/src/` | 449 行 / 3 文件 | 3 个 IPC command（`route_v2*`） |
| CLI | `cli/src/client.rs` | 686 行 | 进程内直连插件树 |
| 网关 | `gateway/server.rs` | 794 行 | HTTP / WS → 父级路由 |

`gateway/server.rs`（794）+ `tauri/route_connection.rs`（167）+ `cli/client.rs`（686）
= **1,647 行**都在做同一件抽象的事：**「把外部请求帧翻译成 `route_v2`，再把响应/流翻出去」**。
README 自己说 gateway 与 Tauri 的 `route_v2` **同构**——既是同构，就应有共享原语。

### 5.2 建议

在 `symbio_core::plugin` 或一个薄 `symbio::host` 模块里提供
**`RouteBridge`**：持有 `Weak<dyn Plugin>` 根 + 一套「帧编解码 + 连接生命周期
（连接 id / 流式回推 / 关闭）」原语。三个宿主各自只保留**传输差异**
（Tauri IPC / stdio / HTTP-WS），运输无关的那半收敛到一处。

**收益**：三份「同构」接线变成一份，估约 **500-700 行**净减，且——
**这是本仓最关键的一处「提机制」**：它让「新增一种接入方式」从「重抄一遍接线」
变成「接一个 transport」。
**风险**：三种传输的背压 / 分帧语义确有差异（`EventBus` 静默丢帧、WS 分帧），
抽取时必须把**传输无关**与**传输相关**切干净，否则抽象会漏。建议先抽
「帧编解码 + 连接 id 生命周期」，**不动**流控。

**可机制化**：`core-surface-audit` 可在抽完后核对新公开面是否落在 `symbio_core`
准入规则内（ADR-023 依赖方数量判据）。

### 5.3 调查结论（2026-09-27）：**否决抽取**

定向调查报告：[`host-seam-bridge-review-2026-09-27.md`](host-seam-bridge-review-2026-09-27.md)

**上面的收益估算（500-700 行）是错的。** 实测三方接线共约 1,650 行，
其中**真正逐字/近逐字相同**的只有一段：

- `tauri/src-tauri/src/commands.rs:53-88` 与 `gateway/server.rs:502-524`
  的「wire → 扩展桶上下文」（Tauri 侧注释已自承「桶名与 gateway 的 `build_ctx`
  同一份契约」）——合并约 **-35 行**；
- `classify_payload` 从 gateway 提到 core（gateway 文档已自述从两个入口提上来）
  ——约 **-10 行**。

**合计约 -45 行，占三方接线的 3%。** 其余 97% 是三种不同的东西：

| 不可合并的原因 | 证据 |
|---|---|
| **连接身份模型不同** | 只有 Tauri 有 `RouteConnectionManager`（id + 生命周期 + 60s 清理）；CLI 无连接概念；网关用 `handle_ws` 栈帧局部变量 |
| **会话载荷的消费动作不同** | Tauri 注册连接并 emit / 网关 HTTP 折叠为最后一帧 / 网关 WS 双向 select / CLI 根本不走这条（订阅 event_bus）——**四个消费动作** |
| **背压是三种不同机制** | CLI：event_bus **静默丢帧** + resync 标记自愈；Tauri：`app.emit` fire-and-forget，**后端收不到任何信号**；WS：TCP 窗口天然背压，**无丢帧无 resync** |
| **错误形状四种** | Tauri `Err(String)` / HTTP `{"error":…}`+400 / WS `{"Error":[…]}` / CLI 转 stderr |

**README 自称的「同构」只成立于「线上类型 + 路由入口」这一层**
（`PluginMessageWire` / `PluginFrame` / `PluginChannel` 早已在 `transport.rs` 收口），
**不成立于宿主接缝这一层**。

强行造统一 `RouteBridge`，会在 `Session` 分支上立即逼出一个带三个 `match host` 的
上帝函数——**收益 45 行，代价是消灭三份已文档化的差异说明**。

**另需纠正一处判断**：`tauri/route_connection.rs`（167 行）**不是**「半成品共享原语」，
它绑定 Tauri IPC 的 emit/连接语义，另外两个宿主用不到——**它待在 Tauri 是正确归属**。

**建议保留的低风险去重**（合计约 -45 行，可择机做）：

1. 把「wire → 上下文」提到 `symbio_core`（如 `transport::ctx_from_wire`）；
2. 把 `classify_payload` / `PayloadDelivery` 从 `gateway/server.rs` 提到 `symbio_core`。

`RouteBridge` 不持有连接表、不启动转发泵、不定义事件名——那三件事分别属于
三个宿主，不是一个共享原语。

---

## 六、`session` 插件：16,802 行的横向重复

`session` 是单插件最大者，内部已有良好分层（orchestrator / context / chat_loop /
transcript / tools / workdir）。仍有**横向重复**迹象：

- `plugin/vdfs_provider.rs` **1,103 行**，与 `agent/host/vdfs.rs`（771）、
  `composite/vdfs.rs`（650）在**「会话目录 → VDFS 目录视图」**这一映射上高度相关；
- `transcript.rs`(533) / `transcript/inbox.rs`(272) / `transcript/deliver.rs`(220) /
  `transcript/frames.rs`(58) 四文件同属转写域，可核查是否有同形的
  「帧投递」逻辑；
- `context/pipeline.rs`(874) 与 `context/window.rs`(490) 的分工边界需文档化
  （README 已有 L0-L6 分层，但**代码侧**是否与分层一一对应可再核）。

**建议**：本条**不急于动手**。ADR-039 已把 session 按域重组过一轮；
下一步应先做一次**「同一语义在 session 内出现几次」**的定向审计，有实测数字再决定，
避免为拆而拆。→ **该审计已做，见 §6.1（三处嫌疑全部不可提取，本条结案）。**

### 6.1 审计结论（2026-09-27）：三处**均不可提取**

定向审计报告：[`session-duplication-audit-2026-09-27.md`](session-duplication-audit-2026-09-27.md)

上面点名的三处嫌疑，逐一核实后**全部是「表面相似，不可提取」**：

| 嫌疑 | 核实结果 |
|---|---|
| `session/vdfs_provider.rs` ↔ `agent/host/vdfs.rs` ↔ `composite/vdfs.rs` | **不可提取**。数据源三者皆异（`SessionStore`+`active_mgr` / `AgentDirStore` / `PluginRegistry`）；agent+composite 的核心是 **dispatch 转发 + 形状校验**（共 23 处），session 的对应物是 **0 处**；三边**共享函数 = 0 个**。相似性来自「都实现同一 trait + 都消费同一批枚举」——那是**协议**，不是逻辑 |
| transcript 域的「帧投递」 | **不可提取**。`transcript.rs::publish` 已是**唯一出口**；`inbox.rs` 的 4 处 `notify` 是**另一个实体**（队列条目 vs 消息节点），地址族/生命周期/载荷全不同。`frames.rs`(58) 是**真复用门面**（7 个文件引用），名实相符 |
| `context/pipeline.rs` ↔ `context/window.rs` | **不可提取**。一为**执行层**（有副作用、要回滚、要发节点），一为**纯策略层**（零副作用、可单测）。README 决策表**已明确**如此分界；合并会破坏「视图层幂等重算」不变量 |

**代码侧与 README 分层一一对应，无错位**。另需纠正一处转述：
README 只有 **L0 / L1 / L2 + 三个未编号层**，**不存在 L3–L6**。

**实际实施的小修正**：`context/window.rs` 有三处硬编码 `truncate_tokens(_, 24)`
（同值但彼此无关联声明，改一处会漏另两处），已提取为
`ENTRY_NAME_TOKEN_CAP`。这是**真重复**，但收益在一致性而非行数。

**本报告刻意标注的「看起来像重复但正确保留」**（四处，均**保持现状**）：

| 项 | 表面相似 | 实际 |
|---|---|---|
| `agent::mount_path` ↔ `composite::child_path` | **逐字节同构**（agent 侧注释自承同义） | 跨插件边界，`plugin-entry-audit` E-009 禁互引；为 10 行下沉 core 增公开面是净负 |
| `inbox::notify` ↔ `transcript::publish` | 都构造 `VdfsChange` + `notify` | **不同实体**，地址族/生命周期/载荷全不同 |
| `MESSAGE_TOKEN_CAP` ↔ `view::FADE_BUDGET` | 同为 2048 | **同值异义**（消息上限 vs 工具结果预算），合并会误伤 |
| 两处 `frame_no += 1` | 同一自增模式 | 分属消息帧与会话状态帧，各 1 行，抽出收益为负 |

**这正是「定向审计」的价值**：四处在没有实测数字时都是「疑似重复」，
核实后**全部是正确保留**。这也再次印证 §十的判据——
**只看行数的工具会把它们全部报为债务。**

---

## 七、机制层自身（`scripts/`）的优化

### 7.1 现象

`scripts/` = **13,176 行**，其中：

| 脚本 | 行数 |
|---|---|
| `plugin-entry-audit.mjs` | 1,033 |
| `protocol-mirror-audit.mjs` | 979 |
| `gen-current-facts.mjs` | 794 |
| `style-audit.mjs` | 619 |
| `schema-audit.mjs` | 440 |
| `grep-audit.mjs` | 417 |
| `core-naming-audit.mjs` | 397 |

（另各带一份同量级的 `.test.mjs`，回归测试规模与实现相当，这是**正确的**。）

### 7.2 观察

- `plugin-entry-audit`(1,033) + `protocol-mirror-audit`(979) 都做**跨文件符号/路由镜像**
  的解析，存在共享抽取空间（token 化、符号收集、路径归一）；
- `gen-current-facts.mjs`(794) 是**唯一的结构事实生成器**，其解析逻辑与
  `plugin-entry-audit` 有重叠嫌疑——**同一份「插件目录 → 注册名 → 路由」的解析
  写了两遍**是有风险的（两处漂移会同时「通过」）。
- `scripts/gate.d/_shared.mjs` **45,981 字节**（≈1,300 行）是共享库，
  已是正确的收口形态；可评估是否把上述解析也搬进去。

**建议**：抽一个 `scripts/lib/parse-rust.mjs`（或并入 `_shared`），
把「Rust 源里提取路由臂 / 插件表 / trait impl」的解析**收敛成一份**，
让生成器与审计器**共用同一个解析器**。收益不仅是行数，更是
**「生成的事实」与「审计的判定」从此不可能打架**——这是机制能力的关键提升。

### 7.3 落地记录（2026-09-27，第一刀）

**范围修正**：把「路由臂 / 插件表 / trait impl 的解析」全收会动到 `gen-current-facts`
的**事实定义**（哪些算工具、哪些算挂载点），那是语义、不是原语——**共用会制造
耦合而非消除重复**。真正被逐字复制、且**换一份就静默漏报**的，是最底层的
「字符串感知字符扫描」那一层。故先收这一层。

新增 **`scripts/rust-scan.mjs`**（唯一真源），收拢四个函数：

| 函数 | 作用 |
|---|---|
| `skipString` | 跳过字符串字面量（含原始串 `r#"…"#` / `br###"…"###`） |
| `stripComments` | 去注释，**不保留**行结构（给「提取事实」用） |
| `blankComments` | 去注释，**保留**行结构（给「要报行号」的审计用，`html` 开关剥 HTML 注释） |
| `matchBrace` | 字符串感知的花括号配对（未配平返回越界一位，便于 `slice` 取到末尾） |
| `testModuleSpans` / `stripTestModules` / `blankTestModules` | `#[cfg(test)]` 模块的定位与两种剔除形态 |

**关键区分（此前被混用）**：`stripComments`（删）vs `blankComments`（抹）。前者给
生成器，后者给审计——用错了行号全错，报告里的 `file:line` 指到无关代码上。

**收敛的实现副本**：

| 原副本 | 处置 |
|---|---|
| `line-count.mjs`（`skipString` / `matchBrace` / `testModuleSpans` / `stripTestModules`） | 改为从 `rust-scan.mjs` 重新导出，**调用方零改动** |
| `gen-current-facts.mjs`（本地 `stripComments` / `skipString` / `testModuleSpans` / `stripTestModules` / `matchBrace`） | 删 155 行，改 import |
| `plugin-entry-audit.mjs`（本地 `stripComments`（行保留版）/ `matchBrace` / `blankTestModules`） | 删 124 行，改用 `blankComments(html:true)` + 共享 `blankTestModules` |
| `core-surface.mjs` 的 `stripComments` | **不动**——它是「文档注释也算符号来源」的粗筛（两份正则即可），与上者判据不同（见 §十「不该做的」） |

**验证**：`gen-current-facts --check` 输出**与重构前逐字节一致**（120 行）——
这是「纯行为保持」的直接证据；`plugin-entry-audit` 11 条规则全过、55 项回归测试全过。

**机制化**：新增 `scripts/rust-scan.test.mjs`（**19 项**），对每种「朴素实现会翻车」
的输入各钉一条：字符串里的 `//`、`format!("{{}}")` 的字面量括号、未跨行的引号、
`mod tests` 中段之后仍有生产代码、畸形输入不死循环。已登记进 `gate.d/30-docs.mjs`
的 `TEST_ONLY`（与 `color` / `gate.d/_shared` 同列）。

**未做（留在下一刀）**：`plugin-entry-audit`(909) 与 `protocol-mirror-audit`(979)
的**上层**解析（路由臂提取、符号镜像）仍各自实现。这两者判据不同（一个是
「路径真实性」、一个是「前后端镜像」），硬合并会产出一个带双分支的解析器——
与 §3 里拒绝「九个工具全收」是同一条判据：**判据是「有没有真逻辑差异」，
不是「行数够不够少」**。

### 7.4 第二刀：给 `doc-link-audit` 补上它看不见的那一半（2026-09-27）

修 §8 断链时发现的问题比断链本身重要：**`doc-link-audit` 报「失效 0 条」已经很久，
而被修的那两个路径它一个字都看不见**——D-001 只认 Markdown 链接语法
（方括号文字 + 圆括号目标），本仓正文却**更常**用行内反引号指路。守卫报 0 不等于
没有坏链，只等于它看不见。新增规则 **D-006**（编号避开 `doc-symbol-audit` 已占的
D-005）。

**实测数据**（`--root=` 夹具 + 真实仓库）：

| 口径 | 条数 |
|---|---|
| 正文里反引号包着的 `.md` 出现次数 | 193 |
| 其中「含斜杠且不含 `./` `../`」⇒ 进入判定 | 79 |
| · 单根解析（相对当前文件）失败 | 34 |
| · **双根解析**（再试仓库根）失败 | **9** |
| 最终报出（修完后） | **0** |

**两条防误报设计**（缺任一条都会被豁免喂到失效）：

1. **双根解析**——`docs/DECISIONS.md` 在模块 README 里是相对仓库根、在 `docs/`
   内部又是相对当前文件，只认一种会把另一种全部误报（34 → 9 就是这一条的效果）。
2. **只判定「含 `/` 且不含 `./` `../`、无通配」的路径**——无斜杠的（`` `README.md` ``）
   相对谁无法确定，判定它们会让守卫满屏误报。

**豁免分两档**（这是本刀唯一被回归测试救回来的一次设计错误）：起初是「头部 15 行内
出现注释 ⇒ 豁免整篇」，结果**行内豁免永远不可达**——一段开头就写行内豁免的文档整篇
被放行。测试 `D-006 行内豁免` 因此**蒙混过关**。改为：行内注释只豁免**本行**，
全文豁免**须为文件首行非空内容**。

**修出的真断链（4 处，全属「引用时路径写短了」）**：
`session/docs/core-loop.md`（DATA_FLOW / PROTOCOLS / PLUGIN_DEVELOPMENT）、
`session/docs/vdfs-session-messages.md`（PROTOCOLS / ROUTES）、
`plugins/model/README.md`（ROUTES）、`session/README.md`（model/README）——
一律补成 `symbio/src/plugins/...` 全路径；另 2 处（`frontend-ui-ux-prd.md` /
`-design.md`）是缺陷记录里的**被引述内容**，加行内豁免。

**机制化**：`doc-link-audit.test.mjs` **新增 9 项**（36 项全过），逐条钉住
「违规必红 / 双根不误报 / 不可判定写法跳过 / 两种豁免作用域 / 空理由不算豁免」。

---

## 八、前端（`tauri/src`，20,269 行）

| 文件 | 行数 | 观察 |
|---|---|---|
| `components/vdfs/DetailForm.vue` | 1,054 | 通用渲染器，承担全部资源类型的表单；是「零页面开发」的支点，**大是合理的**，但可核查是否有按字段类型的重复分支 |
| `stores/sessions.ts` | 1,337 | 会话状态大仓；`sessionTranscriptSync.ts`(535) 已拆出同步，可继续核查 |
| `registry/messageTypes.ts` | 898 | 消息类型注册表，**表驱动是正确形态** |
| `composables/useVdfs.ts` | 851 | — |

前端整体已有正确的「注册表 + 通用渲染器」形态，**不是本轮重点**。
建议只在 §2/§3 落地后，核对前端是否因后端收敛而出现**可删的重复类型定义**
（`schemas/vdfs-form.ts`(483) 与后端 `schemas/detail.rs` 是跨栈镜像，
由 ADR-019 的 `protocol-mirror-audit` 守）。

### 8.1 状态（2026-09-27）：**未核实，估算作废**

§一 速览里给的「~500 行」**未经任何核实**。按 §10.1 新确立的规则
（「先数实例，再给估算」），本条**撤回估算**，状态标为「未核实」。

**已有的相关事实**（来自本轮前后端联动）：
- 后端收敛（§2 / §3）**未**导致前端出现可删的重复类型定义——
  `protocol-mirror-audit` 在所有落地后仍全绿，说明跨栈镜像未被破坏；
- `DetailForm.vue`(1,054) 的「大」是**零页面开发的支点**（一台通用渲染器承载
  全部资源类型），与 §4 的「`web_search.rs` 有 371 行但该拆成独立文件」是
  **相反**的情形——这里的集中是**有意的架构选择**，不是重复。

### 8.2 已按其要求核实（2026-09-27）

§8.1 留的下一步是「数出 `DetailForm.vue` 里按字段类型的分支数与其行数」。数完了，
结论比预期更强：**该机制已经存在，而且比我会提出的方案更好。**

| 测量项 | 实测 |
|---|---|
| `DetailForm.vue` 总行数 | 1,054 |
| 模板里按字段类型分派的臂数 | **7 臂 + 1 默认**（`static` / `form` / `toggle` / `select` / `textarea` / `revealable` / `number` / `datalist` / 默认 input） |
| 该分派链的模板区行数 | **~122 行** |
| 分派依据 | `specOf(f)` → `widgetSpecOf(f.widget)`（**查表**，不是 if 链） |

**关键发现：`tauri/src/registry/formWidgets.ts` 已经是表驱动机制本身**——
`WIDGET_SPECS` 表 + `widgetSpecOf()` 查表 + **8 个语义化访问器**
（`widgetInitialOf` / `widgetToEdit` / `widgetFromEdit` / `widgetIsReadonly` /
`widgetIsFullWidth` / `widgetPlaceholderOf` / …）。`DetailForm.vue` 的 7 臂只是
**按 widget 种类渲染不同 DOM**（`<select>` vs `<textarea>` vs 开关 vs 密码输入），
每种 DOM 结构不同——**这是渲染差异，不是重复**。

**故本条不是「未核实」，是「已核实且不成立」**：§8 想做的事（把按类型的分派收成表）
前端**早已做完**，且做在了比 `DetailForm.vue` 更合适的位置（`registry/`）。
**这是本轮第五条被否决的项，也是唯一一条「目标已实现」的否决。**
若将来前端要动，方向应另找——`DetailForm.vue` 的 1,054 行里没有可收敛的重复。

---

## 九、建议的落地顺序

按「**收益 / 风险**」与「**是否可机制化**」排序（**已完成三项后回填实际结果**）：

| 序 | 项 | 初估 | 实际 | 结果 |
|---|---|---|---|---|
| 1 | §2 配置挂载点 provider 收敛 | 收益最大、风险最低、可加守卫 | **-9 行**（语义副本 4→1）+ 新守卫 E-011 | ✅ 已落地 |
| 2 | §3 `vdfs/tools` 表驱动 | 收益明确、`read` 保留特例 | **-75 行** | ✅ 已落地 |
| 3 | §7 解析器收口 | 为后续所有守卫提速 | **第一刀 -279 行** + 19 项新测试；**第二刀 D-006**（新规则 + 9 项测试） | ✅ 已落地（两刀） |
| 4 | §5 RouteBridge | **收益大** | **-45 行（3%）** | ❌ **否决** |
| 5 | §6 session 定向审计 | 先测后拆 | **三处皆不可提取** | ❌ **否决** |
| 6 | §4 `Capability` 样板 | ~600 行 | **3 处 / 6 行** | ❌ **否决** |
| 7 | §8 前端热点 | ~500 行 | **目标已实现**（`registry/formWidgets.ts` 就是那张表） | ❌ **否决** |

**这张表本身是本报告最有价值的部分**：七条里凡经过实测的，**估算全部偏大或落空**，
其中**四条被完全否决、一条「目标早已实现」**。原始的「建议落地顺序」（1-5 按收益
排序）在实测后重排为「先做能手算清楚收益的、先做能出报告的、最后才做看起来大的」
——而排到最后的四条，全在动手前被数据否决了。

**最值得记的一笔**：看起来收益最大的三条（#1 ~1,200 行 / #4 500-700 行 /
#5 800 行）实测分别为 **-9 / -45 / ~0**；而实际收益最大的 §7 **在初版报告里
连独立的编号都没有**（只是「门禁脚本自身」那一栏）。**收益最大的那件事，
是靠测量找出来的，不是靠排序排出来的。**

每条落地后：
- 重跑 `node scripts/gen-current-facts.mjs`（行数自动刷新）；
- **收紧 `line-budget-audit.mjs` 里对应模块的基线**（棘轮只能往下拧）；
- 若涉及公开面变化，核对 `core-naming-audit` 与 `core-surface-audit`。

---

## 十、不推荐做的事（明确否决）

- **不建议引代码生成（模板/宏批量生成插件骨架）**：ADR-019 已就跨栈契约否决 G3；
  更简单的做法是**泛型 + trait**（§2），而非生成 `.rs` 文件——生成物不可读、
  不可 diff、且与守卫的静态解析冲突。
- **不建议为减行数而拆 `session`**：ADR-039 刚重组完；无实测重复数据前，
  拆分只会增加间接层。**§6.1 已给出实测：三处嫌疑全部不可提取**，
  本条从「不建议」升级为「**有数据支撑的不该**」。
- **不建议把 `symbio_core` 当作「什么都能放」的收容所**：每条下沉都要过
  ADR-023 的依赖方数量判据；§2 的 `PluginConfigMount` 适配器若只有一个插件用，
  就该留在插件侧。

### 10.1 由本轮调查新增的逆判据（**本报告最重要的产出**）

本轮五条「看起来该合并」的候选（§4 / §5 / §6 / §8 共约 8 处重复迹象），
**逐一实测后全部不可提取、被大幅降级、或发现「目标已实现」**。由此确立一条与主线
配对的逆判据：

> **主线判据**：重复能不能变成机制，让第二个人无法再写出第三份？
> **逆判据**：*有没有真逻辑差异*？——**答「有」时就不该抽象，哪怕看起来很像重复。**

配套的四条操作规则：

1. **先数实例，再给估算。** 本轮五条估算（≥6 处 / 600 / 500-700 / 800 / 500 行）
   实测全部落空（4 处 / 6 行 / 45 行 / ~0 / 已实现）。**未核实的估算是有害的**——
   它会让人按错误的优先级动手。要么先数，要么明确标注「未核实的上界」。
2. **「行数够少」从来不是判据。** 判据只有「有没有真逻辑差异」。
   §3 的「九个工具不全收」、§4 的「web 三工具该拆成三个文件」、
   §5 的「不造 RouteBridge」、§6 的「三处皆不可提取」——
   四条拒绝用的是**同一条判据**。
3. **「正确保留」要写下来。** 本报告中标注了 **8 处「看起来像重复但正确保留」**
   （§6.1 四处 + §4 的 `meta` 三处 + §5 的 `route_connection.rs`）。
   不写下来的话，下一轮复核会**重新发现一遍并写成「优化建议」**——
   本轮就是如此（§4 / §5 / §6 的初版建议，正是被本轮自己否决的）。
4. **动手前先全仓搜一遍「这个机制是不是已经存在」（§8.2 新增）。**
   §8 建议「把按字段类型的分派收成表」，而 `tauri/src/registry/formWidgets.ts`
   **早就是那张表**（`WIDGET_SPECS` + 8 个语义化访问器），且位置比我会提的更合理。
   **这类「目标已实现」的否决最容易被漏掉**——因为提议者只看了那个「看起来大」
   的文件（`DetailForm.vue`），没看同仓的注册表。**搜一遍的成本是几分钟，
   按错误建议动手的成本是天级。**

### 10.2 本轮否决分布（一条可复用的经验）

| 否决类型 | 项 | 判据 |
|---|---|---|
| **有真逻辑差异** | §5 RouteBridge、§6 session、§4 `local` 三工具 | 逆判据直接命中 |
| **收益低于噪声** | §4 `execute` 外壳（3 处 / 6 行） | 收益 < `cargo fmt` 抖动 |
| **目标已实现** | §8 前端表驱动 | 全仓搜索找到了既有机制 |

三类里，**第一类最难说服自己放弃**（差异要逐个读代码才看得见），
**第三类最容易漏**（没想到先搜一遍）。两类的共同对策是同一条：
**在写「建议」之前，先把实例逐个打开读**。

---

## 附：量化基线（复核时点）

| 范围 | 实现 | 测试 |
|---|---|---|
| `symbio/src` | 224 文件 / 55,851 行 | 127 文件 / 24,815 行 |
| `cli/src` | 4 文件 / 1,573 行 | 1 文件 / 80 行 |
| `tauri/src-tauri/src` | 3 文件 / 449 行 | — |
| `tauri/src` | 94 文件 / 20,269 行 | 49 文件 / 10,429 行 |
| `scripts/` | — | 13,176 行（含测试） |

测试/实现比 ≈ 0.44，处于健康区间。
