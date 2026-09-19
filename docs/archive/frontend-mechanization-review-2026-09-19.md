# 前端机制化审查（2026-09-19）

状态：一次性评估记录（点时刻快照，非现行规范）
范围：`tauri/src` 全部（84 实现文件 / 17320 行，24 测试文件 / 4699 行，321 用例）
基线：`git rev-parse HEAD` = `5dcaa68` 之后的当前工作区
方法：读代码 + 跑门禁，不依赖记忆。所有数字为实测。

> 本文是**评估记录**，不是规范。机制条款仍以
> [design/vdfs.md](../design/vdfs.md) 与 [design/vdfs-frontend.md](../design/vdfs-frontend.md) 为准。

---

## 0. 结论摘要

| 维度 | 评分 | 一句话 |
|---|---|---|
| **资源链路机制化**（VDFS / 三栏） | **8.5 / 10** | 目标基本达成：「一个协议、一个页面、一处注册」在代码里可数 |
| **消息链路机制化**（聊天 / 转写） | **6.0 / 10** | 分层对，但知识是「有序 if 链」而非数据表，新增类型要改 6–8 处 |
| **状态层**（stores） | **5.5 / 10** | 纯函数已抽离，但 `sessions.ts` 五职责混装 + 6 组镜像状态 |
| **服务层**（services） | **7.0 / 10** | VDFS 门面干净；2 处反向依赖 store，插件路由名未收进常量表 |
| **工程门禁（前端侧）** | **5.0 / 10** | typecheck + vitest 在 CI，但**无 lint / 无覆盖率阈值 / build 不进 PR** |
| **工程门禁（后端侧）** | **9.0 / 10** | clippy `-D warnings` / fmt --check / workspace test / MSRV，硬 |
| **文档一致性** | **9.5 / 10** | `CURRENT.md` 数字与实际**完全一致**，`--check` 在 CI |

**总体判断**：资源管理这条主线已经**做完**了机制化，且做得比绝大多数同类项目彻底——
它有可执行的守卫脚本、有自动生成的事实表、有明确的不变量清单，而且**没有漂移**。
剩下的问题集中在两处：**(a) 消息/转写链路只机制化了一半**；
**(b) 状态层与工程门禁欠账**。前者是设计债，后者是纪律债，都不影响当前可用性。

---

## 1. 现行机制化方案（盘点）

### 1.1 三层分层（资源链路）

```
schemas/      数据契约（对齐后端 protocol.rs）        零组件知识
  ↓
registry/     纯 UI 映射（ext → 渲染器标识 / 图标）     零组件 import
  ↓
*Renderers.ts 标识 → 组件（唯一装配点）
  ↓
components/   视图
```

守卫：`scripts/mechanism-audit.mjs` 六条规则（M-001 … M-006），每条带回归测试，
进 CI 的 `docs,facts` 阶段。

### 1.2 收敛成果（可数的证据）

| 事实 | 证据 |
|---|---|
| 全 App 只有 **2 个真实路由组件** | `router/index.ts:19,30-34`：`MainLayout` + `VdfsView`，其余全是旧地址 redirect |
| 外壳不含导航知识 | `views/MainLayout.vue` 仅 57 行：RouterView + Toast + 全局初始化 |
| 地址页只做地址换算 | `views/VdfsView.vue` 162 行，其中 script 仅 60 行 |
| 三栏只有一份实现 | `components/vdfs/VdfsWorkbench.vue`（唯一三栏控件，绑定数据地址） |
| 前端 `entities/*` 调用点归零 | 全仓仅剩 `router/index.ts:23` 一条兼容 redirect |
| **零工具名硬编码** | `vdfs_read` / `ask_user` / `cmd` 仅出现在注释与测试中，生产代码 0 处 |
| 零 TODO/FIXME/HACK | 全仓 0 处 |
| 零 `@ts-ignore` | 0 处 |

### 1.3 状态驱动（顺序不敏感）——已落实

