# 前端 UI/UX 提升规划（提案）

> 本文回答「**还能怎么提升**」，是**提案**，不是既定事实。
> 「现在是什么」看 [CURRENT.md](../CURRENT.md)（自动生成，唯一权威）；
> 「为什么这样设计」看 [DECISIONS.md](../DECISIONS.md)；
> 前端代码结构看 [tauri/docs/FRONTEND.md](../../tauri/docs/FRONTEND.md)（本文不重复）。

---

## 0. 一句话结论

架构侧「一切皆 VDFS」已经做到极致：一台控件（`VdfsWorkbench`）承载全部资源类型，
新增一类资源前端零开发。这是真实优势，不是问题。

**代价是产品侧心智优先级的缺失**：打开应用是一个**资源浏览器**，不是一个 **AI 工作台**。
会话——用户真正的目的地——只占 56px 图标条中的一格，与 `model` / `mcp` / `skill` /
`agent` / `plugin_manager` 完全平级。

因此本规划的主线只有一条：**在不破坏 VDFS 统一机制的前提下，把「对话」重新放回主位**。
其余改进是这条主线的效率与信任配套。

---

## 1. 现状盘点（事实，带证据）

| 面 | 现状 | 证据 |
|---|---|---|
| 路由 | 9 条 route，**真实组件仅 2 个**（`MainLayout` / `VdfsView`），其余为旧地址 redirect | `CURRENT.md §5.2`、`tauri/src/router/index.ts` |
| 布局 | 三栏：56px 图标 rail（挂载点由后端下发）+ 中栏列表（260px）+ 右栏详情 | `components/common/Workbench.vue` |
| 着陆页 | `/` → `/vdfs` = 绑 `<根>`，即资源根目录 | `router/index.ts:11` |
| 会话 | 无独立组件；就是 `<根>/session` 下的通用 `VdfsCard`，点中 → 右栏 `ChatMainPanel` | `VdfsWorkbench.vue:58-72` |
| 多开 | 单 `activeId`，切换靠 `:key` 重挂载 → 滚动位置 / 输入草稿 / 折叠态全丢 | `ChatMainPanel.vue:95-99` |
| 导航持久化 | **无**。刷新回 `/vdfs`。仅 `symbio.appearance`、systemLocation 持久化 | `stores/appearance.ts` |
| 流式 | 自研插件长连接（非 SSE），单连接 + 指数退避重连；48ms 攒批 + rAF 合并 + 子树签名缓存 | `services/eventBus.ts`、`stores/sessionTranscriptSync.ts:101` |
| 停止 | 发送键与停止键**同一按钮**，靠 `isLoading && !modelValue.trim()` 区分 | `chat/ChatInputArea.vue:40-53` |
| 编辑重发 | 有编辑，**无「编辑后重发」**；删用户消息才回填输入框 | `ModelChatPanel.vue:343-389` |
| 分支 | 消息是 `parent_id` 树，`MessageChildren.vue` 只递归渲染，**无候选切换 UI** | `message/MessageChildren.vue` |
| 虚拟列表 | **0 处**（全仓 grep `virtual` 为空），长会话全量 DOM | — |
| 搜索 | `useVdfs` 无 search/query/sort。而后端 `vdfs/search` 已在前端 13 op 中 → **已建成未接线** | `CURRENT.md §3.2`、`composables/useVdfs.ts` |
| 快捷键 | 仅 Enter 发送 / Shift+Enter 换行 / Ctrl+S / ESC（modal 焦点陷阱） | grep `keydown` 全仓 10 处 |
| 拖拽 / 右键 / 批量 | 均无（grep `drag` / `contextmenu` = 0） | — |
| 加载态 | 纯文字「加载中…」；骨架屏仅 `TurnPending.vue` 一处 | `Workbench.vue:76`、`VdfsWorkbench.vue:125` |
| 断连 | 自动重连 + resync 全量重读，**无全局连接状态指示**（仅 `HomedirEntry` 一个红点） | `services/eventBus.ts:184,420` |
| 引导 | **无 onboarding**。仅空态 `EmptyWorkdirState.vue`。配模型这一步只在 README 里说 | `README.md:37` |
| 视觉 | 零 UI 库，手写；`styles/tokens.css` 245 行语义 token，light/dark 两套，4pt 间距基准 | `styles/tokens.css` |
| 无障碍 | `aria-` 共 19 处 / 10 文件；消息节点头是 `div + @click`，**键盘不可达** | `message/NodeShell.vue:20` |
| 测试 | vitest + happy-dom，19 个 spec；**前端无 e2e** | `tauri/src/**/__tests__` |

