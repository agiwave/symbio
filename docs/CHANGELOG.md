# 功能更新记录

> **文档类型：Reference（参考）** — 按时间倒序的功能/修复事实记录。
> 2026-04-04 及更早的条目已归档至 [archive/CHANGELOG-history.md](./archive/CHANGELOG-history.md)。

> **重要：当前形态声明（2026-07-06 起）**
>
> 本仓库当前形态是 **Rust 核心库 + Tauri (Vue 3) 桌面前端 + E2E CLI**。
>
> - `tauri/` 目录是活跃维护中的桌面前端（一等公民），与核心库版本同步演进。
>
> - `symbio/src/bin/seed_agents.rs` 提供批量灌入种子 Agent 的 CLI（当前唯一二进制入口）。
>
> - 早期"Tauri 时代 = 临时形态"的说法已被否决：前端自 2026-06 多会话体系改造后恢复为**一等公民**。
>
> 下方按日期倒序记录**与代码同步**的功能/修复条目。
> 下方按日期倒序记录**与代码同步**的功能/修复条目，前端 UI 变更同样记录于此。

***

## 2026-09-18: 修复 Telegram LLM 回复链路 + 新增守卫 E-007

**性质：功能修复 + 新增守卫**。承上一条：上一轮修好了 Telegram 的**地址**
（`session/chat` → `session/chat/send`），但那条链路**仍然不通**——本轮把功能修通，
并把病因变成守卫。

**病灶不是「漏传了一个参数」，而是「按值持有兄弟插件」这个设计。**
`telegram/plugin.rs` 有个 `llm_plugin: Arc<RwLock<Option<Arc<dyn Plugin>>>>` 字段，
只在 `handle_start_listener(Some(p), …)` 里被写入，而唯一调用点
`invoke_start_listener` 传的是 `None` ⇒ `process_update` 恒走「LLM 插件未配置」，
每条 Telegram 消息都被回成这句话。`git log` 显示这个形态**从初始提交
（`e00832d`）就是这样**：自始至终没通过。

修复方式是**回到地址规则**（`docs/design/plugin-route-address.md` 规则五）：
删掉该字段与注入参数，`process_update` 改用 `ctx.parent()` 取容器后
`parent.route(ctx)`，地址走 `SESSION_CHAT_SEND`。容器本来就塞好了入口
（`composite.rs` 的 `SimpleRequest::new(Some(composite_weak), None)`，`fork()` 保留
`PARENT`），可运行先例是 `agent/host/subagent.rs` 的三处 `parent.route`。

**同时纠正上一轮的一个错误结论。** 上一条把 `telegram/*` 六条读成「休眠」，
依据是「零调用方」。**零调用方 ≠ 不可达**：网关把请求体里的 `path` 原样转发给
容器 `route`（`gateway/server.rs` 的 `dispatch_once` / `handle_ws`，只过一层只读
白名单）⇒ **对外 API 天然是 `refs=0`**。所以这不是死代码，是一条真的对外通道，
只是它一直在回「LLM 插件未配置」。审计报告段与 design 文档 §4 都补上了这条前提。

**新增守卫 E-007**（`scripts/plugin-entry-audit.mjs`）：插件不得按强引用持有兄弟
插件实例。判据是结构体字段类型含 `Arc<dyn Plugin>` 且不含 `Weak`，四条豁免逐条
对应仓里的真实形态：`Weak`（向上引用父）· `HashMap`/`Vec`/`BTreeMap`（容器按名
持有多个子实例）· 字段名 `parent`/`router`（本仓专指向上引用）· 类型以 `&` 开头
（借用，字段不可能是这个形态 ⇒ 必是形参）。**ERROR 级**——全仓现有形态扫下来零命中。

回归测试 21 → 30 例（新增 9 例覆盖 E-007 的命中 / 四条豁免 / 豁免注释 / 理由为空）。

门禁：`node scripts/gate.mjs` **23/23**。

***

## 2026-09-18: 插件路由地址规则规范化 + `route`/`traverse` 入口审计

**性质：规则 + 守卫 + 修漂移**。新增
[`docs/design/plugin-route-address.md`](./design/plugin-route-address.md)
（地址规则四条 + 审计结果）与守卫
[`scripts/plugin-entry-audit.mjs`](../scripts/plugin-entry-audit.mjs)（E-001 ~ E-006）。

`Plugin` trait 的 `route` / `traverse` 是全仓仅有的两条跨插件寻址通道，而地址是
**字符串**——不会因改名而编译失败，只会静默指向不存在的地方。本次把规则写清、把
规则变成可执行的守卫，并修掉审计抓到的**四处漂移**：

1. **`hook` 的幽灵命名空间**（影响面最大）。`PluginMeta::new("hooks", …)` 与目录名
   `hook`、工厂 id `PLUGIN_HOOK` 都不一致；`Plugin::meta()` 全仓无生产消费方，所以
   它不影响运行，却让 `docs/CURRENT.md` 的生成器与两处文档写出 **`hooks/fire`、
   `hooks/list`、`hooks/register`** 三条不存在的路由。而代码侧唯一在用的
   `paths::HOOK_FIRE = "hook/fire"` 一直是对的——**代码对、文档错**，长期并存。
   修复：首参改 `PLUGIN_HOOK`；`gen-current-facts.mjs` 的路由前缀改用**目录名**
   （防御）；三处文档改正。
2. **`paths.rs` 的两个幽灵常量**：`AGENT_CHAT` / `AGENT_CREATE` 全仓零调用方，
   且 `agent` 的 `route` 恒 `NotFound`——它们描述的路由不存在，是被重构淘汰后忘了删的。
3. **Telegram 的 `session/chat`**：该路径不存在（session 只认 `chat/send` / `chat/abort`），
   那处调用**必定**落到 `NotFound`。改用 `SESSION_CHAT_SEND`。
4. **`session/heartbeat.rs` 的死赋值**：`ctx.set(PATH, "chat/send")` 后走的是直连方法
   （`handle_chat_send_oneoff`），`PATH` 无人读；且值还是相对臂。

同时把调用侧散落的字面量收敛到 `symbio_core::paths` 常量
（`subagent.rs` × 3、`cli/src/client.rs` × 1、`telegram/plugin.rs` × 1），
并补齐该模块的常量与模块文档（原文档声称「所有 route path 必须在本模块定义」，
实际只有 4 个，其中 2 个还是幽灵）。

**审计还报出但本轮未动的**（属能力取舍，非地址规则）：六条「零消费方」路由
——`event_bus/pending/snapshot`（有意保留，是网关对外 API）、`event_bus/ping`、
`gateway/status`、`hook/register`、`hook/list`、`skill/execute`（与
`SkillExecuteTool` 重复实现）、`telegram/*` 六条（当时记为「休眠」——**该判断已在
下一条更正**：它不是死代码，而是对外 API，只是功能坏了）。详见 design 文档 §4。

门禁：`node scripts/gate.mjs` **23/23**（新增两项：守卫本体 + 它的回归测试）。

***

## 2026-09-18: 取消「立即心跳」按钮与 `session/heartbeat/trigger`

**性质：能力取消**（不是迁移）。会话路由表 6 → 5 条。

选项面板上的「立即心跳」按钮（`invoke` 型 `OptionNode`，`id = heartbeat_trigger`，
图标 `play`）与它唯一的后端入口 `session/heartbeat/trigger` 一并删除。

**理由：重复，不是没用。** 心跳的实质是「到点了，往会话发一轮提示词」。
用户想立刻做一次，**在输入框里直接发一条消息就是完全等价的动作**——
同一个动作、同一条路径（`chat/send`）、同一份提示词。
按钮做的也是这件事，只是绕开了对话本身。留着它等于给同一件事两个入口，
两者行为一旦分叉就是一类难以察觉的 bug。

**这不是「保留 vs 迁移」的二选一**。初版审计把它判为「不可迁 → 保留」，
那条理由只回答了「能不能迁 VDFS」，而真问题是「该不该存在」。
审计文档据此新增 **E 档（能力整体取消）** 与 §5.1 的论证。

**能力无损失**：心跳配置（`metadata.heartbeat`）、后台调度器
（`HEARTBEAT_TICK_SECS = 15`）、LLM 可见的 `heartbeat` 工具
（`set` / `get` / `cancel` 三个 action）全部未动——该工具**从来没有**「立即触发」这一格。

删除物：

- `options.rs`：`heartbeat_trigger_option` 方法、`ORDER_HEARTBEAT_TRIGGER`、
  `HEARTBEAT_TRIGGER_ENDPOINT`（`invoke` 型选项的最小样例因此在本插件消失；
  机制本身完好）。
- `plugin.rs`：`"heartbeat/trigger"` 路由臂。
- `heartbeat.rs`：`handle_heartbeat_trigger_oneoff`（35 行）+ import 收窄。
- 测试：`options.test.rs` 删 1 例、加 1 例防回归
  （`heartbeat_trigger_option_is_gone`：按钮不得被加回来 + 心跳表单仍在）。

***

## 2026-09-18: 会话旧路由审计；5 条会话路由退役（含消息增删改）

**性质：审计 + 迁移**。会话路由表 11 → 6 条。

新增审计文档
[`symbio/src/plugins/session/docs/legacy-route-migration.md`](../symbio/src/plugins/session/docs/legacy-route-migration.md)：
按「有没有真实消费方」与「VDFS 侧有没有等价能力」两条判据，把 11 条会话路由分成
**可删 / 可迁 / 保留 / 可评估**四档，并给出影响面与执行顺序。

本轮落地其中两项（纯死码，行为逐字等价）：

- **`session/append` 整条链路退役**（路由 + `invoke_append` + `session_append.rs`）。
  它早无外部消费方，唯一「实现还活着」的理由是编排器**绕路由**调它
  （`orchestrator/entry.rs` 构造 invoke 信封 → `set_payload` → `invoke_append`），
  而 `invoke_append` 的全部内容就是「解载荷 → `open_chat_session` → `append_messages`」。
  已改为**直连引擎**：信封是纯开销，且让一次数据追加看起来像一次跨插件调用。
- **`session/open` 整条链路退役**（路由 + `invoke_open` + `session_open.rs`）。
  它返回的是**进程内句柄**（`ChatSessionHandle`），而句柄交付早已改由编排器直接塞进
  `chat_ctx`（`SESSION_HANDLE`，`entry.rs`），不走路由——路由臂是改架构时留下的空壳。
  顺带修正 `cli/src/client.rs` 与 `cli/docs/architecture.md` 里「需先 `session/open`
  拿通道」的**过期注释**（CLI 实际走 `event_bus/subscribe`）。

**改判一项**：`session/get_messages` 原本列入「可迁 VDFS」（用 `vdfs/stat` 的
`message_count` 替代「读出全部消息看空不空」），实测后**判定不做**——后端没有进程内
消费 VDFS 的先例，`vdfs/*` 线路信封刻意留在 vdfs 插件内部（core 只暴露纯接口），
且 `get_vdfs_root` 只存在于每次能力收集现造的 `DefaultToolVisitor` 上。
走 VDFS 要么新增**插件 → 插件**的编译期依赖，要么新造一项 core 全局 accessor，
两者都比它换掉的东西更重。详见审计文档 §3.4。

**待做项已全部落地**（同日续做）：`chat/update_message` / `chat/delete_message` /
`chat/clear_messages` 三条**已迁 VDFS**（`write(<sid>/消息/<mid>)` /
`action(<sid>/消息/<mid>, "truncate")` / `action(<sid>/消息, "clear")`），路由表 11 → 6 条。
「LLM 能否改写消息」按**全开**决策——LLM 的访问面由 `CapabilityVisitor` 的注册处控制，
那是唯一入口，因此 provider 侧不需要再建闸门。

迁移时两处偏离原计划（都记在审计文档 §3.3 / §7）：

- **清空从 `delete` 改成 `action`**：让同一区段的删除只有一种入口形态。
  清空本可以用 `delete`（`deleted` 落在列表目录上无歧义），改走动作是为了不让使用者
  去记「哪种删除走哪个入口」。
- **补丁的 `id` 由地址补齐**：`ChatMessage::id` 是必填字段，不补的话
  `{"content":"…"}` 会在反序列化阶段被拒，「补丁是字段子集」这条承诺在 `id` 上就是假的。
  **这条是首轮回归测试抓出来的**。

验证：`scripts/gate.mjs` 全绿。测试净增（后端 +10 / 前端 +8），基线只增不减；
`docs/CURRENT.md` 已重生成。

***

## 2026-09-18: 会话级写入并入 VDFS；前端消息域建立渲染注册表

**性质：重构**。两件事，同一目标——**同一件事只有一份实现**。

### 一、会话级写入并入 VDFS

会话的**读**（清单 / 转写）与**实时变更**早已走 VDFS，但写入还留着两条专用路由。
核查后发现它们与 VDFS 侧**功能重叠**：

| 操作 | 旧入口 | 新入口 |
|---|---|---|
| 删除会话 | `session/clear` | `vdfs/delete(.vdfs/session/<id>)` |
| 改 metadata / 标题 | `session/update`（前端） | `vdfs/write(.vdfs/session/<id>)` |

`session/clear` 与 `vdfs/delete` 本就**共用** `delete_session_internal`
（代码注释自己写着前者是"旧路由"）——这不是换一种删法，是收尾没做。该路由连同
`session_clear.rs` / `session_clear.ts` 一并删除。

**两处浅合并收敛成一份代码**：新增 `Session::merge_metadata_object`，
`session/update`（仅 CLI 用）与 `VdfsProvider::write`（前端用）共用它。
此前是两份实现 + 两条注释都写着"语义相同"——**那正是漂移的许可证**：
写注释的人知道它们该一致，但没有任何机制让它们一致。分叉的后果是
"前端改名生效、CLI 改名不生效"这类只在一条路径上出现的差异。

`session/update` 路由**保留给 CLI**：它需要客户端指定会话 id
（`cli/src/client.rs` 自己 `gen_id`），而 VDFS 新建是 provider 生成 id
（id 是存储细节，不属于使用方的知识）。两种模型对"id 归谁"的答案相反，
合并必然要动 `--session <ID>` 的用户可见语义。

**消息级三条（`clear_messages` / `delete_message` / `update_message`）留在聊天协议**：
消息没有 VDFS 写路径，理由见 `symbio/src/plugins/session/docs/vdfs-session-messages.md` §5。
其中 `delete_message` 的迁移代价是丢掉 `deleted_ids` 回执（前端用它做幂等对齐），
`vdfs/delete` 只回 `{path}`——有意不做。

### 二、前端消息域建立渲染注册表

`MessageNode.vue` 从 **1549 行**（63 处条件分支、约 40 个派生布尔）拆成
**~115 行的纯分派器**：「facets → 渲染器标识 → 组件」，与资源域的
`vdfsRenderers.ts` 完全同构。新增一种消息类型的代价从"改 5 处"降到
"词表加一取值 + 映射加一行 + 装配点登记一行"。

配套：消息级业务规则（重试 / 补参 / 重试路由）抽成纯函数；
会话 store 拆出 `sessionTranscript` / `sessionLive` 两个可脱离 Pinia 单测的纯模块；
新增 `scripts/mechanism-audit.mjs`（6 条规则 + 17 例回归测试）守卫机制不变量。

### 顺带修掉的两个真实缺陷（被改造暴露，非引入）

1. **压缩节点从未被渲染**：旧模板里 `v-else-if="isTextLike"` 排在
   `v-else-if="isCompression"` 之前，而压缩节点满足前者 ⇒ `.compress-note` 是死代码。
2. **JSON 着色从未生效**：`.json-*` 类名由 TS 拼字符串产出，规则却写在
   `<style scoped>` ⇒ 编译成 `.json-key[data-v-*]`，`v-html` 注入的元素拿不到该属性。
   已移到全局样式表；生产构建产物 `grep -c 'json-key\[data-v'` = 0 可证。

另修一处既有 flaky 测试（`homedir` 的默认值测试未取串行锁，全量跑偶发失败）。

***

## 2026-09-18: 中止有独立终态——已中止的 Turn 也可重试

**性质：修复**。用户现场报告：reason 阶段点停止后该轮没有「重试」按钮，下一轮想重跑
这一轮只能手敲——体感上中止等于「这一轮结束了」，但内容是半截的。

### 根因

收口路径有三条互相竞速，**给 Turn 的终态不一致**：

| 路径 | 触发 | 终态 | 重试入口 |
| --- | --- | --- | --- |
| A | `chat_loop` 冒泡 `Err(Aborted)` → Error 帧 → `persist_failure` | `Failed` | ✅ |
| B | 循环顶边界 `AbortedAtBoundary` → `finish_turn` 收尾成 `Ok(())` → **无 Error 帧** → 消费循环 break | `Completed`（`converge_inflight`） | ❌ |
| C | `handle_abort` 3s 超时强制复位 → 消费循环提前 break，丢弃未处理 Error 帧 | `Completed`（`converge_inflight`） | ❌ |

reason 阶段中止能否重试，取决于中止信号落在哪个检查点——不合理；`Completed` 还是假话
（Turn 没跑完）。**前端的 `v-if="isFailed"` 是唯一闸门**，后端 `RetryTurn` 实际不校验状态。

### 改动

- 新终态 `MessageStatus::Aborted`——**既不是失败也没跑完**。
- `converge_inflight` 增加 `abort_terminal_of(parent_id)`：根级 Turn 一律定稿 `Aborted`
  （子节点仍是 `Completed`），其它照旧；两条路径写同一个值，竞速因此无害。
- `persist_failure` 增加 `terminal: MessageStatus` 参数；Abort 收口传 `Aborted`（不挂
  `error` 文案——用户自己按的停止，不是故障）；其它失败仍传 `Failed`。
- 前端：`MessageNode` 加 `isAborted`/`isUnsettled`，Turn 级别「重试」按钮的 `v-if` 改为
  `isUnsettled`（即 failed **或** aborted），错误条文案与图标按状态区分：
  - `aborted` → 「⏹ 已中止」
  - `failed`  → 「⚠ <error 文本>」
  - `completed` → 不渲染（无角标、无重试）

### 验证

`abort_terminal_of` 与 `converge_inflight_marks_root_turn_as_aborted` 两例后端断言均
经回退验证（把根 Turn 一律改回 `Completed` 时会红）；前端两条 `vitest`（`MessageNode`
对 `aborted` 终态的渲染、`completed` 终态无角标）同样经回退验证。

### 门禁

后端 714（+2）、前端 179（+2）；`cargo test --lib` 与 `vitest` 均绿；门禁 19/19 全过；
rustfmt 经 `gate --fix` 自动修齐，无须手工改。

## 2026-09-18: 会话 ID 改短 GUID + 压缩失败可诊断 + 连续失败熔断

**性质：修复 + 健壮性**。承接上轮的压缩体验改造，处理用户现场发现的两个问题：
会话目录名是 36 字符长 UUID；某长会话在运行中反复压缩失败且毫无诊断信息。

### 会话 ID 默认短 GUID

- `vdfs_provider.rs` 新建会话从 `Uuid::new_v4()`（36 字符带连字符）改为
  `crate::symbio_core::turn::short_id()`（8 位十六进制），与项目既有的两处短 ID 约定
  （`turn::short_id` / `vdfs_service::entry::auto_id`）一致。
- 会话 ID 同时是 VDFS 目录名（`.symbio/session/<id>`）与用户可见地址，应短而稳定。
- 加目录碰撞重试；重试仍撞则错误外抛（目录冲突属真异常，不静默吞）。
- 已确认无 UUID 格式依赖（后端无 `Uuid::parse_str`，前端只在乐观消息用 `randomUUID`
  而非会话 id），改短安全。

### 压缩失败可诊断

- `compress_snapshot_inner` 全失败出口原统一 `return None`，调用方只知"失败"不知原因。
  新增 `CompressionFailure` 枚举：`Llm(String)`（带 provider 错误文本）/ `InvalidSnapshot`
  / `NoPayoff`，由 `CompressionEmitter::finish(node, status, text, failure_kind)` 写进节点——
  `error` 装人读原因、`meta.failure_kind` 装机读码（与未执行工具节点同字段约定）。
- 实测会话 `09d74431…` 的两条 `Failed` 节点此前 `meta=None`/`error=None`，现在必然带原因。

### 压缩连续失败熔断

- 根因：`auto_compress_process` 只看当前 token 水位，不记得"上次失败"。压缩失败→上下文
  原样回滚→下轮仍超阈值→再压→再败，每轮白等一次数分钟 LLM 往返。
- `ActiveSessionStateInner` 加 `auto_compress_failures` + `auto_compress_circuit_opened_at`，
  三个 `async` 辅助方法集中维护：连续失败达 `COMPRESS_CIRCUIT_LIMIT`(3) 开闸，冷却
  `COMPRESS_CIRCUIT_COOLDOWN`(5min) 内跳过 LLM 压缩（不回滚、不阻塞回复），冷却结束半开
  重试一次；冷却期内的失败不刷新开闸时刻（避免永不重试）。
- **压缩失败不得阻断主回复**：从"让整轮 `TurnExit::Failed`"改为"告警 + 继续"。压缩是优化
  项、历史已回滚，最坏代价是"本轮用更长上下文跑"，不该让用户消息也拿不到回复。

### 门禁

后端 712（+8：短 ID ×2 + 熔断状态机 ×3 + 失败诊断 ×1 + flatten 跳过… 实际相抵后净 +8）、
前端 177；`cargo test --lib` 与 `vitest` 均绿；门禁 19/19（设 `CARGO_TERM_COLOR=never` 规避
cargo 给 `test result:` 套 ANSI 色码导致门禁解析失败的无关问题）。

## 2026-09-18: 压缩成为会话流中的普通节点 + 压缩请求改为"像历史对话一样发"

**性质：体验改造 + 后端算法**。用户两条反馈：

1. 压缩应是会话中间的一个节点，而不是会话窗口顶部的横幅；
2. 后端把整个历史 `Vec<ChatMessage>` 序列化成 JSON 塞进一条 user 消息发给大模型，
   里面带着 `id`/`parent_id`/`seq`/`status`/`meta`/`timestamp` 等模型完全不关心的字段，
   既浪费 token 又冗余。

### 请求形态：从 JSON 转储改为正常对话

- `build_compression_request` 不再 `serde_json::to_string(history)`。改为：**历史消息原样
  逐条透传** + 末尾追加一条纯**指令**消息（`compression_instruction`）。
- provider 层的 `flatten_chat_messages` 会把这串消息投影成模型该看的对话形态
  （Turn→assistant 聚合、ToolCall→tool_calls、结果→role=tool），与一次普通对话**同源**。
  压缩请求因此天然省掉所有存储字段。
- hints 从"嵌进数据 JSON"改为"拼进指令消息"——指令里只描述要 distill 成什么结构，
  不再内嵌 `<state_snapshot>` 标签模板（模板仍只走 system role，这条防线保留）。
- 重试路径顺势变成自然的多轮（历史 → 指令 → 纠正），比原先"再拼一个 JSON"更稳。
- `pending_tokens` 估算口径修正：旧的把**不发给模型**的 `keep_messages` 也算进去了，
  现已只统计真正下发的消息。

### 压缩节点：进会话流

- 新增消息类型 `MessageType::Compression`（`chat_message.rs`，`"compression"`）。
- 自动压缩触发时，在**当前时刻**（user 消息之后）插入一条 `Streaming` 的压缩节点，
  走 `emit_message_patch`——它走 `state.inner.frontends` + VDFS 变更订阅，**不经过**被静音
  的 turn channel，所以压缩窗口内前端能看见这个节点。
- 压缩完成 → 节点定稿为 `Completed`（meta 记 `compressed_count`，渲染"已压缩 N 条历史消息"）；
  LLM 失败 → `Failed`；用户中止 → `Completed`（带 `failure_kind=aborted`，与 S20.3 同一口径，
  不挂 error 文案）。
- 节点同时进**在途缓冲**，切走再切回仍可见（复用 S20.3 的叠加机制）。
- 清除机制：同 S20.3 的 `CompressionEmitter`（原 `PhaseEmitter`），生命周期收在包装层，
  覆盖成功/失败回滚/校验失败/紧急兜底/无收益/中止全部出口；且幂等——重复调用不产生重复节点。

### 移除：会话级 `phase` 机制

上一轮（`e00dc9c`）做的"会话节点 `attributes.phase` + 顶部横幅"被本改造取代——既然压缩
现在有了自己的消息节点，横幅就不再有消费者，整条链路（后端 `ActiveSessionStateInner.phase`
/ `SessionRuntime.phase` / `PHASE_COMPRESSING`、前端 `VDFS_PHASE_COMPRESSING` / store `phase`
/ `ModelChatPanel` 横幅）一并删除，避免死代码触发 `dead-code-audit` 门禁。

### 压缩节点不进请求包

`flatten_chat_messages` 跳过 `Compression` 类型——它是过程记录，不是对话内容，不应再占上下文。

验证：后端 704、前端 177（含 `compression_request` 与 `flatten_chat_messages` 压缩节点跳过两条
新测试，均经回退验证确认会红）；门禁 19/19。

## 2026-09-18: 修复压缩后消息顺序倒挂（用户消息插进更早的历史）

**性质：修复**。用户报"会话中的前端消息显示顺序不对，用户消息显示到了最新消息位置"，
并提示"好像是会话压缩的时候"。按图索骥，找到了一处**可复现**的顺序倒挂。

**根因**：`assign_seq` 对已带 `seq` 的消息一律**保持原值**，只有缺号的才拿 `base+1`
（`base` = 压缩前的水位）。而压缩后的新数组是 `[快照, 保留区…]`：

- 快照是**新建**的（无 seq）⇒ 拿到 `base+1`，成了**最大**序号；
- 保留区沿用**旧的低**序号，原样不动。

于是 `ordered()`（按 `seq` 升序）把**快照排到最后**：压缩后的历史记忆跑到整段转写的
末尾，而本该在末尾的最近几轮（含当前用户消息）反而排到了前面——正是"用户消息插进
更早的历史"的观感。

**改法**：`seq` 必须沿数组**单调不减**。调用方 `replace_messages` 的契约本就是
「数组顺序即权威顺序」，因此 `seq` 与数组顺序不一致时它是**错的顺序**，不是"值得
保留的历史值"。已带序号的消息也要检查：一旦破坏单调性就按数组顺序重新发号。

**正常路径不受影响**：数组顺序本就等于 `seq` 顺序时既有序号原样保留（不会每次落库
都重排历史）——这条由反面测试 `assign_seq_leaves_already_ordered_seqs_untouched` 锁定。

**验证**：先写测试确认它能红（快照 101 而保留区仍是 95），修复后转绿；后端 704（+2）。

