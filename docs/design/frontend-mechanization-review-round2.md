# 前端机制化复核（第二轮）：第一轮未触及的层

状态：一次性复核报告（非规范、非事实表）
范围：`tauri/src`（复核起点 **93 实现文件 / 18948 行；44 测试文件 / 8543 行**）
依据：第一轮 [`frontend-mechanization-review.md`](./frontend-mechanization-review.md) 的结论
（机制主干已成立，缺口在详情渲染器层）+ 本轮对 `schemas/` `services/` `components/` 的逐文件实读
配套：结构事实以 `docs/CURRENT.md` §3、§5.1 为准，本文件只写判断与待办

---

## 1. 评定

**第一轮的结论仍然成立，且它划的边界是对的。** 本轮复核的价值不在推翻它，而在补上它
**没有覆盖的层**：第一轮把注意力全放在"详情渲染器层"，于是 `schemas/` 的**契约词表**、
`services/` 的**错误口径**、以及**新机制与门禁的配套**这三块从未被审过。

复核起点比第一轮时多 1059 行（17889 → 18948），增量主要来自会话域状态同步与压缩链路
（见 `docs/CHANGELOG.md` 2026-09-20）。**这些增量恰好落在第一轮的盲区里**，所以本轮
不是"再看一遍"，而是"看它没看过的地方"。

已扎实、本轮**不动**的部分：

- 地址口径前后端同源、路径代数只有 `schemas/vdfs.ts` 一份（第一轮结论，本轮复核仍成立）；
- `services/plugin.ts` 是 `invoke` 的**唯一**出口：全仓 `invoke` 只出现在该文件，
  所有 service 走 `callPlugin`，调用点零路由字面量；
- `constants/pluginPaths.ts` 是控制面路由的唯一登记处，覆盖率 100%；
- `utils/logger.ts` 是全仓唯一日志出口（`src` 内除它自身外零 `console.*` 直调）；
- `registry/factory.ts`（第一轮 P4）已成「标识 → 组件 + 兜底」的唯一实现，两域各声明一次；
- `styles/controls.css` 已建立**渐进收敛**约定（存量 scoped 副本不动、新组件不复制），
  `.node-act` 已从两处收敛为一份。

**本轮找到三个缺口，全部已就地修复**（G7–G9，见 §2）；另有五个已定位但**未动**的缺口
（G10–G14，见 §3），其中 G10 起属服务层，建议按批次推进。

---

## 2. 机制缺口（本轮已修）

### G7 消息状态词表有两份 —— 最该修

**同一个词表在两处各写一遍**：

| 位置 | 形态 |
|---|---|
| `schemas/chat_message.ts` | `MESSAGE_STATUSES` 数组 + 6 个 `MESSAGE_STATUS_*` 常量 |
| `schemas/vdfs.ts` | 6 个 `VDFS_STATUS_*` 常量（字面量重写一遍） |
| `services/vdfsTranscriptSync.ts` | `messageStatusOf` 的 `switch` 再枚举一遍 |
| `registry/messageTypes.ts` | `MESSAGE_STATUS_LABELS` 文案表 |

即**新增一个消息状态词要改 4 处**——正是 ADR-017 触发条件 1 描述的
「改 A 必须同时改 B 的跨文件连锁」。

**这不是理论风险，它已经造成过故障**：`aborted` 只补进了 `vdfs.ts` 那份，
`messageStatusOf` 的 `switch` 没跟上，于是消费端把整条变更的状态**静默丢弃**
（`default` 分支当时的语义是"没有状态"而不是"认不出"），最终表现为
**中止后的 Turn 显示为已完成、重试入口不出现**——即上一轮修掉的第三个问题。

**修法**（本轮落地）：

- `schemas/vdfs.ts` 的 6 个消息类常量改为 `MESSAGE_STATUS_*` 的**别名导出**
  （值单源，VDFS 侧命名不变，消费方零改动）；本文件只保留会话类状态
  `working` / `active` 的字面量；
- `services/vdfsTranscriptSync.ts` 的 `switch` 改为 `isMessageStatus()` 集合判定
  （由 `MESSAGE_STATUSES` 派生），未知词仍走 `logger.warn` **留痕**；
- 新增词现在只需改 `chat_message.ts` 的**相邻两行**（常量 + 数组），
  文案表由既有单测锁覆盖率。

**护栏**：`schemas/__tests__/vdfs.spec.ts` 新增 3 例——两侧常量同值、两侧集合完全相等
（漏一个或多一个都红）、会话类状态与未知词被 `isMessageStatus` 拦住。

### G8 弹窗外壳四份，其中三份没有键盘可访问性

四处弹窗各自手写遮罩：

| 组件 | 遮罩类 | ESC | 焦点陷阱 | 备注 |
|---|---|---|---|---|
| `ConfirmDialog` | `.confirm-overlay` | ✅ | ✅ | 唯一实现完整的一处 |
| `OptionFormDialog` | `.ofd-overlay` | ❌ | ❌ | |
| `HomedirSwitcher` | `.modal-mask` | ❌ | ❌ | |
| `ModelChatPanel` 编辑浮层 | `.edit-overlay` | ❌ | ❌ | |