**已建成但前端未消费的后端能力**（高性价比，优先接）：`vdfs/search`、消息 `parent_id`
分支树、`context_compact` 上下文压缩、`<根>/session/<id>/workdir` 会话工作区。

---

## 2. 三层问题

- **L1 抵达成本** — 新用户走不到第一次成功对话。冷启动要 3 次点击才见输入框；
  没配模型时界面零提示。这是唯一的**流失漏斗**。
- **L2 效率天花板** — 重度用户被鼠标锁死（无搜索 / 无快捷键 / 无多开），
  且长会话性能会随消息数线性塌方（无虚拟化）。
- **L3 信任与可控** — 看不见 Agent 干了什么、看不见上下文水位、看不见花了多少。
  Agent 类产品的护城河恰在这一层。

---

## 3. 改进地图

> 每项都标注**是否触碰架构约定**。默认原则：VDFS 统一（ADR）不动，
> 「一台控件承载全部类型」不动；动的是**默认落点、入口顺序、以及未接线的能力**。

### P0 — 抵达（建议第一批）

**P0-1 会话优先的默认着陆** ★最高性价比
- 症状：冷启动落在资源根，会话与 MCP / 技能平级。
- 方案（**不新建任何控件**）：
  1. `/` 重定向 `/vdfs` → `/vdfs/session`（同一个 `VdfsView` + 同一个 `VdfsWorkbench` 承接，
     只是绑定地址从 `<根>` 变成 `<根>/session`）；
  2. ~~恢复上次选中：把 `lastAddr` 与 appearance 同源落盘，冷启动直达上次会话~~
     **已下线（2026-09-28）**——见下方「为什么不做」；
  3. rail 里 session 的**首位语义由后端目录顺序下发**，前端不硬编码（保住「新增资源零开发」）。
- 落点：`router/index.ts`、`schemas/vdfsAddress.ts`
- 架构：✅ 零冲突

> #### 为什么不做「回到上次浏览的地址」（2026-09-28 结论）
>
> 本项第 2 条曾落地为 `stores/nav.ts` + 守卫的一级「有位置记忆 ⇒ 回上次地址」，
> 现已**整体删除**（`router/index.ts::coldStartPath` 的注释是权威说明）。两个理由：
>
> 1. **判据拿不到真实的入口地址**：打包后的 webview 里 `location.pathname` 不是 `/`，
>    守卫的 `to.path !== '/'` 每次提前 return，记忆从未被读过；
> 2. **修好判据后它反而"正确地做错事"**：用户退出某智能体内部、重启后又被送回
>    该智能体——那正是他上次停留的地方，还原**成功了**，只是他要的不是这个。
>
> 判「该不该回上次」需要一个稳定的**中性位置**概念，而 VDFS 地址是**分形**的
> （子空间与根空间的挂载名完全同名，实测 `/vdfs/session/<id>` 与 `/vdfs/agent/<id>`
> 结构相同），纯地址形状分不出两者。
>
> **它真正缺的是「回到中性位置」的入口**——那属于导航模型（左栏/中栏切换时把
> 地址带回挂载目录，或给一个显式的退出子空间入口），不属于落点机制。若日后要做，
> 从那里入手，不要再加落点判据。

**P0-2 首次启动引导（onboarding）**
- 症状：README 写着「首次启动后在左侧模型面板新建条目填入 API Key」，界面零引导 →
  无模型 = 发不出消息 = 第一分钟必然卡住。
- 方案：三步行进卡（可跳过、可在「帮助」重开）：① 系统目录已就位（复用 `HomedirEntry`）
  ② 配一个模型（深链直达 `/vdfs/model` 草稿详情页 —— `startNew()` 与 draft 态已支持）
  ③ 发第一条消息（复用 `Session.vue` 的懒创建引导卡）。
- 落点：新建 `components/common/Onboarding.vue`，状态落 localStorage
- 架构：✅ 全部复用已有深链，不新建路由

**P0-3 空态即引导**
- `VdfsWorkbench` 空态「此目录为空 / 点击右上角新建」→ 对 session 目录给
  「开始第一段对话」主按钮；对 model 目录给「接入第一个模型」。

### P1 — 效率（建议第二批）

**P1-1 全局搜索 / 命令面板（Cmd+K）** ★已建成未接线
- 后端 `vdfs/search` 现成。结果按类型分组（会话 / 模型 / 智能体 / 技能 / MCP），
  Enter 直达（push 地址页），并挂动作（新建 X、切模型、切工作区、跳设置）。