***

## 2026-09-18: 压缩期不再"毫无反应"——会话节点新增处理阶段 `phase`

**性质：体验修复 + 一处机制新增**。长会话触发上下文压缩时，前端此前**没有任何等待
提示**，用户视角是"点了发送、界面毫无反应"。

**根因**：自动压缩（`apply_compaction`）跑在每轮 Turn **创建之前**，而它是一次把整段
历史塞进请求体的完整 LLM 请求——长上下文时可达数分钟。这段时间里后端刻意静音了全部
出帧（`send_compression_request` 用哑 `tx` 接住压缩 delta），以免泄漏一个永不 finalize
的空 Turn 骨架；后果是**没有任何消息节点**可供前端渲染，而前端对 `compress` 零引用。

**关键事实**：后端其实**发得出去**——`notify_session_state` 走 `state.inner.frontends` 与
VDFS 变更订阅，**不经过**被静音的那条 turn channel。所以不是"发不出"，是"从没发过"。

**改法**：压缩是**会话级**状态却不是消息节点，因此挂到会话节点的 `attributes.phase`
（`is_working` 回答"忙不忙"，`phase` 回答"在忙什么"）：

- 新增 `PhaseEmitter`：写 `inner.phase` **并**立即下发会话节点变更（只改字段不通知
  等于没改——压缩窗口内没有任何其它变更会把它带下去）；
- 清位收在**包装层**：压缩内核有六个出口（成功 / LLM 失败回滚 / 快照校验失败 /
  输入超限紧急兜底 / 兜底亦无收益 / 用户中止），散落在每个 `return` 前的写法漏一个
  就会让会话空闲后仍挂着"正在压缩"，且无人纠正；
- `phase` 只在**运行中**成立，非运行分支一律丢弃（后端 `from_state` + 前端
  `applySessionNode` + 组件 `isWorkingStatus` 三道同规则防线）；
- 前端在主聊天区给出**会话级压缩提示条**（复用 `session-error-banner` 版式）+
  已用秒数（复用共享的 `useRunningClock`）。

**验证**：后端 702（+2）、前端 181（+4）；门禁 19/19。

***

## 2026-09-18: 会话消息节点前后端一致性——在途消息可见、中止后节点不再停在「运行中」

**性质：修复**。用户点名两个症状，排查后确认**同源**：「存储里的权威副本」与「正在跑
但还没落库的那一份」不一致。

### 一、在途消息可见性

**根因**：assistant 侧节点**每轮结束**才落库（`chat_loop::persist_messages`）；而会话叶子
`read`（`.vdfs/session/<id>`）只序列化存储中的消息。转写列表（`transcript_of`）早已有
`overlay_live` 叠加在途，**叶子没有** ⇒ 同一份数据的两个地址给出不同内容。

前端 `loadMessages` 走的正是叶子（`readVdfs(vdfsSessionAddr(id))`），
`session/get_messages` 已不是前端读入口 ⇒ 流式期间读叶子会少掉整个正在跑的那一轮；
切走再切回，在途消息凭空消失（直到每轮结束落库才回来）。

**改法**：`session_content` 叠加 `live`，与 `transcript_of` 共用 `overlay_live`（同 id 在途
版本胜出，但 `seq` 从落库版本继承——顺序锚点只由存储分配）；子会话叠加**它自己**的在途
缓冲而非父会话的；前端 `hydrateFromHistory` 由**整表替换**改**合并**语义——整表替换是
经典 lost update：一次 IPC 往返期间到达的流式补丁会被覆盖，且它们不会被重发。

### 二、中止后节点停在「运行中」

**缺口 A**：`tool_executor.rs` 的 `PluginPayload::Data(_)` 分支（**非流式**——绝大多数工具
走这条：`read_file` / `web_search` / MCP / `codebase_search`）是一次 `await`，只被 600s 硬
超时包着，**不轮询通道也不看 `abort_flag`**。`handle_abort` 的 3s 兜底一旦触发就强制
`is_working = false`，消费循环下一帧因 `!is_working` **直接 `break`，跳过
`persist_failure`** ⇒ 工具节点永远停在 `Streaming`。这正是"终止后还显示运行中"。

**缺口 B**：`resume` 重跑工具时父 ToolCall 被置 `Streaming` **并已落库**——它不是流式节点，
没有任何后续机制会再碰它；中止时原代码直接 `return Ok(Done)`。

**改法**：
- 新增 `wait_tool_abort`：`route_fut` 与它 `select!`，让非流式工具也能被中止（abort 帧 /
  取消令牌 / 已置位三条来源）。**忽略通道关闭**——对端消失不是中止信号，否则每次正常
  收尾都会被报成"用户中止"（提示音选错音色）。这条曾写错，被回退测试抓到。
- 新增 `converge_inflight`：扫**存储 + 在途缓冲**两个数据源，非终态定稿 `Completed` 并广播
  补丁；`handle_abort` 复位 `is_working` 后**自己**调用（不等 chat_loop），且**幂等**。
- `resume` 中止时父节点用 `not_executed_patch(.., "aborted")` 就地定稿并广播。
- 消费循环出口记 `exit_state`：中止出口发 `aborted`（此前一律 `completed`，与
  `handle_abort` 抢同一个字段，收敛只靠 3s 轮询的时序侥幸）。

**在途集合的唯一定义**：`None` / `Pending` / `Streaming`，**不含 `WaitingUserAction`**——
它不是"正在跑"而是"等用户回答"，中止时抹掉它等于把审批入口删了。该集合由测试直接锁定，
不靠读代码：漏掉任何一个，表现就是一个**永远转下去**的「运行中」，而它不报错、只会一直转。

### 三、验证

后端 **700** 个测试通过（新增 13），前端 **177** 个通过（新增 5）。新增回归覆盖：在途集合、
存储与在途两个数据源、幂等、`wait_tool_abort` 四个出口、中止与完成结局可区分、会话叶子
含在途、resume 父节点定稿、前端水合合并语义。

***

## 2026-09-18: `codebase_search` 从"永远超时"变成可用——嵌入后端换 `ort`，索引落盘并按 mtime 增量重建

**性质：修复**（含一处架构决策推翻）。两件事都不改工具对外的用法，改的是它**能不能用**。

### 一、嵌入推理后端：`tract-onnx` → `ort`（ADR-016）

**症状**：前端一调 `codebase_search` 就卡住，永远"回复中"，没有结果。

**根因不是工具逻辑，是嵌入推理慢到不可能完成**。2026-09-18 实测（本机，release）：

| 输入 | token | tract | ort |
|---|---|---|---|
| 短查询 | 13 | 386 ms | 2.4 ms |
| 一个 40 行块 | ~600 → 截 512 | **9.83 s** | **19 ms** |
| 超长（截到上限） | 512 | 9.80 s | 13 ms |

即 **每 token ≈ 22.5 ms、纯线性**，release 只比 debug 快 16% ⇒ 结构性慢。
本仓 500 个文件 / 6,681 块 ⇒ 全量建索引 **≈ 6 小时**，而工具执行有
`TOOL_EXEC_HARD_TIMEOUT_SECS = 600` 的硬超时。**ADR-014 写的"离线索引 + 单条查询，
秒级可接受"对查询成立、对建索引不成立**，该前提作废。

**改法**：推理后端换成 `ort`（ONNX Runtime），其余一律不动——模型仍是内嵌的 int8
`bge-small-zh-v1.5`，分词仍是 `tokenizers` + `fancy-regex`，后处理仍是
`encode(text, true)` + CLS 池化 + L2 归一化。**不恢复 `fastembed`**：它的 C 编译链主犯
是硬编码的 `tokenizers/onig`，不是 ORT；直接依赖 `ort` 即可拿到同样速度。

**代价（如实记录，不粉饰）**：`ort` 在 Windows x64 拿到的预编译是 **DirectML flavour**
（`ort-sys` 的 `BinariesSource::Pyke`，源码注释写明 "pyke libs always ship compiled with
DirectML on Windows"，用 features 关不掉）⇒ 构建缓存 341 MB `onnxruntime.lib` + 18 MB
`DirectML.dll`；exe **静态导入 `DirectML.dll`**（PE 导入表核对，非延迟加载）。
好在 Win10 1903+ / Win11 由系统提供该 DLL，实测删掉随包那份仍能推理。
**"零 C/C++ 编译"仍然成立**：`cargo tree -i cc / -i ring / -i onig_sys` 三者皆空。

**数值一致性**：tract 与 ort 跑同一批文本，最低余弦 **0.996661**（两引擎自一致性均
≥0.999999，故是引擎差异不是噪声）。换引擎不换语义。

### 二、索引时效性：此前**零机制**，现在落盘 + 按 mtime 增量

**症状**：索引是"首次调用那一刻的快照"。`INDEX_CACHE` 是纯进程内 `HashMap`，
全文件搜 `mtime` / `modified` / `hash` / `watch` / `invalidate` / `stale` **命中 0 次**；
唯一绕过缓存的 `rebuild` 是 LLM 传参、默认 false。**比慢更糟——是静默地错**：
agent 会拿一份不包含自己刚写的代码的索引去搜，且完全无从察觉。

**改法**（`symbio/src/plugins/local/codebase_search.rs`）：

- **每次调用都扫一遍文件指纹**（`stat` 全部候选文件，500 个 ≈ 毫秒级），
  文件集（相对路径 + `mtime` + 字节数）完全一致就直接复用，**一次嵌入都不做**。
- **有差异只重嵌改动过的文件**：未变文件的块原样搬过来，新增/改动重新分块嵌入，
  已删除整条丢掉。
- **索引落盘**到 `{workdir}/.symbio/cache/codebase-index.bin`（与 `agent` 的工作区级
  落位同源；`.symbio` 是隐藏目录、`.bin` 不在扩展名白名单，两重保险确保索引不会
  把自己索引进去）。手写小端二进制格式，**临时文件 + `rename`** 原子替换。
- 返回体新增 `index` 字段（`files` / `chunks` / `reused_files` / `embedded_files`），
  让"索引有没有跟上磁盘"可观测，而不是靠猜——`embedded_files == 0` 即索引已最新。

**实测**（本仓，release）：冷建 59 s（一次性、落盘）→ 冷启动读盘 + 增量 **9.9 ms**
→ 改 1 个文件 **434 ms**。

**已知边界**：指纹用 `mtime` + 字节数，**刻意保留 `mtime` 的写入（如 `rsync -t`）
检测不到**——这是 mtime 型增量的固有取舍，`rebuild=true` 是兜底。

### 三、顺带：生成物不再进索引

`tokenizer.json`（21,277 行）与 `package-lock.json`（7,249 行）两个文件就占掉全库
8,104 块里的 1,426 块（≈18%），且会**挤占 top-k**（词表里全是短 token，对任何查询
都有中等相似度）。按名字排除锁文件/压缩产物 + 按 5,000 行上限排除生成的数据文件
⇒ 块数 8,104 → **6,681**，冷建 69 s → **59 s**，索引 29.5 MB → **25.2 MB**。

***

## 2026-09-18: 删除的两种语义分开——级联截断不再逐条通知（S20.2）

**性质：修复**。修掉一个一直存在的**语义混淆**，顺带把「删除一条早期消息要发上百条
变更」这件事消除。

### 为什么

`deleted` 这一个值此前同时被用来表达两件不同的事：

| 场景 | 真实语义 | 被删数量 |
|---|---|---|
| 工具调用恢复（`resume` 的 `Delete` 帧） | **这一个**节点没了，后面的留着 | 1–N（一棵子树） |
| 删除某条消息（`chat/delete_message`） | 从这里**到列表末尾**全没了 | 可达上百 |

后果有两个，都不轻：

1. **消费者无从分辨**。前端收到的每一条都长得一样，只能靠外部知识去猜是哪一种。
   猜错的代价是实打实的：按级联处理 → 重试一轮会把后面的会话全删掉；
   按单条处理 → 删一条消息后列表尾部残留一堆后端已不存在的节点。
2. **代价与历史长度线性相关**。删一条早期消息要发 N 条变更，而 VDFS 与流式视图
   互不校验——漏一条就永久不一致。

### 变更

- **`symbio_core/vdfs_provider.rs`**：新增 `VDFS_CHANGE_TRUNCATED`——`path` 所指节点
  **及其之后的全部兄弟**已移除。与 `deleted` 的分界写在常量文档里。
  没有选择「在 `deleted` 上挂 `cascade: bool`」：那会让「是哪种删除」变成两个字段
  必须一起读才正确，正是本仓库反复否决的「状态 + 平行标志位」。
- **`session/plugin.rs`**：`emit_message_deleted`（逐条）删除，改为
  `emit_transcript_truncated`（一条）。逐节点删除的唯一来源收敛到 `resume` 的
  `StreamEvent::Delete`（由消费循环直接转译）。
- **`session/handlers.rs`**：`invoke_delete_message` 改发**一条** `truncated`；
  目标消息不存在时**一条变更都不发**（否则消费者会从一条并不存在的节点起截断，
  把整个列表清空）。
- **前端 `stores/sessions.ts`**：新增 `removeFrom(sessionId, messageId)`——按 `seq`
  取「该节点及其后」。判据与后端 `messages.drain(i..)` **同源**（两侧都按
  `seq` 升序），因此是同一个集合；按**排序后的位置**切片而不是比较 `seq` 大小，
  这样缺 `seq` 的旧数据也正确。
  `deleteMessage` 改为**本地先行级联**：UI 立即收敛，不等后端把被删的每一条逐个
  通知回来；随后用权威 `deleted_ids` 幂等对齐；写后端失败则**回滚**本地改动
  （保持「没落库就不显示已删除」）。`removeMessageById` 的存在性检查提到对象展开
  之前——截断会引发一串针对已删节点的冗余通知，每次白拷贝两份对象太亏。
- **前端 `vdfsTranscriptSync.ts`**：`truncated` → `removeFrom`；`deleted` 仍只删一项。

### 验证

- 后端 +3：级联只发一条 `truncated`（不是 N 条 `deleted`）、删末尾一条走同一语义、
  目标不存在时零变更。
- 前端 +8：级联范围（从用户消息 / 从 Turn / 删末尾）、锚点缺失不截断、
  缺 `seq` 的旧数据按位置截断、`deleteMessage` 的权威对齐与失败回滚、
  `truncated` 与 `deleted` 互不污染（后者是工具恢复的回归护栏）。
- 门禁 21/21。

***

## 2026-09-18: 工具调用的「运行中」跨越执行窗口——标签 + 动效 + 已运行时长

**性质：修复（S20.1）**。修掉 S20 交付时留下的一个**显示层缺口**。

### 为什么

S20 把**会话**运行态搬到了节点上，但**工具调用**的运行态断在执行窗口之外：
`finalize_assistant_turn` 在 LLM 流结束时就把 ToolCall 标成 `Completed`。
而那一刻只是"参数齐了"，工具**还没开始跑**。

于是用户看到的是：参数流式期间有「调用中…」标签 → 流一结束标签消失 → 之后整段执行
（一次编译、一次网络请求、一个子智能体跑完，往往是最长的一段）**画面静止、没有任何
运行迹象** → 结果突然出现。窗口越长，这段静默越长，用户无法判断是"还在跑"还是
"卡死了"。会话级活动文字（`正在调用 <name>…`）同样被那条 `Completed` 提前清空。

### 变更

- **`finalize_assistant_turn` 不再定格 ToolCall**：参数齐 ≠ 调用结束。终态只由执行方给出。
- **执行前广播运行中**：`tool_executor::process_tool_calls_async` 在 `execute_tool_async`
  之前置 `Streaming` + `meta.started_at`；`resume` 的 approve / retry / supply
  （三个真正重跑工具的 action）同样处理。
- **"每个 ToolCall 必然到达终态"成为不变量**：被 PreToolUse 拦下 / 用户中止 /
  交互中断而**未执行**的批尾，在函数末尾统一收口为 `Completed` +
  `meta.failure_kind = "not_executed"`（不标 `Failed`：未轮到执行不是错误，标 `Failed`
  会渲染 ⚠ 并让 `get_context_messages` 过滤掉父节点、留下孤儿结果子节点）。
- **前端状态标签覆盖全部非终态**：`运行中`（主色 + 三点脉动 + 已运行时长）/
  `待确认`（`.warn`，此前是**死代码**）/ `失败`；终态刻意不给标签——绝大多数调用都会
  成功结束，给每个成功的调用挂「已完成」只会把真正需要注意的状态淹掉。
- **标题呼吸动效**扩展到工具执行中（与「思考中…」同一手法）。
- **已运行时长**（`运行中 · 47s`，超过 60s 用 `2m05s`）：锚点取后端写入的
  `meta.started_at`，**前端不自造锚点**——用挂载时刻当锚点，切会话/重连后会把
  "已经跑了 3 分钟"显示成"刚刚开始"，恰好丢掉用户最需要的那条信息；
  锚点缺失（旧数据）就不显示时长，宁可不说也不给一个没有依据的数。
- **新增 `composables/useRunningClock.ts`**：全应用共享的秒级时钟，引用计数归零即停表。
  `MessageNode` 是递归组件，每实例一个 `setInterval` 会让定时器数量随会话长度线性增长。

### 不做

- **不新增状态词**：`streaming` 同时覆盖「参数流式」与「执行」两段——对用户而言
  "这次调用还没结束"是同一件事；新增词会牵动 `status-*` CSS 类名（漏改不报错、
  不失败，只会让动画静默消失）。
- **不自动展开运行中的工具调用**：会与用户的手动折叠选择打架；折叠态下的
  标签 + 动效点 + 呼吸标题已是足够信号。

### 文档

- [`node-state-streaming.md`](../symbio/src/plugins/session/docs/node-state-streaming.md)
  新增 §5.3.1（`streaming` 覆盖两段的原因、终态来源表、`meta.started_at` 的取舍）、
  S20.1 迁移表，不变量增至 12 条，验收清单 #3 由「不变」升级为「更强」。

### 门禁

- 前端 +4 用例（运行中标签与动效点、时长两种格式、锚点缺失不显示时长、
  `待确认` 与终态不给标签）；`vitestTests` 基线 160 → 164。

***

## 2026-09-18: 流模式改为「节点状态」驱动——会话运行态上节点，前端不再消费事件序列


**性质：架构调整（S20）**。用户可见行为等价，但**正确性不再依赖事件到达顺序**。

### 为什么

会话的实时显示原先基于**事件流**（`kind = "session"` 的 `Status{busy|idle}` / `Abort` /
`Error`），前端 `switch (event.type)` 逐类处理。代价是正确性依赖顺序，而顺序不是免费
保证的——最直白的证据就是 `eventBus.ts` 里那段「切会话防乱序」的 `replayBuffer`：
**一段只为修顺序而存在的机制，说明模型本身选错了**。

### 变更

- **会话运行态成为会话节点的属性**：`status`（`working` / `active` / `failed`）
  + `attributes.outcome`（`completed` / `aborted` / `failed`）+ `attributes.error`。
  状态类变更（`updated`）**必带全量节点视图**，前端**零回读**。
- **`failed` 是独立状态**，取代「`active` + `last_failed` 布尔」：判据从两处变一处。
- **修掉一处有损映射**：`completed` 与「未标注」曾被后端都映射成 `active`，消费端必须
  把 `active` **猜回** `completed`；现在消息状态原样透传（`active` 仅作旧数据别名）。
- **前端按地址分派**：`sessionRouteOf(地址)`（纯函数），**没有 `switch (event.type)`**。
- **删除的顺序机制**：`sessionBusWatcher.ts`（整模块）、`eventBus.replayBuffer`、
  `eventBus.fetchPendingSnapshot`、`MainLayout` 的 `startSessionBusWatcher()` 接线。
- **错误条改由节点表派生**：有失败节点则隐藏（原先在事件到达时判定，隐含"那一刻恰好
  能看到失败节点"，两条通道先后无法保证）。
- **提示音改由状态迁移触发**：`working → 非 working` 的迁移 + `outcome` 选音色，
  不再靠"谁先到"区分中止与失败。
- **`kind = "session"` 帧保留**：进程内消费者（子会话审批透传、以 `Status idle` 判定
  子会话结束）仍依赖它，前端不再订阅。

### 不做

- **不合并 `streaming` / `working`**：会连带改 `status-*` CSS 类名，而漏改不报错、
  不失败，只会让流式动画静默消失。收益（少记一个词）远小于风险。
- **工具调用请求不另立地址**（S21）：`read(.../消息/<tc-id>)` 的正文就是请求体，
  子节点就是响应——另立地址会让同一份参数存两处。

### 文档

- 新增 [`node-state-streaming.md`](../symbio/src/plugins/session/docs/node-state-streaming.md)：
  节点分类与状态机、传输契约、顺序无关性（含三条残留假设）、前端消费模型、
  11 项「体验等价清单」、10 条不变量。
- [ADR-015](./DECISIONS.md) 记录决策与后果；[DATA_FLOW.md](./architecture/DATA_FLOW.md)
  链路二 #6 由「流式帧推送」改写为「前端显示由节点状态驱动」。

### 门禁

- 后端 +6 单测（`SessionRuntime` 投影 / `session_node` / `session_change` /
  `MessageStatus` 词表与 serde 一致）；前端 +4（`sessionRouteOf` 分派、节点载荷零回读、
  状态迁移驱动提示音、`failed` 作为独立状态）。

***

## 2026-09-18: 系统智能体自身的 `AGENTS.md` 入口从 agent 列表移到设置页

**性质：前端可见行为调整**。地址不变（`.vdfs/agent/AGENTS.md` 照旧可达、读写不变），
只是**不再出现在 agent 挂载根的列表里**。

### 变更
- **挂载根 = 装进来的智能体清单**。此前 `.vdfs/agent/AGENTS.md` 排在列表**最前**，
  用「排最前」来暗示它不是一个包——但列表本身没有语义，读者仍会把它读成某个 agent 目录。
  现在它整条移出列表，挂载根与 session / model 列表同口径：**只有装进来的东西**。
- **入口归设置页**：`AgentPlugin::traverse` 经既有 `ConfigurableVisitor` 通道补一条
  节点（`instruction_node()` 改 `path` 后注册），复用挂载根里那份指令节点，
  不改 `symbio_core`、不新增通道。
- `instruction_node()` 由 `async fn` 私有改为 `pub(crate) async fn`（`list` / `stat` /
  `traverse` 三处共用同一份形状）。

### 门禁
- 新增 `mount_root_lists_only_installed_agents`：断言挂载根只列 agent 目录，
  防止「排最前」这种隐式约定日后被重新引入。

***

## 2026-09-18: 修复本地嵌入模型 tract 加载失败，恢复 `codebase_search` 工具

**性质：修复 + 工具面恢复**。模型文件本身未变，仍是 24 MB 的 int8 `model.onnx`。

### 修复：嵌入服务不再静默降级为 Noop
启动日志曾报 `Failed analyse for node #203 "/Unsqueeze" AddDims`，
`LocalEmbeddingService` 初始化失败后回退 `NoopEmbeddingService`，语义搜索被禁用。

模型是完好的（`onnx.load()` 通过，opset 11 / 527 节点），问题在 tract 侧配置，
两条约束由此确立（已写入 `local.rs` 注释与 ADR-014 修订小节）：

- 必须 `.with_ignore_value_info(true)`：该模型是 ONNX Runtime 动态量化导出，图里带
  `value_info`，把中间张量声明成 `batch_size`/`sequence_length` 符号；tract 拿输入 fact
  的 `1` 与之 unify 时报 `Impossible to unify Sym(batch_size) with Val(1)`。
  这也是「固定 seq=512」兜底无效的原因（照报同一个错）。
- 动态长度路径不要自建 `SymbolScope` + `set_input_fact`：tract 0.23.7 会触发
  `ProofCacheSession scope_id mismatch` 断言（panic）。不覆盖输入 fact 即可。

顺带修掉一个静默失效坑：`outlet_label` 对图输入返回**空串**，输入名改取
`model.node(outlet.node).name`，否则三个输入全部落进「未知输入名」分支而返回 `None`。

**精度实证**（同一句「你好，世界」，7 token，CLS + L2 归一化）：

| 路径 | vs ONNX Runtime 余弦相似度 |
|---|---|
| 动态长度（默认） | **0.999961** |
| 固定 seq=512（兜底） | 0.994482 |

兜底路径精度下降是因为补位改变了 `DynamicQuantizeLinear` 的 per-tensor scale，
故固定长度只作兜底，不作默认。

### 恢复：`codebase_search` 重新挂回工具清单
该工具在 `30ef62c`（"temporarily-disable-broken-codebase-search"）被摘出
`tool_impls`、降级为未使用的局部变量。现随嵌入服务修复一并恢复，
并加了一条断言工具清单的回归测试，避免再次被静默摘掉。

### 门禁
- `BASELINE.rustTests` 657 → 660（新增 3 个测试）。

### 实证
- `cargo +1.93.1 test --lib` **660/660 通过**。
- `cargo clippy -p symbio --lib`：1 warning，位于 `plugins::model::message_builder`（既有，未新增）。

***

## 2026-09-17: 质量收敛——迁移与持久化修复、门禁收紧、吞错清理

**性质：修复 + 门禁收紧**。协议、工具面、前端行为均无变化。

### 修复
- **model 插件迁移**：重写迁移逻辑——已可解析的目标文件为权威（不再被迁移覆写）、
  写后读回校验、失败保留源文件；legacy 清理改为整批确认后才执行；
  迁移源改为读盘上 manifest（不再依赖运行时注册表）；`persist()` 失败向上传播
  （含 `after_uploaded`，不再 `let _ =` 吞错）。
- **home 插件**：`set_workspace` 改为候选内存先落盘、成功后才发布（写盘失败保留
  旧工作区，内存与磁盘不再分叉）；`flush` 从读锁改写锁，消除固定临时文件名并发
  覆盖竞态；移除 set 后的后台 flush spawn。
- **mcp stdio 握手**：`notifications/initialized` 的 stdin 写入/flush 失败向上传播，
  缺 stdin 报错（不再静默）。
- **session 压缩**：删除未经验证 prose 兜底成快照的路径——快照校验重试后仍无效时
  保留原历史并返回 `None`（输入溢出死锁的本地紧急尾压缩仍作最后兜底）。

### 门禁与审计
- `gate.mjs`：vitest 判定收紧——超时即 SIGKILL、`ok` 必须退出码 0 且无信号，
  删除「退出码非 0 但用例数达标」的假失败放行；后端测试同样禁止通过数覆盖失败退出码；
  `BASELINE.rustTests` 638 → 657。
