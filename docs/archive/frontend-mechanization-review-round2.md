# 前端机制化复核（第二轮）：第一轮未触及的层

状态：一次性复核报告（非规范、非事实表）
范围：`tauri/src`（复核起点 **93 实现文件 / 18948 行；44 测试文件 / 8543 行**）
依据：第一轮 [`frontend-mechanization-review.md`](./frontend-mechanization-review.md) 的结论
（机制主干已成立，缺口在详情渲染器层）+ 本轮对 `schemas/` `services/` `components/` 的逐文件实读
配套：结构事实以 `docs/CURRENT.md` §3、§5.1 为准，本文件只写判断与待办
**后续追加**：P8–P12 落地（§6）之后，又完成了「跨栈契约守卫扩展 + ADR-019」——
即本文末尾那节"涉及后端的部分"从**规划**转为**已实施**；同时查清并修掉了
「前端测试跑完不退出」的根因（见 §7 后的说明）。

---

## 1. 评定

**第一轮的结论仍然成立，且它划的边界是对的。** 本轮复核的价值不在推翻它，而在补上它
**没有覆盖的层**：第一轮把注意力全放在"详情渲染器层"，于是 `schemas/` 的**契约词表**、
`services/` 的**错误口径**、以及**新机制与门禁的配套**这三块从未被审过。

复核起点比第一轮时多 1059 行（17889 → 18948），增量主要来自会话域状态同步与压缩链路
（见 2026-09-20 那批提交）。**这些增量恰好落在第一轮的盲区里**，所以本轮
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
（G10–G14）——它们当时只留在判断里没落纸，第三轮已重新实读并逐条实施，见 **§3.2**。

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

## 3. 削减清单（P8–P12）

> **本节的预估已被实施结果修正，实际落点与差异见 §6 / §3.1。**
> 保留原表是为了留下"当时是怎么判断的"这条痕迹——四处预估都偏了，偏法各不相同。

| # | 动作 | 主要落点 | 预计削减 |
|---|---|---|---|
| P8 | 服务层错误口径统一：`withFallback`（列表/读类吞错返兜底）与直抛（写类）两原语 | `services/*.ts`（14 处同构 `try/catch → log → fallback`） | ≈50 行 |
| P9 | `useVdfs` 改走 `subscribeVdfsChanged({prefix})`，删手写的 `affects` / `watched` / watch 块 | `composables/useVdfs.ts` | ≈30 行，并消除漏 unwatch 风险 |
| P10 | 抽 `createVdfsSync({scope, chain\|debounce, sink})` 统一 `sessionNodeSync` 与 `vdfsTranscriptSync` 的同构接线 | `services/` 两文件 | ≈80 行 |
| P11 | 路由登记合并：`constants/pluginPaths.ts`（控制面）与 `schemas/vdfs.ts`（10 个 VDFS op）两处登记 | 两文件 | ≈20 行 |
| P12 | 写入方收敛：`sessionMessages` 5 个写入方、`sessionStatuses` 6 个写入方收敛为 store 的少数命令 | `stores/sessions.ts` + 各写入方 | 不定（**根治竞态**，非行数） |

P12 的风险与收益都最高：它是"多路写同一份数据"，而 ADR-015 确立的
「前端显示由节点状态驱动」正是为了消灭这类竞态。**建议单独一轮、带回归测试做。**
（实施后发现：`sessionMessages` 侧早已收敛，只剩 `sessionStatuses` 侧没跟上；
也没有潜伏 bug——见 §6。）

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

**G7–G9** 同轮完成（提交 `d8a9ccf`）；**P8–P12** 在后续一轮完成（本节）。

四条的结论与 §3 的预估**都不同**——预估只看到了"形状相同的代码"，没看到
"形状相同 ≠ 同一件事"，也没看到既有门禁已经把某条边界钉死了：