- 落点：`services/vdfs.ts` 补 search → 新建 `components/common/CommandPalette.vue`
  → `MainLayout.vue` 注册全局键

**P1-2 键盘快捷键体系**
- 补齐：Cmd+K 面板、Cmd+N 新会话、↑↓ 列表移动、Cmd+1..9 切类别、**Esc 停止生成**
  （现在停止只能点按钮）、Cmd+B 折叠中栏、Cmd+/ 速查。
- 落点：新建 `composables/useHotkeys.ts`（表驱动）+ `MainLayout.vue` 全局层

**P1-3 长会话渲染** ⚠ 风险中
- 分两档，**先做低成本档**：
  - P1-3a（低风险，先做）：折叠已完成的 Turn、折叠长工具输出、限制历史 DOM 上限（"加载更早"已有先例，
    `VdfsWorkbench.vue:74-84` 同款收尾）。
  - P1-3b（高风险）：按 Turn 虚拟化。注意与三处耦合 —— `useChatScroll` 粘底、
    `ModelChatPanel.vue:398` 的 rAF 合并、`useChatConnection.ts:218` 子树签名缓存。
    流式 + 虚拟 + 粘底是经典难点，建议独立技术验证。

**P1-4 多开 / 状态保持**
- 低档（推荐先做，成本小收益大）：`{滚动位置, 输入草稿, 折叠态}` 按 sessionId 存 store，切回还原。
- 高档（后议）：真多标签 + keep-alive。与 `sessionSpace` 全局单例冲突，需先评估。
- 落点：`stores/sessions.ts`、`ChatMainPanel.vue:95-99` 的 `:key` 策略

**P1-5 导航态持久化** — 与 P0-1 合并做。

### P2 — 信任与可控（建议第三批，差异化）

- **P2-1 上下文水位可见**：输入框上方一条细水位条（已用 / 上限 / 压缩阈值），
  临近阈值给「压缩」入口 —— 复用已有的 `context_compact`。后端已暴露该工具，前端零可见性。
- **P2-2 本会话动过哪些文件**：`<根>/session/<id>/workdir` 已存在但未进聊天界面。
  加一个可折叠「变更」面板，diff 复用 `CodeEditor.vue`。这是 Agent 类产品的核心 UX。
- **P2-3 工具授权分级**：`UserPromptNode.vue` 有 approve/reject 但无「本次会话放行」→ 高频打扰。
  按危险度分级（shell / 写文件 / 删文件）+ 三档授权（每次问 / 本会话放行 / 永久放行）。
- **P2-4 用量与成本**：多供应商 LLM 却零用量可见。会话头 / 模型条目显示累计 token；
  成本折算依赖 `model` 插件是否已有价格字段（**若无，需后端先补**）。
- **P2-5 分支切换 UI**：`parent_id` 树已建，重试已产生分支但用户看不见。
  同 parent 多候选时给 `1/2 ‹ ›` 切换器。

### P3 — 打磨（第四批）

- 骨架屏替代「加载中…」文字（2 处）
- 消息悬浮操作条：复制 / 重试 / **编辑后重发** / 分支
- 停止与发送拆成两个独立按钮（现靠 `isLoading && !modelValue.trim()`，语义靠 tooltip 猜）
- 无障碍：`NodeShell.vue:20` 补 `role="button"` + `tabindex`；全局 aria 仅 19 处，需系统补
- 全局连接状态指示（现仅 `HomedirEntry` 一个红点）
- 资源列表：右键菜单 / 拖拽 / 批量操作
- rail 图标补文字标签（现 56px 纯图标，语义靠 tooltip）

---

## 4. 分期路线

| 批次 | 内容 | 主线 |
|---|---|---|
| 第一批 | P0-1 / P0-2 / P0-3 / P1-5 | 抵达：让用户一分钟内发出第一条消息 |
| 第二批 | P1-1 / P1-2 / P1-3a / P1-4(低档) | 效率：把重度用户从鼠标里解放 |
| 第三批 | P2-1 / P2-2 / P2-3 / P2-5 | 信任：看得见 Agent 干了什么 |
| 第四批 | P3 全量 | 打磨 |

第一批和第二批建议**连做**——搜索与快捷键是「会话变多之后」必然要有的东西，
而 P0 让会话变多，会立刻放大 P1 的缺失。

---

## 5. 明确不做（边界）

- **不引入 UI 组件库**。`tokens.css` 的 4pt / 语义 token 体系已成型，
  引入库会与之打架，且当前零运行时依赖是优势。
