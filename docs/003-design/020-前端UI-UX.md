---
kind: design
id: DE-020-前端UI-UX
title: 前端 UI/UX
status: active
created: 2026-09-28
impl: partial
---

# 前端 UI/UX

> 本文回答「**还能怎么提升**」——目标形态，不是现状。
> 「现在是什么」看 [当前事实表](../004-ref/006-当前事实表.md "RE-006-当前事实表")（自动生成，唯一权威）；
> 「为什么这样设计」看该主题设计文档的「关键决策」节；
> 前端代码结构看 [前端架构](../../tauri/docs/110-FRONTEND.md "DE-110-FRONTEND")（本文不重复）。

---

## 0. 一句话结论

架构侧「一切皆 VDFS」已经做到极致：一台控件（`VdfsWorkbench`）承载全部资源类型，
新增一类资源前端零开发。这是真实优势，不是问题。

**代价在产品的目标形态上**：界面的默认形状是**资源浏览器**，而会话——用户真正的
目的地——在 56px 图标条里与 `model` / `mcp` / `skill` / `agent` / `plugin_manager`
平级。抵达面（落点、引导、空态、命令面板）已按这条主线补齐，见 §7 与 §10；
现在剩下的重心在**效率**与**信任**两层。

本规划的主线只有一条：**在不破坏 VDFS 统一机制的前提下，把「对话」放在主位**。

---

## 1. 差距盘点（只列**仍成立**的）

> 结构与规模事实（路由数 / 组件面 / VDFS op / 测试数）的 owner 是
> [当前事实表](../004-ref/006-当前事实表.md "RE-006-当前事实表")，本文不复制——
> 那种数字每次改动都会旧一截，写在这里就是第二真相。
> 本节只回答「离目标形态还差什么」；**已实现的项在 §7 / §10，不在这里出现**。

| 面 | 还差什么 | 对应提案 |
|---|---|---|
| 会话多开 | 单 `activeId`，切会话靠重挂载；真多标签未做 | §3 P1-4（高档）、§5 边界 |
| 列表操作 | 无拖拽 / 无右键菜单 / 无批量 | §3 P3 |
| 断连可见性 | 自动重连在跑，但没有全局连接状态面 | §3 P3 |
| 消息操作 | 有编辑，无「编辑后重发」，无悬浮操作条 | §3 P3 |
| 命令动作 | 面板只做**跳转**；「切模型 / 切工作区 / 跳设置」要的是**执行**，未接 | §3 P1-1 |
| 无障碍 | 消息节点与卡片已键盘可达，其余交互仍以点击为中心 | §3 P3 |

**已建成但前端未消费的后端能力**：`vdfs/search`（命令面板取的是 `tree`，理由见
§10 判据 5）。
上下文压缩**已接线**（`tauri/src/components/message/CompressionNode.vue`）；
未消费的是压缩节点与压缩后快照随节点下发的 `meta`——缺口清单与改进方向见
`tauri/docs/111-INTERACTION.md` 附录（不在此复述）。