| # | 状态 | 落点 | 与预估的差异 |
|---|---|---|---|
| G7 | ✅ | `schemas/vdfs.ts` 消息类常量改别名 + `services/vdfsTranscriptSync.ts` 改集合判定 + 两侧同源单测（3 例） | — |
| G8 | ✅ | 新增 `components/common/BaseModal.vue`；四处弹窗共用；补 `BaseModal.spec.ts`（11 例） | — |
| G9 | ✅ | `scripts/style-audit.mjs` 识别 `*-class` 属性值；顺带删除失去定义的 `.confirm-overlay` | — |
| P8 | ✅ | 新增 `services/fallback.ts`（`withFallback`）；9 处读/列入口改为其调用方 | 预估 −50 行 → 实际净 **+40 行**（见下） |
| P9 | ✅ | `composables/useVdfs.ts` 改走 `subscribeVdfsChanged({prefix: addr})` | 预估 −30 行 → 实际净 **−4 行**；**顺带修掉两个真缺口** |
| P10 | ✅ | 新增 `services/syncLifecycle.ts`；两处 sync 共用其生命周期 | 预估 −80 行 → 实际净 **+50 行**；**修掉 `sessionNodeSync` 漏 HMR 守卫** |
| P11 | ⛔ **不做**（只改文档） | `constants/pluginPaths.ts` 与 `schemas/vdfs.ts` 的注释 | **与既有门禁 M-007 直接冲突**，见 §3.1 |
| P12 | ✅ | `stores/sessions.ts` 加 `commitStatuses` / `updateStatus` / `newLiveStatus` + 4 例回归测试 | 「不定（根治竞态）」→ 实际是**结构收敛**，无潜伏 bug |

生产代码 **+387 / −187（净 +200）**，其中 164 行是 `fallback.ts` 与 `syncLifecycle.ts`
两个新原语（注释约占六成）。**行数是涨的，而且这次涨得有道理**：第一轮的结论
（"真正的收益是把多来源压成单来源，行数只是副产品"）在这里被反过来验证了一次——
当"单来源"本身需要一个具名、有文档、有测试的原语时，行数必然上升。

### P9 顺带修掉的两个真缺口

1. **`useVdfs` 收不到前端本地通道**。它原先用裸 `subscribe({kind: VDFS_EVENT_KIND})`，
   只订总线频道；而会话 store 的乐观写走的是 `publishVdfsChangedLocal`
   （`stores/sessions.ts` 两处）——那条通道只投给 `subscribeVdfsChanged` 的注册者。
   于是"删掉一个会话"在别的页面**不会**让 VDFS 浏览器刷新。改用统一入口后一并拿到。
2. **手写 watch/unwatch 生命周期**（`watched` 变量 + `watch(cwd)` + `onBeforeUnmount`）
   换成了 `eventBus` 已有的**引用计数 + 串行链**实现，漏 unwatch 的风险消失；
   "虚拟根不登记 watch"这条知识也不再由本层复述。

**一处有意的行为变化**：订阅前缀从"当前目录"改成"**绑定地址**"（否则左栏子目录的
增删无法刷新导航——那类变更落在 `addr` 上）。代价是**绑定地址之上**的祖先变更
不再送达本视图（原先 `affects` 会命中）。判断：那属于过度刷新而非功能，
且"前缀即作用域"是事件总线既有的投递语义，不该为它开例外。

### P10 修掉的一个真缺口：`sessionNodeSync` 没有 HMR 守卫

两个同步器都持有模块级订阅句柄，而 `vdfsTranscriptSync` 有 `globalThis` 标记
（注释写明了原因：HMR 重置模块级变量 ⇒ 订第二条 ⇒ 同一条变更被处理两遍），
`sessionNodeSync` **没有**。同一个陷阱、两种命运——这正是"各写一遍"的代价。
`services/syncLifecycle.ts` 把「幂等启动 / 可停 / HMR 守卫」收成一处后，
下一个同步器不可能再漏。

**注意这里刻意没有合并的东西**：合并策略（清单是 800ms 防抖重拉、转写是逐路径
串行链）与订阅原语（清单按作用域前缀、转写按地址自行分派）都**保留差异**——
它们是业务差异，不是重复。强行统一只会得到一个比两份实现更难读的配置对象。

### P12 的边界：它收敛的是结构，不是竞态

§3 把 P12 描述为"根治竞态"。实读后修正：`sessionMessages` 侧**早已**收敛
（`commitMessages` + `updateMessages`，全仓 `sessionMessages.value =` 只有一处），
真正没跟上的是 `sessionStatuses` 侧——`putStatus` 存在，但另有 4 处手写
"展开 → 合并 → 整体替换"。收敛后 `sessionStatuses.value =` 同样只剩一处。

