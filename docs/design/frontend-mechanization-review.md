# 前端机制化复核：剩余空间与代码量削减清单

状态：一次性复核报告（非规范、非事实表）
范围：`tauri/src`（85 实现文件 / 17889 行；25 测试文件 / 5029 行）
依据：`vdfs-frontend.md` 的不变量（路径唯一地址 / 能力只看访问位 / ext 决定详情 /
前端零资源知识 / 实时靠订阅）+ 逐文件实读
配套：结构事实以 `docs/CURRENT.md` §3、§5.1 为准，本文件只写判断与待办

---

## 1. 评定

**机制主干已成立，缺口全部集中在「详情渲染器层」。**

已扎实的部分（不建议动）：

- 地址口径前后端同源（`.vdfs` 为根），路径代数只有 `schemas/vdfs.ts` 一份；
- `services/vdfs.ts` 是纯机械翻译层，零资源知识；`services/session.ts` 已退化为
  纯 VDFS 门面（238 行，无自有路由表），不是"绕过机制"；
- `useVdfs.ts` 是唯一数据层，草稿节点 / 有界列表 / 追加型增量 / 订阅绑定都在一处；
- 契约零组件知识、注册表分层（`vdfsTypes` 零组件 → `vdfsRenderers` 唯一装配）执行严格；
- 新增一类资源确实**前端零改动**（后端加 provider 即可），这条已实测成立。

质量门禁现状（实测）：

| 项 | 结果 |
|---|---|
| `npm run lint` | **0 problem** |
| `node scripts/style-audit.mjs` | 0 error / 7 warn（均 `NodeShell.vue` 的 scoped 死类） |
| 测试规模 | 5029 行 / 25 文件；机制层覆盖好（stores 1075、services 1345、registry 717） |
| 测试盲区 | components 8881 行只有 1080 行测试——4 个详情渲染器基本裸奔 |
| 依赖 | 6 个运行时依赖（vue / pinia / vue-router / marked / tauri api ×2），克制 |

结论：**整体质量高于同类项目**（文档密度、契约分层、门禁洁净度都罕见地好）。
问题不是"能不能用"，而是三处**单点化不彻底**：动作、草稿判据、详情外壳。

---

## 2. 机制缺口（去业务化的剩余空间）

### G1 动作声明有两个主人 —— 最该修

`DetailForm.vue:402` / `VdfsActions.vue:11` / `Session.vue:20` 的注释都写
「机制动作由页面单一定义点计算并注入」，但 `VdfsWorkbench.vue:180` 从未传
`mechanism-actions`。实际是：

- `VdfsFormDetail.vue:173` 自算删除动作；
- `VdfsSessionDetail.vue:66` 自算「浏览内部 + 删除」；
- `VdfsTextDetail.vue:66` / `VdfsReadonlyDetail.vue:71` 干脆把
  `rename` / `delete` 混进自己的 `save` / `reset` 一起手写。

即：同一组「机制级动作」有 4 份实现、2 种形态，且后端 `detail_definition.actions`
本已声明 `delete` 等动作——**同一概念三个来源**（后端定义 / 渲染器硬编码 / 页面注入）。

修法：`useVdfs` 按访问位（`w` ⇒ rename+delete）与草稿态算一次标准动作集，
经 `VdfsWorkbench` 注入所有渲染器；渲染器只声明"我自己的动作"（`save` / `reset` /
`test`）。附带清掉死 prop `deleting`（`ChatMainPanel.vue:99` 与 `Session.vue:29`
接的它全链路无人传，"删除中…"永不出现）。

### G2 草稿判据三份实现

| 位置 | 判据 |
|---|---|
| `useVdfs.ts:543` | `!node.path`（权威口径） |
| `VdfsFormDetail.vue:102` | `!props.node.name` |
| `Session.vue:97` / `VdfsSessionDetail.vue:50` | `!props.node?.name` |

对当前资源三者恰好等价，但已靠注释"解释"而非靠机制"保证"。应收进
`schemas/vdfs.ts` 导出的 `isVdfsDraft(node)`，各处引用。

### G3 `appearance` / `about` 是唯一未定义驱动的详情

`vdfsRenderers.ts:32-33` 注册了两个前端自持面板：`Appearance.vue`（250 行，
主题/字号/提示音硬编码在 `:169-183`）、`About.vue`（75 行）。这违反不变量 4
（前端零资源知识）——它们是唯一"ext 即语义类型名"的特例。