**「后端有能力、前端没消费」与「后端也没有」是两件事**：本表逐行说的都是前者，
后者单独列在
[§11 的前件判据](#前件判据什么时候这活不该在前端开工)——信任面那四项（变更账、授权三档、
用量与花费、分支读法）缺的是后者，它们的后端事实已随 §11 落地、界面随之消费，因此
不在本表出现。
混在一起会让「补个组件」与「先补一条事实」看起来是同一档工作量。

---

## 2. 三层问题

- **L1 抵达成本** — 新用户走不到第一次成功对话：看不到输入框、没配模型时界面零提示。
 这是唯一的**流失漏斗**，也是 P0 那一批的全部理由（现状见 §10：落点恒为会话目录、
 首启引导、空态即引导）。
- **L2 效率天花板** — 键与列表之间仍有断层：多开没有（切会话即重挂载）、
 列表没有拖拽 / 右键 / 批量。
- **L3 信任与可控** — 看不见 Agent 干了什么、看不见上下文水位、看不见花了多少。
 Agent 类产品的护城河恰在这一层。

---

## 3. 改进地图

> 每项都标注**是否触碰架构约定**。默认原则：VDFS 统一（决策）不动，
> 「一台控件承载全部类型」不动；动的是**默认落点、入口顺序、以及未接线的能力**。

### P0 — 抵达（建议第一批）

**P0-1 会话优先的默认着陆** ★最高性价比
- 症状：冷启动落在资源根，会话与 MCP / 技能平级。
- 方案（**不新建任何控件**）：
 1. `/` 重定向 `/vdfs` → `/vdfs/session`（同一个 `VdfsView` + 同一个 `VdfsWorkbench` 承接，
 只是绑定地址从 `<根>` 变成 `<根>/session`）；
 2. ~~恢复上次选中：把 `lastAddr` 与 appearance 同源落盘，冷启动直达上次会话~~
 **不采用**——见下方「为什么不做」；
 3. rail 里 session 的**首位语义由后端目录顺序下发**，前端不硬编码（保住「新增资源零开发」）。
- 落点：`router/index.ts`、`schemas/vdfsAddress.ts`
- 架构：✅ 零冲突

> #### 为什么不做「回到上次浏览的地址」
>
> 本项第 2 条的形态是 `stores/nav.ts` + 守卫的一级「有位置记忆 ⇒ 回上次地址」，
> 该形态**不存在**（删除理由见 `router/index.ts::coldStartPath` 的注释）。不做的两个理由：
>
> 1. **判据拿不到真实的入口地址**：打包后的 webview 里 `location.pathname` 不是 `/`，
> 守卫的 `to.path !== '/'` 每次提前 return，记忆从未被读过；
> 2. **修好判据后它反而"正确地做错事"**：用户退出某智能体内部、重启后又被送回
> 该智能体——那正是他上次停留的地方，还原**成功了**，只是他要的不是这个。
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
 ② 配一个模型（深链直达 `/vdfs/model` 草稿详情页 —— `startNew` 与 draft 态已支持）
 ③ 发第一条消息（复用 `Session.vue` 的懒创建引导卡）。
- 落点：新建 `components/common/Onboarding.vue`，状态落 localStorage
- 架构：✅ 全部复用已有深链，不新建路由

**P0-3 空态即引导**
- `VdfsWorkbench` 空态「此目录为空 / 点击右上角新建」→ 对 session 目录给
 「开始第一段对话」主按钮；对 model 目录给「接入第一个模型」。

### P1 — 效率（建议第二批）

**P1-1 全局搜索 / 命令面板（Cmd+K）** ✅ 已实现（见 §10）
- 后端 `vdfs/search` 现成。结果按类型分组（会话 / 模型 / 智能体 / 技能 / MCP），
 Enter 直达（push 地址页），并挂动作（新建 X、切模型、切工作区、跳设置）。
- 落点：`services/vdfs.ts` 补 search → 新建 `components/common/CommandPalette.vue`
 → `MainLayout.vue` 注册全局键
- ⚠ 落地时取的是 `vdfs/tree` 而不是 `vdfs/search`，因为后者只匹配**文件**、
 找不到作为目录的资源条目；理由与代价见 §10 判据 5。「切模型 / 切工作区 / 跳设置」
 这三类动作没进第一版（它们需要的是**执行**而不是跳转），仍属提案。

**P1-2 键盘快捷键体系** ✅ 已实现（见 §10）
- 补齐：Cmd+K 面板、Cmd+N 新会话、↑↓ 列表移动、Cmd+1..9 切类别、**Esc 停止生成**
 （现在停止只能点按钮）、Cmd+/ 速查。
- 落点：新建 `composables/useHotkeys.ts`（表驱动，**机制唯一实现**）+ 绑定就地声明
 （`MainLayout.vue` 全局层 / `VdfsWorkbench.vue` 列表与类别 / `ModelChatPanel.vue` 停止）
 + 新建 `components/common/HotkeySheet.vue`（速查表**渲染登记表**，不是第二份清单）
- ⚠ **Cmd+B 折叠中栏不做**（本条是有名有姓的裁决，不是漏项）：中栏的显隐由内容决定
 （`Workbench.vue` 的 `showList`——列表为空且右栏铺着草稿时整栏收起），没有任何
 「用户按一下就折叠、并记住这个选择」的形态。要做折叠得先有一个**把手**：一条窄条，
 点击即展开、快捷键只是它的别名。判据是反对「唯一出口是快捷键」——一个只靠组合键
 存在的 UI 面，用户在速查表之外没有任何办法发现它。窄条属于布局演进，不在本批。

**P1-3 长会话渲染** ✅ 低成本档已落地（见 §10），虚拟化档**裁决不做**
- **P1-3a（已落地）**：这一条提案里的三项，本仓的实情是——**折叠长工具输出早有**
 （工具调用与思考默认单行、压缩记忆默认收起、`depth ≥ DEEP_COLLAPSE_LEVEL` 一律收起、
 长输出靠 `max-height` 限高），**「整轮收起」不做**（正文是会话流的主体，把已完成的一整轮
 收成一行等于让用户读不了历史），本批补的是**剩下的那条：历史 DOM 上限**。
 落地形状是**渲染层窗口**而不是数据分页——转写仍整份装载（权威副本在存储，
 分页会把「读全部历史」这条链路搞出第二个真相），只让一帧交出最近若干条根节点。
 - 落点：新建 `composables/useTranscriptWindow.ts`（只管这一帧铺哪些根节点）
 + `ModelChatPanel.vue`（显式入口与高度差补偿）
 + `stores/sessionUiState.ts`（记住**离底距离**与**窗口条数**这一对）。
- ⚠ **P1-3b 按 Turn 虚拟化不做**（有名有姓的裁决，不是漏项）：提案给它列的三处耦合，
 P1-3a 落地后**一条都不成立**——粘底反而变简单了（最新一条恒在 DOM 里，贴底就是滚到底）、
 rAF 合并管的是「每帧滚几次」与窗口无关、子树签名缓存以节点为粒度也无关。
 于是虚拟化只剩一条收益：「一帧的 DOM 恒定」。而窗口下 DOM 的增长只发生在
 用户自己往上翻或新消息到达时——那时增长正是他要的东西。为这条收益付的代价是
 绝对定位测量 + 动态高度 + 与粘底互相纠错三件事叠在一起，没有对应的用户可见收益。

**P1-4 多开 / 状态保持** ◐ 低档已落地（见 §10），高档仍是提案
- 低档（推荐先做，成本小收益大）：`{滚动位置, 输入草稿}` 按 sessionId 存 store，切回还原
 ——已落地；**折叠态未纳入**（消息节点的折叠由各 `NodeShell` 自持，没有按会话的键）。
- 高档（后议）：真多标签 + keep-alive。与 `sessionSpace` 全局单例冲突，需先评估。
- 落点：`stores/sessionUiState.ts`、`ChatMainPanel.vue` 的 `:key` 策略

**P1-5 导航态持久化** — 与 P0-1 合并做。

### P2 — 信任与可控（建议第三批，差异化）

- **P2-1 上下文水位可见** ✅ 已落地（见 §11）—— 输入区上方一条细水位条：本轮请求开头的估算 / 上限 / 自动整理线，
 条上的刻度就是「后端会在这里接手」的那条线。**不给「压缩」入口**：手动重试只由失败压缩节点下发，
 一条健康的水位线后面没有可点的东西（裁决见 [D-137](../../crates/symbio-plugin-session/README.md "crates/symbio-plugin-session/README")）。
- **P2-2 本会话动过哪些文件** ✅ 已落地（见 §11）—— 会话头的「变更」抽屉：行 = 路径 + 动作
  + 轮号 + 次数，点行经既有 VDFS 树打开**当前**文件（父目录页 + `?select=`，见
  [DE-010](010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §4.5）。
  **不做 diff**：变更账记的是「谁在哪一轮改过这个路径」，`artifact.added` 不带写入前全文，
  没有旧内容就没有 diff 可画；把它画出来必然是在编旧内容（裁决见 [D-169](001-核心架构.md "DE-001-核心架构")）。
- **P2-3 工具授权分级** ✅ 已落地（见 §11）—— 确认卡三档放行：批准本次 / 本会话放行（抬会话阈值）/
 永久放行（抬该工具上限，跨会话生效，撤销在设置页删行）；按危险度分级（shell / 写文件 / 删文件）。
  裁决见 [D-170](008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系")。
- **P2-4 用量与花费** ✅ 已落地（见 §11）—— 会话头常驻一格 + 读数面板「用量」段：输入 / 输出 /
  合计（多主体才逐条摊开），配了价的 Provider 再**按币种各一行花费**；一次实测都没有就整段不出现。
  价格住在 Provider 自己的配置里（`pricing`，不是按模型名猜的表），金额在 adapter 边界折好后入格
  （裁决见 [D-171](001-核心架构.md "DE-001-核心架构")）。
- **P2-5 分支切换** ✅ 已落地（见 §11）—— 带标注的节点头部常驻 `1/2 ‹ ›`，点箭头切「此刻读哪一支」。
 它的前提是一条裁决而不是一个组件：重试此前是**删除式**重跑（日志上不产生第二支，树上没有候选可切），
 现已裁成「并存几支 + 一个选中态」——读法入格、历史不删（裁决见 [D-173](001-核心架构.md "DE-001-核心架构")）。

### P3 — 打磨（第四批）

- 骨架屏替代「加载中…」文字（2 处）
- 消息悬浮操作条：复制 / 重试 / **编辑后重发** / 分支
- 停止与发送拆成两个独立按钮（现靠 `isLoading && !modelValue.trim`，语义靠 tooltip 猜）
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
`tauri/docs/110-FRONTEND.md` 顶部指向**本文**。视觉细则若将来要立，另建
`frontend-ui-ux-design.md` 并在本文登记——`frontend-ui-ux-prd.md` 与
`frontend-ui-ux-design.md` 两个文件名都不存在。

---

## 7. 已实现

> 本节记**已实现的事实**，与前六节的**提案**分开——提案会变，事实由代码决定。
> 逐条对应上文的编号；未列出的项仍属提案，**不要**当既成事实引用。

| 编号 | 内容 | 实现落点 |
|---|---|---|
| （用户点名） | 详情页「进入下一级」入口**恒在最右**、图标改为「进入」形 | 位置由 `mergeDetailActions`（`schemas/vdfs-form.ts`）统一重排；图标在 `registry/vdfsIcons.ts`；左间距在 `VdfsActions.vue` |
| P2-2（部分） | 列表筛选（当前目录内，纯投影） | `useVdfs.ts` 的 `filteredItems` / `filterEmpty` / `filterTruncated` + `VdfsWorkbench.vue` 输入与空态 |
| P1-5 / P0-1 | 导航态**就是浏览器地址**：刷新与深链都按地址重建，冷启动恒落会话目录（不做「回到上次位置」，理由见 P0-1 那条「为什么不做」） | `router/index.ts::coldStartPath`、`services/vdfsScheme.ts::ensureSessionMountDir`、`schemas/vdfsAddress.ts` |
| P0-3（部分） | 空候选给出可执行去向（不深链具体目录） | `ChatOptionBar.vue` |
| P3（两小项） | 骨架屏替代「加载中…」文字；消息节点头键盘可达 | `components/common/SkeletonBlock.vue`、`VdfsCardSkeleton.vue`；`NodeShell.vue` 的 `role="button"` + 键位 |

**明确仍未做**（提案状态不变）：P1-4 的高档（真多标签）、
P2-3 授权分级、P2-4 用量成本、P2-5 分支切换。
（P0-2 引导、P1-1 命令面板、P1-2 快捷键体系、P1-3a 历史 DOM 上限**已落地**（§10），
第三批的 P2-1 上下文水位**已落地**（§11）——已实现项同时挂在「未做」清单和「已实现」表上，
两处各说一遍就必然漂移。
Cmd+B 折叠中栏与 P1-3b 按 Turn 虚拟化**不是漏项**，是被裁决不做，见 §3 P1-2 / P1-3。）

**落地时确立的两条判据**（后续同类改动沿用）：

1. **位置是机制知识，不是资源知识**。「进入下一级」可能来自渲染器、也可能来自后端
 详情定义，故右置只能在 `mergeDetailActions` 做；交给各渲染器排，后端新增的动作就
 排不到位置。同理，`actions` / `busy` / `disabled` **三个数组必须一起置换**。
2. **筛选是「在当前页里找」，全库搜索是「在整库里找」，两者不能混**。把 `vdfs/search`
 接进中栏会在结果不落在当前目录时摧毁「点目录即钻入」的空间心智；全库搜索的归属是
 命令面板（P1-1）。

---

## 8. 已实现

| 内容 | 实现落点 |
|---|---|
| 会话头部的「进入下一级」移到**最右** | `ChatMainPanel.vue`：动作区移到最后一位并加左分隔线（`.header-actions`）。**注意**：第一批的改动只覆盖了 `VdfsActions` 内部排序，而会话头部的动作区在 `ChatMainPanel` 自己的 DOM 顺序里排在按钮之前——位置由**两处**共同决定 |
| 「清空历史」整体下线（前后端） | 前端：`ChatMainPanel.vue` 按钮/对话框/handler、`stores/sessions.clearMessages`、`services/session.clearMessages`；后端：`vdfs_provider.rs` 转写区段的 `(None, CLEAR)` 分支、`commands.rs` 的 `clear_messages`（已随路由一起删除）。`VDFS_ACTION_CLEAR` 本人保留（收件箱仍在用） |
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
| 详情子表单 `DetailForm` | 无 | 字段图标走 `fieldIcon`（emoji 表），与上面两条链完全无关 |

**根因**：三处各自实现了「取图标」这件事，没有单一实现。

### 9.2 修法

`registry/vdfsIcons.ts` 提供**唯一**的节点取图标函数 `iconForNode(node)`，
`cardIconOf` 与 `useVdfs.navItems` 都调用它；中间层
`getVdfsIcon` / `getVdfsIconFor` / `dirIconOf` **不存在**（留着就还会有人
从它们那里另走一条回退）。回退链**具体在前、笼统在后**：

```
1. kind:config_type 项级（最具体）
2. config_type 配置键（裸分区名，如 appearance）
3. name 名单级（<根> 挂载点：session / model / …）
4. kind 类型级（笼统容器：dir / file，最后兜底）
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

---

## 10. 已实现：抵达与导航

| 内容 | 实现落点 |
|---|---|
| 会话「进入下一级」不再由前端硬编码——与 agent / model / mcp 同一条通道（后端声明、前端只渲染） | 后端 `crates/symbio-plugin-session/src/options/mod.rs`（`session_detail_actions`，挂在会话 `schema.actions` 上）；前端只做投影 `schemas/vdfs-form.ts` + `components/vdfs/VdfsSessionDetail.vue` |
| 挂载层（`<根>` 的直接子节点）的左栏回到**类别清单**——冷启动落点 `/vdfs/session` 的左栏不再为空 | `composables/useVdfs.ts`（`atMountLevel`）+ `components/vdfs/VdfsWorkbench.vue`（`onRailSelect`） |
| 首启引导：三步（系统目录 → 配模型 → 发第一条消息），可跳过、可从「关于」重开；**只在尚未配好模型时自动弹** | `stores/onboarding.ts` + `components/common/Onboarding.vue` + `MainLayout.vue` + `components/settings/About.vue` |
| 空态即引导：「新建第一个 ⟨类型标题⟩」按钮（标题来自类型自述，新增资源零改动） | `components/vdfs/VdfsWorkbench.vue` 的 `empty` 插槽 |
| 会话级 UI 状态保持：输入草稿与**离底距离**按会话记忆（切走再切回不丢） | `stores/sessionUiState.ts` + `components/ModelChatPanel.vue` |
| P1-1 命令面板（Cmd/Ctrl+K）：一次 `vdfs/tree` 取「类别 + 条目」两层，本地筛词、按类别分组、Enter 直达地址页；空词给「新建 ⟨类别⟩」动作 | `utils/palette.ts`（候选计算）+ `components/common/CommandPalette.vue` + `services/vdfs.ts` 的 `treeVdfs` + `MainLayout.vue` 的全局键 |
| P1-2 快捷键体系：`mod+k` 面板、`mod+n` 新会话、`mod+/` 速查、`mod+1..9` 切类别、`↑↓` 列表移动、`Esc` 停止生成——机制一份、绑定就地声明、速查表渲染登记表 | `composables/useHotkeys.ts`（判键 + 三条守卫 + 派发 + `formatHotkey`）+ 三处绑定（`MainLayout.vue` / `components/vdfs/VdfsWorkbench.vue` / `ModelChatPanel.vue`）+ `components/common/HotkeySheet.vue`；每条绑定各有用例（机制、速查表、外壳、工作台、会话面板五处 spec） |
| P1-3a 历史 DOM 上限：打开会话那一帧只铺最近 40 条根节点，更早的**按批展开**（显式入口，不是自动续拉），展开时按高度差保住视线；阅读位置与窗口条数**成对**按会话还原 | `composables/useTranscriptWindow.ts`（锚定与展开）+ `composables/useChatScroll.ts`（`gapFromBottom` / `scrollToGap`：「离底距离」这一表示的唯一实现）+ `components/ModelChatPanel.vue`（入口与补偿）+ `stores/sessionUiState.ts`；用例见 `useTranscriptWindow.spec.ts`（形状判据）、`useChatScroll.spec.ts`（换算与还原）、`ModelChatPanel.window.spec.ts`（面板确实按窗口渲染） |

**本节的判据**：

1. **「某个类型的特有动作」属于后端**。会话详情是注册 editor（聊天工作区），但
 「有哪些动作」不是呈现细节——它与 agent / model / mcp 一样由 provider 声明
 （`DetailDefinition.actions`），前端只按机制键（`is_existing`）投影。前端硬编码
 一条类型动作，等于把「这个类型能做什么」拆成两处，且新增动作必须改前端。
 判据（`when` / `disabled_when`）的求值也只有一处实现（`projectDetailActions`），
 `DetailForm` 与自定义渲染器共用它、只换作用域。
2. **「这一步做完了没」由数据判据给，不由流程状态给**。引导的三步各自读后端事实
 （有没有模型 / 认不认得出会话挂载点），因此已配好的用户不会被一张「请去配置」
 的空话卡片拦在门口；而「读到第几步」这种流程状态根本不落地。
3. **跨「按地址重挂载」保留的状态放 store，且只放纯 UI 状态**。会话面板是按
 `activeAddr` 重挂载的（`:key` 策略），组件内状态必然随重挂载蒸发；`sessionUiState`
 按会话 id 存草稿与滚动位置。它**不缓存消息**（消息的权威来源仍只有 VDFS 变更
 一条通道），也不记**附件**（object URL 的所有权契约要求 revoke 只发生在一处）。
4. **导航判据从地址结构导出，不从类型名导出**。「哪一级是类别层」= 根的直接子节点，
 这条规则不需要任何类别清单，也不需要后端新字段；而「会话页左栏为空」这个故障
 正是把「左栏 = 绑定地址的子目录」直译的后果——会话是叶子，没有子目录。
5. **全库跳转的信源是 `tree`，不是 `search`**。`vdfs/search` 的匹配对象只有**文件**
 （后端 `host::search_via` 显式跳过目录节点），而这个地址空间里的资源条目（会话 /
 模型 / 智能体……）就是目录——用文件搜索去找它们会一无所获，而且失败得很安静。
 `tree` 一次请求交回「地址 + 自述」的扁平清单，层级由每条的 `path` 表达：
 **一次 IPC + 本地筛词**，打字不再发请求，面板不会把后端读放大乘上按键次数。
 代价随行说清：`depth = 2`（类别 + 条目，会话内部的消息归那一页的筛选），
 被截断时后端点名的 `pending` / `truncated` 原样呈现成「结果可能不全」——
 不把「没返回」演成「不存在」（与 §7 判据 2 是同一条纪律的两半：那条管中栏，这条管面板）。
6. **「去某一类」与「在这一类新建」是同一个目的地**，面板因此只给一条入口。
 工作台进入带 `new_type` 的目录且当前没选中任何项时会自动铺好一张草稿
 （`VdfsWorkbench` 的 `autoDraftDone`），所以「新建会话」的动作目标就是 `<根>/session`。
 做出两条长得不同的入口，等于把同一件事在界面上写两遍——而第二遍没有第二份实现撑着。
7. **快捷键的形状与「一台控件承载全部类型」是同一条纪律的两半：机制一份、绑定就地声明**。
 判键（`mod` 的平台差、大写锁定、必须按 Shift 才出得来键面的那几键、修饰组合恰好相等）
 与三条守卫只有一处实现，`when` / `run` 住在**状态所在的那个组件**：面板显隐在外壳、
 列表移动在工作台、这一轮跑没跑在会话面板。反例是外壳为每个页面维护一张「本页有哪些键」
 ——页面控件卸载后那张表就是假的。登记表随挂载注销（`onScopeDispose`），
 于是速查表**只能**描述在场的 UI：`HotkeySheet` 渲染 `hotkeyTable()`、每次打开重读一遍，
 键位与说明因此不可能与绑定漂移（写死的对照表会在下一次改绑时变成假账）。
8. **一条键的三种「让路」各挡住一种静默失效**，写错的表现为「按了没反应」，不报错：
 **打字优先**只管裸键——带 `mod` 的组合是打不出来的字，一并让路会把命令面板挡在它的
 头号使用场景（正写着消息）之外；**模态让路**（`.modal-mask`，与 `BaseModal` 的类名契约）
 挡的是「一次 Esc 同时关掉确认框并中止会话」；**`when` 不成立 ⇒ 不吞键**挡的是死键——
 `Ctrl+7` 在类别只有三个时必须原样留给浏览器（那是切标签页），被一条什么都不做的绑定
 吃掉比没有这条绑定更糟。
9. **限制历史 DOM 上限改的是「一帧交出多少」，不是「数据有多少」**。转写的权威副本在
 存储、一次 `vdfs/read` 拿全，这条链路是对的；给它加分页等于再造一个真相来源（谁持有
 「完整历史」从此有两处答案）。窗口因此切在**根节点**上——一个根带着自己整棵子树，
 按条切会把一棵树劈成上下两半，按根切最多是「整轮不在」：后者是缺内容，前者是坏内容。
 又因为**起点只往左移**（新到达的节点一律往尾部加），流式期间不会有节点从用户视线的
 **上方**被挤掉——按「最近 K 条」滑动的窗口每轮把界面往上拽一次，那是比卡顿更难归因的故障。
10. **可记忆的位置必须用在这个形状下仍然稳定的一端来量**。会话流的尾部在长、DOM 的
 起点又随窗口变，`scrollTop` 于是两头都不稳：隔一次挂载，同一个数字指的不再是那条消息。
 「离最新一条多远」两端都稳（最新一条恒在窗口内），故 `sessionUiState` 记的是**离底距离**，
 换算与还原只有 `useChatScroll` 一处实现（贴底判定读的是同一个数——分两处写就会出现
 「界面上算在底部、存下来却按不在底部还原」）。它与**窗口条数**成对：窗口决定 DOM 里有
 多少内容，位置决定落在这段内容的哪一处，只还原其一会落到窗口顶部。

---

## 11. 已实现：信任面（第三批，进行中）

| 内容 | 实现落点 |
|---|---|
| P2-1 上下文水位可见：输入区上方一条细水位条，读数**全部来自后端**（前端零口径、零业务词）。一条 = 本轮请求开头的估算 / 上限，参考线 = 自动整理线；缺数整条不出现 | 后端：`crates/symbio-plugin-loop/src/context.rs`（`context_watermark`，与压缩判定同一个阈值函数）→ `chat_loop/inputs.rs` ③′ 测量 → `io.rs::emit_streaming_start` 挂本轮骨架 `meta.context` → `session/types.rs::latest_context_watermark`（会话投影，落 `session.json`）→ `stats_reading.rs::context_reading_of`（出口排版，产出方言 `ReadingSection` + `ReadingBar`）→ `plugin/nodes.rs::session_node` 的 `context_watermark` 属性；前端：`components/common/ReadingMeter.vue`（读数的紧凑渲染器，与 `ReadingPanel.vue` 同一个方言）+ `ChatComposer.vue` 装配 + `schemas/reading.ts`（`bar` 几何槽与 `readingSectionOf` 边界收口）+ `services/session.ts` / `stores/sessionLive.ts`（清单与运行态帧同一条通道）；裁决见 [D-137](../../crates/symbio-plugin-session/README.md "crates/symbio-plugin-session/README") |
| P2-3 授权分级（三档）：确认卡四键——批准本次 / 本会话放行 / 永久放行 / 拒绝。本会话放行把**会话阈值**抬到本次调用的风险档（同档及以下在本会话里不再问）；永久放行把**该工具的上限**抬到本次风险档（跨会话生效，高于上限仍审批）；卡上没有风险档时两档都不出现（没有「抬到哪一档」可言） | 后端：判定只有 `CapabilityRiskLevel::approval_basis(risk, threshold, approved, grant)`（三档依次为 approved / threshold / grant）——执行闸门与授权级联的记录方问同一个函数；阈值来自 `ctx[RISK_LEVEL]`，工具清单来自 `ctx[APPROVAL_GRANTS]`（`SessionConfig.approval_grants` 经 `resolve_session_params` 落进 ctx）；放行事实 `authz.granted` 带 `basis` 与 `grant`；写入口 `session/grant`（只抬不降，经 `PluginConfigFile::apply` 的唯一写路径）——用例 `the_session_threshold_decides_whether_a_call_is_asked_about` / `a_per_tool_grant_authorizes_only_that_tool` 钉住两档的边界。前端：`components/message/InteractionNode.vue`（四键；档位校验走 `isSessionRiskLevel`）+ `ALLOW_FOR_SESSION_KEY` / `ALLOW_FOREVER_KEY`（两个注入面，`ModelChatPanel.vue` 提供，闭包持会话 id）+ `stores/sessions.raiseSessionRiskLevel`（只抬不降）+ `services/sessionGrant.ts` + `schemas/session_meta.ts`（`riskRankOf`）。裁决见 [D-170](008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") |
| P2-4 用量与花费：读数面板的「用量」段给输入 / 输出 / 合计（多主体时才逐条摊开），**配了价的 Provider 再按币种各一行花费**（标题随之变成「用量与花费」），**会话头常驻一格**同一个数；一次实测都没有就整段 / 整格不出现，没配价格就没有花费行 | 后端：`turn_runner/facts.rs::output_fields` 把 `LlmTurn.usage` 与 `LlmTurn.spend` 写进 `response{port=model}` 的 `usage` / `spend` 键（「测到了」的唯一记号：键在不在）→ `plugins/session/token_ledger.rs`（`token_ledger`：按内容的 `principal` 累计，`calls` 数的是实测条数）与 `session/spend_ledger.rs`（`spend_ledger`：按**币种**分桶、桶内按主体累计纳金额，`charges` 数的是带金额的应答条数）→ `session/stats.rs` 的 `tokens` / `spend` 两列 + `usage` 字段 → `stats_reading.rs::usage_section`（**唯一装配**：面板那一段与会话头那一格调同一个函数，token 走 `format_tokens`、金额走 `format_money`，两处单位各全仓唯一实现）。价格来自 `ModelProviderConfig.pricing`（Provider 自己的配置），金额在 adapter 边界折好后入格（[D-171](001-核心架构.md "DE-001-核心架构")）。前端：`components/session/ChatMainPanel.vue` 把 `stats.usage` 交给 `ReadingMeter`（与抽屉里的 `ReadingPanel` 同一方言的两个渲染器，零口径）；读取时机是「切会话 / 一轮跑完」，**不轮询** |
| P2-5 分支切换：重试留下**并存几支 + 一个选中态**。带标注的节点头部常驻 `‹ 1/2 ›`（两端到头置灰而非消失——读数不许随位置改形状）；点箭头只改**读法**，一条历史都不删 | 后端：两条事实 `turn.branch{of,into}`（这一支存在）与 `turn.branch_selected{selected}`（此刻读哪一支），在 `symbio_core/event/grid.rs` 注册为 `(Turn, Asserted)`（I2 因此都带 `produced_by`）→ `symbio_core/projection/branch.rs`（`BranchLedger::fold` 按**成员根节点 id** 并组、两类事实共同写组的 pick 且组内后到胜出；`stamp_branches` 在纯投影 `rebuild` 末尾盖 `meta.branch{group,candidates,index,total,selected}`）→ 写侧 `plugins/loop/stage_exec.rs`（重试准备段落 `turn.branch`，actor 取 `request_principal` 不是字面量）+ `chat_loop.rs::republish_branch_group`（轮次收尾按组重发标注帧——新支的节点要等本轮落盘才进投影）→ 读侧两条路、两条问法：**请求包**在 `projection/transcript.rs` 折叠（未选中支的助手侧条目判 `exclude_from_context`，正文照进转写、切回去还在；`prompt_fidelity` 认同一份折法，否则每次重试报一串假「丢事实」），**节点树**在 `chat_session/read.rs::get_context_messages` 调 `branch_hide_unselected`（只供水位与汇报）；`get_messages` 永远交全树。子树口径 = `parent_id` ∪ `meta.turn_root`，唯一实现 `node_build::subtree_of`→ 入口 `plugin/vdfs_messages.rs` 的 `switch_branch` 臂 + `commands.rs::switch_branch`（落点是成员根自身 `<sid>/message/<mid>`，载荷 `{to}`；非兄弟一律 `Invalid`）。前端：`registry/messageFacts.ts`（`messageBranchOf` / `messageBranchNeighbor` / `messageTurnRoot`——`meta` 键名只在这里出现，M-001）→ `stores/sessionTranscript.ts::dropUnselectedBranches`（纯函数，可见集仍只有 `visibleMessages` 一个答案）→ `composables/useChatConnection.ts::switchBranch` + `SWITCH_BRANCH_KEY`（注入面，与 `RESUME_KEY` 同一条理由）→ `components/message/NodeShell.vue`（渲染器不认「重试 / 分支」这些业务词：后端没盖标注就是不是一支）+ `styles/controls.css` 的 `.node-act.neutral`。裁决见 [D-173](001-核心架构.md "DE-001-核心架构")；用例：`projection/branch.test.rs`、`chat_loop.test.rs`（补发）、`vdfs_provider.test.rs`（切换只改读法 / 切到当前不写 / 非兄弟被拒）三处后端，`messageFacts.branch.spec.ts`、`sessionTranscript.spec.ts`、`useChatConnection.spec.ts` 三处前端 |
| P2-2 变更账：`session/changes` 给本会话改过的文件——每路径一行（最近在前），行**呈现就绪**（`{path, address, op, label, turn, changes}`），前端不解析工具参数；会话头的「变更」抽屉点行打开**当前**文件 | 后端：`symbio_core/projection/changes.rs`（`workdir_changes`：从 `artifact.added` 折「成功的写族工具调用」，每路径最新一条胜出 + 次数；界外 / 虚拟根下的路径不计；`vdfs_edit` 结果补 `path` / `replaced`，无操作靠字段判而非匹配文案）→ `session/changes.rs`（`read_changes` + 路由；`address` 是相对会话容器的 `workdir/<path>`，`has_wal` / `readable` 把三种空分开；读侧闸与 `session/stats` 同一道）→ `plugin.rs` 的 `"changes"` 臂（前端常量随门禁生成）。前端：`services/sessionChanges.ts`（原样透传）→ `components/session/SessionChangesList.vue`（纯排版器，三种空各一句话）→ `ChatMainPanel.vue` 的抽屉（与读数共用一个抽屉位；开关 / 代次守卫 / 错误上浮）；点行上抛 `open-node`，经 `Session` → `VdfsSessionDetail` → `VdfsWorkbench`（拆父目录 + 末段名）→ `VdfsView` push `?select=`，由 `useVdfs` 落位（当前目录定在绑定地址自身；不在本页则 `stat` 一次），见 [DE-010](010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §4.5。边界：用户编辑器保存 / shell / MCP / 其它进程的写入没有轮归属，留在实时面不入账；**没有写入前全文，因此不做 diff**（裁决见 [D-169](001-核心架构.md "DE-001-核心架构")） |

**本节的判据**：

1. **一条属性的实时性来自「交付出口共用」，不是来自轮询**。水位经 `session_node` 出来，而
   `vdfs/stat` / `vdfs/list` / 运行态帧三条路径共用这一个构造点 ⇒ 前端在**已经有**的那次回包里
   收到它（`listItemPatchOf` 就地补丁），零新增请求。反过来说，为「实时」加一条 `stat` 专用回读
   会把热路径请进来，而它买到的东西这个构造点已经白送。
2. **呈现所需的换算住在交付出口，不住在组件**。条的填充与参考线是后端除好的 0..1 比率，
   组件只做 `× 100` 这个 ratio→% 的单位换算；阈值系数由同一个函数同时喂压缩触发与出口读数。
   前端一旦自己写 `used > 0.7 * limit`，界面上的刻度就和后端的动手时机成了两笔账，漂了表现为
   「还没到刻度，历史已经变短」。同族纪律：[D-111](../../crates/symbio-plugin-session/README.md "crates/symbio-plugin-session/README")。
3. **新能力优先复用既有方言，落不进去就扩方言，不自造形状**（[DE-019 前端机制化](019-前端机制化.md "DE-019-前端机制化")的直接应用）。
   这条线**不写成 `{ used_ratio, compress_ratio }` 两个业务键**：业务键让渲染器认识「已用」与
   「压缩线」，换一个产者（成本、记忆）就得改前端，而这正是 `present/reading` 的模块文档立起来
   要挡的事。落不进去的那一格因此**扩方言**：`ReadingBar { level, marks, tone }` 是一个通用几何槽，
   里面没有一个主题词，`marks` 支持任意多条参考线，不认它的渲染器照常只排行（几何是呈现提示，不是新事实）。
   跨栈镜像同步登记这一对（`protocol-mirror-audit` D 组），漂移即红。
4. **缺数就整条不出现**。这一格缺省的含义是「本会话还没记录水位」（存量会话 / 一轮没跑完），
   补 0 会把它读成「上下文是空的」；同理后端一侧缺一个数字就少一行、没有分母就不发 `bar`，
   绝不为凑齐四行编一个数（见 `stats_reading.test.rs`）。会话头那一格是同一条判据的另一面：
   一次实测都没有 ⇒ 后端连那个键都不发，头上就什么都不画（不是一条空条、不是「0 token」）。
5. **不给这条线配「压缩」按钮**。提案原文写着「复用已有的 `context_compact`」，那句话随
   [D-072](../../crates/symbio-plugin-session/README.md "crates/symbio-plugin-session/README") 一起过期：自动整理由后端判，手动重试只由**失败的压缩节点**下发，一条健康的水位线
   后面没有任何可点的东西。入口不补齐不是缺口——把不存在的能力画成按钮，用户按下才发现界面在骗他。
6. **「批这一次」与「以后别再问」是两笔事实，界面不许把它们合成一个按钮**。`approve` 落的是
   `authz.granted`（一次调用一笔，随轮次进出日志）；「本会话放行」改的是**会话 metadata 的一个
   字段**（阈值），而阈值是「比它更危险才问」的那条线。做成「批准」加一个「以后别再问」复选框，
   后果是用户以为自己只批了一次、实际放开了整档工具——两笔事实的生命周期完全不同，界面上的
   粒度必须跟着分开。粒度也决定了它的边界：抬阈值按**风险档**放（同档别的工具一起免问），
   **按工具**免问需要一份跨会话授权清单，那是第三档「永久放行」缺的东西（见下文前件表）。
   顺序上「先抬阈值、再批准」不可颠倒：恢复载荷里的 `risk_level` 取自会话状态，抬晚了这一轮
   仍按旧阈值判，表现是「刚点了放行又弹一次」（组件用例钉的就是这条顺序）。
7. **「打开另一个节点」只上抛，不导航**。变更行指向的是一个文件，而文件没有自己的地址页
   （详情就是三栏的右栏）。渲染器因此只 `emit('open-node', addr)`，由三栏控件用路径代数拆成
   「父目录 + 末段名」、宿主 push `?select=`（[DE-010](010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §4.5）。
   在渲染器里 `useRouter` / 直接 push 看起来只差一行，但它把导航权搬进了详情层：同一个渲染器
   放到别的宿主（预览、嵌入）里就会越权改主人的地址。专属文件页 / 浮层同理——那会绕开
   「一台控件承载全部类型」，等于为一种资源开第二条呈现通道。
8. **切换分支只改读法，前端不重算「默认读哪一支」**。`switchBranch` 打的是 `switch_branch` 动作
   （落一条 `turn.branch_selected`），它不删节点、不动运行态、不重挂载会话——所以本地状态里
   什么都没有被「记住」，重进会话读回来的还是同一条事实。藏的那一份只在**可见集**这一层发生
   （`dropUnselectedBranches` 是纯函数，`visibleMessages` 仍是唯一答案；`removeMessages`
   一次都不许调，用例钉的就是这条）。读数全部来自后端盖的 `meta.branch`：形状不齐
   （`candidates.length !== total`、下标越界）一律读成「不在组里」 ⇒ **宁显示两支，也不静默
   少一条历史**。前端如果替后端补一个默认（「没标注就读最后一条」），同一个 WAL 就在前后端各有一棵
   树，而漂的那一边是用户的屏幕。
9. **「还有另一支」是常驻读数，不是悬停操作**。切换器放在节点头部且不随 hover 出现：它说的不是
   「这里能点」，而是「这条回答是一组里的第几支」——藏进 hover 就等于没有，用户不会去悬停一条他
   以为只有一份答案的消息。到头的那一侧按钮**置灰而不是消失**，读数 `1/2` 的形状不随位置变
   （控件因位置改宽度，整排头部会跟着抖）。

### 前件判据：什么时候这活不该在前端开工

一行的「谁先动」是后端时，前端那一批的判据里必须先有一条**后端判据**（一条事实、一列读数或一条
裁决），否则那一批做完只是把空态画得更好看。**今天这张表是空的**——§1 的每一行都是「后端已有能力、
前端未消费」或纯界面项；新登记一条差距时先问这题，答案是「事实面上没有」就把它记在这里，别开工画组件。

| 要呈现的东西 | 它的前提是什么 | 前提不在事实面时的后果 |
|---|---|---|
| 「几个候选可切换」 | **存在两个候选**。切换器不是「两个按钮」，是「同一 parent 下有两个孩子」 | 界面画出一个永远只有一个选项的切换器，或者更糟：前端自己造第二个候选，用户切过去发现日志里没有它 |
| 「这次花了多少钱」 | 有一个**折算后的金额**可搬。钱不是实测值，是价格 × 用量 | 前端按模型名猜单价 = 替用户拍一个他没同意过的价（见 §11 P2-4 与 [D-171](001-核心架构.md "DE-001-核心架构")） |
| 「本会话改过哪些文件」 | 有一条**带轮归属的写入事实** | 拿 `FsWatcher` 当权威 ⇒ 清单随「有没有打开页面」而变（见 [D-169](001-核心架构.md "DE-001-核心架构")） |

这三行不是别人的经验，是本文自己踩过的三处：**每一条都先被当成「补个组件」提出来**，而真正要补的
是它脚下的那条事实。三条如今都在事实面有了落点（分别是
[D-173](001-核心架构.md "DE-001-核心架构") /
[D-171](001-核心架构.md "DE-001-核心架构") /
[D-169](001-核心架构.md "DE-001-核心架构")，
实现面见下面的三条 P2 行）。它们留在表里是因为这张表记的是**判据**：再来一条「要呈现 X」，
先照这三行问一遍 X 的前提是不是事实。

**会话头那一格为什么单独下发、而不是让前端从 `panel` 里挑**：方言里的 `ReadingSection`
只有 `title` 没有身份字段，按中文标签定位等于把标签变成跨栈的键——改一个字，头上就
静默少一格（而界面少一格不报错）。让产者多交一格（`session/stats.usage`，与面板那一段
调同一个 `usage_section`），前端一个字符串都不用认。这一格**不是**对 `panel` 的复制：
两个消费方要的量不同（抽屉要全部分区，头上只要一行）。