**这首先不是"重复"，而是可访问性缺陷**：后三处的键盘用户按 ESC 关不掉弹窗，
Tab 会把焦点送到遮罩背后的页面上。

顺带查出两处硬编码（都不跟随主题或层级约定）：

- `ModelChatPanel.vue` 的 `.edit-overlay` 写死 `z-index: 100` —— 低于
  `--z-overlay`(1000) 与 `--z-dialog`(1500)，会被其它浮层盖住；
- 同处遮罩底色写死 `rgba(0, 0, 0, 0.45)` —— 浅色主题下比别的弹窗更暗、
  暗色主题下反而更浅（`--overlay` 在两个主题下分别是 `rgba(15,23,42,.45)` /
  `rgba(0,0,0,.6)`）；
- `HomedirSwitcher.vue` 的遮罩用 `--z-overlay`(1000)，低于其它弹窗的
  `--z-dialog`(1500)。

**修法**（本轮落地）：新增 `components/common/BaseModal.vue` —— 遮罩 + 面板 +
ESC + Tab 焦点陷阱 + 打开时自动聚焦，四处共用；`ConfirmDialog` 只留按钮语义，
其余三处只留自己的尺寸与边框。

类名契约：遮罩恒带机制类 `modal-mask`，面板恒带 `modal-panel`，调用方用
`panelClass` 追加语义类（`confirm-dialog` / `ofd-dialog` / `modal` / `edit-box`）。
**刻意不提供 `maskClass`**：遮罩样式在四处之间没有差异，多一个可传类名只会让
"某个弹窗的遮罩长什么样"变成两处可写。

**实现时踩到并修掉的一个坑**：`watch` 不加 `immediate: true` 时，
**挂载即可见**的弹窗（`OptionFormDialog` 恒传 `:visible="true"`）不会自动聚焦——
它没有"变化"可触发 watch。`BaseModal.spec.ts` 的对应用例正是为这条写的。

### G9 门禁不认识"prop 传类名"

`BaseModal` 落地后 `style-audit` 立刻报 4 个假警告（`.confirm-dialog` / `.ofd-dialog` /
`.modal` / `.edit-box` "未在本组件中使用"）：这些类名只出现在 `panel-class="…"` 的
**属性值**里，而审计只认 `class="…"` 与 `:class`。

**新机制必须让既有门禁认识它**，否则每引入一处就要往白名单里塞一条——白名单会掩盖真死类。

**修法**：`scripts/style-audit.mjs` 的模板提取增加 `*-class="a b"` 属性值识别
（计入**静态**使用，不给 Transition 那种 soft 待遇）。

修完再跑：**0 错误 0 警告**。并且这次调整暴露出一个**真问题**——`confirm-overlay`
失去样式定义后，审计立刻报"模板中使用了未定义的类"；该结论正确，遂删除这个
已无定义的类名（测试改由 `modal-mask` / `confirm-dialog` 定位）。

---

## 3. 削减清单（已定位，**未动**，按批次推进）

| # | 动作 | 主要落点 | 预计削减 |
|---|---|---|---|
| P8 | 服务层错误口径统一：`withFallback`（列表/读类吞错返兜底）与直抛（写类）两原语 | `services/*.ts`（14 处同构 `try/catch → log → fallback`） | ≈50 行 |
| P9 | `useVdfs` 改走 `subscribeVdfsChanged({prefix})`，删手写的 `affects` / `watched` / watch 块 | `composables/useVdfs.ts` | ≈30 行，并消除漏 unwatch 风险 |
| P10 | 抽 `createVdfsSync({scope, chain\|debounce, sink})` 统一 `sessionNodeSync` 与 `vdfsTranscriptSync` 的同构接线 | `services/` 两文件 | ≈80 行 |
| P11 | 路由登记合并：`constants/pluginPaths.ts`（控制面）与 `schemas/vdfs.ts`（10 个 VDFS op）两处登记 | 两文件 | ≈20 行 |
| P12 | 写入方收敛：`sessionMessages` 5 个写入方、`sessionStatuses` 6 个写入方收敛为 store 的少数命令 | `stores/sessions.ts` + 各写入方 | 不定（**根治竞态**，非行数） |

P12 的风险与收益都最高：它是"多路写同一份数据"，而 ADR-015 确立的
「前端显示由节点状态驱动」正是为了消灭这类竞态。**建议单独一轮、带回归测试做。**

---

## 4. 不建议做的

第一轮的 §4 全部继续有效（不拆 `stores/sessions.ts`、不合并 VDFS 与消息域渲染器、
不让 store 直接吃 `VdfsNode`、不追求 100% 覆盖）。本轮补充：