彻底解：setting provider 照常下发 `schema`（`form` 定义），前端为
**前端自持的取值命名空间**提供一处值源（read/write 落到 appearance /
soundSettings store），于是这两个页面变成纯定义，组件归零。
代价：需要一条"值源"接缝（约 100~120 行）。**收益最大、风险也最高，建议最后做。**

### G4 消息域是并行机制，不是机制的一部分

`registry/messageTypes.ts`（795）+ `messageRenderers.ts`（35）与
`vdfsTypes.ts`（102）+ `vdfsRenderers.ts`（36）**完全同构、各自实现**：
`schemas/chat_message` → 纯映射 → 唯一装配点。加上
`components/message/*`（9 个约 1700 行）与 `components/vdfs/*` 两套渲染体系。

两者分层都对，但"标识 → 组件 + 兜底"这套机制被发明了两次。
最小值：抽出 `registry/factory.ts`（`createRendererRegistry<TKey>()`，
含 resolve / register / get / 兜底），两侧各声明一次。

### G5 三栏组件三层套娃

`VdfsWorkbench` → `Workbench` → (`NavRail` | `VdfsShell`)，
而三者都只被链上下一级使用（`VdfsWorkbench.vue:21`、`Workbench.vue:34`）。
其中已死的分支：

- `Workbench.vue:33` 的 `content` 插槽（应用外壳模式）无人使用；
- `Workbench.vue:21-22` 的 `back` / `backTitle` → `NavRail.vue:15-23` 的返回键
  是死的（`VdfsView.vue:37` 自己在 `rail-header` 里注入了返回键，还复制了
  `.nav-btn.back` 样式）；
- `VdfsShell.vue:35-53` 的默认「新建」按钮死（`VdfsWorkbench` 恒传
  `hide-default-new`）；`VdfsShell.vue:58-60` 的 `meta` 插槽已被注释掉，
  但 `Workbench.vue:46` 仍在转发；
- `Workbench.vue:43-49` 五个 `v-if="$slots[x]"` 纯转发,是维护税而非抽象。

### G6 详情外壳（标题 + 动作区 + 错误条）四处手抄

`.detail-head / .head-title / .head-actions` 在 `VdfsTextDetail.vue:115-154`、
`VdfsReadonlyDetail.vue:128-158`、`VdfsMessageDetail` 各一份，
`DetailForm.vue:654-731` 是第四份变体（`.form-header`，注释自认"同构"）。
`controls.css:8-9` 也自述存量组件持同名 scoped 副本。

---

## 3. 削减清单（可验证的行数账）

| # | 动作 | 主要落点 | 预计削减 |
|---|---|---|---|
| P1 | 三栏扁平化：删 `VdfsShell`，导航栏并入 `Workbench`；清死插槽/prop | `Workbench.vue` `VdfsShell.vue` `NavRail.vue` | ≈190 行 |
| P2 | 抽 `DetailShell.vue`（head + actions + error 条），四个渲染器共用 | 4 个 `vdfs/*.vue` + `DetailForm.vue` | ≈150 行（含 CSS ≈90） |
| P3 | 动作单点化（G1）+ 草稿判据归位（G2） | `useVdfs` `VdfsWorkbench` 4 渲染器 | ≈120 行 + 消一个 bug 面 |
| P4 | 注册表工厂化（G4），两侧共用 | 4 个 registry 文件 | ≈120 行 |
| P5 | 死代码：`utils/time.ts::formatTime`（0 引用）、`mkdirVdfs`（0）、`treeVdfs`（0）、`NavRail` 返回键、`VdfsShell` 默认新建 | — | ≈90 行 |
| P6 | 交互收口：`ChatMainPanel.vue:171,179` 的 `window.prompt/confirm` 改用机制内联重命名 + `ConfirmDialog` | `ChatMainPanel.vue` | ≈25 行 |
| P7 | 时间格式化归一到 `utils/time.ts`（现三份：`VdfsWorkbench.vue:517`、`VdfsReadonlyDetail.vue:96`、`time.ts`） | — | ≈20 行 |

合计 **≈700 行（17889 的 ~4%）**。总量不惊人——**真正的收益是 P1–P3 把三处
"多来源"压成"单来源"**，行数只是副产品。P4/P5/P7 是纯减法，零语义风险。

---

## 4. 不建议做的

- 不拆 `stores/sessions.ts`（1041 行 / 50+ 导出）：它已是 setup store，
  纯逻辑（`sessionLive` / `sessionTranscript`）在上一轮已抽出，再切只会换来
  跨模块调用；它是"会话域"这一 scope 的天然 owner。