- `grep-audit.mjs`：S-002 默认扫描全部插件（原仅 `plugins/agent`）；新增行级豁免
  `// grep-audit-allow S-002: 理由`（`vdfs/fs.rs:509` 已豁免一处误报）；
  新增 6 个回归测试（`scripts/grep-audit.test.mjs`，接入 gate docs 阶段）。

### 实证
- `cargo +1.93.1 test --lib` 657/657；`cargo +1.93.1 clippy -D warnings`（symbio + cli）绿；
  `node scripts/gate.mjs` 19/19（锁定工具链 1.93.1）。

***

## 2026-09-17: 本地嵌入切到 `tract-onnx`（`fastembed` 废弃，最后一条 C 编译链 `onig_sys` 退出）

**性质：依赖树收敛 + 实现替换**。协议、工具面、前端行为**均无变化**；取舍与实证见 ADR-014。

### 改动
- `symbio/Cargo.toml`：移除 `fastembed`；新增 `tract-onnx = "0.23.5"`（实际解析 0.23.7）与
  `tokenizers = { version = "0.22.2", default-features = false, features = ["fancy-regex"] }`。
  **不用伞包 `tract`**（多带 6 个 tract-* crate 及其依赖）。
- 新增 `providers/embedding/local.rs`：tract 纯 Rust ONNX 推理（符号化 seq 维一次编译、
  CLS 池化 + L2 归一化、`spawn_blocking`），删除 `fastembed.rs`；
  嵌入服务注册 id `"fastembed"` → `"local"`（`EMBEDDING_LOCAL`，消费方仅 `codebase_search`）。
- 删除 fastembed 专用侧车文件 `config.json` / `special_tokens_map.json` / `tokenizer_config.json`
  （tokenizers 只需 `tokenizer.json`）。

### 实证（不靠推断）
- `cargo tree -i fastembed / ort-sys / onig` → 全部 **"did not match any packages"**；
  `ureq` 下载链（ort-sys 的 build-dep）与 `winapi` 族一并退出，共 19 包；
  lock `[[package]]` 357 → 398（tract 11 件套及其纯 Rust 依赖进入）。
- **~391 MB 的 ONNX Runtime 预编译开销消失**（Windows x64 `directml` flavour：341 MB `.lib`
  + 18 MB DLL）；Windows/macOS 构建 **C/C++ 编译归零**。
- **二进制体积 A/B 实测**（同 release profile、同 rust-lld，worktree 隔离重建 2f9c04a 作基线）：
  `symbio-cli.exe` 59.8 MiB（fastembed+ort）→ 61.2 MiB（tract），**+1.4 MiB（+2.3%）**——
  exe 基本持平略增，收益在构建链与交付面（零 C 编译、326 MiB 预编译缓存可删、
  不再需要 DirectML.dll），见 ADR-014。
- 数值对齐：tract vs fastembed 同模型同分词器，**余弦相似度 0.999558**（int8 计算序差异，
  语义等价）；输出 L2 归一化对齐（fastembed 实测范数恒为 1.0）。
- `cargo check --lib --tests --locked` / `clippy -D warnings` / `fmt --check` 全 0；
  `cli` 亦 `cargo check --locked` 通过；`node scripts/gate.mjs --only=docs,facts` 8/8、
  `--only=msrv` 2/2（1.91.0 实编译，tract 0.23.7 MSRV 与本仓持平）。

***

## 2026-09-17: TLS 后端切到平台原生栈（`aws-lc-sys` / `rustls` 族退出依赖树，`cmake` 消失）

**性质：依赖树收敛**。协议、LLM 工具面、前端行为**均无变化**；依赖树净减 **27 个包**
（`[[package]]` 386 → 357，唯一包名 357 → 330；新增仅 2 个）。

### 起因：只剩两条路，取舍见 ADR-013
`reqwest` 的 TLS 原走 rustls，而 rustls 的两个官方密码学后端**都是 C**：默认 `aws-lc-rs`、备选 `ring`。
`aws-lc-sys` 是整份 BoringSSL（414 `.c` + 178 `.cc` + 941 汇编 / 145,513 行 C / 69 MB），
且**无条件**依赖 `cmake`。纯 Rust 的 `rustls-rustcrypto` 上游标注 DO NOT USE IN PRODUCTION
⇒ 只剩「换 provider（`ring`）」与「换掉 rustls（OS 原生栈）」两条路。选后者，唯一理由是它能把
目标平台上的 **C 编译真正降到零**（Windows = SChannel、macOS = Security.framework，皆纯 Rust FFI），
`ring` 路线只是把 69 MB 的 C 换成 8.3 MB 的 C。

### 改动：一处依赖声明
`symbio/Cargo.toml` 的 `reqwest` 由隐式 default 改为显式 feature 集：

```toml
reqwest = { version = "0.13.4", default-features = false, features = [
    "json", "stream", "charset", "http2", "system-proxy", "native-tls",
] }
```

`default-features = false` 会关掉 `default` 里的 `default-tls`(=rustls) **以及**三个非 TLS 能力，
故 `charset` / `http2` / `system-proxy` 必须显式补回——否则响应体编码判定、HTTP/2、
系统代理读取会**静默**失效。

### 实证（不靠推断）
- `cargo tree -i aws-lc-sys --target all`（及 `-i aws-lc-rs`、`-i rustls-platform-verifier`）
  → **"did not match any packages"**，彻底不在图里；`rustls` / `ring` / `rustls-webpki` /
  `hyper-rustls` / `tokio-rustls` → "nothing to print"，无任何消费方。
- `cargo tree -e features -i reqwest@0.13.4` 的实际 feature 集合即为上表七项，
  **无 `default-tls` / `rustls` / `__rustls-aws-lc-rs`**。
- `target/debug/.fingerprint/reqwest-*/lib-reqwest.json` 里记录的 `features` 也正是这七项
  ⇒ 落盘产物确实按新 feature 编出，而不是"以为改了"。
- 锁文件**净删 29 个包**：`aws-lc-sys` `aws-lc-rs` `cmake` `dunce` `fs_extra` `jobserver`
  `rustc_version` `cfg_aliases` `combine` `chacha20` `cpufeatures` `rand_pcg` `lru-slab`
  `quinn` `quinn-proto` `quinn-udp` `rustc-hash` `rustls-native-certs` `rustls-platform-verifier`
  `rustls-platform-verifier-android` `simd_cesu8` `simdutf8` `tinyvec` `tinyvec_macros` `web-time`
  `jni` `jni-macros` `jni-sys` `jni-sys-macros`；**净增 2 个**：`hyper-tls` / `tokio-native-tls`
  （`native-tls` 与 `schannel` 早已因 `ort-sys` 的 build-dependency `ureq` 在树里）。
  **无任何既有包被升版**（`rand` / `rand_core` 只是多版本中的一份被移除）。`cli/Cargo.lock` 同步。
- **`cmake` 从构建前置工具里消失**：它的 `[[package]]` 整块已不在锁文件里。

### 代价（明确接受，见 ADR-013）
- **引入平台分支**：Windows → SChannel、macOS → Security.framework（两者零 C 编译）、
  **Linux → 系统 OpenSSL**（需 `libssl-dev` + `pkg-config`）。CI（Ubuntu）与本地（Windows）
  从此跑不同 TLS 栈，TLS 版本上限 / 密码套件 / 错误文案随 OS 变。
- 顺带修正 `symbio/.cargo/config.toml` 里「零 C/C++ 工具链参与」这句不准确的注释：只有**链接**
  阶段成立（rust-lld）；**编译**阶段仍剩 `onig_sys`。

### 附带收益：MSRV 门禁的「真编译」分支本机跑通了
前一条（MSRV 接入 CI）留下的未验证项，其根因**不是** MSRV 也不是沙箱限制，而是 `aws-lc-sys`
的 C 编译（写 `.obj` 被拦 → `fatal error C1056`）。它退树后，本机沙箱里
`node scripts/gate.mjs --only=msrv` **完整通过**：

```
▸ symbio: cargo check --all-targets @ 1.91.0 … ok (3m 51s)
▸ cli:    cargo check --all-targets @ 1.91.0 … ok (3m 40s)
  通过 2 / 2   全部通过   MSRV_GATE_EXIT=0
```

同一沙箱里 `onig_sys` 的 C 编译**正常成功**，反证了失败与「沙箱不能编 C」无关。
⇒ MSRV 的真编译结论现在本地可复现，不必再等 CI；该项可从「未验证」划掉。

**验证**：`cargo fmt --all -- --check` / `cargo check --lib --tests --locked` /
`cargo clippy --all-targets -- -D warnings` 全 0；`cli/` 亦 `cargo check --locked` 通过；
门禁 `--only=msrv` 2/2（见上）。

**未验证项（诚实标注）**：Linux（OpenSSL）与 macOS（Security.framework）两个分支**本机无法实跑**
（本机只能构建 Windows/SChannel 分支），跨平台正确性由 CI 给出结论。

***

## 2026-09-17: 摘掉白带的 `hf-hub`（`ring` 彻底退出依赖树）+ MSRV 接入 CI 实编译校验

**性质：依赖清理 + 门禁补漏**。协议与 LLM 工具面**无变化**；依赖树净减 **72 个包**。

### 依赖：`ring` 的引用是白带的，删掉即净减 72 个包
- `fastembed` 默认 features 里的 `hf-hub-native-tls` 打开了 `hf-hub → ureq`，而 `ureq` 的
  `rustls` feature 硬编码 `_ring`（= `rustls?/ring`）—— 这是 `ring` 在本仓**唯一**来源。
  但嵌入模型根本不需要下载：`providers/embedding/fastembed.rs` 把 `model.onnx` /
  `tokenizer*.json` 全部 `include_bytes!` 编进二进制，只走 `try_new_from_user_defined`
  （吃 bytes），日志亦写着 "offline (in-memory) mode"；`hf-hub` 只门控 `pull_from_hf` /
  `load_tokenizer_hf_hub` 这类下载路径，从未被调用。
- 故改为
  `fastembed = { default-features = false, features = ["ort-download-binaries-native-tls"] }`：
  连带去掉 `hf-hub`、`ureq`（runtime 那份）与 `image-models`（`image` 及其编解码链
  `rav1e` / `ravif` / `exr` / `tiff` / `png` / `gif` / `webp` / `zune-*` 等）。
- **实证（不靠推断）**：`cargo tree -i ring --target all` 无输出；`target/debug/.fingerprint/`
  下 `ring-*` 最新时间戳停在改动之前，而同一时刻 `rustls-*` 有当日新指纹 ⇒ ring 不再被编译。
- `Cargo.lock`：`[[package]]` 条目 **460 → 386（-74）**，唯一包名 **429 → 357（-72）**，
  **零新增**。锁里 `ring` 那个 `[[package]]` 条目成了无消费方的残留（`rustls` / `ureq`
  的依赖列表都已不含它），cargo 未回收，不影响构建。
- 剩余 C 依赖只有 `onig_sys`（←`onig`←`tokenizers`）与 `aws-lc-sys`（←`rustls`），
  都在真实使用链上，本次不动。
  **（`aws-lc-sys` 已于同日更晚的 TLS 条目中移除，见文件顶部；现仅剩 `onig_sys` 一条。）**

### MSRV：从「注释里的数字」变成 CI 实编译校验
- 此前 `rust-version = "1.91"` 只在注释里声明，而本机与 CI 都锁 `rust-toolchain.toml` 的
  1.93.1 ⇒ **没有任何一次构建用过 1.91**，等于没声明。
- `scripts/gate.mjs` 新增 **msrv 阶段**（顺序：后端 → 前端 → 静态审计 → **MSRV** → 事实文件）：
  从 `Cargo.toml` 的 `rust-version` 读下限（**唯一真相源**，`1.91` 自动补全为 `1.91.0`），
  用 `RUSTUP_TOOLCHAIN` 覆盖工具链文件，对 `symbio/` 与 `cli/` 各跑一次
  `cargo check --locked --all-targets`。
- 两个要点都**不是平台限制**，而是防假结论：
  ① `RUSTUP_TOOLCHAIN` **只对 rustup 装的 cargo 生效**（发行版包 / homebrew 的 cargo 会静默
  忽略）⇒ 阶段先探 `rustc --version`，实际编译器 ≠ 声明值就**跳过并提示**，绝不拿默认编译器
  冒充 MSRV 结论；
  ② 换编译器会让 target 缓存整体失效 ⇒ 用独立 `CARGO_TARGET_DIR`
  （`.workbuddy-ai/msrv-target/`，已 gitignore），否则每跑一次门禁就触发一次整树重编。
  判据只有版本号与退出码，不依赖 OS / shell。
- CI 新增独立 job `msrv-check`：`rustup toolchain install 1.91.0 --profile minimal` 后跑
  `gate.mjs --only=msrv`（独立 cache key，不与 1.93.1 的 artifact 混用）。
- 注释同步（`symbio/Cargo.toml`、`symbio/rust-toolchain.toml`）：删掉「MSRV 未被 CI 验证」，
  改为指向该 job。`CONTRIBUTING.md` §3 补上 `--only=msrv` 用法与独立 target 的说明。
- 顺带修 `gate.mjs` 顶部注释里一处早已损坏的字符（U+FFFD）。

**验证**：`cargo fmt --all -- --check` / `cargo check --lib --tests` / `cargo clippy
--all-targets -- -D warnings` 全 0；门禁 `--only=docs,facts` **7/7**；msrv 阶段的跳过分支
已实跑（临时把 `rust-version` 指向未安装版本 → 正确跳过并给出安装提示）。

**未验证项（诚实标注）**：MSRV 的**真编译**分支在本机沙箱跑不通——`aws-lc-sys` 的 C 编译
在沙箱内写 `.obj` 失败（`fatal error C1056`，`cl.exe` 是找得到的）。故该分支的首次真实
结论由 CI 给出；本地装了 1.91.0 后可自行 `node scripts/gate.mjs --only=msrv` 复现。

> **已解决（同日更晚的 TLS 条目）**：根因是 `aws-lc-sys` 的 C 编译，与 MSRV、沙箱均无关。
> `aws-lc-sys` 随 TLS 后端切换退树后，该分支在本机沙箱**完整通过**（`symbio` + `cli` 各
> `cargo check --all-targets @ 1.91.0`，2/2，`MSRV_GATE_EXIT=0`）。CI 的首次真实结论虽仍由
> `msrv-check` job 给出，但本地已可复现，不再依赖 CI。

***

## 2026-09-17: 质量收敛 —— 删零引用依赖 / 处置游离审计脚本 / 接线 ask_user / 动作按钮收敛

**性质：依赖清理 + 审计体系补漏 + 一个功能接线**。协议无变化；LLM 工具面 **+1**。

### 依赖：删掉 4 条 C/C++ 编译链里纯属白费的一条
- `symbio/Cargo.toml` 删除 `sqlite-vec`：**全仓零代码引用**（`symbio/src` 里 `sqlite` 的命中
  全是「解释已删存储后端」的注释，`cli/src`、`tauri/src-tauri/src` 零命中）——它是
  ADR-008 回退多存储后端后的遗留，却独自把 `cc` 拖进构建。`Cargo.lock`（含 `cli/`）同步收缩。
- 另三条（`onig_sys`←`fastembed`、`ring` / `aws-lc-sys`←`rustls`）仍在，属真实使用或
  无净收益的取舍，本次不动。
  **（后续进展：`ring` 随 hf-hub 摘除、`aws-lc-sys` 随 TLS 后端切换，先后退出依赖树——见上两条条目。）**

### 审计体系：4 个游离脚本，2 删 2 接
- `scripts/` 下有 4 个脚本**既不在 `gate.mjs` 也不在 CI**：
  - **删** `test-capabilities.mjs` / `test-capability-chain.mjs`：它们验证的是 `agent_query` /
    `agent_store` / `agent_metacognition` 等 **10 个已不存在的能力工具**（ADR-005 / ADR-009
    回退后 capability 面早已不是这一套），且需 release 构建 + 真实 LLM 才能跑 —— 是死检查，
    留着只会误导。
  - **接** `dead-code-audit.mjs`（**判定型**，并入 docs 阶段；有死码即失败）与
    `schema-audit.mjs`（**报告型**，退出码恒为 0 ⇒ 只有它**崩溃**才让门禁红，这正是防它腐烂的机制）。
- **修 `schema-audit.mjs` 两处缺陷**（不修就等于把一份会说谎的报告接进门禁）：
  1. `ROOT` 由 `process.cwd()` 改为按脚本位置推导（原来在仓库根之外跑会全盘错位）；
  2. 前端模块引用只认 `from '...schemas/x'`，**漏掉 schemas 内部的相对导入与 `export * from`**
     ⇒ `vdfs-form.ts` 被误判成「死文件」；改为解析相对路径 + 别名，并把测试文件排除出被审计集合
     （仍计入消费方）。顺带修掉一处重复 `rel()` 映射（过去靠 cwd == 仓库根掩盖）。
- **`doc-link-audit.mjs` 整体豁免 `docs/archive/`**：归档记录的是**当时形态**，其失效链接改写
  等于篡改历史。现在报「扫描 204 条、失效 0（豁免 38 个归档文件）」，长期噪音消失。
- `dead-code-audit` 的「导出级检查」55 条属**降级提示**（对「仅在本文件按类型用」「经 barrel
  再导出」都会命中），默认只印个数，明细加 `--verbose` —— 否则每次门禁刷 50+ 行噪音。
- **检查清单不再重抄**：`CONTRIBUTING.md` 与 `.github/workflows/ci.yml` 里逐项列举的审计清单
  本就违反 CONTRIBUTING 自己的「权威清单只在 gate.mjs」约定，已改为指向 `gate.mjs`。

### 功能接线：`local/ask_user` 从「暂未注册」转为生效
- 该工具（267 行，整文件 `#![allow(dead_code)]` + 文件头自述「暂未注册」）已接入
  `plugins/local/plugin.rs` 的 `tool_impls`，`local` 原生工具 **4 → 5**。
- 接线依据：它依赖的机制**早已端到端跑通**——`plugin.rs::emit_confirm_prompt`（工具审批）
  产出的是同形状的 `user_prompt` 节点，前端 `MessageNode.vue` 的提问卡（选项 / `Other` 自由输入 /
  提交）与回答回填链路现成；`session` 侧 `turn.rs` / `resume.rs` 的注释本就写着「confirm/ask_user」
  两种情况。故这是把已建好的能力接上，不是新建能力。
- 顺带：`AskUserTool` 不再持 `SecurityPolicy`（它不碰文件系统，该字段纯属 dead）；
  `policy` 风险表把它显式归为 `Low`（否则默认 `Medium`：会话阈值设为 low 时，
  **提问本身要先过一次审批**）。
- 自动模式（`mode == "auto"`）不产节点，返回 `tool_unavailable` 让模型自行继续，不阻塞。
- 文档同步：`local/README.md`、`docs/reference/ROUTES.md` §Local 插件；`docs/CURRENT.md`
  的 §2 工具表已自动收录 `ask_user`。

### 前端：详情页动作按钮收敛到唯一实现
- `VdfsReadonlyDetail.vue` / `VdfsTextDetail.vue` 原先各自手写 `action-btn` / `danger-btn`，
  而机制实现是 `VdfsActions`（`.ea-btn`，图标优先 + busy_label + tooltip）。两者现已收敛，
  同一个交互（重命名 / 删除 / 保存 / 还原）在全仓只有一份实现。
- 连带清理：`.danger-btn` 失去**最后两个**消费者 ⇒ 从 `controls.css` 删除（连带文件头清单）；
  它唯一引用的令牌 `--text-inverse` 按既有约定登记进 `style-audit` 的 `ALLOW_UNUSED_PROPS`
  （令牌层是完整语义层，与 `--radius-xs` 同例）。

### 注释纠错（非漂移）
- `symbio/rust-toolchain.toml` 的 MSRV 注释**错位**：它挂在 `components` 上方、描述一个该文件里
  并不存在的字段。已改写为说清 **channel（1.93.1，本机+CI 固定使用）≠ MSRV（1.91，语言特性
  下限：`submit_object_creator!` 依赖 const 上下文 `TypeId::of`）**，并注明 MSRV 未被 CI 验证。
- `symbio/Cargo.toml` 的 MSRV 注释里那句「主要使用 1.95.0 stable」是过时描述，已删。

***

## 2026-09-17: 修复前端样式审计暴露的既有问题（VDFS 详情按钮无样式 / 死样式 / 审计假阳性）

**性质：真 bug 修复 + 死样式清理 + 审计脚本修复**，无协议与行为变化。

- **修真 bug：VDFS 详情页的动作按钮一直是无样式的原生按钮**。`VdfsReadonlyDetail`
  与 `VdfsTextDetail` 用 `class="btn" / "btn primary" / "btn danger"`，但 `.btn`
  **在任何地方都没有定义**（全仓该名字只存在于 `HomedirSwitcher.vue` 的 scoped 样式，
  且 scoped 属性对子组件元素不生效；`git log -S'.btn {' tauri/src/styles/` 为空，
  说明全局 `.btn` 从未存在）。改为 `controls.css` 的共享控件类：
  `.action-btn`（保存）/ `.action-btn secondary`（还原、重命名）/ `.danger-btn`（删除）
  —— 与同页 `VdfsWorkbench.vue` 的既有写法一致，且复活了此前零引用的 `.danger-btn`。
- **删死样式**：`DetailForm.vue` 的 `.path-pill`（无任何模板/脚本引用，唯一消费者已
  消失）、`controls.css` 的 `.field-label`（全仓零引用）及其在文件头清单中的登记。
- **修 `style-audit.mjs` 的数组分支**：`:class="[...]"` 原先在收完字符串字面量后
  直接 `return`，把**模板字面量前缀**与**对象键**两类痕迹全丢掉 —— 最常见的
  `` :class="[`st-${node.status}`, …]" `` 因此被误报成「定义未使用」（`.option-btn.st-error`）。
  现改为显式 `addKeys` / `addPrefixes` / `addTernary` 收尾；数组分支**刻意不套用**
  裸标识符简写规则：数组里的裸标识符是变量（`[statusClass, …]`），当成类名会凭空
  造出「使用未定义」的 ERROR（修的过程中确实撞到过 `.statusClass`）。
- **审计新增两处「无引用但属正常」登记**（都写明原因，见脚本头注）：
  `ALLOW_UNUSED_SCOPED` —— 后端协议词表驱动的修饰类（`DetailAction.style` →
  `.ea-btn.primary/.danger`；`DetailBadge.style` → `.badge.disabled/.accent`，
  词表在 Rust 侧，静态不可解析）；`ALLOW_UNUSED_PROPS` —— 设计系统成员
  （`--accent-subtle-border` / `--surface-fade` / `--color-banner-bg` / `--radius-xs`
  / `--font-size-xl` / `--font-weight-regular`），令牌层按语义组与刻度成组定义，
  未被消费不等于死代码。**结果：16 条警告 → 0 错 0 警。**
- **删过期检查 `grep-audit.mjs` 的 I-014-light**：它审的 `CognitiveUnit` 已随认知体系
  回退从 `symbio/src` 彻底消失（`typed_unit` / `meta_belief` / `is_meta` 亦零引用），
  4 处命中实为 `agent_run` 的 **JSON Schema 键**（`"description":`），且其引用的
  「PLAN M-1」也已不存在。删后 grep-audit 0 错 0 警。
- **补全协议文档**：`symbio_core/schemas/detail.rs` 的 `DetailAction.style` 注释漏了
  **实际在产出**的 `divider`（mcp / agent / skill / model 详情的分隔线），改为按真实
  语义描述（空格分隔的 class 修饰词，`icon` 可叠加 `danger`，`divider` 渲染分隔线）。

***

## 2026-09-17: 清理 OAB v1 遗留的悬空 id 常量、收敛 `agent_run` 工具名

**性质：死代码清理 + 单一真相源**，无行为变化（回归测试照旧锁住对外工具名）。

- **删 `ids.rs` 的「Agent 能力 id」区**：`CAPABILITY_AGENT_CHAT` / `_IDENTITY` /
  `_COGNITION` / `_CREATE` 四者在 `symbio/src` 与 `tauri/src` 中**零引用**——它们是
  OAB v1 的能力对象 id，随同日 `refactor(agent): 删除 OAB v1 装配实现` 一并失去注册方。
  `ids.rs` 的职责是「经 `submit_object_creator!` 注册到全局注册表的对象 id」，
  这四条已不属其中（`docs/DECISIONS.md` ADR-009 早已记录 `_COGNITION` 是悬空常量）。
  原地留一行移除说明，以免被当成漏项补回。
- **修 `ids.rs` 头部两处过期内容**：收益说明里引用的 `AGENT_CAPABILITY_IDS` 已不存在；
  命名约定列表中的「Agent 能力」「Model 协议」「存储后端」三类在本文件都没有对应区
  （Model 协议早已明确不在此定义，存储后端选型已删）。并补一句边界：
  **LLM 工具名不归本文件**（它是 `CapabilityMeta.name` 短名，属各插件实现细节）。
- **`agent_run` 工具名收敛为常量**：`plugins/agent/host/subagent.rs` 里它此前有**两个**
  硬编码点——`CapabilityMeta.name`（LLM 看到的短名）与审批续跑载荷里的 `tool_name`
  （`session/resume` 据此决定重执行哪个工具）。只改一处会**静默断掉审批续跑**，
  故收敛为本模块私有的 `NAME`；测试继续断言字面量 `"agent_run"`，以锁住对外契约。

***

## 2026-09-17: 智能体自身的 `AGENTS.md` 归 agent 插件（v2 落地，第二步）

**取代同日上一条中的「作用域」做法**（`REQUIRED_PLUGINS` 含 `work`、转发时把 `WORKDIR`
指向子 Agent 目录）——那条是错的，详见下。

- **智能体域归一处**：`{homedir}/AGENTS.md`（系统智能体）与 `<agent dir>/AGENTS.md`
  （子智能体）两份指令文件**都归 agent 插件**，各自带可编辑地址
  （`.vdfs/agent/AGENTS.md` / `.vdfs/agent/<id>/AGENTS.md`）、两道容量闸门与片段注入。
  读写面与注入面因此落在同一个所有者上——`谁能读写它，谁负责注入它` 才完整。
- **`REQUIRED_PLUGINS` 收窄为 `["mcp","skill"]`**：只放解释能力资产的插件。
  - `work` 移除：它的作用域语义是 `ctx[WORKDIR]`（工作区）。**转发时不再覆写
    `WORKDIR`**——覆写让它的名字与实际管的东西对不上，并与系统侧那个实例读同一份文件、
    注入两次。子树现在继承父会话的 `WORKDIR`。
  - `setting` 移除：它是**设置页的入口**（自有分区 + 各插件配置清单），不是任何内容
    文件的所有者；它在子树里既无挂载点也无可声明配置（注册会串味），无事可做。