- **不把 `VDFS_STATUS_*` 整体删掉改叫 `MESSAGE_STATUS_*`**：别名已让值单源，
  再改就是全仓改名，收益仅是命名统一，代价是几十个调用点的无谓 diff。
- **不给 `BaseModal` 加更多 props 去"支持"尚未出现的需求**（如内建 Teleport、
  自定义 Transition 名）：内建 Teleport 会让所有使用方的 DOM 位置一并改变
  （测试里 `wrapper.find` 直接找不到内容），收益不抵代价——需要 Teleport 的
  调用方自己包一层即可（`OptionFormDialog` 就是这么做的）。
- **不为减少行数而拆 `BaseModal`**：它 180 行里有约 60 行是注释（解释为什么这样做），
  拆掉注释换来的行数没有意义。

---

## 5. 顺带发现（供后续复核者避坑）

- **`DetailForm.vue` 的 `inset: 0` 不是弹窗遮罩**，是 `.toggle-slider`（开关滑块）。
  本轮扫描时曾被误判为"第 5 处弹窗遮罩"——记在此处，免得下一个人再数错。
- **`registry/vdfsCards.ts` 的 `STATUS_OF` 不是消息状态词表**：它是**资源卡片健康
  状态**（`working` / `active` / `disabled` / `error` / `warning`），与消息的
  `pending` / `streaming` / … 是两套互不相干的词表。两者只有一个词重合（`working`），
  不要"顺手合并"。
- `schemas/vdfs.ts` 现在是 `schemas/chat_message.ts` 的**下游**（前者 import 后者）。
  这条依赖方向是刻意的：消息状态词 ⊂ 节点状态词，权威定义在消息契约层。
  往 `vdfs.ts` 里再加消息类状态字面量会破坏 G7 的收敛。

---

## 6. 落实状态

本轮（G7–G9）**同轮完成**；§3 的 P8–P12 未动，留作后续批次。

| # | 状态 | 落点 |
|---|---|---|
| G7 | ✅ | `schemas/vdfs.ts` 消息类常量改别名 + `services/vdfsTranscriptSync.ts` 改集合判定 + 两侧同源单测（3 例） |
| G8 | ✅ | 新增 `components/common/BaseModal.vue`；`ConfirmDialog` / `OptionFormDialog` / `HomedirSwitcher` / `ModelChatPanel` 四处共用；补 `BaseModal.spec.ts`（11 例） |
| G9 | ✅ | `scripts/style-audit.mjs` 识别 `*-class` 属性值；顺带删除失去定义的 `.confirm-overlay` |
| P8–P12 | ⏸ 未动 | 见 §3 |

改动规模：12 个既有文件 **+262 / −246**，另新增 `BaseModal.vue` 与其单测。
**行数净变化接近零**——本轮收益不在行数，而在"4 处 → 1 处"的词表收敛、
4 处弹窗的键盘可访问性，以及门禁对新机制的识别能力。这与第一轮的判断一致：
**真正的收益是把"多来源"压成"单来源"，行数只是副产品。**

验证：`vitest run` 45 文件 / 631 通过（起点 44 / 615，净增 16 例）；
`vue-tsc --noEmit` 干净；`style-audit` 0 错误 0 警告。

---

## 附：涉及后端的部分（整体规划，不在本轮实施）

`schemas/` 与 Rust 结构体目前是**手工镜像**（如 `chat_message.ts` ↔
`symbio_core/schemas/session/chat_message.rs` 逐字段对应），
`scripts/protocol-mirror-audit.mjs` 只覆盖 3 个常量。可自动化的候选：

| 数据源 | 可生成的产物 | 规则 |
|---|---|---|
| Rust enum + `serde(rename_all = "snake_case")` | `CHAT_ROLES` / `MESSAGE_TYPES` / `MESSAGE_STATUSES` | 枚举取值 → 常量 + 联合类型 |
| `vdfs_provider.rs` / `protocol.rs` 的 `pub const X: &str` | `VDFS_*` 操作 / 状态 / ext / action 常量 | 常量名与值直取 |
| 对应 Rust struct | `vdfs-form.ts` / `options.ts` / `session_list.ts` 等接口 | 字段名与类型映射 |

**不可生成**（纯业务判断，必须手写）：`messageTypes.ts` 的文案/折叠/优先级、
`vdfsCards.ts` 的三条约定、`vdfs-form.ts` 的 `mergeDetailActions` /
`detailPresetPatch`、`vdfs.ts` 的路径代数、`vdfsAddress.ts`。

**为什么不在本轮做**：引入 `ts-rs` / `schemars` 会改变"契约层是手写的"这一前提——
它同时影响 ADR（需要新决策记录）、`protocol-mirror-audit` 的定位（从"校验手写镜像"
变成"校验生成产物未漂移"）、CI（需要把生成步骤接进门禁），以及"前端零资源知识"
这条不变量的边界。**这是一次架构级改动，应按 ADR 的粒度整体规划后再分步实施**，
不适合混在机制化收敛批次里顺手做。