- 不把 VDFS 与消息域**合并成一套渲染器**：两者分发键不同（`ext` vs 消息类型
  词表），强行统一会造出联合分发器，比两套同构更贵。只共享"注册表机制"（P4）。
- 不为消掉 `services/session.ts` 的形状适配而让 store 直接吃 `VdfsNode`：
  该文件 238 行承载的是"文档形状属于会话域"这条边界，去掉会把知识推进 store。
- 不追求 100% 测试覆盖：优先补 `VdfsTextDetail` / `VdfsReadonlyDetail` /
  `VdfsSessionDetail` 三个渲染器的动作装配断言（P2/P3 改动的护栏）。

---

## 5. 顺带发现（低优先，但建议一并处理）

- **注释与代码不一致**：`DetailForm.vue:193-195`、`VdfsActions.vue:11`、
  `Session.vue:20` 描述"页面单点注入机制动作"，代码里不存在（G1）。
  注释比重复更贵——它会骗过下一个人。
- **属性透传**：只有 `About.vue:26` / `Appearance.vue:154` 声明了
  `inheritAttrs: false`；`VdfsTextDetail` / `VdfsReadonlyDetail` 未声明，
  而 `VdfsWorkbench.vue:189` 恒传 `:testing`，会作为 DOM 属性落到根元素。
- **文档漂移**：`vdfs-frontend.md` §2.2 已实现表未含 `VdfsCard` /
  `Workbench` / `VdfsMessageDetail`；§7.1 的 S3 记「消息（转写）不经 VDFS」，
  而现状是消息已在 VDFS 上（`VDFS_EXT_MESSAGE` + `services/vdfsTranscriptSync.ts`
  + `.vdfs/session/<id>/消息/<mid>`）。§7 是进度档案、按约定不改写，
  但建议在 §2.2 补一行现状校正，避免读者以 S3 为准。
- `VdfsActions.vue` 注释提到的第三类消费方（"自定义渲染器经
  mechanism-actions prop 接收"）目前无实例。

---

## 6. 落实状态（复核后同轮完成）

§3 的削减清单已全部落地；G3 按本文建议**留到最后、本轮未做**。

| # | 状态 | 落点 |
|---|---|---|
| P1 | ✅ | `NavRail.vue` / `VdfsShell.vue` 删除，导航栏与列表并入 `Workbench.vue`；死插槽 `content` / `meta`、死 prop `back` / `backTitle`、死默认新建按钮一并清掉 |
| P2 | ✅ | 新增 `components/vdfs/DetailShell.vue`（标题 + 动作区 + 错误条）与 `rendererContract.ts`（props 一份接口）；四个渲染器 + `DetailForm` 共用，`inheritAttrs: false` 兜底随之不再需要 |
| P3 | ✅ | `useVdfs.mechanismActions` 单点计算（访问位 + 草稿 + 系统地址）；`mergeDetailActions`（同 id 去重 + divider）唯一装配；草稿判据收进 `schemas/vdfs.isVdfsDraft`；死 prop `deleting` 全链路移除 |
| P4 | ✅ | 新增 `registry/factory.ts`：`createRendererRegistry<TKey>` 成为「标识 → 组件 + 兜底」唯一实现，VDFS 域与消息域各声明一次（两张表互相隔离） |
| P5 | ✅ | `utils/time.ts::formatTime`、`services/vdfs.ts::mkdirVdfs` / `treeVdfs` 删除（均零引用）；`NavRail` 返回键与 `VdfsShell` 默认新建随组件删除 |
| P6 | ✅ | `ChatMainPanel` 的 `window.prompt` → 内联重命名栏、`window.confirm` → `ConfirmDialog` |
| P7 | ✅ | 时间格式化归一：`relativeTime`（列表项）/ `formatDateTime`（详情字段），秒 / 毫秒两口径在 `utils/time.ts` 一处处理 |
| G3 | ⏸ 未做 | 按本文「收益最大、风险也最高，建议最后做」的结论顺延；`Appearance` / `About` 仍是前端自持面板 |

§5 的顺带发现同步处理：注释与代码不一致（P3 后注释成立）、属性透传
（P2 的契约声明后不再往 DOM 写机制字段）、文档漂移（已在校正
`vdfs-frontend.md` §2.2 并补 S3 现状说明）。测试护栏按 §4 的建议补了
三个渲染器的动作装配断言（`components/vdfs/__tests__/VdfsDetailActions.spec.ts`）。