- **`session` 不再读 `{homedir}/AGENTS.md`**：删掉 `prompt::global_instruction()` 与
  `session-global-instructions` 段。会话不是那个文件的所有者（既不给地址也不限容），
  读一遍注入只是把「智能体自身的指令」临时挂在会话上。
- **记忆的读写与闸门只在内核**：`AgentDirStore` 只回答「记忆文件在哪」（`memory_path`），
  不再自带读写与自己的字节闸门；读写、两道闸门、片段排版、节点形状全部来自
  `symbio_core::memory`（与 work / session 同源）。
- **旧装配清理**：旧 agent 目录里被自动补建的 `work/PLUGIN.yml` 由
  `archive_retired_work_tree` 改名为 `PLUGIN.yml.disabled`（= 卸载，幂等且可逆）。
- **挂载根保留名**：`.vdfs/agent/AGENTS.md` 是本应用自身的指令，不是名为它的 agent 目录
  （agent id 首字符必须是小写字母或数字，不可能相撞）。

规范同步：`docs/design/agent-directory-spec.md` §6.2 从「`work` 实例换作用域」
改为「所有权判据 + 禁止靠改指作用域实现」，附录 A.1 / A.3 记录新做法。

***

## 2026-09-17: 子 Agent 改为 composite 插件树（Agent 目录规范 v2 落地，第一步）

规范见 [`docs/design/agent-directory-spec.md`](./design/agent-directory-spec.md)
（取代 `open-agent-bundle-spec.md`）。本次是**代码侧第一步**，只加新路径，
旧的 OAB v1 约定目录装配仍保留，按 `manifest.yaml` 的 `spec` 分流。

- **子 Agent = 一棵 composite 插件树**：`{homedir}/agent/<id>/manifest.yaml` 声明
  `spec: agent-dir/v2` 的目录，由 agent 插件照 `home` 造 `worker` 的同形写法挂成
  composite（`PLUGIN_DIR` + `REQUIRED_PLUGINS = ["mcp","skill","work"]`），
  **惰性构造 + 缓存**（全量预建会连带启动每个子 Agent 的 MCP server）。
- **注册代理层** `agent/host/scope.rs`：子 Agent 树的注册经它转发，
  一切注册项加来源前缀——提示词段 / VDFS 用 `agent/<id>/<name>`，工具用
  `agent_<safe_id>_<name>`（工具名进 function-calling 协议，只收 `[A-Za-z0-9_-]`）。
  加前缀后**名字根本不冲突**，于是并集与遍历顺序无关
  （`Composite::traverse` 遍历 `HashMap`，顺序不确定，不能依赖"后注册者胜"）。
- **单槽注册不转发**：`register_vdfs_root` / `register_model_provider` 被丢弃——
  子 Agent 的容器会把自己登记成 VDFS 根，原样转发会劫持系统侧全部挂载点。
- **作用域**：转发时把 `WORKDIR` 指向子 Agent 目录，其中的 `work` 实例因此拥有
  `<agent dir>/AGENTS.md`，而不是沿用父的 workdir（规范 §6.2：一个作用域只有一个
  所有者，否则同一份记忆被注入两次）。
- **skill 插件**：技能检索加上**本插件自己的目录**作为第一来源，子 Agent 的
  `skill` 实例因此自包含（技能就在 `<agent dir>/skill/` 下），不再依赖全局配置。

未动 `symbio_core`：代理层实现的是既有 `CapabilityVisitor` trait，放在 agent 插件内。

测试 650（+3，基线同步更新）；门禁 14/14。

## 2026-09-16: session 存储根改为插件实例目录（补完上一条的未竟项）

上一条遗留：`session_storage_dir()` 仍按插件名反推落位。现已收口——**会话存储根 =
本插件自己的目录**，`paths` / `memory` / `tool_result_guard` 这些按 id 派生路径的
自由函数一律**由调用方把根传进去**：

- `paths::session_dir(root, id)` / `session_subdir(root, id, subdir)`：只做
  「id → 目录名」，不认识任何全局布局；
- `memory::memory_path(root, id)` / `store(root, session_id, …)`、插件的
  `store_with(root, …)` 同步加 `root`；
- `guard_tool_result(…, root)` / `resolve_archive_dir(root, …)` /
  `archive_full_text(…, root)`；调用点 `tool_executor` 与 `compress` 都拿得到
  `ctx`，经 `dir_from_ctx(ctx, PLUGIN_SESSION)` 取自己的目录；
- `SessionPlugin::storage_dir(&self)` 成为生产唯一入口；
  `session_storage_dir()` 降级为 **`#[cfg(test)]` 回退**（无实例时按插件名取常规落位）。

顺带修了一个被掩盖的问题：原有用例把记忆写进**全局回退**目录、却断言插件实例读到
它——两条路径恰好同值时能通过。现在用例改用 `p.storage_dir()`。

门禁：`gate.mjs --fix` 16/16（647 = 基线）。

***

## 2026-09-16: 插件落位去掉 `plugins/` 这一层 —— 插件直接并列在系统根下

`<homedir>/plugins/<插件>/` → `<homedir>/<插件>/`。那一层不承载任何语义（既非挂载点、
也不参与寻址），只是把每个插件的路径都加深一段。

### 1. 插件**不再知道**自己被放在哪

改的不只是路径，更是依赖方向：**插件目录由父插件经 `PLUGIN_DIR` 传入**（`dir_from_ctx`），
插件实现与注释里都不再出现 `<homedir>/…` 这类硬编码：

- `agent`：`AgentDirStore::new(global_root, workdir)` —— 根由调用方给（装配态来自
  `AgentPlugin` 自己的 `PluginDir`），不再自己拼 `HomedirRegistry/…/agent`；
  `AgentRunCapability` 增加 `agent_dir_root` 字段透传，工作区级改 `{workdir}/.symbio/agent`。
- `model` / `mcp` / `skill`：存储根改为本插件自己的目录（`DirVdfs::at` / `SingleFileVdfs::at`），
  不再按插件名反推落位；`skill` 新增 `dir` 字段。
- `DirVdfs::for_category` **删除**（已无使用者）；`SingleFileVdfs::for_category` 保留但
  标注**仅供迁移**（model 迁移旧分类 `ai`）。
- `skill` 的默认 `skill_dirs`：`{HOMEDIR}/plugins/skills` → `{HOMEDIR}/skills`
  （`{HOMEDIR}` 是用户可改的配置占位符，语义就是「系统目录」）。

⚠️ **未完成的一处**：session 的 `session_storage_dir()` 仍走 `category_dir(PLUGIN_SESSION)`
—— `paths` / `memory` / `tool_result_guard` 是按 id 派生路径的**自由函数**，拿不到插件
实例。已在函数文档标注这是**无实例回退**，装配态下取值与插件自己的目录相同。

### 2. 容器扫描：区分「不是插件」与「是个坏插件」

插件根现在是系统根本身，其下**本来就有非插件目录**。`provider_of` 改为三态返回：
目录里没有 `PLUGIN.yml` → `Ok(None)`，静默跳过（只记 debug）；有但不可解析 / 未声明
`plugin_provider` → `Err`，**告警**（"配了一半"是用户需要知道的）。

### 3. 文档同步

`PLUGINS_DIR` 常量删除；插件与 `providers/` 的注释、13 份 `docs/`、事实生成器
`gen-current-facts.mjs` 一并更新。`docs/CHANGELOG.md` 的历史条目**未改**（历史就是历史）。

### 数据迁移

⚠️ **代码不做迁移**：既有安装需把 `<homedir>/plugins/*` 上移到 `<homedir>/` 下。

***

## 2026-09-16: CI 收口为同一个 `gate.mjs` —— 检查逻辑只剩一处

`.github/workflows/ci.yml` 此前手写全部命令，与本地 `gate.mjs` 是**两处真相**（且 CI 用
`--workspace`、本地用 `--lib`，口径本就不同）。现在三个 job 都调同一条命令：

| Job | 命令 |
|---|---|
| `rust-checks`（matrix dev/release） | `node scripts/gate.mjs --only=backend --ci --profile=${{ matrix.profile }}` |
| `frontend-checks` | `node scripts/gate.mjs --only=frontend` |
| `docs-validation` | `node scripts/gate.mjs --only=docs,facts` |

- 原先散在 job 里的 `cargo fmt` / `clippy` / `build` / `test`、`npx vue-tsc`、
  `npm test`、`grep-audit` / `style-audit` / `gen-current-facts --check` 全部删除，
  改由脚本按阶段编排；`paths` 触发器新增 `scripts/**`（改脚本本身也要触发 CI）。
- **覆盖度不降反升**：`symbio/Cargo.toml` 是单 package（`--workspace` 与默认等价），
  而脚本额外检查 `cli/` 这个 CI 从未覆盖的独立 workspace。
- `gate.mjs` 新增 `sumInt()`：`cargo test --workspace` 会为每个测试目标各打一行
  `test result:`，原先 `grabInt` 只抓第一行（可能拿到某个小目标的 0），CI 模式下现在
  **求和**展示（仅信息用途，判定仍只信退出码；`FAILED` 行不计入）。

***

## 2026-09-16: 知识回归文档 —— 新增 `doc-find`，记忆从 15KB 压到 1.6KB

**问题**：长期记忆里囤了大量「某机制是什么」的摘要。核查后发现**九成已在文档里**
（`docs/design/vdfs.md`、各模块 `README.md` / `docs/`、以及源码 `//!` 模块文档，
如 `symbio_core::memory` 的三层记忆说明）。记忆长的根因不是文档缺，而是**文档下沉后
找不到**，于是每轮都把摘要复制一份进来，而复制必然漂移。

### 1. 新增 `scripts/doc-find.mjs` —— 全仓文档检索

```bash
node scripts/doc-find.mjs 三层记忆          # 搜 md + Rust 文档注释
node scripts/doc-find.mjs canonicalize_loose --rs --context 3
node scripts/doc-find.mjs 闸门 --limit 30
```

- 同时搜 `*.md` 与 `*.rs` 的 `//!` / `///`；`--md` / `--rs` 可限定。
- ⚠️ Rust 侧除文档注释外**也匹配定义行**（`fn` / `struct` / `const` …）：只搜注释会漏掉
  最常见的「搜符号名」用法——`canonicalize_loose` 的说明写在上方 `///` 里，那三行并不含
  该词，真正的命中行是 `fn canonicalize_loose(...)`。
- `CHANGELOG` 命中**排序靠后**（它是历史，不是「现在是什么」）；排除 `archive/`、`node_modules/`、
  `target/`、`.git/`、`.symbio/`、`.workbuddy*/`。

### 2. 文档三处收口

- `CONTRIBUTING.md` §3：原「提交前清单」表格（7 行手写命令）**删掉**，改指向 `gate.mjs`
  —— 它与 CI 是两处真相，必然漂移；新增「本机操作陷阱」小节（cargo/git 不接管道、
  Windows 下 `-F "C:/..."`、批量改名不用 `git rm/mv`、rustfmt 只用 `cargo fmt` 等）。
- `docs/README.md`：文档下沉原则补「知识只写一处，靠检索而非记忆」+ 快速导航加检索行。
- 测试布局约定补一句「由 `test-layout-audit.mjs` 判定」。

### 3. 长期记忆 `.workbuddy-ai/memory/MEMORY.md`

15.2KB → **1.65KB**：只留两个入口（`doc-find` / `gate.mjs`）、三条协作约定（提交精确
指定文件、改机制同步 `docs/reference` + `CHANGELOG`、`.workbuddy-ai/` 勿删），
并在文件头写明 **发现「只在记忆里」的知识 = 文档缺了，写回文档而不是写进这里**。

### 门禁

`gate.mjs --only=docs,facts` **5/5**（grep / style / doc-link / test-layout / 事实文件一致）。
链接失效仍为 17 条，全在 `docs/archive/`。

***

## 2026-09-16: 门禁收口为一条命令 —— `node scripts/gate.mjs`

「改动后必跑」原先是一条**记在文档与记忆里**的清单：四道后端命令、两条前端命令、
三个审计、一个必须最后跑的事实文件，外加一串坑。靠人背迟早漏一步，而且每加一条
要求记忆就更长一寸——它本该是代码。

现收进 `scripts/gate.mjs`（纯 Node、跨平台，与既有 `*-audit.mjs` 同风格）：

- **一条命令跑全量**：后端（含 `cli/` 这个独立 workspace）→ 前端 → 审计 → 事实文件。
  顺序有语义：事实文件由代码生成，故必须排在最后。
- `--only=` / `--skip=` 跑单阶段；`--fix` 先格式化 / 重生成再检查；`--ci` 对齐 CI 的
  `cargo test --workspace`；`--profile=` 额外跑 `cargo build`（本地默认不跑，省几分钟）。
- **坑封进代码，不再靠记忆**：不接管道（spawn 实时读流有进度 + 全文落
  `.workbuddy-ai/gate-logs/` 可回溯，判定**只信退出码**）；clippy 的 `[sandbox] … 拒绝`
  只提示不算失败；`rustfmt` 只用 `cargo fmt`；vitest 跑完**不自退**故总结行一出即收尾
  （不必干等 180s 超时）；通过数**只增不减**，高于基线时提示更新 `BASELINE`。

⚠️ CI（`.github/workflows/ci.yml`）目前仍是手写命令；`--ci` / `--profile=` 已为对齐它
做好准备，但**改 CI 需在 Actions 上验证后再合**，本次未动 `.github/`。

## 2026-09-16: 会话标题 / 摘要动态化 + 设置列表无状态

### 1. 会话：标题 = 用户最后说的话，摘要 = 助手最后的回答

清单一行原本是「第一句提问 + 最后一条任意消息」——会话名被第一句话**永久钉住**，
越聊越对不上。改成两端都取**最新**：

- `derive_session_title`：**最后一条**含文本的用户消息（原先是第一条）⇒ 列表名
  跟随「最近在聊什么」。
- `derive_session_summary`：只认**助手**消息（原先是任意角色的最后一条）⇒ 摘要是
  最近一条**有文本**的回复；纯工具调用 / 推理消息没有正文，会继续往前找。

⚠️ **显式命名仍然优先**：`display_title()` 里 `metadata.title` 依然排第一——用户
自己起的名字不该被自动派生顶掉，否则重命名就失效了。所以「动态」只对**未显式命名**
的会话生效，这是刻意的取舍。

两者都是 `SessionSummary::of()` 在 `save` 时算好的**投影**（清单不读消息），
每轮对话后列表刷新即见新标题，不需要额外机制。

### 2. 设置列表：不显示状态点

节点 `status` 的缺省值是 `active`，于是**静态**资源也被画上一个绿点——那是个
不存在的信息。新增 `VDFS_STATUS_NONE`（空串）：`setting` 插件对**分区节点**与
**插件配置条目**两类都显式声明无状态，前端据此不渲染状态点
（`:show-status="Boolean(n.status)"`）。

⚠️ 这是**声明出来的无状态**，不是「忘了填」：缺省仍是 `active`，只有显式清成
空串才失去状态点。

### 3. 顺带：状态点 hover 提示改为文案映射

`status` 原值（`working` / `active`…）此前直接当 tooltip，等于把后端枚举念给用户
听——与 `ext` 徽标是同一类问题。改为中文文案映射（进行中 / 就绪 / 已停用 / 出错），
未知取值返回**空串**：宁可不显示提示，也不漏出枚举。

### 4. 文档

- `vdfs.md` §3.2：`status` 词表补 `VDFS_STATUS_NONE`
- `vdfs-frontend.md`：§4.2 增「状态点」条；§5.1 / S3 校正会话标题的派生描述

## 2026-09-16: 列表只呈现用户语义 —— 去掉 `ext` /「可写」等机制字段

列表是**挑一个东西**的地方，不是看结构的地方。原先每一行都挂着两种机制信息：
文件徽标显示 `ext`（`form` / `session` / `model`），标签行显示由访问位派生的「可写」。
前者是**渲染器键**（「谁来渲染这一项」），后者是**能力判据**（「能不能保存」）——
两者都由机制消费，不该出现在给用户看的列表里。

### 1. 前端：一处收敛（`VdfsWorkbench.vue`）

- **徽标只给目录**（子项数）；文件不再给徽标。`badgeOf` 不再回退到 `n.ext`，
  `badgeKindOf` 随之删除（目录恒为 `primary`）。
- **标签只留用户语义**：后端声明的 `meta_tags`（VDFS 只透传）+ 相对时间；
  删除按 `vdfsAccessOf(n).write` 生成的「可写」标签，import 里去掉 `vdfsAccessOf`。
- 覆盖范围：全 App 只有一处列表生产点（`VdfsWorkbench.vue` → `VdfsCard`），
  会话 / 设置 / 模型 / MCP / 技能 / 智能体列表**同时**收敛，没有第二处要改。

### 2. 后端：`setting` 分区节点不再写 `description`

「该分区数据由前端状态自持，VDFS 侧无正文」是一句**机制说明**，而 `description`
只出现在用户看的列表副标题里。删掉赋值（`label` = 「外观」「关于」已足够），
理由写进 `section_node` 的文档注释。

### 3. 不动的部分（有意保留）

- **不在后端删字段**：`ext`（渲染器分发）、`access`（能力判据）、`schema`（详情呈现）
  都由机制消费，详情页 / 能力判据 / 渲染器分发都要用。该收敛的是**渲染层**。
- **详情页保留机制字段**：`VdfsReadonlyDetail` 是结构浏览器 / 调试视图（S14），
  「看这一项」时路径 / 状态 / 访问位 / 呈现扩展名有解释价值。
- **新建类型选择面板保留 `t.ext`**：它是交互控件（新节点选择器）而非列表项，
  两三个候选之间 `ext` 是主要区分。

### 4. 文档

- `docs/design/vdfs-frontend.md`：§4.2 增「呈现口径（用户语义优先）」；§8 增两条
  一致性要求（列表不得渲染机制字段；不得为列表好看而在后端删字段，provider 也不
  要把机制说明写进 `description`）。

## 2026-09-16: 新建 = 直接进入该类型的详情页（草稿态）+ 无名字新建

「点新建」原先走一条**独立于详情页**的通道：先弹一个命名输入框，填完名字才写文件。
这条通道把「新建」和「查看 / 编辑」割成了两种形态，而名字本不该由使用方先给——
**`id` 归 provider**。四件事一起收口。

### 1. 前端：新建 = 选中一张草稿节点

- **草稿节点**（`useVdfs.draftNodeOf`）：`path === ''`、`name === ''`、`access = 'w'`，
  其余呈现字段留空——「还没有的东西」不假装有内容（新建态就是缺 id / 名字的那一页）。
  它与「选中一项」走**同一条**详情通道：同一个 `ext` → 同一个渲染器。
- 命名输入框、`createTyped(type, name)` 一并删除；`startNew(type)` 只做「选中草稿」。
  连续新建由新增的 `draftSeq` 作临时身份（草稿没有路径，`:key` 会撞在一起而残留
  上一份输入）。
- 两条配套约束：`select()` 对草稿**直接返回**（没有内容可读，去读只会落到目录地址
  上）；`refresh()` 清理选中态时**跳过草稿**（它本来就不在清单里）。
- 草稿详情页剥掉以「资源已存在」为前提的动作（删除 / 测试连接 / 浏览内部 / 导出）——
  点下去只会打到一个不存在的地址上。
- `source = file`（zip 整包导入）是**唯一**不进详情页的形态：内容在打开详情页之前
  就已经齐备，没有「边看边填」的过程。

### 2. 机制：`new_types` 必须声明「落成后的节点」

类型清单里的 `ext` 一直是**呈现扩展名**（地址末段后缀，provider 用 `id_of` 按它剥
条目 id），但它**不是**渲染器键——model / mcp / skill 落成后统一是 `ext = form`。
于是「点新建」若按 `ext` 选渲染器就会落到通用兜底，而不是同一张表单。`VdfsNewType`
因此补两个字段描述落成后的节点：

| 字段 | 语义 |
|---|---|
| `node_ext` | 落成后的节点 `ext`（**渲染器键**）；缺省 = 与 `ext` 相同（会话即如此） |
| `schema` | 落成后的节点 `schema`（`form` 渲染器所需的定义） |

漏了 `node_ext` 草稿会落到通用兜底，漏了 `schema` 会渲染出空表单。model / mcp / skill
三个 provider 已在 `root_new_types()` 回填（`node_ext = form` + 详情定义），且定义与
节点 `schema` 同源（`detail_definition()` 唯一出处）。

### 3. 后端：写目录自身 = 「新建一个，名字由你定」

`vdfs/write` 的 `path` 现在有两种目标形态，实现方都必须考虑：

- **具名节点**：常规的「写这个节点」，不存在则看 `create` 位；
- **目录自身**（`""`）：使用方**没有给名字**——这正是「新建」在机制上的形态。

容器原先对空 `rel` 直接 `Forbidden("目录不可写")`，把这条形态堵死了。现在改为
**原样转发**给子 provider 判定（与 `action` 对空 `rel` 的处理一致），并把 provider
生成的名字补回树内全路径。四个 provider 随之支持无名字新建：

- `session`：id 由 `Uuid::new_v4()` 生成，从 JSON 体读 `metadata`（浅合并）与可选
  `title`；**无名字时不写 `title`**（留给 `display_title` 从首条消息派生）。
- `model` / `mcp` / `skill`：无名字时用新增的 `entry::auto_id(kind)` 生成
  `<kind>-<8hex>`（带前缀、`safe_segment` 安全）。
- 整包导入（`mcp` / `skill` 的二进制分支）**仍要求有名字**：名字来自地址末段，
  「无名字导入」无从命名，明确拒绝。

### 4. `create` 位只管「不存在时怎么办」

原实现把 `create = true` 理解成「忽略使用方给的内容、一律落最小默认配置」。草稿
详情页填好字段再保存时，这会让**用户填的每一个字段都丢掉**。现在语义收窄为：

| 目标 | `create = false` | `create = true` |
|---|---|---|
| 已存在 | 覆盖（`created = false`） | 覆盖（`created = false`） |
| 不存在 · **具名节点** | 写入型资源**就地创建**（`created = true`）；「更新既有对象的字段」型语义可报 `NotFound` | **创建**（`created = true`） |
| 不存在 · **目录自身** | 报错（没有可覆盖的目标） | **创建**，名字由 provider 生成 |

⚠️ 具名 + 目标不存在**不要**一律报 `NotFound`：配置型资源的地址**就是它的身份**
（`model` / `mcp` / `skill` 皆如此），不存在就建一个——这正是「有名字但文件不存在
则自动创建」。只有「写的是某个既有对象的一个字段」（如会话 metadata）才该拒绝。

内容一律取自本次写入；唯一例外是**内容为空**（「先建一个，随后再填」，如新建会话），
此时 provider 落一份自己的最小合法内容。

### 5. 顺带

- `stores/sessions.ts::createSession` 改为一次 `vdfs/write` 到会话挂载根，id 由后端
  给出（原先前端 `createSessionId()` 预造 id + `session/update`，绕开了「id 归
  provider」）。空地址响应被显式挡掉（`vdfsBase('')` 返回虚拟根哨兵，`if (!id)`
  挡不住，会插一条 id 为 `.vdfs` 的幽灵会话）。
- `plugins/vdfs/host.rs` 的测试替身同步新语义（它原先带着与生产容器同构的
  `Forbidden` 守卫，留着就是给下一个人挖坑）。
- 测试文件按约定拆分：`model/plugin.rs` 与 `skill/plugin.rs` 的内联 `mod tests`
  移到同级 `plugin.test.rs`；`mcp` 新建 `plugin.test.rs`。

### 6. 容器：写入响应的相对路径必须补成树内全路径

`CompositeVdfs` 的职责表一直写着「全路径回填：子节点 / 内容 / 写入响应的 `path` 补成
`<子目录>/<rel>`」，但实现只对**空串**兜底。provider 生成的名字（如 `openai-1`）非空，
于是原样漏了出去：访问层把它当**树内全路径**翻译成 `.vdfs/openai-1`——一个并不存在的
地址。前端 `write()` 拿 `resp.path` 去清单里找刚建出来的那一项，找不到，**只能停在
草稿上**（正是「点新建后进不去详情页」的另一半原因）。

现在 `read` / `write` 两处统一走 `CompositeVdfs::fill_path`：provider 给的**非空**路径
一律按「子树内相对路径」补前缀，只有空串（= 未填，如物理层无从表达新名字）才用请求
地址兜底。`read` 同理——model / mcp / skill 三个 provider 都把收到的相对路径原样回显
（`VdfsContent::text(path, …)`），不补前缀就会给出 `.vdfs/<rel>` 这个错地址。
两条测试锁定：`dir_root_write_is_forwarded_and_path_is_prefixed`、
`read_content_path_is_prefixed_with_dir`。

***

## 2026-09-16: 三层记忆归属收口 + 系统提示词单一通道 + `SessionConfig` 下沉

三件事同一个判据：**一个作用域只有一个所有者，核心只留跨插件共享的抽象**。

### 1. 三层记忆：一个作用域一个所有者（新增 `work` 插件）

工作区 / 会话 / 智能体三层记忆形态完全相同——**一个 UTF-8 文本文件 + 两道容量闸门 +
一行头信息的提示词片段 + 一个 VDFS 读写节点**。三份实现就是三份会各自漂移的口径
（「写侧是拒绝还是截断」「读不到算不算错误」一旦分叉，用户看到的行为会随「这条记忆
属于哪一层」而变，而用户无从知道差异从哪来），因此收敛为**一份内核**：

| 层 | 所有者 | 物理落位 | VDFS 地址 |
|---|---|---|---|
| 工作区 | **work（新插件）** | `{workdir}/AGENTS.md` | `.vdfs/work/AGENTS.md` |
| 会话 | session | `{会话目录}/AGENTS.md` | `.vdfs/session/<id>/AGENTS.md` |
| 智能体 | agent | `{agent 目录}/AGENTS.md` | `.vdfs/agent/<id>/AGENTS.md` |

- **内核** `symbio_core::memory`（`MemoryFile` / `InjectedMemory` / `SegmentSpec` /
  `NodeSpec` / `render_segment`）：读写、两道闸门、片段排版、节点形状。给的是
  **值对象 + 纯函数**而非 trait——「provider」的前提是调用方需要运行时多态
  （`VdfsProvider` 三后端 / `ModelProvider` 各家模型），记忆不是，全项目没有一处
  `dyn MemoryProvider`。