**没有发现潜伏 bug**：4 处手写点的 `last_event_at` 当时都恰好写对了。但**最容易写错
的就是它**——条目一旦存在，`getSessionStaleReason` 就按 `last_event_at` 算"多久没消息了"，
初值若写成 0，刚建出的条目会立刻被判成"状态已过期 N 分钟"（N 自 Unix 纪元起算），
界面凭空报"连接已断开"，**而没有任何测试会失败**。因此新增 4 例回归测试锁住它，
并用变异检查验证过：把 `newLiveStatus()` 的初值改回 0，对应用例**确实变红**。

---

## 3.2 第三轮：G10–G14（已定位并实施）

> **本节的由来（一条值得记的教训）**：§1 原先写着「另有五个已定位但**未动**的缺口
> （G10–G14，见 §3）」，而 §3 是 P8–P12 的削减清单、**从来没有这一节**——那五个缺口
> 只存在于写报告时的判断里，从未落纸，`git log -S G10` 只能查到这一行悬空引用。
> 第三轮把它们重新实读、逐条核对后补齐在此。**"已定位"不等于"已记录"**；判断不落纸
> 就等于没做过。

| # | 缺口 | 类别 | 与预估的差异 |
|---|---|---|---|
| G10 | **M-006 的扫描范围漏了 `stores/` 与 `registry/`** | 门禁盲区 | 当时没写；实读才发现是**门禁自己**的漏 |
| G11 | `services/vdfs.ts` 三处读/列入口没走 `withFallback` | 重复错误口径 | 与当时分档一致（P8 的漏网） |
| G12 | `MessagePromptKind` 判别式**类型**在 3 文件重抄 | 重复词表 | 比预估窄：只收敛类型，见下 |
| G13 | 文本渲染器子集三份写法 | 重复词表 | **判据与预估相反**：三份里有**两份不是同一个集合** |
| G14 | `SessionMode` / `SessionRiskLevel` 联合在 3 文件重抄 | 重复词表 | 比预估多一层：`metadata` 是 `any`，枚举只在运行期 |

### G10 M-006 的扫描范围是漏的（门禁自己的缺口）

M-006 的规则没错，但它的 `auditFiles(...)` 只喂了 `components/ + composables/ +
services/`。于是 `stores/sessionTranscript.ts` 的**四处**字面量比较长期无人看守——
而该文件**明明已经 import 了 `MESSAGE_STATUS_*` 常量**，同文件两种写法并存。

教训与 G9 同源但更尖锐：**门禁的"扫描范围"和它的"规则"一样会漏，而漏了不会红**。
规则写错至少还能被"注入违规看它红不红"的回归测试发现；范围漏了连那个都发现不了
（测试夹具也铺在那个范围里）。

已把范围扩到 `stores/` + `registry/`（76 个文件），并修掉四处。

### G11 三处读/列入口没走 `withFallback`

P8 当时收敛了 9 处，漏了同文件里的 `listVdfs` / `statVdfs` / `readVdfs`——
而 `ensureVdfsRoot`（同一文件）已经走了原语，所以是"同一文件两种写法"。

三处的**兜底语义并不一致**（空目录 vs `null`），这正是 `withFallback` 的设计前提：
兜底值在**调用点**写出来，类型系统保证它与成功值同型。日志级别也照原样保留
（`statVdfs` 的失败是**预期内**的，降为 `debug`——否则日志失去信噪比）。

顺带补了 5 例回归测试：原语化是**行为保持**的重构，而没有测试就证明不了"保持"。

### G12 `MessagePromptKind`：只收敛**类型**，不收敛校验

`'question' | 'confirm'` 作为**类型**在 `registry/messageTypes.ts` 抄了两遍
（字段类型 + `promptKindOf` 返回类型）。现由 `schemas/message_prompt.ts` 导出
`MessagePromptKind = MessagePrompt['kind']`，消费点一律引用。

**没有**一并收敛 `promptOf` 里那句 `p.kind !== 'question' && p.kind !== 'confirm'`：
它验的是 `meta.prompt as MessagePrompt` 这个**转型后的值**，必须逐词判断——
那里是运行期枚举的正当位置，把它换成词表反而会绕开类型收窄。