- **不为每种资源做专属页面**。VDFS 统一 + 「新增资源前端零开发」是本项目的核心机制优势。
  缺优先级是**落点问题**，不是**机制问题**（见 P0-1）。
- **不做多窗口**。先做状态保持（P1-4 低档），多窗口留到真有需求再议。

---

## 6. 待办：文档断链

<!-- doc-link-allow D-006: 本节引用的三个文件名**均不存在**——前两个是已修缺陷的被引述内容，第三个是"将来要立"的占位，都不是入链 -->
`tauri/docs/FRONTEND.md` 顶部引用了 `docs/design/frontend-ui-ux-prd.md` 与 `docs/design/frontend-ui-ux-design.md`，**两个文件均不存在**。
**已修（2026-09-27）**：改为指向本文。视觉细则若将来要立，另建 `frontend-ui-ux-design.md` 并在本文登记。

---

## 7. 已落地（2026-09-27，第一批）

> 本节记**已实现的事实**，与前六节的**提案**分开——提案会变，事实由代码决定。
> 逐条对应上文的编号；未列出的项仍属提案，**不要**当既成事实引用。

| 编号 | 内容 | 实现落点 |
|---|---|---|
| （用户点名） | 详情页「进入下一级」入口**恒在最右**、图标改为「进入」形 | 位置由 `mergeDetailActions`（`schemas/vdfs-form.ts`）统一重排；图标在 `registry/vdfsIcons.ts`；左间距在 `VdfsActions.vue` |
| P2-2（部分） | 列表筛选（当前目录内，纯投影） | `useVdfs.ts` 的 `filteredItems` / `filterEmpty` / `filterTruncated` + `VdfsWorkbench.vue` 输入与空态 |
| P1-5 / P0-1（部分） | 导航态持久化（浏览器地址级，根不记） | `stores/nav.ts` + `router/index.ts` 默认重定向 + `MainLayout.vue` |
| P0-3（部分） | 空候选给出可执行去向（不深链具体目录） | `ChatOptionBar.vue` |
| P3（两小项） | 骨架屏替代「加载中…」文字；消息节点头键盘可达 | `components/common/SkeletonBlock.vue`、`VdfsCardSkeleton.vue`；`NodeShell.vue` 的 `role="button"` + 键位 |

**明确仍未做**（提案状态不变）：P0-2 引导、P1-1 命令面板、P1-2 快捷键体系、
P1-3 长会话渲染、P1-4 多开、P2-1 上下文水位、P2-3 授权分级、P2-4 用量成本、P2-5 分支切换。

**落地时确立的两条判据**（后续同类改动沿用）：

1. **位置是机制知识，不是资源知识**。「进入下一级」可能来自渲染器、也可能来自后端
   详情定义，故右置只能在 `mergeDetailActions` 做；交给各渲染器排，后端新增的动作就
   排不到位置。同理，`actions` / `busy` / `disabled` **三个数组必须一起置换**。
2. **筛选是「在当前页里找」，全库搜索是「在整库里找」，两者不能混**。把 `vdfs/search`
   接进中栏会在结果不落在当前目录时摧毁「点目录即钻入」的空间心智；全库搜索的归属是
   命令面板（P1-1）。

---

## 8. 已落地（2026-09-27，第二批）

| 内容 | 实现落点 |
|---|---|
| 会话头部的「进入下一级」移到**最右** | `ChatMainPanel.vue`：动作区移到最后一位并加左分隔线（`.header-actions`）。**注意**：第一批的改动只覆盖了 `VdfsActions` 内部排序，而会话头部的动作区在 `ChatMainPanel` 自己的 DOM 顺序里排在按钮之前——位置由**两处**共同决定 |
| 「清空历史」整体下线（前后端） | 前端：`ChatMainPanel.vue` 按钮/对话框/handler、`stores/sessions.clearMessages`、`services/session.clearMessages`；后端：`vdfs_provider.rs` 转写区段的 `(None, CLEAR)` 分支、`commands.rs::clear_messages`。`VDFS_ACTION_CLEAR` 本人保留（收件箱仍在用） |
| 会话内部导航图标语义化 | 后端 `workdir/mod.rs` 补 `KIND_SUB_SESSIONS` / `KIND_WORKDIR`（协议词）；前端 `registry/vdfsIcons.ts` 登记 `messages` / `inbox` / `subsession` / `dir` 四张独立图标 |

**第二批确立的判据**：

1. **一个功能下线要连着它的「替代入口」一起删**。「清空历史」在 VDFS 上的形态是
   `action(…/message, "clear")`，而它与 `delete(<根>/session/<id>)` 在用户眼里是同一件事的两条路。
   只删 UI 会把「重叠」留在协议层，下一个人照样能把它接回来。