- **两道闸门**：写侧**拒绝**（不截断、不部分写入——静默丢内容是最坏的失败）；读侧
  **截断** + 明确告知地址。`total_bytes` 取读到的全文长度而非文件 metadata，与写入
  闸门同一把尺子，避免「显示 8000 字节、再写一点就被拒」的错位。
- **作用域闸门**：无作用域（无工作区 / 无会话 / 无智能体）是**正常状态**而非错误
  ——读→空串、注入→`None`、写→明确报错，闸门在构造点一处收口。
- **删除语义三层统一**：记忆一律 `Forbidden`——删除即丢失全部长期事实，抹掉之后没有
  东西能找回来；要清空就写入空内容（一次可读、可审、可撤销的显式动作）。
- **消除重叠**：`{workdir}/AGENTS.md` 原先被 session（当「工作区指令」进
  `req.system_prompt` 基座，只读、无地址）与 work（当「工作区记忆」进注册段，可读写、
  有地址与容量）**各注入一次**。按「**谁能读写它，谁负责注入它**」收口到 work，
  session 不再读它。副作用一并修掉：走 `req.system_prompt` 会**顶掉**模型插件注册的
  人格（旧优先级链第一位是显式值），改走注册通道后指令与人格**并列共存**。
- `{homedir}/AGENTS.md`（全局）**不建设**为记忆层（无地址、无容量、模型改不动），
  但从 `req.system_prompt` 挪到注册通道，归 session。判据因此是一条可检验的线：
  **内核收「模型能自己改的东西」（要有地址、要限容）；只读指令走普通片段即可。**

### 2. 系统提示词：两条通道合并为一条

`register_system_prompt_segment` / `list_system_prompt_segments` **删除**，只留
`register_system_prompt` / `list_system_prompts`。语义收敛为一条：**注册在这里的东西
一定会送达模型**——全部条目按注册顺序拼接，不做「取一个」的竞争。

「取一个」这个需求本身站不住：人格的**选择**发生在**注册期**（model 插件已按
`PROVIDER_ID` 解析出唯一生效 provider 后才注册），消费侧再按 key 选一次是空动作；
且实测 model 插件那两处「双键注册」（`provider_id` 键 + `"default"` 键）写的还是
**同一份内容**。`resolve_system_prompt` 从「基座竞争 + 片段追加」简化为
「**显式段（若有）+ 全部注册段 + 全空兜底**」。

### 3. `SessionConfig` 从 `symbio_core` 下沉到 session 插件

它原先住在 `symbio_core::schemas::session::session_config`，与同目录的 `session_chat` /
`chat_message` / `session_update` 并列，但两者性质不同：那几个是**跨插件协议**
（agent / local / model 都按同一份定义拼 WS 帧与 payload），而 `SessionConfig`
**全仓引用只在 `src/plugins/session/` 内**，前端与 CLI 零镜像、`lib.rs` 也不导出——
它描述的是「本插件自己的旋钮」。判据与 `ChatSession` 契约下沉时同一条（见
`session/chat_session.rs` 模块文档）：**核心架构只保留跨插件共享的抽象，不承载单一
模块的内部定义。** 新位置 `plugins/session/config.rs`，与 `work` / `agent` 的
`config.rs` 回到同一形态。

落盘契约不变：字段名仍是 `PLUGIN.yml` 里的键，`#[serde(default)]` 仍在，未知键仍被
静默忽略，**存量配置无需迁移**。

### 4. agent 记忆的**存储**也收口到内核（三层不再有第二份闸门）

前面只统一了 agent 记忆的**渲染**（复用内核 `render_segment`），读写仍走
`AgentDirStore::read_memory` / `write_memory`——它自带一份字节闸门与自己的错误文案，
与 work / session 两层仍是**第二份口径**。本次收口：

- `AgentDirStore` 只保留 `memory_path`（回答「记忆文件在哪」），
  `read_memory` / `write_memory` **删除**——存储层不再持有闸门；
- 新增 `agent/host/memory.rs`（本层的「个性」：落位 / 地址 / 标题 / 空提示 / 节点规格），
  与 `work/memory.rs`、`session/memory.rs` 回到同一形态；
- `AgentPlugin::memory_store(agent_dirs, agent_id)` 成为**记忆的唯一构造点**：作用域
  （agent 目录是否存在）与两道闸门在此一次收口，`vdfs` 的 list / stat / read / write 与
  `traverse` 的注入全部复用它；
- VDFS 记忆节点改由内核 `MemoryFile::node` 产出（`list` 与 `stat` 同源），手搓的
  `memory_node(size)` 删除。**两处顺带对齐**：节点 `title` 由文件名改为「智能体记忆」
  （与「工作区记忆」「会话记忆」一致），`kind` 由**无人登记**的 `memory` 改为所属插件的
  场景标签 `agent`（前端 `registerVdfsIcon` 里没有任何 `memory` 条目，是个没有消费者的
  死标签）；`updated_at` 也随内核带上（此前缺，列表无法按时间排序）。
- **读失败不再降级成一段「暂无记忆」**：那会让模型以为确实没有，比不注入更坏；
  改为记 warning 后**不注册**该片段（人格在上面已注册，不受影响）。

### 其他

- 新增两个配置项（session 与 work 各有）：`memory_max_bytes`（写闸门，默认 16 KiB）、
  `memory_inject_max_bytes`（注入预算，默认 4 KiB）。
- 新增模块文档：`plugins/work/README.md`（此前 15 个插件里唯一缺 README 的）、
  `plugins/session/README.md` §四「会话记忆」。
- 修正 3 处失效引用：`agent/host/store.rs` 的 `symbio_core::agents_md`（模块已并入
  `memory`）、`session/docs/core-loop.md` 的 `build_system_prompt`（函数已删）、
  `session/docs/complexity-audit.md` 与 `mechanism-audit.md` 保留原样（**审计快照**，
  记录的是当时形态与行号，不追改）。

***

## 2026-09-16: 读侧修复第一批（agent 自评估落地）

一个真实消费者（审查本仓库的 agent 会话）报告了「理解这套设计太贵」的五类摩擦。
经逐条对代码核实后落地第一批修复：