`eventBus.replayBuffer`（切会话防乱序缓冲）**确已删除**：全仓无实现，仅剩 3 处注释引用
（`eventBus.ts:182`、`vdfsTranscriptSync.ts:23`、`schemas/vdfs.ts:469`）。

替代方案是**按地址分派**：`sessionRouteOf(path)`（`schemas/vdfs.ts:446`）把变更地址解成
`session` / `messages` / `message` 三个目标，每个目标只认「这个地址现在的状态」，
不需要知道「上一条事件是什么」。运行态搬到**节点属性**上
（`status` + `attributes.outcome` + `attributes.error`，`schemas/vdfs.ts:460-508`）。

这是一次正确的架构选择，不是权宜之计。

---

## 2. 去业务化残留（机制化还有多少空间）

### 2.1 会话地址模板硬编码在前端 ⚠️ 与 §8 直接冲突

**证据**：`schemas/vdfs.ts:405,408,411,416,421,446-458`

```ts
export const VDFS_SEG_MESSAGES = '消息'      // 405
export const VDFS_SESSION_DIR = 'session'    // 408
export function vdfsMessagesAddr(id) { ... } // 416
export function sessionRouteOf(path) { ... segs[1] !== VDFS_SEG_MESSAGES ... } // 454
```

`design/vdfs-frontend.md` §8 明写：「前端**不得**硬编码资源类型、标签、能力或**路径模板**」。
会话的区段名（`消息`）与挂载名（`session`）是**后端 provider 的私有知识**
（`plugins/session/plugin.rs::SEG_MESSAGES`），前端持有了一份副本。