⚠️ **并发现一个守卫边界**：后端是以**裸 JSON 字面量**产出这两个词的
（`plugins/local/ask_user.rs` 的 `"kind": "question"`、`plugins/local/plugin.rs`
的 `"kind": "confirm"`），**不是** serde 枚举 ⇒ C 组（只认 `rename_all` 枚举）
**看不见它**。后端改这两个词不会有任何守卫变红。这是**已知缺口**，已写进
`schemas/message_prompt.ts` 的注释，不要把它当成"已被守住"。

### G13 三份写法里有**两份不是同一个集合**

这是"形状相同 ≠ 同一件事"最干净的一个例子：

| 位置 | 集合 |
|---|---|
| `useVdfs` 的追加守卫 | `{text, markdown, json, message}` |
| `VdfsWorkbench` 的渲染数据 | `{text, markdown, json, message}` |
| `useVdfs` 的**读取**守卫 | 上面四个 **∪ `{form}`** |

表单的字段值同样取自节点正文（`vdfs/read` 的 `text`，前端 parse 成对象后作为显式
入参交给渲染器），所以读取守卫多一个 `form`；而**追加**对表单毫无意义。

因此给出**两个**具名谓词（`isTextualRenderer` / `rendererReadsNodeText`）而不是
硬凑一个。测试里有一条专门断言**两者的差集恰好是 `form`**——泛泛地测两个 `true`
是测不出这个区别的。

### G14 `metadata` 是 `any`，枚举只活在运行期

`SessionListItem.metadata` 的类型是 `Record<string, any>`（后端回包没有类型兜底）。
于是"取值校验"只能靠运行期枚举，而枚举必须只有一处——先前
`stores/sessionLive.ts` 手写 `m.risk_level === 'low' || … === 'medium' || … === 'high'`，
**风险等级多一个取值时会被静默丢弃**（既不报错也不生效）。

现在词表在 `schemas/session_meta.ts`（`SESSION_RISK_LEVELS` / `SESSION_MODES`），
校验改为 `includes`。这也让 `risk_level` **可被 C 组守卫**——裸字面量联合在运行期
不存在，守卫看不见它（详见文末"附"里的 C 组扩面）。

### 3.2.1 规模与验证

| 指标 | 值 |
|---|---|
| 生产代码 | 11 文件（`mechanism-audit.mjs` + `protocol-mirror-audit.mjs` + 9 个 `tauri/src` 源文件） |
| 测试 | 2 文件（`vdfsTypes.spec.ts` +5、`vdfs.spec.ts` +5）；守卫回归 30 → **35** |
| 新增原语 | 无（本轮全是"收口"与"补漏"，不引入新抽象） |

验证（全部通过）：`protocol-mirror-audit` **A 31 + B 2 + C 6 + D 23，Errors 0**；
`protocol-mirror-audit.test.mjs` **35/35**；`mechanism-audit` 七条规则全过（M-006 现覆盖
**76 文件**）；`vitest run` **47 文件 / 661 测试**，exit=0；`vue-tsc --noEmit` 干净；
`gate --only=frontend` **4/4**。

---

## 3.1 一条**不要做**的合并：P11 与门禁 M-007 冲突

§3 的 P11 提议把 `schemas/vdfs.ts` 的 10 个 VDFS op 并入
`constants/pluginPaths.ts`（理由是后者的文档自称"插件路由名的唯一登记处"）。
本轮照做后，**`mechanism-audit` 立刻报 10 个 M-007 错误**：

> M-007：地址常量的定义权在 `schemas/vdfs.ts` —— 此处不得再定义一份（导入使用是允许的）

即：**项目早已有一条门禁把这条边界钉死了**，而 P11 的前提（"两处登记应合并"）
与它直接矛盾。且两处登记本来就是**互不相交**的集合（`pluginPaths.ts` 里没有任何
`vdfs/*`），不存在重复定义可删——§3 写的"≈20 行"无从谈起。

**正确动作是让文档与门禁一致，而不是让代码迁就文档**：`pluginPaths.ts` 的文档改为
「**控制面**插件路由名的唯一登记处」，并写明 VDFS 常量归 `schemas/vdfs.ts`、
由 M-007 钉住、**不要往这搬**；`schemas/vdfs.ts` 的常量处补一句反向指引。
两层分开的理由是实的：VDFS 常量是**契约**（要与响应类型、变更词汇、地址代数同处
一个文件），控制面路由是**前端组织的路由表**，不是同一个登记处的两半。