- **`content_search` 输出剥 `\\?\` 前缀**（`621c17a`）：搜索根经 `canonicalize` 后
  walk 出的每行路径都带 Windows 扩展前缀，模型可见输出每行白占 ~30 字符。
  复用既有 `normalize_path_for_comparison`，仅展示侧剥前缀、IO 仍走原路径。
- **补 `plugins/vdfs/README.md`**：15 个插件里唯一缺模块文档的恰是文件系统本身。
- **文档漂移修正 6 处**：① SYSTEM_MAP 存储层仍写「SessionStore = file/sqlite/memory」
  （后端已删）；② docs/README「14 个插件全覆盖」实为 15（漏 vdfs）；③ README 插件清单
  表与树图漏 vdfs；④ README 示例路由 `model/chat` / `local/shell` 均已非路由
  （model 无自有路由）；⑤ ROUTES.md 仍列 `model/chat` 为可用路由；⑥ CONFIGURATION.md
  两处把已删除的 `store_kind` 当现行配置。
- **11 篇 ADR 各补一行「当前状态」**（已实现（现行）/ 部分实现 / 已被取代 / 已回退 /
  未落地）：读者第一次能在不反推代码的情况下分辨终态。核实中的两个关键发现：
  ADR-004 的 Gemini/Anthropic 适配器**真实存在**（评估者存疑的那条反而可信）；
  ADR-009 的 CU 认知层**已整体不在代码里**——`agent_cognition` 只剩 `ids.rs`
  一个无实现的悬空常量（评估者未核实到的更大漂移）。

门禁：clippy 0 告警；544 单测全过；fmt 0 差异；doc-link-audit 17 失效（全在
archive，与基线一致）；grep-audit 0 err。

***

## 2026-09-16: 读侧修复第二批（骨架化摘要 + 读侧事实表 + CI 防漂移）

第二批针对 agent 自评估的头号摩擦——「输出骨架化让我永远无法一次建模」
与「没有可核对的'现在是什么'」：

- **骨架化摘要升级（`session/context_window.rs`）**：
  - 列表类 JSON 摘要从 `count=23 first[name=.editorconfig]`（一个条目）
    升级为 `count=23 names=[8 个条目名,…+15]`——一次摘要即可建模目录/搜索
    结果，而不是逐轮重跑工具；pretty-print 多行 JSON 在 64 KiB 内整体解析
    （只解析首行 `{` 必然失败，摘要退化成 `"count": 16,` 中间行切片）。
  - 摘要 token 上限 48 → 64：`truncate_tokens` 的截断口径是「token × 2 → 字符」，
    48 装不满一屏条目名；条目名预算按同一口径推导（`DIGEST_CHAR_PROXY_PER_TOKEN`），
    保证 `names=[…]` 收尾不被截断（截在半个名字上等于噪声，实测踩坑）。
  - 占位符新增**取回指引** `Re-run <tool> to get the full output.`：核对成本
    从"猜摘要够不够"变成一次可预期的工具调用（「核对不划算就放弃核对」的
    最小成本解）。
- **读侧事实表（`docs/CURRENT.md` + `scripts/gen-current-facts.mjs`）**：
  插件 × 注册名 × VDFS 挂载点 × 自有路由臂 × LLM 工具 × 核心 trait × 存储布局，
  **全部从代码提取**、提取不到的标「运行期动态」不臆造。
  实现上修了三处会让"事实"变错的坑：`#[cfg(test)]` 内的假 meta
  （vdfs 被抽成 `fake`）按花括号配对精确剔除；测试模块之后还有生产代码的文件
  （model 的 `.vdfs/model` 挂载点）不能一刀截断；注释剥离必须字符串感知
  （否则 `"https://…"` 被砍断，扫描器还会死循环）。
- **CI 门禁**：`gen-current-facts.mjs --check` 进 ci.yml 与 release.yml——
  改插件/工具/路由忘跑生成器即失败；README / docs/README 指明以生成表为准。
- **ADR-012**：把「读侧成本是设计约束」立为正式决策（写侧省的是改代码，
  读侧成本随读者数相乘；且核对贵过阈值时，模型会开始用自信语气包装
  未验证结论）。

门禁：session 模块 198 单测全过；全库测试 / clippy / fmt 见同日批次复核。

***

## 2026-09-16: 「当前时间（Unix 毫秒）」收敛为 `symbio_core::clock::now_ms`

同一语义此前在**三个层各写了一份，共 7 处**：

| 位置 | 原写法 |
|---|---|
| `symbio_core/turn.rs`（2 处内联） | `time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000` |
| `plugins/session/heartbeat.rs`（`pub(crate) fn now_ms`） | 同上 |
| `plugins/session/plugin.rs`（私有 `fn now_ms`） | `SystemTime::now().duration_since(UNIX_EPOCH)…unwrap_or(0)` |
| `providers/vdfs_service/memory.rs`（私有 `fn now_ms`） | 同上 |
| `plugins/session/chat_session.rs`（`pub(crate) fn now_millis`） | `time::OffsetDateTime` 版 |
| `handlers.rs` / `orchestrator/entry.rs` / `resume.rs` / `types.rs` / `heartbeat_tool.rs` | 内联表达式 |

两种写法对本项目时间范围等价，但**分散在层间意味着改口径时必然漏改**（`plugin.rs`
与 `vdfs_service/memory.rs` 的 `SystemTime` 版本还带 `unwrap_or(0)` 兜底，会静默产生
`1970-01-01`）。现统一为 [`symbio_core::clock::now_ms`](../symbio/src/symbio_core/clock.rs)：

```rust
pub fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}
```

- 模块名取 `clock` 而非 `time`——后者会与外部 crate `time` 同名，使 crate 内
  `time::OffsetDateTime` 的解析产生歧义（同 `plugin.rs` 不能用 `mod vdfs` 的陷阱）。
- 共 **11 个文件、17 处调用点**改为引用同一实现；3 处本地定义 + 4 处内联表达式删除。
- **有意保留**（语义不同，非「当前时间」）：`chat_loop/compress.rs:411`（单位是**秒**）、
  `vdfs/host.rs` / `vdfs/physical.rs` / `vdfs_service/entry.rs` 的测试临时目录名
  （用 `as_nanos()` 求**唯一性**，不是时间戳）、`to_unix` 一族与 `workdir.rs:154`
  （转换**任意** `SystemTime`，不是「此刻」）。

***

## 2026-09-16: session 插件模块拆分（S1–S4）—— 消除 2300 行超长文件

`chat_loop.rs` 达 2401 行、`orchestrator.rs` 1513 行、`plugin.rs` 1480 行——单文件承载
3~6 类不相干职责，靠"文件"这一层已无法表达边界。分四步拆分，**全程零行为变更**
（只搬家：不改任何判定 / 阈值 / 文案 / 执行顺序）：

| 步 | 前 | 后 |
|---|---:|---|
| **S1** | 21 个文件带内联 `#[cfg(test)] mod tests`（合计 3713 行） | 测试外置 ⇒ 生产 **16647 → 12419** 行 |
| **S2** | `chat_loop.rs` 2401 | **434** + `chat_loop/{state,inputs,turn,compress,io}.rs` |
| **S3** | `plugin.rs` 1480 | **523** + `plugin/nodes.rs` 501 + `plugin/vdfs_provider.rs` 512 |
| **S4** | `orchestrator.rs` 1513 | **320** + `orchestrator/{broadcast,consume,entry,failure}.rs` |

**保真纪律**：每步做「归一化代码行多重集比对」——把旧文件与「新父文件 + 各子模块」
归一化（去 `//!` / `use` / `mod` / 空行、去行首缩进，抹平 `pub(crate)` 等可见性前缀，
还原 `super::super::`）后比较行多重集，**差异必须逐条归因**。S2 归因 13 条（6 处路径加深 +
2 处 rustfmt 折行 + 5 行 re-export 脚手架）、S3 归因 21 条、S4 拆分脚本落盘后**归因 0 条**。

⚠️ **S4 复测更正**（2026-09-16 补测）：`cargo fmt --all` 之后再测，S4 为 旧独有 6 行 /
新独有 20 行，**全部可归因**——8 行（4 条语句）是「`super::` 加深 → 超 `max_width`
→ rustfmt 折行」的连锁，6 行是 `impl` 由 1 块拆成 4 块多出的 `impl`/`}`，1 行是新增模块
分工注释，−1 行来自后续 `now_ms` 提交。**教训：保真校验必须在 `cargo fmt` 之后复测**；
归因表比「差集为 0」这个数字更有价值（详见
[session/docs/module-layout.md §4.6](../symbio/src/plugins/session/docs/module-layout.md)）。

**可见性口径**（S2 新确立，S3/S4 沿用）：

- 模块内共享面 = 子模块 `pub(crate)` 条目 + 父模块 `pub(crate) use`；
  ⚠️ `pub use` 要求条目本身是 `pub`，否则 `E0364`/`E0365`。
- 跨模块访问的**字段**必须逐个标 `pub(crate)`——Rust 的**字段可见性不随结构体**。
- 子模块统一 `use super::*;`：子模块可看到父模块的**私有**条目，故父模块的 `use`
  清单即子模块的共享导入面；**只被子模块使用**的导入**不会**触发 `unused_imports`。
- 被搬移代码里的 `super::X::` 需**加深一层**（S2 5 处 / S3 19 处 / S4 12 处）。

**踩过的坑**：① 子模块**不能与作用域内的 `use` 同名**——`plugin.rs` 已有
`use crate::symbio_core::vdfs;`，故子模块取名 `vdfs_provider` 而非 `vdfs`（否则 `E0255`，
且模块内 `vdfs::X` 会解析到自己）；② **自由函数不能落进 `impl` 块**——S4 的
`subtree_of` 若随 `persist_failure` 一起塞进 `impl SessionPlugin`，报
`E0425: cannot find function`；③ 段间的"分节注释"要并入**下一个**块，否则会留在空档里。

评审、目标结构与逐步实施记录见
[`symbio/src/plugins/session/docs/module-layout.md`](../symbio/src/plugins/session/docs/module-layout.md)。
门禁：`cargo test --lib` **544 passed / 0 failed**（基线未减）；clippy `--all-targets -- -D warnings` 零告警。

---

## 2026-09-16: 测试文件扁平化（`X.rs` + `X.test.rs`）与测试归位

**问题**：此前"实现与测试分文件"用的是 `<module>/tests.rs`，于是每个被测模块都多出一个
**只放一个测试文件**的目录——全仓 32 个（`workdir/`、`compression/`、`chat_session/`……）。
目录本身不携带信息，却把"模块"与"目录"两个概念混在了一起。

**改法**：测试文件与实现文件**同级**，同名加 `.test` 后缀，靠 `#[path]` 属性定位：

```rust
// workdir.rs 末尾
#[cfg(test)]
#[path = "workdir.test.rs"]
mod tests;
```

`#[path]` 相对**声明它的文件所在目录**解析，故测试文件与实现同级；**模块路径不变**
（仍是 `workdir::tests`），`use super::*;` 的语义与内联 `mod tests { … }` 逐字一致
——这是一次纯文件搬迁，不涉及任何可见性或导入面调整。

- 32 处扁平化，移除 30 个只为放测试而建的目录；`chat_loop/`、`plugin/` 两个目录**保留**
  （另有真实子模块，目录本身有存在理由）。
- **例外**：模块文件是 `mod.rs` 的（`store/mod.rs`、`plugins/agent/host/mod.rs`），
  测试放同级 `tests.rs`，已是正确形态，不动。
- 验收：逐文件**行多重集指纹**比对（改名前后逐字相同）+ 断言 32 个实现文件都带上
  `#[path]` + `cargo check --tests` 0 错。

**测试归位（高内聚）**：S1 曾把测试搬出实现文件，但 `plugin/tests.rs` 一个文件同时服务
`plugin.rs` / `plugin/nodes.rs` / `plugin/vdfs_provider.rs` 三个实现——**测试一处、被测试
代码一处**，违反高内聚。现按"一个实现文件对应一个测试文件"归位：

| 实现文件 | 测试文件 | 用例 |
|---|---|---:|
| `plugin.rs` | `plugin.test.rs` | 6 |
| `plugin/nodes.rs` | `plugin/nodes.test.rs` | 15 |
| `plugin/vdfs_provider.rs` | `plugin/vdfs_provider.test.rs` | 6 |
| `chat_loop.rs` | `chat_loop.test.rs`（`gate_turn` 契约） | 9 |
| `chat_loop/state.rs` | `chat_loop/state.test.rs`（`StopSignal` + `TurnRequest`） | 9 |
| `chat_loop/inputs.rs` | `chat_loop/inputs.test.rs`（`resolve_system_prompt`） | 6 |

`chat_loop/gate_tests.rs` / `chat_loop/stop_signal_tests.rs` 已并入上表并删除；
用例数**零丢失**（`chat_loop` 24、`plugin` 27）。

**顺带清理**：删除 `symbio/src/plugins/skill/plugin/tests.rs`——自初始提交起就**没被任何
`mod` 声明引用**的孤立文件，它引用的 `SkillPlugin::classify_skill_source` /
`get_skill_detail` 在当前代码里**已不存在**（"技能来源分类"功能整体下线），
故那 9 个用例从未运行、也不可能编译。删除后 `skill/plugin/` 目录随之消失。

**约定入文档**：测试文件布局写进 [`CONTRIBUTING.md`](../CONTRIBUTING.md) §4「测试文件布局」；
`session/docs/module-layout.md` 增补 §4.4（扁平化）/ §4.5（归位）两节实施记录。

---

## 2026-09-16: session 文档下沉插件目录（高内聚）+ 实现与测试分文件

**文档下沉**：session 相关的 9 份文档从系统级目录迁入 [`symbio/src/plugins/session/docs/`](../symbio/src/plugins/session/docs/)
（新建 [索引](../symbio/src/plugins/session/docs/README.md)），落实 `docs/README.md` 早已写明的
「单模块文档放在该模块目录内」原则——此前该原则只在 `turn-tool-mechanisms.md` 上兑现过。

| 原位置 | 新位置 |
|---|---|
| `docs/design/session-core-loop.md` | `session/docs/core-loop.md` |
| `docs/design/session-perf.md` | `session/docs/perf.md` |
| `docs/design/context-compression-design.md` | `session/docs/context-compression-design.md` |
| `docs/design/heartbeat-mechanism.md` | `session/docs/heartbeat-mechanism.md` |
| `docs/design/vdfs-session-messages.md` | `session/docs/vdfs-session-messages.md` |
| `docs/design/cascading-options-mechanism.md` | `session/docs/cascading-options-mechanism.md` |
| `docs/architecture/session-mechanism-audit.md` | `session/docs/mechanism-audit.md` |
| `docs/architecture/session-complexity-audit.md` | `session/docs/complexity-audit.md` |
| `docs/architecture/session-module-layout.md` | `session/docs/module-layout.md` |

去掉冗余 `session-` 前缀（目录已表达归属）。共改写 16 处文档内相对链接 + 14 处外部引用
（`docs/design/vdfs.md`、`README.md`、`cli/docs/usage.md`、`local/README.md`、
`session/plugin.rs`、`session/store/mod.rs`、`tauri/src/schemas/session_meta.ts`、
`tauri/src/services/sessionBusWatcher.ts`、本文件历史条目）。
路径书写约定：**同插件内用插件相对 `docs/x.md`，跨模块用仓库根相对**。

**审计工具扩范围**：`scripts/doc-link-audit.mjs` 原只扫 `docs/`，文档下沉后新位置将不受保护。
扫描根扩为 `docs` / `symbio/src` / `tauri` / `cli` / `examples` + 根目录 `*.md`
（跳过 `node_modules` / `target` / `dist` 等）。覆盖面 172 → 223 条链接。
顺带暴露并修复 `CONTRIBUTING.md` / `CODE_OF_CONDUCT.md` 的 7 条**既有**坏链
（根目录文件却写 `../` 前缀，等同跳出仓库）。

**实现与测试分文件**：session 插件 21 个文件的 `#[cfg(test)] mod tests { … }` 内联块
移出为同目录 `<module>/tests.rs`（沿用 `store/tests.rs` / `chat_session/tests.rs` 既有约定），
生产代码从 16,647 行降到 12,419 行（测试 4,313 行独立）。同时产出
[模块分工评审](../symbio/src/plugins/session/docs/module-layout.md)（`chat_loop.rs` 2400 行
的超长文件问题、目标结构、S1–S4 执行顺序）。

---

## 2026-09-15: 节点隐藏属性（`hidden`）与设置页的插件配置清单

改造三之后，左侧导航多出 `local` / `web` / `gateway` 三个入口——它们的全部内容只有
一份配置文档，没有可浏览的用户资源；而设置页却只剩「外观 / 关于」两项，各插件的配置
文档散落在各自目录里，用户在设置页看不到它们。两件事都由**机制**解决：没有引入新协议
字段，也没有引入「挂载」这个状态。

- **隐藏属性是机制级的节点属性**（`VdfsNode::hidden`）：与文件系统的隐藏属性同义，
  文件 / 目录通用。语义只有两条——**列表里不出现**（父目录 `list` 的结果不含它，
  `children` 计数同理），**可达性不受影响**（按路径 `stat` / `read` / `write` /
  子树操作一概照常）。因此它既不是权限（那是访问位），也不代表节点不在系统里。
  过滤由**产出列表的一方**执行：容器对自己合成的子目录清单、以及任何子 provider
  交回来的 `list` 结果做同一条过滤，标了 `hidden` 的节点不因来自哪个 provider 而异。
- **provider 的根也不过是一个目录节点**：所以「它显示还是隐藏」由
  `VdfsProvider::root_hidden()` 回答——与 `root_access` / `root_status` /
  `root_new_types` 同构，容器在合成该目录节点时回填进 `VdfsNode::hidden`。没有新的
  索引表，也没有「挂载清单」这类中间物。当前标为隐藏的是内容仅一份配置文档的
  `web` / `local` / `gateway`（各自的 `.vdfs/<插件>/PLUGIN.yml` 照常可寻址）。
- **第三条收集通道 `ConfigurableVisitor`**：与能力 / 选项并列，共用同一次 `traverse`
  广播，各有自己的 ctx 键与降级行为。插件在 `traverse` 里调
  `announce_configurable(&ctx, &self.config_file)` 声明「我有一份配置文档」；容器用
  一个**共享**收集器收下（不是像 VDFS provider 那样逐子插件一个——声明自带目录名，
  不存在归属歧义），并把结果**写回请求 ctx**，同一次请求里稍后被委派的 provider
  （即设置插件）据此读到清单：无需反查插件目录、无需硬编码插件表、也不需要协议上的
  新字段。
- **设置清单 = 各插件交出来的配置条目 + 自有分区**（配置在前——那才是用户要在设置页
  动手的东西；`appearance` / `about` 垫后）：条目由 `entry_of(&ConfigFile)`
  生成——名字用**目录名**（列表内唯一，也是前端查图标的键），地址用**真实地址**
  `<目录名>/PLUGIN.yml`，标题 / `ext` / 表单定义取自 `ConfigFile::node()`（定义因此
  只有一份来源）。设置页只是「指路」：点开读写的还是拥有者那份文件，不代理读写、也
  不复制配置。`kind` 留空，由消费方（设置插件）按自己所在的场景填 `setting`。
  图标不进协议——前端按 `kind:<目录名>` 查 `registry/vdfsIcons.ts`（已补
  `setting:telegram`）。
- **词汇口径**：VDFS 下一切都是目录和文件，「挂载」只是动词、不是状态值；节点也不
  需要 `link`——地址由节点自己的 `path` 表达。二者都不进协议。

***

## 2026-09-15: 插件配置回到插件目录（`PLUGIN.yml`），父插件不再代管配置

上一版把配置做成了「一个普通节点」，但**存储仍由父插件代管**：写配置者推切片、
`home` 合并落盘到 `<homedir>/config.yaml` 的 `symbio.plugins.<名>`。于是「一个插件的
配置」横跨三处（home 的合并规则、composite 的分发规则、插件自己的读取），而配置
**却不在插件自己的目录里**——插件目录因此不能整体拷贝移植。

- **一个插件 = 一个目录**：`<homedir>/plugins/<插件>/` 里既有配置（`PLUGIN.yml`）
  也有该插件自己的数据 / 资源，因此整个目录可直接拷贝移植。系统级插件（`home`
  与容器 `composite`）的目录是**系统根本身**，配置在 `<homedir>/PLUGIN.yml`——
  这同时消掉一个自举环：若 `home` 住在 `plugins/home`，容器扫描插件根时会把它当
  普通插件再构造一次，而那个 `home` 又会构造容器。
- **`PLUGIN.yml` 规范**：一个 YAML 映射，身份字段 `plugin_provider`（工厂 id）/
  `plugin_name`（实例名）**不参与配置反序列化**（`PluginDir` 读写时自动剥离 / 补回），
  其余键即插件配置。加载判据 = 文件可解析、且 `plugin_provider` 指向已注册的工厂。
- **容器改为「目录驱动」**：`composite` 扫描**自己目录下的 `plugins/`**（父插件经
  ctx 键 `PLUGIN_DIR` 告知它的目录），逐目录构造并挂载。它**不内置任何插件清单**
  （通用容器，可以嵌套另一个容器）——「必需插件」由构造者经 ctx 键
  `REQUIRED_PLUGINS` 传入（`home` 传的是 `SYSTEM_PLUGINS`），容器只负责把缺失的
  目录 / 配置文件补出来（只补身份字段，缺省字段由插件自己的 `Default` 兜底）。
- **配置归插件，写自己的文件**：`ConfigFile::apply` = 校验 → 落内存 → 落自己的文件
  → 广播。`save_config` 路由、`ConfigSlice` 载荷、`home::merge_slice`、
  `composite` 的向上转发、`ConfigDoc` / `SEG_CONFIG` / `is_config_path` 全部删除；
  `ConfigDoc` 由 `symbio_core::plugin_dir` 的 `PluginDir` + `ConfigFile` 取代。
- **地址从保留段变成真实文件名**：`.vdfs/<插件>/PLUGIN.yml`（原来是
  `.vdfs/<插件>/配置`）。`ext = form` 显式声明覆盖由文件名推导出的 `yml`——
  呈现方式由声明决定，不由文件名猜。网关只读白名单随之改判 `…/PLUGIN.yml`。
- **一次性迁移**：`home` 首次启动读旧 `config.yaml` 的 `symbio.plugins.*`，逐项写
  各插件目录的 `PLUGIN.yml`（目标已存在则跳过），随后把旧文件改名
  `config.yaml.migrated` 留档——天然只生效一次。旧形态里把资源明细混在配置中的
  插件自己消化：`model` 把遗留 `providers` 搬成 `provider.json` 后把配置**归一**为
  只有跨条目状态；`mcp` 搬完 `servers` 后用 `PluginDir::remove_keys` 把遗留键摘掉。
- **顺带**：`model` 的 `parent` 通道（仅用于推切片）随之成为死代码并删除；
  修掉两处既有 clippy 报错（`&mut Vec` → `&mut [_]`、`SettingPlugin::default()`
  → `SettingPlugin`），本地 `cargo clippy -- -D warnings` 恢复全绿。

***

## 2026-09-15: VDFS 机制细节收敛（重复挂载名、`kind` 口径、会话单条定位）

- **重复挂载名不再静默丢一份**：`CompositeVdfs` 收集子目录时改为**先排序、再去重**
  （键 `(order, 目录名, 插件名)`），重名 `warn` 并点名双方。此前 `list("")` 会列出
  两个同名子目录、而路径解析只命中一个——后来者完全不可达，且「谁胜出」取决于
  `HashMap` 的枚举顺序（同 `order` 时完全随机）。
- **删除冗余的 `entry::dir_node`**：它只是 `VdfsNode::dir(..., VdfsAccess::LIST)`，
  且把构造器已写好的 `kind` 再复写一遍。三处挂载根（`single_file` / `memory` /
  `dir` 的 `stat("")`）直接调用构造器。
- **文档**：`vdfs.md` §3.2 明确 `kind` 只有一个词表（`dir` / `file` 只是构造器给的
  **缺省场景标签**，可被插件名等覆盖），目录性永远只由 `l` 位表达；并列出全部机制
  字段名为**保留字**（`attributes` 会 flatten 到顶层，场景字段不得与之同名）。
- **会话单条定位不再全量读**：`SessionStore` 新增 `load_session_checked`（未命中给
  `None`），`session_of` 由「`list_sessions()` 全量读并解析后 `find`」改为按 id 直取
  ——`stat` / `read` 单个会话的开销不再随会话总数增长。`load_session` 改为
  `load_session_checked` + 缺省空会话，语义不变。

***

## 2026-09-15: 插件配置地址化，`CONFIG_GET`/`CONFIG_SET` 协议废弃

插件配置过去是一条私有路由（`<插件>/config/get|set`），并由 `setting` 插件**代理 +
硬编码映射** 4 个插件分区——于是同一份配置有两个地址（`.vdfs/setting/session` 与
`session/config/get`），`setting` 必须认识每个插件名，字段定义与校验也寄居在它那里。

- **配置 = 一个普通节点**：`<挂载根>/配置`（`ext = form`、`rw`、`schema` = 该插件
  自己的详情定义）。读写用的就是 `vdfs/read` / `vdfs/write`，与任何其它资源同一条
  链路——因此前端与 LLM 用同一种方式改配置。定义与校验**回归配置的拥有者**，
  默认值从各自的 `Default` 读出（不再有第二份 schema 字面量）。
- **落盘靠推送**：写配置者把自己的切片推给宿主（`save_config` 载荷
  `ConfigSlice { plugin, config }`），`home` 按 `plugin_provider` 定位既有条目后
  **逐键合并**再原子落盘，**不再反向拉取**任何插件的配置。切片里没有
  `plugin_provider` 的插件（model / mcp）其配置**就是**它们的资源树，不另设配置文档。
- **`setting` 瘦身为无状态 provider**（794 → 约 250 行）：只剩 `appearance` /
  `about` 两个前端自持分区。**「设置」页不再聚合插件配置**——会话 / 本地 / 网络 /
  开放接口的设置现在在各自的挂载点下（`.vdfs/<插件>/配置`），Telegram 首次获得
  可编辑的配置入口。
- **前端**：`DetailForm` 的 `config` 绑定与 `DetailDefinition.load_path/save_path`
  删除（绑定模式只剩 upload / info / option）。

***

## 2026-09-15: 会话存储由「trait + 三后端」收为一个具体类型

`plugins/session/store` 原本是 `SessionStore` trait + `FileSessionStore` /
`SqliteSessionStore` / `InMemorySessionStore`，由配置项 `store_kind` 经
`create_store` 选型。两个前提经取证证伪（`docs/architecture/session-*-audit.md`
早已判定 `store_kind` 为「可配置但不可用」的死机制）：sqlite 前端零引用、默认恒为
`file`、不支持子会话清单、且仍需一个磁盘目录放压缩存档；memory 表达的不是「另一种
存储」而是「要不要持久化」。

- **删除** `sqlite.rs`（199 行）与 `memory.rs`，`store` 收为单文件实现 +
  一个 `tests.rs`；`SessionStore` 由 trait 变为具体类型，落盘 / 不落盘是构造选型
  （`SessionStore::new(base)` / `SessionStore::ephemeral()`），`async_trait`、
  `create_store` 工厂、`Arc<dyn SessionStore>` 一并消失。
- **删除** `StoreKind` 枚举与 `SessionConfig::store_kind` 字段（与既有
  `storage_dir` / `session_id` 同一处置：旧配置残留键由 serde 静默忽略，无迁移）。
  `rusqlite` / `tokio-rusqlite` 依赖随之移除。
- **寻址接入宿主层**：`paths::safe_id` 不再自带一份规则，委托
  `providers::vdfs_service::entry::safe_segment`；`session_storage_dir()` 委托
  `entry::category_dir(PLUGIN_SESSION)`。会话目录名与 VDFS 资源条目目录名从此
  同一份规则（顺带把 `.` / `..` / 控制字符防护带进会话侧——旧实现只替换 `/ \ :`）。
- **刻意不改用 `vdfs_service` 三型**：`DirVdfs` 的「条目内部可下钻」会把
  `session.json` / `messages/` / `tool_archives/` / `transcripts/` 变成对外地址，
  而会话要求 `<id>` 是叶子、内部只以人读语义段呈现；消息内联在 `session.json` 里，
  条目不是文件字节。理由与规范 §13.4「目录自管的类型自己落盘」同一条判据。
- **新增测试**：截断 JSON 自愈、原子写不留 `.tmp`、恶意 id 不得逃出存储根、
  临时与落盘两种驻留方式契约逐条一致。

`cargo test --lib` 495 passed；`clippy --all-targets` 零告警；`fmt --check` 干净。

***

## 2026-09-15: 废除 storage_service，资源存储收敛为 VdfsProvider 的三个集中实现

**两套并行的资源访问抽象合并为一套**：上一条目（S16）删掉了「差异集中在一张 trait」的
适配层，但落盘那一层仍是与 VDFS 并行的私有抽象——磁盘资源用 `EntityStore` 的
`list_entities` / `read_entity` / `write_entity` 表达，再由每个插件手翻成
`VdfsNode` / `VdfsContent`。本次把这一层也换成讲 VDFS 的话的实现（决策见
[DECISIONS.md](./DECISIONS.md) ADR-011，机制定位见
[design/vdfs.md](./design/vdfs.md) §11 与 §13.4）。

- **删除**：`providers/storage_service`（`StorageService` / `EntityStore` /
  `EntityStoreError` / `FileEntityStore` / `path_resolver::safe_id`，含一份从未参与
  编译的孤儿文件 `entity_store.rs`）；`symbio_core/entities.rs` 的 13 个存储原语自由
  函数与 `EntityError`；`symbio_core/providers/storage.rs`（trait + `categories` /
  `manifests` 常量，类别段名从此就是插件名 `PLUGIN_*`，主文件名写在各插件内部的
  `const MANIFEST`）；`symbio_core/schemas/entities.rs` 里的 `EntitySummary` /
  `EntityUploadResponse` / `EntityExport` 与 `ENTITY_MODEL`…`ENTITY_SETTING` 常量
  ——该文件收敛为纯 `DetailDefinition` 表单方言模块。
- **新增** `providers/vdfs_service/`：三个**本身就是 `impl VdfsProvider`** 的集中实现
  ——`SingleFileVdfs`（一个条目 = 一份主文件，条目内部不外露；消费者 `model`）、
  `DirVdfs`（一个条目 = 一个目录，可下钻，主文件承载内容；消费者 `skill` / `mcp`）、
  `MemoryVdfs`（条目只在进程内；消费者 `model` 的 VDFS 清单镜像）。共享的寻址与落盘
  原语在 `entry.rs`（`category_dir` / `safe_segment` / `entry_dir` / `split_rel` /
  `id_of` / `pack_name_of` / `Entry` + 读写删），整包 zip / base64 与导出载荷
  `VdfsPack { id, filename, b64 }` 在 `pack.rs`。**不是新抽象**：没有 trait、没有
  注册表、没有适配器，差异（呈现、写前校验、写后内存同步）仍在各插件的 `impl` 里由
  调用点以普通参数传入；也**不走** `create_object` 工厂（不存在第二种实现）。
- **磁盘布局一个字没改**：仍是 `<homedir>/plugins/<category>/<id>/<manifest>`，三型
  只是三种访问拓扑，换拓扑不动数据。因此对外**零变化**——地址、`ext`、`schema`、
  访问位、导出包的线上字段全部原样。
- **事件通道收敛为一条 `kind = "vdfs"`**：`event_bus.rs` 的 `KIND_ENTITY` 与
  `publish_entity_changed` / `publish_entity_status` / `try_publish_*` 全部删除。
  生命周期与运行时状态变化一律经 `vdfs::host::notify_change` 广播，由 provider 的
  `watch` 经门面补成展示地址后以 `VdfsChangeEvent` 下发。
  **明确代价**：`notify_change` 只报 `(kind, path, change)` 三元组、不带载荷（不为此
  扩展 core 协议），故 `created` / `updated` / `deleted` 由消费者**防抖重拉**收敛；
  带载荷的增益投递只存在于 provider 自己实现的 `watch` 里。
- **协议层零改动**：`symbio_core/vdfs_provider.rs`（纯接口）与 `plugins/vdfs/*`
  （协议 / 访问层 / 物理层）本次未修改——`vdfs_service` 是对该接口的实现，不是扩展。
- **前端连带改动**：`services/eventBus.ts` 的 `KIND_ENTITY` 与实体生命周期 / 状态
  分支退场，清单与状态角标一律订阅 `kind = 'vdfs'`；资源协议面不变。
- **文档**：删除废止 stub `design/entity-provider-mechanism.md`（内容并入 `vdfs.md`
  §13.4 与 ADR-010/011）；`archive/` 两份实体机制档案补终局说明并修失效引用；
  `vdfs-frontend.md` 新增 S17；`PROTOCOLS.md` 删除「统一实体管理」整节并新增工厂
  适用边界；`ROUTES.md` 删除 `{plugin}/entities/*` 各节；`DATA_FLOW.md` 资源链路
  补「落盘在哪一层」一跳；`CONFIGURATION.md` 删除代码中并不存在的 `storage.backend`
  配置项、改为如实描述各类数据的落盘位置；`SYSTEM_MAP.md` 与 model / home / agent /
  setting / composite 五个插件 README 同步现状口径。

## 2026-09-15: 废除实体提供者机制（各插件直连 VDFS）+ 修复 SKILL.md 保存即损坏

**VDFS 收敛终局**：删除 `EntityProvider` trait（20 个钩子）、`provider_registry()` 注册表与
`EntityVdfsAdapter`（1504 行）。每个资源插件**直接实现 `VdfsProvider`**，用现有的
`list` / `stat` / `read` / `write` / `delete` / `action` / `watch` 表达自身语义；
`vdfs_provider.rs` 未做任何修改。跨插件共享的只剩 `symbio_core/entities.rs` 的
**存储原语自由函数**（写盘 / 删除 / 导入 / 导出，无 trait 约束）。

- **对外无变化**：挂载名（`.vdfs/model` / `.vdfs/skill` / `.vdfs/agent` 等）、节点形状、
  `ext` / `schema` / 访问位全部保持原样，前端零改动。
- **本次补上直连实现**：`model` / `skill` / `agent`（`session` / `setting` / `mcp` 此前已直连）。
- **实时能力不丢失**：适配器的变更广播提到 `vdfs/host.rs`（`notify_change` / `watch_changes` /
  `unwatch_changes`），**按 kind 全局持有**——同一 provider 每次 `traverse` 都会新构造，
  按实例持有会让订阅与投递配不上对。
- **修复：SKILL.md 保存即损坏**。表单保存链路把 Markdown 当 JSON 值写入，落盘内容变成
  `"---\nname: ...\n"`（外层引号 + 换行被转义成字面两字符），而读取端 `parse_skill_md`
  以 `strip_prefix("---\n")` 起手 ⇒ 必然失败：**保存报成功、文件却是坏的**，下次加载解析不出
  frontmatter，详情表单也读不到字段值。新增纯文本写盘原语 `entities::write_entity_text`
  （Markdown 主文件专用；`write_entity_manifest` 专用于 JSON 主文件），并以「写进去的必须能被
  读回来」回归测试钉住。zip 整包导入路径不受影响。
- **已删除的失效代码**：`EntityStatusResponse`、`ENTITY_STATUS_CONNECTED` /
  `ENTITY_STATUS_FAILED`（随 `test_status` 钩子一起失去用途）。
- **文档**：`design/entity-provider-mechanism.md` 归档（原路径留废止 stub 指向 `vdfs.md` §13.4）；
  `vdfs.md` §13.4 / `DECISIONS.md` ADR-010 / `vdfs-frontend.md` S16 同步；新增
  `scripts/doc-link-audit.mjs`（站内相对链接审计，默认只报告、`--strict` 才拦截）。
- **验收**：`cargo check --lib --tests` 0 error / 0 warning；`cargo clippy --workspace --all-targets`
  零告警；`cargo test --workspace` **464 passed / 0 failed**；`dead-code-audit` 0 文件 / 0 行。

## 2026-09-13: Session 插件机制收敛（配置契约单一真源 + 会话/骨架化实现去重 + 存储后端补齐）

对 session 插件做了一轮机制审计并据其落地（取证记录见 `symbio/src/plugins/session/docs/mechanism-audit.md`，
早期复杂度审计 `symbio/src/plugins/session/docs/complexity-audit.md` 已归档为历史版本）。以下为**对外可见**的行为/配置变更：

- **`max_tool_rounds` 默认值 `15` → `0`（`0 = 不限制`）**：此前配置面声明"默认 15"与 README 宣称的
  "实质无上限"互相矛盾，且 `chat_loop` 另持一份 `unwrap_or(15)` 兜底。现默认值、schema 描述、
  请求视图三者同源，契约翻译唯一入口 `SessionConfig::model_chat_max_tool_rounds()`（`0 → None`）。
  行为影响：未显式配置过的会话不再在第 15 轮工具调用处熔断。
- **`max_messages` 去掉 `.max(500)` 硬下限**：设置面板中小于 500 的值此前静默失效，现按用户设定生效；
  `0` 表示不限制（与其余窗口类配置语义一致）。
- **`context_messages = 0` 不再越界 panic**：`prune_historical_tool_calls` 对 `keep_turns = 0` 早返回。
- **`tool_context_window` 与 `context_messages` 解耦**：工具结果骨架化/保留窗口只由
  `tool_context_window` 控制，不再被 `context_messages` 连带关闭（此前两者门控纠缠导致"只调一个
  参数却同时改变两条链路"）。
- **新增 `prune_tool_history`（默认 `true`，保持原行为）**：置 `false` 时存储严格保留完整原文，
  工具链裁剪完全交给请求视图（不落库）。
- **轮次淘汰不再物理删除工具归档文件**：FIFO 淘汰与存储期 prune 均只删消息节点；
  `tool_archives/` 的磁盘生命周期唯一归 L0 守卫的 `TOOL_ARCHIVE_KEEP` 滚动策略（消除"配置说保留
  归档、实际文件已被删"的矛盾）。
- **`SessionConfig.session_id` 字段删除**：全仓零读取的死字段（会话身份由目录名/请求注入决定）。
  旧 `session_config.json` 中残留的该键被 serde 静默忽略，无需迁移。
- **fade（老旧工具结果淡化）阈值迁入配置**：`fade_activate_rounds`（默认 40）/
  `fade_keep_recent_turns`（默认 12）取代 `chat_loop` 内的硬编码常量。
- **`store_kind` 新增 `memory` 且 sqlite 后端纳入回归**：`store_kind` 此前"可配置但 sqlite 无测试、
  memory 未接线"；现三种后端共享同一份 `SessionStore` 契约测试。
- **`resume` 与 `message` 互斥**：`session/chat` 同时提供两者时显式报错，不再出现"user 消息被静默
  落库、但请求实际走 resume 分支"的半生效状态。
- **内部结构收敛**（无外部行为变化）：会话引擎实现 `impl ChatSession for` 由 4 降到 1
  （`EphemeralChatSession` / `FallbackChatSession` 删除，临时与降级会话改为
  `PersistentChatSession` + `InMemorySessionStore`）；头尾切分/截断机制唯一化到 `session::text_split`；
  `config_schema()` 从 142 行降到 83 行且不含任何 `default` 字面量；`orchestrator.rs` 三分支重复的
  会话派生字段收敛为 `req_base`；`workdir` 候选路径列表提取为单一常量。
- **验收**：`cargo check --workspace` 0 error / 0 warning；`cargo clippy --lib --tests -- -D warnings`
  零告警；`cargo test --lib` **337 passed / 0 failed**（基线 278 → 313 → 337，新增 59 个用例）。
  关键缺陷均做了变异验证（把修复回退后对应测试确实 FAILED），非恒真断言。

### 同轮收尾：两项"需用户决定"的遗留实施项（2026-09-13 授权实施）

审计中明确标注"超出本次授权范围、需单独决策"的两项，经授权后已实施：

- **`load_history` 序列化行为归一**（复杂度审计 §8.2-P1⑤ 原刻意保留项）：`model_chat::Request` 中
  它是唯一**缺少** `skip_serializing_if` 的 `Option` 字段——同结构体内自相矛盾：`None` 时其他可选
  字段消失、它却输出 `"load_history":null`。现补齐属性，使序列化**键集恒定**（`None`/`Some(true)`/
  `Some(false)` 三态均可无损往返）。语义零变化（`None` 与 `Some(true)` 本就同为"加载历史"）；
  该结构体为 session→model 的**纯进程内**契约，无 TS 对应文件、不落库、`cli/` 与 `tauri/src-tauri/`
  均不引用，故无跨语言兼容风险。补 3 个 serde 契约测试，并用穷尽结构体字面量（新增字段即编译失败）
  锁定契约；变异测试确认有拦截力。
- **批次 D：`run_chat_loop` 拆分**（机制审计 §4-批次 D 原暂缓项）。前置条件为"watchdog 与
  `stop_session` 竞态"，经取证确认为**真实缺陷**，先修复再拆分：
  - **竞态修复**：消费循环的提前出口（watchdog 超时、业务 Error 帧）会跳过 `ai_control_tx = None`
    清理，留下指向已关闭通道的**陈旧 sender**；而 `handle_abort` 恰以 `ai_control_tx.is_none()`
    作为子任务退出判据 → abort 必然空等 3s 兜底、Abort 帧投递到死通道被静默丢弃。新增
    `AiControlGuard`（`Drop` 守卫，与既有 `WorkingGuard` 同型）：登记时快照 request_id，注销时
    仅当仍是本轮登记才清除——**任何出口（含 panic）都不可能跳过清理**，且不会误伤下一轮的新登记。
    配套 3 个回归测试（Drop 注销 / disarm 不重复注销 / 陈旧守卫不误伤新登记），变异测试双向验证。
  - **拆分**：`run_chat_loop` 640 行 → **404 行**（纯骨架：装载上下文 → 消费流 → 收尾分派），
    提取 `close_turn`（241 行：截断续写 / 主动压缩拦截 / 工具分发 / 父节点状态落库 / 停等判定，
    以 `TurnFlow::{NextTurn,Finish}` 回传循环决策）与 `run_context_compact`（压缩执行）。
    **拆函数不拆行为**：搬移段与拆分前逐行比对，241 行区间仅 11 处差异，全部为机械改写
    （借用形式 `&mut out` / `&channel`、出口 `continue`→`NextTurn`、`return Ok(())`→`Finish`），
    三条出口路径与拆分前逐一对应；`tool_rounds` 改传 `&mut`（否则计数不推进，已在编译期暴露并修正）。
    文档强调的不变式"终态唯一落库点在 orchestrator"未被搅浑——`finalize_assistant_turn` 调用点
    数量与位置与拆分前一致。
- **本轮验收**：`cargo test --lib` **343 passed / 0 failed**（337 基线 + 3 serde 契约 + 3 守卫回归）；
  `cargo clippy --lib --tests -- -D warnings` 零告警；`cli`、`tauri/src-tauri` 两 crate `cargo check` 通过。

***

## 2026-09-11: ModelProvider 纯 trait 化（ModelProtocol 完全内化进 model 插件 + session 压缩路径走 execute_turn）

- **核心 `ModelProvider` 重写为纯 object-safe trait**（`symbio_core/model_provider.rs`）：方法集 `provider_id()` / `api_protocol()` / `rate_limit_ms()` / `max_context_tokens()` / `effective_context_tokens()`（async，= min(用户设置, 服务探测)）/ `execute_turn()`（async，五态错误映射内聚于实现方）。Session 的模型契约收敛为 `Arc<dyn ModelProvider>` 单一形态；`FinishReason`/`Usage`/`ProtocolEvent` 保留 core，`TurnOutput`/`PluginChannel`/`PluginError` 等既有类型不动。
- **`ModelProtocol` 完全内化进 model 插件**：trait（钩子 get_api_url/get_headers/prepare_request/parse_response_line/ping/query_context_limit，全部收 `&ModelProviderConfig`）、`resolve_protocol_id` 别名表、`MODEL_PROTOCOL_*` 注册常量、`ReasoningConfig`（serde 形态冻结）全部迁入 `plugins/model/`；`symbio_core` 不再导出任何协议概念，`ids.rs` 四常量删除。协议实现文件为纯机械替换（签名 `&ModelProvider` → `&ModelProviderConfig`，参数名 provider → cfg；已验证协议体仅使用 config 同名字段）。
- **新增 `bound_provider.rs`**：`BoundProvider(cfg: ModelProviderConfig, protocol: Arc<dyn ModelProtocol>)` 实现 core trait——身份/限流/上下文参数读 cfg，`execute_turn` 五态机（Aborted/RetryWithoutContextId/Err/RateLimited/Ok→parse_sse_stream）与 `effective_context_tokens`（min(用户设置, query_context_limit 探测)）自旧 core 结构体固有方法原样迁入，语义不变。`model_providers.rs` 删除 `into_model_provider` 构造器，保留为纯持久化 schema。
- **`parse_sse_stream` 闭包化**（`symbio_core/turn.rs`）：泛型 `<P: ModelProtocol + ?Sized>` 参数改为 `parse_line: impl Fn(&str) -> Vec<ProtocolEvent>` 行解析闭包，core 转录机器不再依赖任何协议抽象；>256 字节部分行分支（`try_parse_partial_sse_line`）与 LineProgress 去重逻辑不动。
- **session 压缩路径收敛**：`run_compression_llm` 原手动拼装（prepare_request + execute_post_with_abort + PostResult 五态匹配 + parse_sse_stream）整体替换为单次 `provider.execute_turn(system_prompt, messages, &[], root_id, muted, abort_flag)`；`ChatOrchestrator.provider` 与 CapabilityVisitor 注册槽位改 `Arc<dyn ModelProvider>`；orchestrator 的 provider_id/rate_limit_ms/max_context_tokens 字段访问改方法调用。
- **验收**：cargo check 零警告、cargo test --lib 286 项全绿、clippy 零错误零警告（tools.rs 测试桩同步重写为实现新 trait 的 MockProvider）。

## 2026-09-11: ModelProvider 类型体系统一（合并 Entry/Config 为单一核心定义 + 按上下文单注册 + CapabilityVisitor 简化）

- **核心 `ModelProvider` 由 trait 改为具体结构体**（`symbio_core/model_provider.rs`）：自含身份（provider_id/protocol_id/system_prompt/rate_limit_ms）+ 全量模型参数（model/api_base/api_key/temperature/max_tokens/max_context_tokens/reserved_tokens/timeout_secs/api_protocol/store/reasoning，自 `ModelConfig` 迁入）+ `protocol: Arc<dyn ModelProtocol>` 协议适配器。原 `description` 字段删除（全仓无消费者）。
- **原协议 trait 更名 `ModelProtocol`**：纯钩子集（get_api_url/get_headers/prepare_request/parse_response_line/ping/query_context_limit），全部改收 `&ModelProvider`；`execute_turn` 从 trait 移除，改为 `ModelProvider` 固有方法（prepare_request → execute_post_with_abort → parse_sse_stream），另提供固有委托方法与计算参数 `effective_context_tokens()`（= min(max_context_tokens, query_context_limit)）。
- **删除 `symbio_core::schemas::model::model_config`**：`ModelConfig` 全部字段并入 `ModelProvider`；`ReasoningConfig` 迁入 `model_provider.rs`；`schemas/model/` 模块整体移除。默认 `max_context_tokens` 统一为 262_144（修复原 model_config 307_200 与 ModelProviderConfig 262_144 的分叉）。
- **model 插件按上下文注册唯一生效 Provider**：traverse 从"注册全部 enabled 条目"改为"解析链 ctx[PROVIDER_ID] > default_provider_id > 首个 enabled → 仅注册该 Provider"（含其系统提示词双键注册：id 键 + "default" 键）；`model_providers.rs` 保留为持久化 serde schema（JSON 兼容），`to_model_config` 替换为 `into_model_provider(protocol_id, protocol)` 构造器。
- **CapabilityVisitor 简化**：删除 `ModelProviderEntry` 与 `list_model_providers`；`register_model_provider(Arc<ModelProvider>)`（覆盖语义）+ `get_model_provider() -> Option<Arc<ModelProvider>>`（无 id 参数）；`DefaultToolVisitor` providers 改单槽。
- **session 消费收敛**：`run_chat_loop_task` 解析链（get_model_provider(id) → find is_default → first listed）收敛为单次 `get_model_provider()`；`ChatOrchestrator` 改持 `ModelProvider` + 构造时预计算 context_limit，6 处消费点切换（api_protocol 日志、nudge 阈值、execute_turn、自动压缩触发、压缩溢出守卫、压缩请求）。
- **顺手修复**（非重构引入、新工具链 lint）：gateway/plugin.rs 测试 `create_config` 与 schemas/options.rs 两处 struct/enum literal 省略字段警告、homedir.rs 文档注释 `>` 行首误读为 markdown 引用。
- **验收**：cargo check 零警告、cargo test --lib 286 项全绿（基线 228 → 新增 58，含重写的 tools.rs 单槽覆盖测试）、clippy 零错误零警告。

## 2026-09-11: 会话心跳任务（heartbeat 工具 + CLI 守护模式 + homedir 环境变量优先级修复 + 空闲基线语义修复）

- **会话心跳调度器**（`plugins/session/heartbeat.rs`）：每进程每 15s 扫描本 store 全部会话，对「已启用 + 空闲满阈值（interval + 每会话固定 jitter ≤30s）」的会话注入 `hb_<sid>_<毫秒>` 心跳消息（`meta.heartbeat=true`，提示词作为用户消息进入正常回合管线）。`is_working` 会话绝不触发（防重入）；触发即写内存锚点（防热循环）；锚点首见回退 `session.updated_at`（进程重启/多进程兜底 = 重启追赶）。
- **heartbeat 设置工具**（`plugins/session/heartbeat_tool.rs`）：agent 可调用 `heartbeat` 工具对本会话 `set`（interval ≥10s / prompt / include_history，部分更新）/ `get` / `cancel`（停用保留配置）；配置持久化于 `session.json` 的 `metadata.heartbeat`，写回刷新 `updated_at`。
- **CLI 心跳守护模式**（`cli/`：args/client/main）：`symbio-cli --heartbeat` 驻留进程，为本 homedir 下所有启用心跳的会话触发空闲心跳并渲染状态；与 `-m`/`--repl` 互斥，模式判定顺序 `--heartbeat` 优先。
- **homedir 优先级修复**（`symbio_core/homedir.rs`）：`HomedirRegistry` 优先级改为 **`SYMBIO_HOMEDIR` 环境变量 > bootstrap 文件 > 默认 `<cwd>/.symbio`**（修复前 bootstrap 优先，`--homedir` 被静默覆盖）；新增回归测试 `test_env_var_overrides_bootstrap`。
- **空闲基线语义修复**（`plugins/session/heartbeat.rs`）：空闲基线改为 `max(内存锚点, 磁盘 updated_at)` —— 修复前空闲时钟从「上次触发/上次消息接收」起算，回合结束后 14.1s 即重触发（违反「无活动之后满 interval」契约）；修复后空闲严格从活动结束（最后一次落盘）起算，E2E 43 次触发最小间隔 36.2s 全部合规。新增测试 `idle_baseline_prefers_latest_activity`（全套 286 项测试通过）。
- **文档**：新增 `symbio/src/plugins/session/docs/heartbeat-mechanism.md`（语义契约/调度细节/工具 API/E2E 摘要）；`cli/docs/usage.md` 补守护模式、`--heartbeat` 参数与 homedir 优先级链。

## 2026-09-09: L1 消息压缩豁免与批次保护（ToolCall 参数永久豁免 + 最近 N 条原文保护 + 头尾保留 + 删除死代码路由）

- **删除死代码路由 `session/compress`**：`invoke_compress`（handlers.rs）全仓无任何调用方（tauri 前端、examples、bin 均未引用），且与自动压缩路径保护语义不一致（无"保护最新一条"切分、无角色豁免，误用反而会压缩当前任务指令）。随路由一并删除：`SessionCompressRequest` schema（schemas/session/session_compress.rs）与 mod 声明、plugin.rs 路由分发、ROUTES.md 条目；`ChatSession::compress_messages` 默认实现的 doc 同步。压缩统一走自动路径（`compress_temporary_messages` → 批次覆写）。

- **L1 批次压缩增加"最近 N 条内容节点"原文保护**：`PersistentChatSession::compress_messages` 新增 `keep_recent` 语义（`SessionConfig::compress_keep_recent`，默认 3，serde 默认兼容旧配置）——从尾部倒数最近 N 个 Text/Reasoning 内容节点跳过压缩（ToolCall/ToolResult 不占名额），且最后一条消息永不压缩（与自动压缩 `messages[..len-1]` 保护语义对齐）。自动路径 `compress_temporary_messages` 无需改动即同等受益（保护在批次覆写内部读取配置）。

- **L1 单消息压缩 ToolCall 参数永久豁免**：`compress_message` 对 `msg_type == ToolCall` 直接返回 None 不压缩不写存档——工具调用参数被骨架化后，模型在请求视图里看到"自己上次执行了一个参数为存档占位符的 edit"，会误记自身行为。新增测试 `toolcall_args_never_compressed`。

- **L1 保留策略从"仅尾部"改为"头尾保留"**：对齐 L0 `split_head_tail` 策略——保留头部 1/4 行 + 其余尾部行（首行常含结论/路径/计划骨架，仅留尾部会挤出关键头部），单行超长退化按字符截断行首；压缩头文案同步为「保留开头 N 行与结尾 M 行内容」。新增测试 `long_line_under_token_cap_not_compressed`，`normal_messages_unaffected` 断言同步头尾格式。

- **配置新增**：`SessionConfig::compress_keep_recent: usize`（默认 3），`session/config/schema` 自动透出。

## 2026-09-09: 上下文压缩体系 P1-P2（存档迁移 + 取回协议统一 + JSON 语义摘要 + 工具输出瘦身 + 快照版本指纹）

- **P1-1 L0 工具结果存档迁移至会话目录**：`guard_tool_result` 的全文存档从系统临时目录迁至 `<homedir>/plugins/session/<safe_id>/tool_archives/`（`safe_id` 将 `/\:` 替换为 `_`；session_id 缺失或目录创建失败时回退临时目录）——工具结果语义上是会话资产，历史写入临时目录会被 OS 清理造成死链。文件名改为 `tool_{毫秒}_{token数}_{内容FNV指纹}.txt`（FNV-1a 64 取高 32 位 hex），杜绝旧实现"同秒同 token 数互相覆盖"的碰撞；每次写入 best-effort 清理旧档，按修改时间保留最新 `TOOL_ARCHIVE_KEEP=20` 个文件。`guard_tool_result` 签名增加 `session_id: Option<&str>`，调用点（tool_executor）从 ctx 的 `SESSION_ID` 取值传入；新增 `archive_into_dir`（目录注入，供测试）与 `resolve_archive_dir`。新增测试 `same_milli_same_tokens_do_not_collide` / `prune_keeps_only_latest_files` / `archive_prefers_session_dir_when_session_id_given`。

- **P1-2 三层压缩取回协议统一**：L0 占位与 L1 压缩头统一追加取回提示「取回：local/file_read 该路径，按 offset/limit 分段读取」——此前各层只给存档路径不给取回方法，模型需自行猜测。L3 fade 按设计不写存档（`archive_path` 为 none）不变；L0（`[... ...]`）与 L1（`<!-- -->`）前缀标识保持各异，供 `decompress_message` 识别防重复压缩。

- **P2-1 骨架化摘要 JSON 语义感知**：`context_window.rs` 的 `first_line_digest` 在首行以 `{`/`[` 开头时改走 `json_digest`：解析 JSON 后提取 count/total/total_count/size 等计数字段与 entries/results/items/data/files 首数组首项的关键字段（name/path/file/title/id/type/status），输出 `count=16,name=main.rs,...` 形式替代盲切片——长 JSON 结果被骨架化后仍保留行数与内容类型等语义线索。数组不占计数；数组兜底 `items=元素类型列表`、对象无计数字段兜底 `keys=键名列表`；整体仍受 `SKELETON_DIGEST_TOKEN_CAP=48` 截断。新增测试 `json_result_gets_semantic_digest`。

- **P2-2 目录列举结果瘦身**：`dir_list` 的 entries 从 `{name,type,size,modified}` 精简为 `{name,type}`——size/modified 对模型定位目录结构无增益且逐条挤占 token；目录优先排序与 MAX_ENTRIES 上限不变。glob（file_search）结果本就是纯相对路径数组，无需改动。

- **P2-3 快照协议版本与提示词指纹**：新增 `COMPRESSION_PROTOCOL_VERSION="v2"` 常量与 `compression_prompt_fingerprint()`（对 `get_compression_prompt()` 全文取 FNV-1a 64 高 32 位 hex）；被动压缩与主动 `run_context_compact` 两处快照 meta 注入 `protocol_version` / `prompt_fingerprint` 字段——离线审计快照时可确认由哪版协议与提示词产出，提示词后续演化不再造成快照溯源歧义。meta 增字段安全（`should_start_compression` 只读 post_tokens 做 ×1.15 迟滞）。

- **测试口径**：`cargo test --lib` **246 passed / 0 failed**（本轮新增 5：guard 3 + context_window 2）；`cargo clippy --all-targets` 0 警告 0 错误。

***



- **骨架化参数定位锚点（链路保持）**：窗口外 ToolCall 骨架化时参数占位符保留关键定位参数回声（`path`/`command`/`url`/`pattern`/`file_paths`/`query`，单条约 10 Token，新增 `anchor_of_args`）——否则"读过某文件第 N 行"这类结果摘要因缺失文件路径而无法回溯，历史逻辑链路断裂。新增测试 `skeletonized_call_and_result_keep_anchor_param` / `skeletonized_without_anchor_falls_back_to_generic`。

- **单行长内容压缩失效修复（bug）**：单行大 JSON/URL/base64 原先绕过两层防线——L0 守卫 `split_head_tail` 首行永远整行保留（超预算不生效）；L1 脱水仅按行数触发（行数=1 不触发）。修复：L0 head/tail 超预算时按字符截断兜底；L1 触发条件增加"单行超长 token 超预算"（`message_archive.rs`，保留内容再按字符截断）。新增测试 `single_long_line_message_is_token_capped` / `single_long_line_is_char_truncated`。

- **策略保留优先级修复（bug）**：`context_window.rs` 的 `is_stale` 原逻辑全局窗口判定优先于工具级保留策略，导致 LastOnly 工具（todo_write）的最新调用滚出全局窗口（15 个 ToolCall）后被骨架化，模型丢失任务清单引发重写。修复后**策略保留优先于全局窗口**：声明 `LastOnly`/`LastN` 的工具其最近 N 次调用即使滚出全局窗口也完整保留；未声明策略的工具仅受全局窗口约束。新增测试 `last_only_latest_call_survives_beyond_global_window`。

- **骨架化"整条丢弃"改为"保留一行摘要"**：新增 `SKELETON_DIGEST_TOKEN_CAP=48` 与辅助函数 `is_failed_result`（结构化优先：`meta.success` 存在即直接采信短路返回，避免"0 failed tests"文本误判）、`error_digest`（failure_kind + 工具名 + 首行原因）、`first_line_digest`、`truncate_tokens`（CJK 友好，字符预算 = token×2）。占位符：失败 → `[System Info: Tool result failed: {kind} ({tool}): {cause}. Full output skeletonized.]`；成功 → `[System Info: Tool result received successfully. Output skeletonized. Summary: {首行}]`。新增测试 `skeletonized_failure_keeps_error_digest` / `skeletonized_success_keeps_first_line_digest` / `failure_detection_prefers_structured_meta`。

- **todo_write 结果瘦身**：返回值从 `{success, count, todos(全量), markdown(全量渲染), message}` 改为 `{success, count, message}`——全量清单对当轮是重复（输入参数刚写过），历史由 LastOnly 策略保证最新一次完整保留。确认 tauri 前端无专用渲染依赖，瘦身安全。

- **压缩提示词强化**：`compression.rs` `get_compression_prompt()` 追加高信噪比规则：跨区块去重（同一事实只出现一次）、只留结论丢过程度量（行数/字节数/读取范围/报错转储）、错误只留"结论+原因"一行、可低成本核实的疑问先核实再入 open_questions、有 todo 清单时 in_progress 引用不复述。

- **Clippy 清零**：修复 rust 1.93 新 lint 全部 22 个警告（needless_borrow ×12 / doc_lazy_continuation ×3 / empty_line_after_doc_comments / bool_assert_comparison / field_reassign_with_default / needless_range_loop / question_mark / single_match / too_many_arguments 加 `#[allow]`），新代码零警告，`cargo clippy --all-targets` 0 警告。

- **测试口径**：`cargo test --lib` 实际执行 **236 passed / 0 failed**（此前 README 的 239 为 ripgrep 统计误差，以 cargo 执行为准）。

***

## 2026-09-07: 文档体系重构（文档下沉原则落地）

- **确立"文档下沉"原则**：单模块文档放模块目录内（`README.md` + 可选 `docs/`），系统级文档只保留跨模块核心逻辑并引用模块文档；每篇职责一句话见 [README.md](./README.md) 的"模块文档地图"。

- **模块文档全覆盖**：14 个插件 `symbio/src/plugins/*/README.md` 全部就位（新增 agent / local / web / gateway / home / composite / setting / hook / event_bus / skill 十篇；重写 model——Phase E 后 model 为无状态单轮 LLM 网关 `execute_turn`，旧"工具执行/审批流分发器"描述作废）；前端新增 [tauri/README.md](../tauri/README.md) + [tauri/docs/FRONTEND.md](../tauri/docs/FRONTEND.md)。

- **历史实施日志归档**：`symbio/docs/model-session-refactor.md`、`turn-tool-mechanisms.md` 原文移入 [archive/implementation-logs/](./archive/implementation-logs/)（加状态横幅）；机制现行版精简下沉为 [session/docs/turn-tool-mechanisms.md](../symbio/src/plugins/session/docs/turn-tool-mechanisms.md)；`symbio/docs/` 目录清空。

- **去重**：[session/docs/context-compression-design.md](../symbio/src/plugins/session/docs/context-compression-design.md) 瘦身为 L0-L6 分层总览 + 取舍原则 + 不变量，各层阈值与实现细节归 [session/README.md](../symbio/src/plugins/session/README.md)；[OVERVIEW.md](./architecture/OVERVIEW.md) 删除 agent 模块内部细节章节，"插件不各自维护文档"的旧约定改写为下沉原则。

- **口径修正**：单元测试数以实际统计为准修正为 **239**（原 README 355 / 重构日志 227 均不准）；README/SYSTEM_MAP 同步 model 与 session 职责新表述。

***

- **只读模式不再放行网关自身配置**：`is_readonly_allowed` 从未匹配 `gateway/config/get`
  （白名单只有精确 `config/get` 与前缀 `config/get*`），而 `config.rs` 的单测却断言它放行，
  该测试一直失败。按"网关配置含 `inbound_token`，只读模式下放行等于把令牌读走"的判定，
  确认**拒绝**为正确语义：测试改为断言拒绝，并在函数文档与
  [CONFIGURATION.md](./reference/CONFIGURATION.md) 写明理由。
  **验证**：`cargo test --lib gateway::config` 3 passed / 0 failed。

- **新增设计稿** [session/docs/context-compression-design.md](../symbio/src/plugins/session/docs/context-compression-design.md)：
  针对"长对话上下文超限导致会话中断"，盘点现有四层压缩（轮次窗口 / 工具结果窗口 /
  单条消息存档 / 自动摘要），定位 7 条根因（其中 `finish_reason` 全链路未解析、
  token 估算误用 UTF-8 字节数、`max_tokens` 默认值与模型能力脱钩为 P0），
  给出"四层 token 预算模型"与 P0~P3 落地计划。

- **P0 已实现**（同日，设计稿状态同步更新）：
  - `symbio_core/tokenizer.rs`：纯 Rust 启发式 tokenizer（按字符类别加权、`chars()` 口径
    修复中文低估 2 倍的字节估算 bug）+ `CalibratedTokenizer` 用 provider `usage` 滑动校正
    （反馈必须用原始估算 `count_raw`，否则自反馈收敛到 √(真实比值)）。
  - `ProtocolEvent` 新增 `Finish` / `Usage`；**四个协议全部解析**（openai_chat /
    openai_responses / anthropic / gemini），`TurnOutput` 贯通。
  - `chat_loop`：主循环改 `loop` **移除 `max_tool_rounds` 硬性上限**（默认无限轮次，
    仅显式配置时作软上限并明确提示）；`finish=Length` 纯文本截断→自动续写（≤3 次），
    工具参数截断→明确报错；截断 Turn 落 `meta.finish_reason="length"`。
  - L0 `tool_result_guard.rs`：超 8192 token 的工具结果存档 + head/tail 摘要；
    `fade_aged_tool_results` 老化淡化：超 40 轮后仅压缩早于最近 12 个 user turn 的
    工具结果（可经存档取回），**绝不触碰 assistant 文本/推理**（保护思维链）。
  **验证**：`cargo check --lib` 0 警告 0 错误；tokenizer / tool_result_guard 单测 7 passed。

## 2026-09-04: 统一资源管理器动态 provider 注册——六类资源 + 导航/设置全部注册驱动

在 `ResourceProvider` trait 收敛之后，把"哪几种资源、怎么导航、怎么展示"彻底交给注册表：

- **注册表扩展为六类**：`provider_registry()` 新增 `setting`（`supports_upload=false`、
  `compact_list=true`、`nav=settings`）。`ResourceProviderInfo` 与下发 `ProviderInfo` 新增
  `compact_list`（列表简洁模式）与 `nav`（左侧主导航归属 `resources`/`settings`，空串不进导航）。

- **左侧导航全部动态驱动**：`MainLayout` 资源区 `v-for="p in resourceNav"`、设置区
  `v-for="p in settingsNav"`——"设置"入口由注册表动态生成，删掉手写按钮；session 不进导航
  （走独立会话主入口）。新增 provider 只需在后端登记一条 `nav`，导航自动出现。

- **设置页 = 统一资源实例**：删除 `components/SettingsPage.vue`，`/settings` 复用
  `ResourceManagerView`（types='setting'）。5 个分区（appearance/session/local/web/about）经
  setting 插件 `resources/list` 下发，前端按 `kind:ext` 复合键注册各自 editor（
  `tauri/src/components/settings/`），未命中回退 kind 级 / 通用兜底——与文件系统"扩展名决定编辑器"同构。

- **列表尊重服务器返回顺序**：`buildMixedItems` 删除前端 name 排序，改为按 activeTypes 顺序 +
  各类型 `resources/list` 原序（设置分区严格按后端声明顺序展示，不再被中文名重排）。

- **editor 契约清理**：5 个设置表单 `defineOptions({ inheritAttrs:false })` 阻断未消费 props
  落根 DOM；动态 `<component>` 加 `:key` 避免切换资源/类型时表单状态残留。

- **死代码清理**：删除 `schemas/resources.ts` 无引用的 `ResourceType` / `ResourceUploadRequest` /
  `ResourceDeleteRequest` 导出（请求体统一走对象字面量）。

- **验证**：cargo clippy 零告警、363 测试通过；vue-tsc 零错误、vitest 61/61、vite build 成功。

## 2026-09-03: 资源管理页体验修复——表单注册表、路由刷新、主题令牌化

修复统一资源协议落地后的四个回归问题：

- **Model 表单退步修复（详情差异化机制）**：明确"列表统一、详情差异化"架构——
  `ResourceManagerView` 新增 `FORM_COMPONENTS` 注册表按类型注入专属表单组件，未注册类型走
  通用兜底（zip 面板 / JSON 编辑器 / 只读详情）。恢复 `constants/modelProviders.ts`，新建
  `ModelProviderForm.vue`（恢复提供商预设、模型候选、API Key 显隐、启用开关、高级设置折叠、
  校验连接、跳过校验保存、设为默认等全部旧表单能力，样式迁移到新设计令牌）。

- **后端支持表单控制标记**：model 插件 `validate_manifest` 支持 manifest 携带
  `skip_validation`（跳过连接校验，不落盘）与 `is_default`（设为默认，随 manifest 落盘）；
  `on_uploaded` / `load_from_storage` 读取 `is_default` 标记（显式标记 > 现有指向 > 首个可用），
  "设为默认"重启后不再丢失。

- **页面切换列表不刷新修复**：四个资源路由共用 `ResourceManagerView`，Vue Router 复用组件
  实例导致 `onMounted`/事件订阅不再执行、列表残留上一类型数据。`MainLayout` 的 `RouterView`
  加 `:key="route.path"` 强制重建。

- **布局重构**：zip 上传创建面板改为居中卡片式；mcp/skill/agent 详情区新增统一工具栏
  （测试连接/删除按钮同一行右对齐），替代原先散乱堆叠的操作行。

- **主题令牌化**：`ResourceManagerView` / `ResourceShell` / `ResourceDetailPanel` /
  `ModelProviderForm` 全部样式迁移到 `--surface-* / --text-* / --border-* / --accent / 语义色`
  设计令牌，替换硬编码色值与 `rgba(0,0,0,…)`（深色下不可见）——输入框深色下白底、
  hover 叠加失效、状态徽标颜色不适配等深浅色主题问题一并修复。

- **验证**：cargo clippy 零告警、362 测试通过；vue-tsc 零错误、vitest 35/35、vite build 成功。

## 2026-09-03: ResourceProvider trait 化——资源协议公共流程收敛核心层

在统一资源协议之上再做机制收敛：`resources/*` 五操作的公共流程（列表包装、zip/manifest 上传、
幂等删除、状态事件推送）由 `symbio_core::resources::dispatch` 统一承载，各插件只实现
`ResourceProvider` trait 的差异化钩子：

- **核心**：新增 `ResourceProvider` trait（`kind` / `category` / `manifest_file` +
  `list_items` / `summarize` / `validate_manifest` / `on_uploaded` / `on_deleted` / `test_status`
  钩子，多数带默认实现）与 `dispatch` 统一分发入口；插件 route 顶部一行接入，非资源路径返回
  `None` 继续 match。新增 `ResourceGetRequest` schema 与 dispatch 单测（7 例）。

- **五插件接入**：skill / agent / mcp / model / session 全部改为实现 trait 钩子，删除各自手写的
  `resources_*_list/upload/delete/status` 方法。纯目录资源（mcp/skill）只实现 `summarize` 等
  轻钩子；model/session 重写 `list_items` 接管独立数据源；agent 在 `on_uploaded`/`on_deleted`
  中失效索引缓存（顺带修复上传/删除后列表不刷新的潜伏 bug）；mcp 的 `test_status` 连接测试结果
  经 `dispatch` 统一推送 resource 事件总线。

- **前端死代码清理**：删除无引用的 `ModelProvidersSettings.vue` / `ModelProviderCard.vue` /
  `constants/modelProviders.ts`（model 表单已由 `ResourceManagerView` 通用 JSON 表单承载）；
  删除 `services/resources.ts` 死导出 `resourcePath`/`toSummary` 与 `services/modelProviders.ts`
  死导出 `generateUniqueProviderId`（及其 spec）。

- **验证**：cargo clippy 零告警、362 测试通过；vue-tsc 零错误、vitest 35/35、vite build 成功。

## 2026-09-03: 统一资源协议落地（model/mcp/agent/skill/session 五类资源）

围绕"机制统一、最小差异化"重构前端与前后端协议，一份页面实例化多类资源：

- **后端统一协议**：新增 `symbio_core::schemas::resources`（`ResourceSummary` / `ResourceCapabilities` /
  `ResourcesListResponse` / `ResourceUploadRequest` / `ResourceDeleteRequest` / `ResourceStatusRequest`）与
  `symbio_core::resources`（`decode_zip_b64` / `parse_zip` / `strip_common_root` / `extract_zip_to_entity`）。
  model / mcp / agent / skill / session 统一暴露 `resources/list|upload|delete|status`；zip 资源（mcp/skill/agent）
  以文件名即目录名解压到 `~/.symbio/plugins/<category>/<id>/`。

- **能力开关驱动差异**：`capabilities_for(kind)` 成为后端单一真相源，前端 `ResourceManagerView` 按
  `zip_upload` / `independent_form` / `realtime_status` / `mutable` / `test_connection` / `read_only` 驱动 UI。

- **前端统一**：新增 `services/resources.ts` 与 `schemas/resources.ts`；`ResourceManagerView` 一份页面实例化
  model/mcp/skill/agent；删除遗留 AgentView/McpView/SkillView/ModelProvidersView 及 agents/mcpServers/skills 等
  旧 service 与孤儿 schema。

- **session 并入体系**：会话管理列表改用 `worker/session/resources/list` 统一契约（含 is\_working/message\_count/
  metadata 扩展），前端 `listSessions` 由 `ResourceSummary` 映射；`resources/status` 提供实时工作状态轮询。

- **model 收尾（消除双协议）**：chat 侧 `listModelProviders` 改从统一 `resources/list`（`worker/model`）读取——
  列表项 `extra` 展开完整 `config` 与 `is_default`，与资源管理页共用同一入口；`resources/upload` 对齐旧
  `providers/set` 补齐保存前校验 + 落盘；删除遗留 `providers/list|get|set|delete|set_default|test` 六个路由及
  对应前后端 CRUD schema/service（`modelProviders.ts` 仅保留映射 `listModelProviders` 与纯工具函数）。

- **清理**：删除后端孤儿 schema（session\_list/skill\_get/skill\_list/mcp\_servers）与 agent 旧 list/get/delete 路由、
  mcp 旧 `servers/*` 路由、skill 旧 list/get 路由。

## 2026-09-02: 停止按钮失效 + 工具命令白名单误报修复

- **修复停止按钮失效**：`sessions` store 的 `refreshList` 只把后端 `session/list` 返回的权威 `is_working` 合并进 `list`（卡片圆点），未回填 `sessionStatuses`——而停止按钮的 `isLoading` 只读后者。页面重载/视图挂载后，运行中的会话按钮显示为禁用的「发送」，点击无反应。现在 `refreshList` 把后端运行态回填到 `sessionStatuses`（仅 false→true 升级；true→false 收敛仍由事件流 idle/Abort/Error 负责，避免快照竞争误降级）

- **修复工具调用「命令不在允许列表中」误报**：`SecurityPolicy` 默认白名单补充常用命令（flutter/dart、node/npx/pnpm/yarn/bun、powershell/pwsh/cmd、pip、cp/mv/touch/ln/tar、head/diff/sed/awk/which、taskkill/ipconfig/netstat 等）；新增 `normalize_base_command` 归一化——去掉路径前缀与 `.exe/.cmd/.bat` 扩展名（`npm.cmd run build`、`python.exe` 此前均无法匹配白名单，`rm.exe` 的风险等级也会被误判为 Low）；`rm` 等高风险命令加入白名单仅消除误报文案，仍受 `block_high_risk_commands` 策略阻止（附 4 个单元测试）

## 2026-09-02: 工具失败语义收敛（recoverable 标记：信息性 vs 待恢复）

- **问题**：工具失败无条件显示重试按钮并自动展开，但 auto 模式下会话仍在运行——错误结果已喂给 LLM 继续处理，重试无意义（resume 在会话忙碌时也会被后端拒绝）；`retry`/`supply` 实际只为 **interactive 模式**（失败即暂停等用户恢复）设计

- **后端**：`chat_loop` 因工具失败退出循环（needs\_user\_action）时，给触发的 Failed ToolCall 打 `meta.recoverable=true` 并广播+持久化——服务端唯一真相，区分「信息性失败」与「待恢复失败」

- **前端**：重试/补充参数按钮仅对 `recoverable` 失败显示；可恢复失败自动展开（需要操作入口），运行中失败保持单行红标（可手动展开看错误，无按钮）

- **设计取向**：长期看 `supply` 应与 ask\_user/user\_prompt 结构化提问统一（参数修复走表单而非 JSON 补丁），保留但视觉降级

## 2026-09-02: 会话流响应体验重设计（Turn 响应分组 + 两级重试分派）

对齐 Claude / Codex 类智能体 UI，前后端配合（协议改动最小化）：

- **后端**：`session/orchestrator.rs::persist_failure` 失败终态（含 Turn）以 `StreamEvent::Update` 广播（时序在 Error 事件之前）——实时画面改信服务端，前端删除客户端启发式错误标记（刷新前后不再不一致），Error 事件仅承担 transport 级 ephemeral 兜底

- **前端 MessageNode.vue**：

  - 根级助手 Turn 改为**响应分组透明容器**，三形态：① Turn 无子节点且运行中 → 三点脉动「正在思考…」骨架；② 子节点（思考/正文/工具）出现后容器完全隐藏、直接纵排；③ Turn 失败 → 组级错误条 + 重试（`retry_turn`：删响应子树重建）

  - 思考节点**始终单行**：流式中「思考中…」呼吸动效，完成后「思考」+ 摘要预览，点开看全文

  - 工具调用**始终单行**（名称 + 状态标签，失败红标 + 悬停 ↻ 就地重试），例外：内含待审批子节点 / 可补充参数时自动展开

  - 工具卡片展开后**三段式**：请求（参数 JSON）/ 过程（子会话实时流，无子会话的工具整段隐藏）/ 结果（工具返回 + 审批提问），各自独立响应流

  - 正文（助手文本）始终展开；工具结果失败仅在结果段呈现错误文本，重试入口在工具行

- **重试两级分派**（`ModelChatPanel.handleRetry` 按 `msg.type` 路由）：工具失败 → `retry`（仅重执行该工具）；其余失败 → `retry_turn`（整轮响应重建）——两个粒度不再混淆

## 2026-09-02: 会话请求修复（temperature 序列化精度）+ 测试/构建卫生

- **修复 LLM 请求 400（temperature 参数非法）**：`ModelConfig.temperature` / `ModelProviderConfig.temperature` 由 `f32` 改为 `f64`——`f32 0.7` 经 serde\_json 升位 f64 后序列化为 `0.699999988079071`，被限制 2 位小数的模型 API 拒绝；f64 round-trip 精确，默认值恒为 `0.7`

- 修复 `time.spec.ts`「今天」用例硬编码日期随时间腐化（改用 `Date.now()` 相对构造）

- `tauri/package.json` 声明 `"type": "module"` + `vite.config.ts` 改用 `import.meta.dirname`，消除 Vite configLoader native 模式的 ESM/CommonJS 混用警告

## 2026-09-01: 全量依赖升级（Rust / Node / 前端 / CI）

**Rust**（保留精确版本，symbio/Cargo.toml）：thiserror 2.0.20、dirs 6、notify 8.2、fastembed 6.0.2、dashmap 6.2.1、which 8.0.6、reqwest 0.13.4、time 0.3.55、rusqlite 0.37 + tokio-rusqlite 0.7 配套锁定等。代码适配：

- tokio-rusqlite 0.7 移除 `Error::Other`，`call` 闭包直接返回 `Result<R, E>`，10 处调用点改写并显式标注 `rusqlite::Error`

- time 0.3.55 deprecated `format_description::parse`，改用 `parse_borrowed::<2>`（chat.rs / system\_prompt.rs）

- tauri/src-tauri 旧 Cargo.lock 与 rusqlite 0.37 冲突（links = "sqlite3"），重新生成

**前端**（tauri/package.json）：vite 8、@vitejs/plugin-vue 6、vitest 4、pinia 4、vue-router 5、marked 18、katex 0.18、mermaid 11、milkdown 7.22、@tauri-apps/\* 2.11。配置适配：

- vite 8（rolldown 内核）不支持对象形式 `manualChunks`，改为函数形式（vite.config.ts）

- vite 8 不再内置 esbuild，`minify: 'esbuild'` 改为 `'oxc'`

- typescript 保持 5.9：TS 7（native）与 vue-tsc 3.3 不兼容（实测 ERR\_PACKAGE\_PATH\_NOT\_EXPORTED）

**CI**：node 20→22（20 已 EOL）、checkout v5 / setup-node v5 / cache v4（消除 Node 20 deprecation 警告），release.yml 同步升级。

**验证**（2026-09-01）：

- `cargo fmt --check` / `cargo clippy -D warnings`：0 error

- `cargo test --workspace`：355 passed

- tauri/src-tauri `cargo check`：通过

- `npx vue-tsc --noEmit` / `npm test`（18 passed）/ `npm run build`：通过

***

## 2026-09-01: 连接测试 ping 请求修复 + CI 三处门禁修复

**一、模型连接测试报 400（`max_tokens must be greater than 2`）**

- 根因：`handle_ping` 探活请求硬编码 `"max_tokens": 1`，GLM 的 OpenAI 兼容网关要求 `max_tokens > 2`。

- 修复：三个协议（`openai_chat` / `anthropic_messages` / `gemini_api`）的 ping 请求统一调大到 16。

**二、GitHub CI 持续失败（三个 job 各一处根因）**

| job             | 根因                                                                                     | 修复                                                                                                                       |
| --------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| rust-checks     | CI 与本地 rustfmt 版本漂移导致 `fmt --check` 失败；rustfmt.toml 含 5 个 nightly-only 选项被 stable 静默忽略 | `rust-toolchain.toml` 锁定 `channel = "1.93.1"`；CI 改用 `actions-rust-lang/setup-rust-toolchain@v1` 读取该文件；清理 nightly-only 选项 |
| frontend-checks | `setup-node` 的 `cache: 'npm'` 在仓库根目录找不到 lock 文件（实际在 `tauri/`）                          | 增加 `cache-dependency-path: tauri/package-lock.json`                                                                      |
| security-check  | `cargo audit` 报 RUSTSEC-2025-0068：`serde_yml` 不维护且有 soundness 问题                       | 全量替换为维护中的 fork `serde_yaml_ng`（API 兼容，9 文件 20 处）                                                                         |

**验证**（2026-09-01）：

- `cargo fmt --all -- --check`：0 diff

- `cargo clippy --workspace --all-targets -- -D warnings`：0 error

- `cargo test --workspace`：355 passed / 0 failed

- `bash scripts/grep_audit.sh`：0 errors

- `npx vue-tsc --noEmit`：0 错误；`npm test`：18 passed

- Cargo.lock 已确认无 `serde_yml` 残留

***

## 2026-09-01: Model Provider"测试连接"路由修复

**Bug**：在 Model Provider 添加新模型时点击"测试连接"，报错
`Composite: 路径 'model_providers/test' 无法识别或子插件未挂载`。

**根因**：前端 `ModelProvidersView.vue` 的 `handleTest` 硬编码调用了 `model_providers/test`，
但该路径从未存在——Model Provider 管理路由的正确前缀是 `worker/model/providers/*`
（见 `tauri/src/services/modelProviders.ts` 的 `MODEL_PROVIDERS_PATH` 常量），
且后端此前**没有**独立的"测试连接"路由（只有 `providers/set` 会在保存时顺带校验）。

**修复（前后端联动）**：

| 端          | 改动                                                                                                                                                   |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| 后端 schema  | `symbio_core/schemas/model/model_providers.rs` 新增 `model_providers_test` 模块（Request 含 `provider` + `skip_validation`，Response 空）                     |
| 后端路由       | `plugins/model/plugin.rs` 新增 `providers/test`——复用 `validate_provider`（无副作用校验），**不写注册表、不落盘**，因此未保存的草稿配置也能直接测试；失败返回 `ValidationError("连接测试失败: {err}")` |
| 前端 schema  | `tauri/src/schemas/model_providers.ts` 新增 `ModelProvidersTest` namespace                                                                             |
| 前端 service | `tauri/src/services/modelProviders.ts` 新增 `testModelProvider()`（走 `worker/model/providers/test`）                                                     |
| 前端视图       | `ModelProvidersView.vue` 的 `handleTest` 改用 `testModelProvider`，移除 `model_providers/test` 硬编码与不再使用的 `callPlugin` 导入                                   |

**验证**（2026-09-01）：

- `cargo test --lib`：355 passed / 0 failed

- `cargo clippy --all-targets -- -D warnings`：0 error

- `cargo fmt --all -- --check`：0 diff

- `npx vue-tsc --noEmit`：0 错误

- `npm test`（vitest）：18 passed

**附带收益**：`providers/test` 作为无副作用路由，也是后续在设置表单中"实时校验"（输入即测）的稳定后端锚点。

***

## 2026-09-01: 全库质量门禁回归修复 + 前端测试基建

**背景**：项目级质量审计发现三处"门禁失真"：

1. `cargo clippy --lib` 实际存在 **12 个 warning**（文档声称 0）；
2. `cargo fmt --check` 存在 **83 处历史偏差**（CI 的 `--check` 门禁实际从未在当前 toolchain 下通过）；
3. 前端**零测试**（CI frontend job 仅有 vue-tsc + 空 sanity check）。

**变更**：

### 一、Rust 侧（业务行为零变化）

| 修复                                                            | 位置                                                                                                       |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `redundant_closure` ×3                                        | `symbio_core/event_bus.rs`（×2）、`providers/embedding/fastembed.rs`                                        |
| `collapsible_else_if`                                         | `plugins/agent/handlers/system_prompt.rs`                                                                |
| `unnecessary_if_let`（`.flatten()`）                            | `plugins/local/codebase_search.rs`                                                                       |
| `manual_div_ceil`                                             | `plugins/local/file_read.rs`                                                                             |
| `let_and_return`                                              | `plugins/mcp/http.rs`                                                                                    |
| `doc_overindented_list_items`                                 | `plugins/model/message_builder.rs`                                                                       |
| `unnecessary_cast`                                            | `plugins/session/heartbeat.rs`                                                                           |
| `ptr_arg`（`&mut Vec` → `&mut [T]`）                            | `plugins/skill/plugin.rs`                                                                                |
| `derivable_impls`（`#[derive(Default)]` + `#[default]`）        | `symbio_core/schemas/mcp/mcp_config.rs`                                                                  |
| `doc_lazy_continuation`                                       | `symbio_core/schemas/session/session_chat.rs`                                                            |
| `field_reassign_with_default` ×4（**测试代码**，`--all-targets` 门禁） | `plugins/mcp/http/tests.rs`（Default 赋值改结构体初始化语法）                                                         |
| 死代码清理                                                         | `plugins/agent/core/mod.rs::query_relation_names`（零调用）；`plugins/agent/capabilities/mod.rs` 3 个 v10 预留死常量 |
| `cargo fmt --all`                                             | 全库 83 处历史偏差统一，fmt 门禁恢复有效                                                                                 |

### 二、前端测试基建（从 0 到 1）

| 项                                | 说明                                                                            |
| -------------------------------- | ----------------------------------------------------------------------------- |
| 引入 `vitest@2.1.9`（devDependency） | 与 vite 6 对齐的稳定版本                                                              |
| 新增 `tauri/vitest.config.ts`      | `@` alias 与 vite.config 对齐；node 环境（纯逻辑层）                                      |
| 新增 18 个单元测试                      | `src/utils/__tests__/time.spec.ts`（6）+ `message.spec.ts`（12，多模态文本提取 / 消息键稳定性） |
| `package.json`                   | 新增 `test` / `test:watch` script                                               |
| CI（`.github/workflows/ci.yml`）   | frontend-checks job 新增 `npm test` 门禁                                          |

**验证**（2026-09-01）：

- `cargo test --lib`：355 passed / 0 failed

- `cargo clippy --lib -- -D warnings`：0 warning

- `cargo clippy --all-targets -- -D warnings`（CI 同款，含测试代码）：0 error（另修复 `plugins/mcp/http/tests.rs` 4 处 `field_reassign_with_default`）

- `cargo fmt --all -- --check`：0 diff

- `npm test`：18 passed（vitest）

- `npx vue-tsc --noEmit`：0 错误

**设计决策记录**：审计中评估的"认知内核与 Agent 插件壳解耦"（CognitionService trait 化 / crate 拆分）经确认**不采纳**——认知与智能体是一一对应的共生关系，`agent` 插件的"认知中心"内聚形态是设计使然。详见 `symbio/src/plugins/agent/docs/CHANGELOG.md` 同日条目。

***

## 2026-07-06: 项目级文档系统性同步 + 历史文档清理

**背景**：用户两轮反馈：

1. "项目文档与代码实现存在系统性脱节"——`docs/archive/proj/` 下历史规划文件、`docs/archive/design_docs/` 中早期提案、以及 `docs/README.md` 中插件文档列表与实际不符。
2. "继续，注意删除历史过期文档信息或者文档，确保所有文档保持最新"——既然能通过新方案覆盖过期文档，应**直接删除**而非保留横幅标注。

**变更**：

### 一、新增权威改进方案

1. **新增** **`docs/archive/proj/IMPROVEMENT_PLAN_2026.md`**（**权威改进方案**）

   - 基于 2026-07 当前代码状态的项目级下一阶段改进计划

   - 10 个改进方向（P0-P3）：文档脱节修复 / Plugin Channel 跨进程 / MCP 成熟化 / Skill 实战化 / E2E CI 化 / 可观测性 / HNSW ANN / 前后端类型同步 / 外部插件 / 移动端

   - 季度路线图（2026 Q3 / Q3-Q4 / 2027 Q1-Q2 / 2027+）

   - 取代 `PLAN.yml` / `TASK_INDEX.md` 作为项目要做的事的**唯一权威来源**

### 二、`docs/README.md` 修复

- §3 `design_docs/`：删除 `ARCHITECTURE_IMPROVEMENT.md` / `MODEL_CHAT_REDESIGN.md` / `COMPARISON_WITH_QWEN_CODE.md` 链接（已删除）

- §5 agent 插件文档列表：移除不存在的 `PROMPT_ARCHITECTURE.md` / `OPERATIONS.md` / `CODE_ANALYSIS_REPORT.md` 引用

- §6 "早期与产品向文档"：移除 `docs/archive/proj/` 引用（已清理为只剩 `IMPROVEMENT_PLAN_2026.md`）

- 添加"§6.1 现行改进方案"小节，链接到新的 `IMPROVEMENT_PLAN_2026.md`

### 三、删除过期历史文档（用户要求"删除"而非"加横幅"）

| 删除文件                                                     | 原因                                                                                                            |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `docs/archive/design_docs/ARCHITECTURE_IMPROVEMENT.md`   | Skill/Subagent/Hook 提案已**全部落地**（plugins/skill/, agent\_create+agent\_chat 组合, plugins/hook/）                  |
| `docs/archive/design_docs/MODEL_CHAT_REDESIGN.md`        | 扁平消息树设计已落地（`chat_message.rs` + Tauri `MessageNode.vue`）                                                       |
| `docs/archive/design_docs/COMPARISON_WITH_QWEN_CODE.md`  | 报告差异**多数已通过新增/重写插件弥合**，继续保留会持续误导                                                                              |
| `docs/archive/proj/PLAN.yml`                             | 2026-03 早期规划，与当前代码严重脱节                                                                                        |
| `docs/archive/proj/TASK_INDEX.md`                        | 2026-03 早期任务索引，24 个任务多数不适用                                                                                    |
| `docs/archive/proj/tasks/T001-project-infrastructure.md` | 早期任务                                                                                                          |
| `docs/archive/proj/tasks/T002-markdown-editor.md`        | 早期任务                                                                                                          |
| `docs/archive/proj/tasks/T004-docker-environment.md`     | 早期任务                                                                                                          |
| `docs/archive/proj/tasks/T010-rnaseq-template.md`        | 早期任务                                                                                                          |
| `docs/archive/proj/MODEL_CHAT_IMPLEMENTATION_PLAN.md`    | Phase 1-4 已落地，Phase 5 部分落地                                                                                    |
| `docs/archive/proj/MODEL_CHAT_IMPROVEMENT_PLAN.md`       | 已落地且**内容存在事实性错误**（声称的 `mcp/` / `hooks/` / `subagent/` / `workflow/` / `hmemory/` / `checkpoint/` 目录**实际不存在**） |

### 四、其他修复

- `docs/archive/design_docs/HISTORY_AND_REVIEWS.md`：

  - 修复"当前形态"错误描述（原称"纯 Rust 核心库 + E2E CLI"，但前端已于 2026-06 恢复为 Tauri）

  - 新增"2026-06 — Tauri 桌面前端恢复（当前形态）"里程碑小节

- `docs/archive/proj/IMPROVEMENT_PLAN_2026.md`：

  - 移除 §5 中指向已删除文件的引用

  - §2.1 改为"✅ 已完成"，列出全部已完成的删除项

**影响**：

- ✅ `docs/` 中无任何**事实性误导**文档

- ✅ `docs/archive/design_docs/` 仅保留 `HISTORY_AND_REVIEWS.md`（关键里程碑回顾）

- ✅ `docs/archive/proj/` 仅保留 `IMPROVEMENT_PLAN_2026.md`（项目级改进方案）

- ✅ "项目要做的事"有了单一权威来源（`IMPROVEMENT_PLAN_2026.md`）

- ✅ 关键决策轨迹仍在 `HISTORY_AND_REVIEWS.md` 中可追溯

**未改动**：

- 业务插件自包含文档（`symbio/src/plugins/agent/docs/`）保持不变

- `docs/explanation/*` / `docs/reference/*` / `docs/how-to/*` 权威文档保持不变

- `docs/CHANGELOG.md` 本文件**追加**本节记录

- `docs/ideas/*` 创意文档已整体移入 `docs/archive/ideas/`（属于产品方向探索，不在主文档树清理范围）

***

## 2026-06-15: 文档系统性更新

**背景**：上一轮代码与文档脱节（Tauri / Vue 引用遍布 `docs/`，但代码已剥离前端），用户要求按当前代码系统性更新项目文档。

**变更**：

1. **根** **`README.md`** **全面重写**

   - 移除 Tauri / Vue 全部引用；

   - 明确项目当前形态为"纯 Rust 核心库 + E2E CLI"；

   - 新增插件清单、能力路由示例、CLI 用法、快速开始、最小工作流。

2. **`docs/README.md`** **文档中心索引重建**

   - 重新组织为"核心架构设计 / 开发构建 / 设计草案 / 插件自包含 / 历史参考"五段；

   - 标注哪些文档"权威"、哪些"仅作历史参考"。

3. **`docs/explanation/*`** **与** **`docs/reference/*`** **三份文档全部更新**

   - `ARCHITECTURE.md`：补充分形路由树示意、插件清单、内核模块表；

   - `OPERATION_MECHANISM.md`：移除 Vue EventHandler / useChatEventHandler 等前端细节；

   - `API_DESIGN.md`：聚焦 V3.0 上下文注入版的 `Plugin` Trait 与 `PluginPayload` 4 态。

4. **`docs/how-to/*`** **三份文档全部更新**

   - `DEVELOPMENT_GUIDE.md`：聚焦机制化、Trait 抽象、Agent 子系统规范；

   - `BUILD_GUIDE.md`：移除 `pnpm tauri dev` 等前端命令，补 `cargo` 命令与排错；

   - `PLUGIN_DEVELOPMENT_GUIDE.md`：以 `weather` 插件为例演示完整链路。

5. **`docs/archive/design_docs/HISTORY_AND_REVIEWS.md`** **重写**

   - 按 v0.1.x / v0.1.5+ / v8 / v9 / v9.1 五段回顾关键里程碑；

   - 总结"机制化 vs 硬编码 / 文档代码同源 / identity 本质 / 前端剥离"四条经验。

6. **未改动文件**：

   - 业务插件自包含文档（`symbio/src/plugins/agent/docs/`）保持不变；

   - 本文件下方历史记录按"历史参考"原样保留。

***

## 2026-07-06: MCP 插件重构——对齐系统工具机制 + 清理误删

**背景**：用户两次反馈纠正早期对 MCP 插件的错误理解：

1. "前端并不负责任何 MCP 的调用，前端只是配置"——纠正了之前把 MCP 客户端实现归到前端的错误方向。
2. "call\_tool / discover / list\_tools 不是被调用的，系统的工具有现成机制（参考 web 插件等），所以 mcp 插件不会主动被调用的"——纠正了"为 MCP 单独设计一套调用 API"的过度设计。

**结论**：

- **后端**承担 MCP **配置管理**（CRUD）+ **客户端 transport**（stdio / http）

- **前端**仅做配置 UI（CRUD）

- MCP 工具通过 **系统统一的** **`Capability`** **trait +** **`traverse`** **+** **`tool_visitor`** **机制**集成到 agent——与 `web` 插件完全对齐

**变更**：

### 一、恢复 + 完善后端 MCP 客户端

1. **恢复** **`mcp/stdio.rs`** **+** **`mcp/http.rs`** **+** **`mcp/types.rs`**（误删纠正）

   - `mcp/stdio.rs`：stdio transport（每次调用临时 spawn 子进程 + kill）

   - `mcp/http.rs`：http transport（每次调用新建短连接）

   - `mcp/types.rs`：JSON-RPC 2.0 协议层类型（`JsonRpcRequest` / `JsonRpcResponse` / `McpTool` / `McpToolCallResponse` / `McpInitializeResponse` 等）

2. **新建** **`mcp/manager.rs`** —— 无状态 transport 路由器

   - `discover_tools(name, config)`：按 `transport_type` 路由到 stdio / http + 应用 `include_tools` / `exclude_tools` 过滤

   - `call_tool(name, config, tool_name, args)`：同上

   - **不维护**"激活集合"等运行时状态——是否可见由 `McpConfig.servers[name].enabled` 决定

3. **新建** **`mcp/capability.rs`** —— `McpToolCapability`

   - 把单个 MCP 工具包装为标准 `Capability`（`meta()` + `execute(ctx)`）

   - 命名规则：`mcp.<server_name>.<tool_name>` 三段式

   - 分类：`CapabilityCategory::Mcp`（新增变体）

### 二、集成系统工具机制

1. **改造** **`McpPlugin::traverse`**（参考 `WebPlugin::traverse`）

   - 每次 `parent.traverse(TRAVERSE_AVAILABLE_TOOLS)` 时遍历 `McpConfig.servers` 中 `enabled=true` 的项

   - 对每个 server 调 `McpManager::discover_tools` 动态发现工具

   - 把每个工具构造为 `McpToolCapability` 注册到 `ctx.get(CAPABILITY_VISITOR)`

   - agent 通过 `tool_visitor.invoke("mcp.<server>.<tool>", ctx)` 调用（与 `web_search` 等一致）

### 三、配置层统一 + 持久化

1. **升级** **`mcp_config::McpServerConfig`** 为完整版

   - 新增 `transport_type`（Stdio / Http / Sse）

   - 新增 `url`（http/sse 必填）

   - 新增 `include_tools` / `exclude_tools`（白/黑名单过滤）

   - 持久化路径不变：`~/.symbio/plugins/mcps/<name>/server.json`

2. **更新** **`servers/set`** **校验**：按 `transport_type` 校验必填字段（stdio → command；http/sse → url）

### 四、清理过度抽象

1. **删除 5 个多余 schema**：

   - `mcp_call_tool` / `mcp_discover` / `mcp_list_tools` / `mcp_register` / `mcp_unregister`

   - 这些功能通过 `Capability` trait + `tool_visitor` 机制实现，不再需要单独 schema

2. **删除** **`McpManager`** **的过度抽象**：

   - 移除 `register` / `unregister` / `is_active` / `active_servers` 集合

   - 移除 `tools_to_capabilities` / `list_capabilities` / `shared_manager` / `register_result_message` 等辅助

   - 移除 `types::stdio_command_args_env` 等内部辅助

### 五、Skill 插件清理

1. **删除未使用的** **`load_budget`** **/** **`estimate_tokens`** **方法**（之前为了"预留"留下但实际未使用）

### 六、文档同步

1. **`docs/archive/proj/IMPROVEMENT_PLAN_2026.md`** **§2.3** 重写：反映"前端只做配置 + 后端实现 transport + 系统工具机制集成"的正确方向
2. **`mcp_servers.rs`** **注释** 修正：删除"前端 tauri 端处理"的错误描述

**验证**：

- `cargo check`：✅ 零错误零警告

- `cargo test --lib`：✅ 233 tests passed

***