2. **动作动词按对象收窄，不按词面复用**。`clear` 现在只剩收件箱一个语义（取消排队），
   转写区段上不再有它——`truncate` 是那里唯一的集合动词。
3. **「图标优先顺序」要用「具体 vs 笼统」判，不是「类型 vs 实例」判**。列表卡片早已是
   「项级 → kind → 名字」三级回退，侧栏却只有名字一级，于是插件管理插件的各分区在左栏
   全是文件夹（见 §9）。修的时候**不能再想当然地「kind 优先」**——`<根>` 挂载点的
   `kind` 全是 `dir`，kind 优先会让它们整排退成同一张图（§9.3 是完整事故记录）。
4. **每个用例自带超时值，按满负载耗时给**。`coldStart.spec.ts` 每个用例都重载 router
   依赖图（约 2.4s），贴着 5s 默认超时，全量并发时随机变红。给超时值而不是加 `retry`
   ——后者会把真实的性能回归也一起吞掉。

---

## 9. 已修：图标查找收成唯一实现 `iconForNode`（**含一次回归事故**）

### 9.1 收敛前的问题

「同一个问题在三处各有各的写法」：

| 查表点 | 收敛前的回退链 | 问题 |
|---|---|---|
| 列表卡片 `cardIconOf` | 项级 `kind:ext` → kind → 目录名 | 完整，是**正确形态** |
| 侧栏 `useVdfs.navItems` | 目录名（一级） | **缺项级**：`plugin_manager` 的分区（`config_type = appearance / session / …`）在左栏全部退成文件夹 |
| 详情子表单 `DetailForm` | 无 | 字段图标走 `fieldIcon()`（emoji 表），与上面两条链完全无关 |

**根因**：三处各自实现了「取图标」这件事，没有单一实现。

### 9.2 修法

`registry/vdfsIcons.ts` 提供**唯一**的节点取图标函数 `iconForNode(node)`，
`cardIconOf` 与 `useVdfs.navItems` 都改为调用它；此前的中间层
`getVdfsIcon` / `getVdfsIconFor` / `dirIconOf` **全部删除**（留着就还会有人
从它们那里另走一条回退）。回退链**具体在前、笼统在后**：

```
1. kind:config_type   项级（最具体）
2. config_type        配置键（裸分区名，如 appearance）
3. name               名单级（<根> 挂载点：session / model / …）
4. kind               类型级（笼统容器：dir / file，最后兜底）
```

### 9.3 事故：把「kind 优先」当成了修法（**留作判据**）

改第一版时把侧栏第一跳写成 `getVdfsIcon(n.kind) ?? …`（kind 优先，名字兜底
被 `getVdfsIconFor` 内部的 `icons[kind]` 回退**提前截胡**）。后果：

- `<根>` 的六个挂载点由 `VdfsNode::dir(...)` 产出，`kind` **全是 `"dir"`**
  （`VDFS_KIND_DIR`；后端自测 `composite/vdfs.test.rs` 即断言此值）；
- 于是六项**全部**命中 `dir` 那张图 → 主界面左栏「会话 / 模型 / 智能体 / MCP /
  技能 / 插件管理」整排退成同一个默认文件夹。

**判据**：`kind` 的取值分两类——**具体的身份**（`session` / `model` /
`plugin_manager:appearance`）与**笼统的容器形态**（`dir` / `file`）。
「优先」只对前者成立；后者对区分同类成员**零信息量**，永远该排在最后。
顺序不是实现细节，**顺序就是这条规则的实现**。

### 9.4 护栏

| 用例 | 钉住什么 |
|---|---|
| `registry/__tests__/vdfsIcons.spec.ts` | 回退链各步的命中与未命中；夹具用**真实** `kind: 'dir'` |
| `composables/__tests__/useVdfs.spec.ts` | 让真实挂载点形状流过真实 `navItems`，断言六项图标**互不相同** |
| 同上（第二条） | `cardIconOf` 与 `navItems` 对同一节点取**同一张**图 |

两处都做过**反证**：删掉「名字兜底」那一步，用例即红并报
「expected 1 to be 6」（六项退成一张图）——与用户所见症状一致。

⚠️ 夹具里的 `kind` **必须是 `'dir'`**，不能图省事写空串：后端从不发空 kind，
用不存在的形状做夹具，真回归来了照样是绿的。

**这条与 §8 判据 3 是同一件事的两半**：判据 3 说的是「项级优先」，本节说的是
「优先顺序要在**同一个地方**实现一次」。