---

## 7. 附录：改动规模与验证

| 指标 | 值 |
|---|---|
| 改动文件 | 16（生产 11 / 测试 5），另含 `docs/CURRENT.md` 重生成 |
| 生产代码 | +387 / −187（净 +200） |
| 新增原语 | `services/fallback.ts`、`services/syncLifecycle.ts` |
| 新增测试 | 15 例（`fallback` 5、`syncLifecycle` 6、`sessions` 状态通道 4） |

验证（全部通过）：`vitest run` **47 文件 / 646 测试**；`vue-tsc --noEmit` 干净；
`style-audit` 0 错误 0 警告；`mechanism-audit` 七条规则全过；`grep-audit` 0 错误；
`protocol-mirror-audit` / `schema-audit` / `dead-code-audit` / `doc-link-audit` /
`test-layout-audit` / `plugin-entry-audit` 全过；`gen-current-facts --check` 一致。

> **测试卡死根因（已修，此前判断是错的）**：全量 `vitest run` 曾**跑完不退出**——
> 汇总行已打印、测试全绿，进程却挂着，`timeout` 必被触发。本文件早先把它记成
> "环境问题（`TMP=C:\Temp` 导致 Vite SSR 缓存 `EPERM`）"，**那个判断是错的**：
> 换成干净临时目录后照样挂。
>
> 实测结论（同机、同用例、同命令，只改池类型）：
>
> | 池 | 结果 |
> |---|---|
> | `forks`（vitest 4 默认） | 挂起 |
> | `forks` + `--no-file-parallelism` | 挂起（⇒ 与并发度无关，是池实现本身） |
> | `threads` | **正常退出** |
>
> 修法是 `tauri/vitest.config.ts` 显式写 `pool: 'threads'`（不依赖默认值——默认值
> 随 vitest 版本变，而"挂不挂"不该由版本决定）。修后全量 **47 文件 / 646 测试，
> exit=0**。
>
> 这个故障难发现，是因为它**只影响进程退出、不影响测试结果**：报告一切正常，
> 只有"命令不返回"这一条线索，于是很容易被归因成环境抖动。

---

## 附：涉及后端的部分 —— 已按 ADR-019 规划并实施

`schemas/` 与 Rust 结构体是**手工镜像**（如 `chat_message.ts` ↔
`symbio_core/schemas/session/chat_message.rs` 逐字段对应）。本节原先把"跨栈重复"
列成两条路（生成侧 `ts-rs`/`schemars` vs 检查侧扩审计），并判定要先按 ADR 的粒度
整体规划。

**规划已完成并落地**（见 `docs/DECISIONS.md` 的 **ADR-019**）：

| 数据源 | 处置 | 落地形式 |
|---|---|---|
| `vdfs_provider.rs` / `protocol.rs` 的 `pub const X: &str` | ✅ 纳入守卫（**自动发现**） | A 组：同名交集逐字比对，**3 → 31 条** |
| Rust enum + `serde(rename_all = "…")` | ✅ 纳入守卫（**集合相等**） | C 组：**6 张词表**（角色 / 类型 / 状态 / 恢复动作 / 选项节点类型 / 工具风险等级），`snake_case` 与 `lowercase` 都守得住 |
| 对应 Rust struct 的**字段名** | ✅ 纳入守卫 | D 组：**23 对**结构体字段（VDFS 契约 + `ChatMessage` + 详情方言 9 对 + 选项机制 5 对），只查"前端字段能否在后端线格式里找到" |
| 对应 Rust struct 的**类型映射** | ⛔ 不做 | 正则读不出 `Option<u64>` → `number`，泛型 / 嵌套会失控；收益低于字段名 |
| `messageTypes.ts` 文案 / `vdfsCards.ts` 约定 / `vdfs-form.ts` 派生 / 路径代数 | ⛔ **不可生成** | 纯业务判断，必须手写 |