**为什么守卫没拦住**：M-002 只匹配 `.vdfs` 字面量（`/['"`]\.vdfs(?:\/|['"`])/`），
而这里用了 `vdfsJoin` + 常量拼装，正好绕过。

**收敛方向**（两条，代价不同）：

- **轻**：`sessionRouteOf` 保留在前端（事件分派确实需要一个本地解析器），但把两个常量
  改为**由后端下发**——`.vdfs/session` 的节点属性里带 `segments`（或让 provider 直接
  声明「转写列表地址」）。代价：新增一个协议字段。
- **重**：订阅时把「我关心的地址前缀」注册给后端，由后端按订阅分派语义目标，
  前端彻底不解析。代价：改 core 协议，且与「订阅是纯地址匹配」的现有模型冲突。

**我的建议**：暂不改。理由是这两个常量是**协议地址的一部分**（与 `.vdfs` 本身同级），
改它等于把「地址长什么样」也变成运行期数据，收益不抵复杂度。**但应该在 §8 里把它
写成显式例外**，并给 M-002 补一条规则：新增此类常量必须在 `schemas/vdfs.ts` 内，
且不得在 `components/` / `composables/` 里再出现一次。现状是**规则与实现不一致**，
这比实现本身更危险——下一个人会以为「常量表」是允许的，然后在别处再开一份。

### 2.2 消息类型/状态知识是「有序 if 链」而非数据表 ⚠️ 最大的一处

**证据**：`registry/messageTypes.ts`（660 行）

同一份「工具调用」类型清单在 **7 个函数**里各写一遍：
`messageIcon`(394-404) / `messageTitle`(412-427) / `messageStatusTag`(437-450) /
`messageHeadModifier`(478-487) / `isRunningAction`+`showsLiveBadge`(495-515) /
`messageDefaultOpen`(537-550) / `messageRendererKey`(627-635)。
`REASONING` 同样重复 6 处。

**后果**：新增一种消息类型要改 **6–8 个函数**。这与 `registry/vdfsTypes.ts` 的
「加一行」形成鲜明对比——同一个人写的两套注册表，一套是表，一套是 if 链。

**收敛方向**（低风险、高收益）：

```ts
interface MessageFacet {
  renderer: MessageRenderer
  icon: string
  title: string
  statusTag?: (s: string) => string | undefined
  headModifier?: string
  running?: boolean
  defaultOpen?: boolean
}
const FACETS: Record<MessageType, MessageFacet> = { ... }
```

七个函数塌缩成七个字段读取，660 行预计降到 ~250 行，且新增类型 = 加一行。
**这一项是整个前端机制化剩余空间里投入产出比最高的。**

### 2.3 「消息内容 → 文本」重复 5 份

| 位置 | 行号 |
|---|---|
| `composables/useMessageContent.ts` | 36-49 |
| `utils/message.ts` | 12-22 |
| `composables/useChatConnection.ts` | 119-130 |
| `stores/sessionTranscript.ts` | 50-59 |
| `components/ModelChatPanel.vue` | 273-279, 300-302 |

五种写法各处理一遍 `string` / `{text}` / `{parts[]}`。**应只留 `useMessageContent` 一份**
（它是纯函数、已有测试），其余四处改为调用。

### 2.4 业务文案与裸类型字面量（M-006 覆盖不到的地方）

**证据**：`services/vdfsTranscriptSync.ts:249-258`

```ts
if (msg.type === 'reasoning') store.putStatus(..., { activity: '正在思考…' })
else if (msg.type === 'tool_call') ... `正在调用 ${msg.name}…`
```

裸字符串比较消息词表——M-006 的规则**恰好就是这个**，但它的扫描范围是
`walk(path.join(SRC, 'components'), isVue)`（**只有 `components/` 且只有 `.vue`**）。
`services/` 与 `composables/` 是盲区。

另有第二份 activity 文案：`stores/sessionLive.ts:117,197`（`'处理中…'`）。

**收敛方向**：M-006 扫描范围扩到 `services/` + `composables/` + `.ts`。
预计会新爆几处，正好是要修的地方。

### 2.5 插件路由名未进常量表

`constants/pluginPaths.ts` 只有 4 个常量（全是 session 相关），而实际硬编码的
插件路由名有 5 处：

| 位置 | 字面量 |
|---|---|
| `services/eventBus.ts:124` | `'event_bus/subscribe'` |
| `services/home.ts:68` | `'home/get_homedir'` |
| `services/home.ts:105` | `'home/reload'` |
| `services/home.ts:128` | `'work/get_workspace'` |
| `services/home.ts:144` | `'work/set_workspace'` |

外加 `services/plugin.ts:365` 的 `target === 'gateway'` 特判，和 `services/home.ts:131`
硬编码的 `'~/projects'`。

**收敛方向**：全部搬进 `constants/pluginPaths.ts`，让「前端知道哪些插件路由」
变成一个可检索的清单（该文件头注释本来就写了这个意图，只是没执行）。

---

## 3. 简化 / 优化空间

### 3.1 17 个零引用依赖 + 构建分块残留 ⚡ 立竿见影

实测引用文件数：

| 依赖 | 源码引用 |
|---|---|
| `@milkdown/*`（12 个直接依赖，node_modules 里 27 个包） | **0** |
| `mermaid` | **0** |
| `katex` | **0** |
| `prismjs` / `prism-themes` / `@types/prismjs` | **0** |
| `marked` | 1（`useMarkdown.ts:7`） |

`package.json` 直接依赖 23 个，其中 **17 个零引用**。
`vite.config.ts:27-56` 的 `manualChunks` 仍在给 `mermaid` / `elkjs` /
`@milkdown/plugin-diagram` / `@codemirror` 分块——**对已删除代码的残留**，
且注释里还在解释「mermaid 动态导入 chunk 会超过 700KB」。

**动作**：删 17 个依赖 + 清 `manualChunks` 分支。这是全篇唯一「删了立刻见效、
零风险」的一项。

### 3.2 `stores/sessions.ts` 1096 行，五职责混装

| 职责 | 行号范围 | 该待在哪 |
|---|---|---|
| 状态定义（12 个 ref） | 100-168 | 留下 |
| 派生读 + 纯函数 | 171-214, 265, 924 | **纯函数模块**（`getSessionStaleReason` 内含硬编码 30 分钟阈值，`nextSeq` / `syncMessageCount` 亦然） |
| 写原语 | 216-331 | 留下 |
| 水合 / 回填 | 333-409 | 留下（但 403-408 的 `setLastWorkdir` 副作用不该在这） |
| 清单 CRUD | 411-526 | `createSession` 的地址拼接 + 乐观插入(480-526) → **service** |
| 消息 CRUD + 看门狗 | 659-931 | `fetchTranscript` 的 JSON 解析(716-730)、`persistStuckFailure`(875-921) → **service** |
| 事件订阅 | 933-1034 | **副作用模块**（现在硬编码在工厂尾部，不可启停、不可单测） |

同文件的 `sortTranscript` / `previewOf` / `mergeMessagePatch` **已经**抽到
`sessionTranscript.ts` 了——说明「该抽纯函数」这个标准是有的，只是执行不一致。

### 3.3 6 组镜像状态（同一份真相两处表示）

| 真相 | 表示一 | 表示二 | 对账函数 |
|---|---|---|---|
| 会话运行态 | `list[i].status` | `sessionStatuses[id].status` | `mergeListWithLive`(446) + `workingUpgradesOf`(453) |
| 标题 | `list[i].metadata.title` | `titles[id]` | `rename`(648-656) 双写 |
| 失败 | `sessionErrors[id]` | 消息节点 `error` | 注释自认互斥(163-168) |
| 最近工作目录 | `lastUsedWorkdir` | `plugin.ts::lastWorkdir` | `getLastWorkdir()`(58) |
| 转写（核心） | store `sessionMessages` | `useVdfs` 的 `items` + `nodeText` | 无（靠地址分流约定） |
| 失败类别 | `status=='failed'` | `MESSAGE_FAILURE_KIND_ERROR` | — |

前四组**可以直接收敛**（镜像存在的唯一理由是「写一处忘写另一处」，而它们靠对账函数维系，
本身就是症状）。第五组是**设计上的**双消费者（聊天工作区走 store、VDFS 消息详情走 useVdfs），
`MainLayout.vue:36` 的注释已说明「按地址分流，互不重叠」——**当前正确，但是脆的**：
一旦哪天有人在 VDFS 页里把消息详情和聊天面板同时打开，就会叠字。建议在
`useVdfs.applyAppend` 处加一条断言/注释固定这条不变量。

### 3.4 服务层反向依赖 store

- `services/vdfsTranscriptSync.ts:83` → `useSessionsStore()`（184, 245, 267 直接操作）
- `services/completionChime.ts:26` → `useSoundSettingsStore()`（105 调用）

service 依赖 UI 状态，分层不闭合。建议改为注入回调（`startTranscriptSync({ sink })`），
顺带让这两个模块可单测（目前都是零测试）。

### 3.5 大组件与重复

| 文件 | 行数 | 问题 |
|---|---|---|
| `components/vdfs/DetailForm.vue` | 941 | 机制内置的通用表单渲染器，职责单一但巨大（可拆 widget 分派表） |
| `components/ModelChatPanel.vue` | 576 | 直连 store（`useSessionsStore`）+ 看门狗 + 滚动 + 编辑浮层 |
| `components/chat/ChatOptionBar.vue` | 528 | `181,217` **直接改写 prop 对象**（`node.children` / `node.value`），破坏单向数据流 |
| `components/message/NodeShell.vue` | 407 | 与 `TurnGroupNode.vue` 重复 `.node-act` 样式(291-309 vs 124-142) 与 `typeClass`/`statusClass` |
| `components/session/ChatMainPanel.vue` | 368 | 直连 store + 用原生 `prompt()` / `confirm()`(171,179) |
| `components/chat/ChatInputArea.vue` | 303 | — |

另有 `ModelChatPanel.vue:126` `provide('resume')` ↔ `ToolCallNode.vue:134` /
`UserPromptNode.vue:137` `inject('resume')`——绕过 props/emits 且类型不安全。

### 3.6 类型安全

- `tsconfig.json`：`strict: true` + `noUnusedLocals` + `noUnusedParameters`（配置到位）
- `any` 23 处（`plugin.ts` 占 ~18 处）、`as unknown as` 4 处、`@ts-ignore` 0 处
- **`package.json` 没有 typecheck script**——`vue-tsc` 在 devDeps 里，只由
  `scripts/gate.mjs` 调用。本地开发者跑 `npm run` 看不到类型检查入口。

---

## 4. 工程门禁实况（实测）

| 门禁 | 配置 | 在 CI？ |
|---|---|---|
| `vue-tsc --noEmit` | ✓ | ✓（`gate.mjs --only=frontend`） |
| `vitest run` | ✓ | ✓ |
| `npm run build` | ✓ | ✗（只在 `release.yml` 打 tag 时） |
| ESLint / Prettier | **无配置** | ✗ |
| 覆盖率阈值 | **无** | ✗（devDeps 无 `@vitest/coverage-*`） |
| `gen-current-facts --check` | ✓ | ✓ |
| style-audit / mechanism-audit / doc-link / dead-code | ✓ | ✓ |
| cargo clippy `-D warnings` / fmt --check / workspace test / MSRV | ✓ | ✓ |

**测试实况**：`24 files / 321 tests passed`（实测 1.09s）。
零测试的关键模块：`services/plugin.ts`（661 行 IPC 内核）、`services/eventBus.ts`（421 行）、
`services/home.ts`、`services/options.ts`、`services/systemLocation.ts`、
`composables/` 8 个中的 7 个、`router/index.ts`、`registry/vdfsRenderers.ts`、
40 个 `.vue` 中的 36 个。

**测试输出有噪音**：`streamFlow.spec.ts` / `MessageNode.spec.ts` 触发了未 mock 的
`services/plugin.ts` 真实报错（`transformCallback` undefined）——测试仍绿，
但日志里堆着栈。属可清理项。

---

## 5. 建议路线

### P0（低风险、高收益，建议立刻做）

1. **删 17 个零引用依赖 + 清 `manualChunks` 残留**（`package.json`、`vite.config.ts`）。
2. **`registry/messageTypes.ts` 改为 `Record<MessageType, MessageFacet>` 数据表**
   —— 660 行 → ~250 行，新增类型从「改 6–8 处」变成「加一行」。
3. **M-006 扫描范围扩到 `services/` + `composables/` + `.ts`**，修掉新爆出的字面量比较。
4. **`package.json` 加 `"typecheck": "vue-tsc --noEmit"`**（一行，让门禁在本地可见）。

### P1（结构性，需要排期）

5. **「消息内容 → 文本」5 份收敛为 1 份**（`useMessageContent`）。
6. **插件路由名全部搬进 `constants/pluginPaths.ts`**，消掉 `plugin.ts` 的 gateway 特判。
7. **`sessions.ts` 按「纯逻辑 / service / UI 副作用」三段拆分**，`getSessionStaleReason` /
   `nextSeq` / `syncMessageCount` 下沉为纯函数；4 组可收敛的镜像状态收敛掉。
8. **两个 service 的 store 依赖改为注入**，顺带补测试。
9. **补覆盖率门禁**（`@vitest/coverage-v8` + 阈值），优先补 `plugin.ts` / `eventBus.ts` /
   `useChatConnection.ts` / `router`。
10. **把 `npm run build` 加进 PR CI**（现在构建错误只在上线时暴露）。

### P2（纪律与记录）

11. **修 §8 与实现的矛盾**：把「会话区段名常量」写成显式例外，并给 M-002 补一条
    「此类常量只能存在于 `schemas/vdfs.ts`」的规则。
12. **加 ESLint**（至少 `no-restricted-imports` 固化 M-003/M-004/M-005，比正则守卫可靠）。
13. 清理组件重复样式（`.node-act` / `typeClass` / `statusClass`），
    收敛 `ChatOptionBar` 的 prop 直改，`provide/inject('resume')` 改 props/emits。

---

## 6. 一句话总结

**资源链路已经「做完」了**——不是「做得不错」，是真的可以宣布完成：新增一类资源
后端实现一个 `VdfsProvider` 就行，前端零改动，而且这条性质有可执行的守卫在守。
**消息链路只做了一半**：骨架（分层、注册表、纯函数、地址分派）都对，
但知识没集中到一张表里。**欠账在状态层与前端门禁**：`sessions.ts` 该拆、
lint 和覆盖率该有。以上没有一项是「推倒重来」，全是收敛。

---

## 7. 落地记录（2026-09-19 同日执行轮）

上一节的 13 项**除 11 外全部落地**。逐项对照（括号内是实测结果，不是计划）：

| # | 项 | 结果 | 实测 |
|---|---|---|---|
| 1 | 删零引用依赖 | ✅ | 直接依赖 23 → 6；构建 185 模块 / 118KB gzip（原 mermaid 单块 >700KB） |
| 2 | 补 `typecheck` | ✅ | `npm run typecheck` 可见 |
| 3 | `messageTypes` 改数据表 | ✅ | 7 个 if 函数 → `Record<MessageType, MessageFacet>`；**行数未降**（322→363），收益是「新增类型改 1 处」+ 漏登记变编译错误 |
| 4 | M-006 扩范围 | ✅ | 新爆 4 处（全在 services/composables）并修掉 |
| 5 | 取文本 5 份 → 1 份 | ✅ | 下沉到 `schemas/chat_message.ts`；修掉 `ContentPart[]` 读成空串的真 bug；删死代码 48 行 |
| 6 | 路由名收进常量表 | ✅ | 全 `src` 字面量残留归零 |
| 7 | 拆 `sessions.ts` | **部分** | 1096 → 1022（订阅移出为 `sessionNodeSync`、`fetchTranscript` 下沉 service、看门狗口径抽纯函数）；**6 组镜像状态未收敛** |
| 8 | 解除 service→store | ✅ | 两个注入点（`startTranscriptSync(sink)` / `setChimeSettingsSource(fn)`），`CompletionKind` 三份定义收敛为 `SessionOutcome` |
| 9 | 覆盖率门禁 | ✅ | `@vitest/coverage-v8@4.1.11`；阈值 全局 40 / `src/registry/**` 80；实测 42.27%；已验证超阈值 exit=1 |
| 10 | build 进 PR CI | ✅ | 加在 gate 的 frontend 阶段（CI 就是跑 gate） |
| 11 | 修 §8 矛盾 | **未做** | 见下 |
| 12 | ESLint | ✅ | 只做 `no-restricted-imports` 分层 + `vue/no-mutating-props`；修掉 `VdfsActions` 函数与 prop 同名 |
| 13 | 组件清理 | 部分 | `.node-act` 双份 → 全局 `controls.css`；`provide/inject('resume')` → 类型化 `RESUME_KEY`；`typeClass/statusClass` 判定为不值得抽 |

### 11 的最终处理：**两个段名已从前端代码中踢掉**（运行期发现）

结论先说：`VDFS_SESSION_DIR` 与 `VDFS_SEG_MESSAGES` **已不存在于前端任何生产代码**。
地址方案是运行期数据（`services/vdfsScheme.ts`），按数据认出来，不是常量。

查证后端后的准确事实（**已更正本文件早期版本的两处错误**）：

- 挂载段名在后端 `symbio/src/symbio_core/ids.rs:40`（`PLUGIN_SESSION = "session"`）；
  转写段名在 `symbio/src/plugins/session/plugin/nodes.rs:288`（`SEG_MESSAGES = "消息"`）。
- 早期版本称「消息目录节点的 kind 与 name 同名」——**错误**。`VdfsNode::dir(name,
  title, access)`（`vdfs_provider.rs:516`）的第二个参数是 **title**，kind 恒为
  `VDFS_KIND_DIR`。正确的是 name 与 title 同名，消息目录因此与 `子会话` / `工作目录`
  无法按 kind 区分。
- 早期版本称「前端无法单方面消除」——**也不成立**。两个段名都能按数据认出来。

**怎么认出来的**（都需要后端配合一处，一共三行）：

1. **挂载段**——本来就能认，无需改后端。composite 的 `dir_node()`
   （`plugins/composite/vdfs.rs:185`）把 `p.root_new_types()` 挂到了挂载点节点上，
   会话 provider 声明的是 `VdfsNewType::new(VDFS_EXT_SESSION, "会话")`；
   前端 `VdfsNode` 已有 `new_types` 字段 ⇒ 列 `.vdfs` 根，找 `new_types` 含
   `ext === 'session'` 的子节点。左栏本来就是 `listVdfs` 动态枚举的
   （`useVdfs.ts:144`），所以此前 `VDFS_SESSION_DIR` 是**唯一**写死的挂载名。
2. **转写段**——需要后端给一个稳定标识。kind 是**场景可自定义**的（会话叶子就是
   `n.kind = PLUGIN_SESSION`，`nodes.rs:136`），故新增
   `VDFS_KIND_MESSAGES = "messages"`（`vdfs_provider.rs`）并在 `internal_dirs()` 里
   给转写目录打上（`nodes.rs`）。段名仍是展示名 `消息`，但**标识**交给 kind——
   段名随文案调整时，消费者按 kind 照样认得出。

**代价（写清楚，它不免费）**：解析要列目录，因此存在一段**引导窗口**——应用刚起来、
还没拿到会话清单时转写段尚未解析，此间到达的转写变更无法路由（会被跳过）。
此前写死常量时不存在这个窗口。缓解：`MainLayout` 启动即触发解析，`refreshList`
拿到会话后补一次，新建会话后再补一次；而引导窗口内没有任何会话被展示。
另外挂载目录只依赖根清单（零会话也能解析），所以列清单 / 读 / 删 / 改 metadata
这类只碰会话叶子的操作**不受引导窗口影响**——这也是把它与转写段分开缓存的理由。

**守卫同步改造**（`scripts/protocol-mirror-audit.mjs`）：

- A 组镜像从四组降为三组：段名镜像消失（前端不再持有），新增
  `VDFS_KIND_MESSAGES` 的跨栈校验；`VDFS_EXT_SESSION` / `VDFS_EXT_MESSAGE` 保留。
- 新增 **B 组缺席检查**：`VDFS_SESSION_DIR` / `VDFS_SEG_MESSAGES` 不得再出现在
  前端生产代码里（扫描 85 个文件，排除 `__tests__`——测试持有协议夹具是它的职责）。
  这是防回归：写死常量一旦悄悄回来，门禁立刻红。
- 回归测试 9 条，含「写回前端生产代码 ⇒ 红」「写进 __tests__ ⇒ 不红」。
- 后端侧：`internal_dirs()` 的断言里钉住 `kind == VDFS_KIND_MESSAGES`。

按「例外要踢出代码、不要写进文档」的原则，§8 **未添加任何例外条款**。

### 更正本文件上一节的判断

- 「`ChatOptionBar.vue:181,217` 直接改写 prop 对象」**是错的**：`node` 来自组件自持的
  `useSessionOptions()` composable，不是 props；改的是自己拥有的状态，
  不算单向数据流违规（`vue/no-mutating-props` 也未报）。
- 「`typeClass` / `statusClass` 重复」是 `status-${x}` 一行模板串，抽公共函数不划算。

### 门禁口径变化（2026-09-19 起）

frontend 阶段 = `vue-tsc --noEmit` → `vitest run --coverage` → `vite build` → `eslint`。
测试基线 321 → 319（删死代码 12 例、新增 14 例，`gate.mjs` 的 `BASELINE` 已注明理由）。