> **后续修订（2026-09-23）**：上表的 C / D 组条目数是**当时**的实测值。
> 会话选项机制 schema 化下线后，C 组撤除「选项节点类型」并新增「详情方言取值原语」
> （`DETAIL_PICK_*` ↔ `DETAIL_PICKS`）⇒ 仍 **6 张**但成分不同；D 组撤除选项机制 5 对
> ⇒ **18 对**。**数字以 `scripts/protocol-mirror-audit.mjs` 的 `ENUM_SETS` /
> `STRUCT_SETS` 长度为准**，本报告不追着改（它是一次性复核，不是事实表）。

**决策是"审计"而非"生成"**（ADR-019）：`serde` 在**格式层**已单源，重复只在
**符号层**（Rust 标识符 vs TS 常量名）；且 95 处 serde 标注里的 `untagged` /
`tag = "type"` / `skip_serializing_if` 让机器导出不可靠。既有 `X-001..X-003`
正是"审计镜像"的先例——扩展它与既有机制同向，引入生成器则是架构级改动。

**G3（`Appearance.vue` / `About.vue` 去语义）一并复核并否决**：不变量 4 的原文
（`vdfs-frontend.md:42-43`）**明文允许**前端持有 `ext → 渲染器` 映射，被禁的是
「资源类型清单、标签、路径模板、能力开关」。G3 的前提（`appearance` / `about`
是"唯一 ext 即语义类型名的特例"）不成立——`vdfsRenderers.ts:24-31` 里
`form` / `session` / `message` / `text` 同样是 ext → 渲染器。

**顺带补掉的真实缺口**（不在原规划内，实施中发现）：

- **`ResumeAction` 的线格式词在前端有两份手写副本**（`useChatConnection` 的载荷
  联合类型、`messageTypes` 的重试分派），而 Rust 单测
  `resume_action_wire_words_are_snake_case` 的注释**早就点名要求**"前端
  `ResumePayload.action` 的字面量必须与它逐字相等"——契约喊了话，前端没接。
  现已建 `RESUME_ACTIONS` 词表，5 处生产字面量全部收敛到常量。
- **`ChatMessage` 的两个死字段**（`agent_id` / `prompt`）：D 组上线当次抓出。前端
  单方面声明，后端从不下发——`agent_id` 在 `meta` 里（`agentNameOf` 读 `meta.agent_id`）、
  `prompt` 在后端是 `#[serde(skip_serializing)]`（内容在 `meta.prompt`）；前端对两者
  均**零点访问、零构造点**。已删除，并在接口上方留了说明。
- **D 组覆盖面扩展（9 → 23 对）**：随后纳入两批"前端逐字段镜像了整套形状"的契约
  ——`detail.rs` ↔ `vdfs-form.ts`（9 对，`DetailField` 17 字段一个不落）、
  `options.rs` ↔ `options.ts`（5 对，`OptionNode` 16 字段一个不落）。纳入前用探针
  逐对比对，14 对全部 0 差异。**未纳入的反例**：前端只挑几个字段用的响应结构
  （如 `ChatMessage.response_id`）——它本就该按需取，不构成"第二份真相"。
- 审计脚本的 `✓` / `✗` 曾被编码事故替换成 `?`（红绿都显示 `?`，且 NO_COLOR 下
  无从区分），已修。
- **C 组从 4 张扩到 6 张，并修掉两处"守卫自己看不见"的写法**（第三轮，见 §3.2）：
  - **`rename_all` 的取值改为自动读取**。原先把 `snake_case` **硬编码**成检查项
    （`hasRenameAllSnakeCase`），于是 `#[serde(rename_all = "lowercase")]` 的枚举
    （`RiskLevel`）虽然前端有镜像却**无人看守**——"枚举类型对了、属性取值没覆盖到"
    是守卫自己的漏。现在读出声明的取值再选转换规则（`CASE_CONVERTERS`），**不支持的
    取值直接报错**而不是猜。
  - **词表元素允许裸字符串字面量**。原提取器只认"本文件的字符串常量名"，于是
    `OPTION_TYPES = ['invoke', 'sub', 'form']` 这类**只出现一次**的词被拒。守卫要的是
    "取值只有一处"，不是"每个词都有名字"——给只出现一次的词硬造常量名，只会得到一层
    **无人引用的间接**。
  - 新纳入 `OptionType ↔ OPTION_TYPES`、`RiskLevel ↔ SESSION_RISK_LEVELS` 两张词表。
