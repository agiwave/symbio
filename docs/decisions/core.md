# ADR 正文 · 平台基座

> **文档类型：阐述** — 本文件是 [DECISIONS.md](../DECISIONS.md) 的**正文分册**之一，
> 只记录**为什么这样设计**（状态 / 决策 / 理由 / 被否决的方案 / 后果与不变量）。
> 索引与域分配规则在 [DECISIONS.md](../DECISIONS.md)；「现在是什么」看 [CURRENT.md](../CURRENT.md)。

**本册范围**：插件架构 / 路由 / 注册 / 执行接口 / `symbio_core` 准入 / 身份 / 生命周期 / 对象创建。

---

## ADR-001: 分形插件架构

**状态**：已接受

**决策**：容器与叶子使用同一套 `Plugin` 接口。`home` → `worker(composite)` → 14 个业务插件即是这棵树（插件清单见 [CURRENT.md](../CURRENT.md) §1）。

**理由**：自相似（任意插件可被容器包裹以叠加中间件）、可组合（新能力只需实现 `Plugin` 并注册）、可测试（每个插件可独立测试）。

**后果**：路由解析有运行期开销（可忽略）；需要 `inventory` 静态注册（ADR-007）。

---

## ADR-002: 路径即路由

**状态**：已接受

**决策**：用 `/` 分隔的字符串定位能力（如 `agent/chat`）。

**理由**：LLM 输出字符串即可调用工具；新增能力不改路由逻辑；工具名自带命名空间、避免冲突。

**后果**：路径是运行期字符串、无编译期检查；**资源与配置不走 `{plugin}/{action}`**，统一经 `vdfs/*` 协议与 `<根>/…` 地址（ADR-011 / ADR-025 / ADR-031）。路由清单由 [CURRENT.md](../CURRENT.md) 提供，不在此维护。

---

## ADR-003: Session 作为编排入口

**状态**：已接受

**背景**：早期 `agent` 插件独占会话编排，耦合严重。

**决策**：`session` 是唯一编排入口（核心循环 `chat_loop`），`agent` 只负责「人格」。

**理由**：关注点分离；同一 Session 可绑定不同 Agent；支持无 Agent 的纯工具模式。

**后果**：Session 需要收集全树工具；Agent 插件适配为被动角色。

---

## ADR-006: 薄宿主层设计

**状态**：已接受

**决策**：Tauri 后端仅暴露 3 个 command——`route_v2` / `route_v2_send` / `route_v2_close`。

**理由**：极简适配层（业务逻辑全在核心库）；前端无业务规则；宿主可替换。

**后果**：前端无法直接调用插件、必须经路由；有 IPC 序列化开销。命令面现状核对见 [CURRENT.md](../CURRENT.md) §5。

---

## ADR-007: 静态注册 (inventory)

**状态**：已接受

**决策**：用 `submit_object_creator!` + `inventory` 做编译期静态注册。

**理由**：零配置（新增插件无需改注册代码）；编译期保证；惰性初始化（首次使用时收集）。

**后果**：依赖 `inventory` crate；注册顺序不确定（通常无关）。

---

## ADR-019: 跨栈契约**手工镜像 + 审计守卫**，不引入代码生成；G3（前端面板去语义）否决

**状态**：已接受（含一项否决）

**背景**：前端 `tauri/src/schemas/` 与后端 `symbio_core` 之间是**手工镜像**。复核给出两条路：引入 `ts-rs` / `schemars` 从 Rust 导出 TS，或扩展 `scripts/protocol-mirror-audit.mjs` 把已存在的镜像纳入守卫。当时实测后者**只守 3 个常量**，而前端实际持有 31 个后端协议词的副本——剩下 28 条无人看守，后端重命名 `vdfs/list` 或某个状态词时，前端会静默失效而**所有守卫仍绿**。同一份复核还提出 **G3**：把 `Appearance.vue` / `About.vue` 由前端自持判为「违反不变量 4（前端零资源知识）」。

**决策**：
1. **不引入代码生成**，`schemas/` 继续手工镜像 Rust 契约。
2. **跨栈一致性交给审计，且审计要「自动发现」而不是「手工登记」**：
   - **A 组**：前端 `VDFS_*` 字面量常量 ↔ 后端同名常量逐字比对，**取同名交集**——新增常量自动进入守卫。名字不同的登记在 `ALIASES`；前端自持的必须登记在 `LOCAL_ONLY` 并写明理由，**「没登记」会报错**。
   - **C 组**：后端**闭集取值集合** ↔ 前端词表数组，要求集合相等（不比顺序）。后端两种写法都收：带 `#[serde(rename_all = …)]` 的**枚举**（转换规则按后端声明自动分派，遇到不认识的取值**报错而不是猜**）与一组 `pub const PREFIX_*`（**常量组**，字面即线上取值）。
   - **D 组**：后端**结构体** ↔ 前端**接口**的**字段名**。只查一个方向——前端持有的字段必须能在后端线格式里找到（或登记为「前端自持」）；只比字段名不比类型。
   - **E 组**：后端文件顶部的 `// Corresponding Frontend: <路径>` 必须指向**真实存在**的文件。约定收紧为：**写了就必须指向真实文件；前端没有镜像就别写**——假指针比没有更糟。
3. **G3 否决**。`Appearance.vue` / `About.vue` 不违反不变量 4。

**理由**：
- **格式层已经单源，重复只在符号层**：两侧共用同一套契约名（`snake_case` 线格式），`serde` 就是那个单源点——生成器要解决的问题，格式层已经解决了。
- **AST 无关，机器导出不可靠**：`chat_message.rs` 里 95 处 serde 标注，其中 `untagged` / `tag` / `skip_serializing_if` 都会让导出的 JSON Schema 与实际线格式不符；要生成就得先为这些特例写规则。而**生成器错一次，错的是全部**，手工镜像错一处只错一处。
- **能自动化的与不能自动化的要分清**：能机器比对的只有**闭集取值**；文案与折叠策略、路径代数这些是**业务判断**，生成器也造不出来。
- **G3 的前提与不变量 4 的原文不符**：[design/vdfs-frontend.md](../design/vdfs-frontend.md) 明文允许「`ext → 渲染器`、子目录名 → 图标」这类纯 UI 映射，而 `form` / `session` / `message` / `text` 同样是「ext 即语义类型名」的**渲染分派键**。

**后果与不变量**：
- 后端改一个契约词而前端没跟 ⇒ 审计立刻红，报出两侧文件与常量名。
- **新增契约词的成本是「两侧各加一行」**，而不是「跑一次生成」——契约改动本就该被两侧同时看见。
- **类型映射仍无守卫**（`Option<u64>` ↔ `number` 这类）：它需要一张类型映射表且泛型 / 嵌套会失控，收益也低于字段名。
- **D 组的覆盖面按「前端逐字段镜像了它」这一判据推进**：前端只挑几个字段用的响应结构**不登记**（它本就该按需取，多抄只制造噪音）。机制没了，形状也就没了——登记随之撤除。
- 审计脚本的**回归测试**必须与登记清单同步铺满：新增一条登记，测试夹具就要多铺一对。这是刻意的耦合，保证夹具与脚本清单不会悄悄脱节。

---

## ADR-020: 执行期与传输层**分离**——`EventSink`（出）+ `AbortSignal`（入）取代 `PluginChannel` 的双职责

**状态**：已接受（决策 7 已被 ADR-021 推翻）

**背景**：流式链路曾有 8 跳，其中两跳是纯开销，根因是 `PluginChannel` **一个类型承担了两种语义**：**出**（`serde_json::to_value(NodeOp)` → `PluginFrame::Data` → 通道 → 消费循环 `from_value::<NodeOp>` 再解回来）与**入**（Abort 帧 + `abort_flag` 的 100ms 轮询 + `cancel_token`，**三源并存**）。派生出四类结构性冗余：同进程调用付进程外代价（两端**始终同进程同 spawn**）；「静默」只能靠所有权手术（造哑 sender + `mem::replace` 对调 + drain task——**把「不说话」表达成了「换一根管子」**）；中止三源各有各的竞态；消费循环被迫「按帧醒来」而不能只做生命周期管理。

**决策**：
1. **执行期与传输层分离**，各用语义单一的原语：`EventSink`（出，`Direct(Arc<dyn TranscriptWriter>)` \| `Null`）与 `AbortSignal`（入，`Arc<AtomicBool>` + `CancellationToken` 合一）用于**执行期**；`PluginChannel` **退回纯跨进程传输**（前端实时面）。
2. **`abort()` 是唯一置位入口**，置位即唤醒。中止**不再有帧、不再有轮询、不再有第二个来源**。
3. **`EventSink::Null` 是「本次调用不产生可见事件」的类型级表达**，取代哑通道 + drain task。
4. **删除 `ControlSignal` 协议面**。
5. **消费循环退化为纯生命周期管理**：一次 `spawn` + `select!` 三臂（任务结束 / 看门狗 / 状态被接管），不再收帧、不再解码、不再分派。
6. **`AbortGuard::disarm` 注销登记时一并 `abort()`**，把历史上「通道被 drop ⇒ 对端读到关闭 ⇒ 视作中止」这条隐式语义**显式化**并由测试锁定。

**理由**：进程内调用不该付进程外的代价——既然两端同进程同 spawn，直接调 `TranscriptWriter::apply` 就是正确形状；「不说话」应当是类型选择而非所有权交换，`Null` 让压缩路径「绝不产生可见事件」成为**编译器可检查**的性质；中止的复杂度来自「多源」，三源合一**消除了一整类竞态**；`EventSink` 认识 `ChatMessage` 但不认识 `Transcript`，于是 `symbio_core` 不必知道会话存储的存在，而「转写只有一个写入点」仍由类型保证。

**后果与不变量**：
- 后端两跳 serde 消失；`ControlSignal` 引用归零；压缩的哑通道 hack、`resume` 的临时通道 + drain、消费循环的嵌套 spawn + keepalive 三处冗余消失。
- **新增一条不可回退的约束**：`EventSink::Direct` 的写入者必须**只**经 `TranscriptSink::apply` 落转写；任何绕过它的直接 `Transcript::apply` 调用都会破坏「唯一写入点」。
- **保住的语义**：转写唯一写入点；`Warn` 归会话级状态不进转写；「中止不是失败」（结局 `aborted` 而非 `completed`，在途根 Turn 定稿 `Aborted`，重试入口不消失）。
- **决策 7（执行期原语经 `ctx` 传递）已被 ADR-021 推翻**；`EVENT_SINK` / `ABORT_SIGNAL` 两个 `SymbioKey` **仍然有效**——它们仍是信封承载出口 / 中止的键，只是读侧收口到 `ExecEnv::from_request` 一处。
- 机制细节见 [session/docs/core-loop.md](../../symbio/src/plugins/session/docs/core-loop.md) §6（双原语）与 §7（工具侧收敛）。

---

## ADR-021: 两个执行接口**同形**——`ExecEnv` 具名化，拆信封收口到一处

**状态**：已接受。**本 ADR 部分推翻 ADR-020 的决策 7**。

**背景**：ADR-020 把**数据面**收干净了（通道换成出口 / 信号），但**签名（控制面）**没动，于是同一个「执行期」概念有两种到达方式：`Capability::execute` 把出口 / 中止藏在 `ctx` 的两个无名键里、参数靠 `ctx.payload::<Value>()` 无类型自读、返回多态载荷；`ModelProvider::execute_turn` 则是显式类型化参数与类型化返回。代价是**每个工具都得记住「我该读哪些键」**，改一处漏另一处是必然的。

**决策**：
1. **新增具名类型 `ExecEnv { sink, abort }`**，两个接口共用，于是同形：

   ```text
   Capability::execute(args, env, ctx)      -> Result<Value, PluginError>
   ModelProvider::execute_turn(inputs, env) -> Result<TurnOutput, PluginError>
   ```

2. **差别只剩 `ctx`，且这是真实差异**：工具是**被路由、被注册**的（要转发 `session/chat/send`、要解析 VDFS 挂载、要读会话身份），所以还需要信封；模型执行不被路由，也就没有信封。不强行抹平。
3. **`Capability::execute` 返回 `Result<Value, _>`**，不再经 `PluginPayload` 那层多态载荷——工具从来只用 `Data` 一个变体。
4. **`capability_invoke(cap, ctx)` 是唯一「拆信封」的地方**：`args = payload ?? Null`、`env = ExecEnv::from_request(ctx)`、结果装回 `PluginPayload`。装饰器（`PrefixedCapability` / `SecureToolWrapper`）**不拆不装**，原样透传。
5. **`ExecEnv::from_request` 的缺席语义照搬 ADR-020**：没有 `EVENT_SINK` ⇒ `Null` 出口，没有 `ABORT_SIGNAL` ⇒ 永不中止。因此 `route()` 直连调用仍然**自然静默**，「有没有出口」仍不需要第二条调用路径。

**理由**：「统一」的判据是**调用方能否只看签名就正确调用**；**拆信封只该有一处**（多态载荷是路由层的形态，工具不该为它付代价）；**保留 `ctx` 是承认真实差异**，不是妥协——硬把 `ctx` 塞进 `ExecEnv` 会把「这次调用从哪条路径来」与「这次调用要怎么跑」重新混成一团。

**后果**：24 个 `impl Capability` 换签名（2 个用 `env`，22 个 `_env`，前缀即文档）；签名从 1 参变 3 参是**刻意的**——执行期环境是接口的一部分，不该因今天没人用就从签名里消失（消失的表现是下次有人要发增量时又去 `ctx.get(EVENT_SINK)`）。

---

## ADR-023: `symbio_core` 的准入规则 = **依赖方数量**，不是「够不够底层」

**状态**：已接受

**背景**：`symbio_core` 是**跨模块的系统架构**，不是通用工具箱——它的作用是给**互相不可见**的插件提供唯一的共同可见处。批次 J 曾以「这是跨插件约定」为理由往 core 里加了两样东西（工具结果字段名读取器、`CapabilityVisitor::resolve_name` 默认方法），**都是错的**：两者都只有 `session` 一个模块依赖。判错的根源是把「**约定**」与「**跨模块**」当成一回事——约定可以只在一个模块内成立。

**决策**：
1. **判定依据是依赖方数量**：只被一个模块依赖的内容一律**下沉回该模块**。不问「它够不够底层」，只问「除它之外，还有谁依赖」。
2. 内容进 core 时，**在模块文档里写下依赖方对照表**：谁依赖、依赖哪个函数、改它要同时改谁。
3. **模块文档里写反面例子**，让下一次想往 core 加东西的人先看到自己被拒的先例。

**理由**：core 的**体积即耦合面**——往里加一个单消费方的东西，收益是省一行 `use`，代价是让一个模块的内部实现细节获得「架构级」地位，之后想改它要先说服所有读 core 的人它不是契约；「够不够底层」是**主观且无法证伪**的判据，最终会变成「我觉得它挺通用」，而「依赖方数量」可 `grep` 一遍就有答案。

**后果**：`symbio_core::capability::tool_name` 保留（**两个模块**依赖：`model` 出、`session` 入），且带依赖方对照表 + 反面例子；同一份逻辑若真被第二个模块需要，**要等第二个消费者出现再上提**——期间可能先复制一份，这是**有意接受**的（复制是可见的债，过早抽象是不可见的债）；「跨模块」本身仍需人工判断。

---

## ADR-032: 插件**身份归 `PLUGIN.yml`**——`PluginMeta` 从「元信息」降为「出厂自述」

**状态**：已接受（**含未完成项**）

**背景**：评审记录了一处**双源**：`Plugin::meta()` 返回 `PluginMeta`（含 `id` / `name` / `version` / `author`），而 trait 注释**自承**「唯一运行期消费方是容器」，插件的身份实际来自各自的 `PLUGIN.yml`。后果已经显形：`PluginEntry` 的 `title` / `description` / `version` 取自 `meta()`，而**停用的插件刻意不被构造**，于是那几个字段在停用态为空、消费方按目录名兜底——同一个「插件叫什么」在不同地方可能给出不同答案。从「一个插件 = 一个目录、目录可整体搬走」这条不变量看，**身份是目录的属性，不是构造物的属性**——停用与否不改变「它是谁」。

**决策**：
1. **身份归 manifest**。`PLUGIN.yml` 新增身份保留键（`plugin_title` / `plugin_description` / `plugin_version` / `plugin_author`），运行期**唯一**消费路径是 manifest。
2. **`PluginMeta` 语义从「元信息」改为「出厂自述」**，字段分两类：**出厂种子**（`name` / `description` / `version` / `author`，**仅装配期一次**——容器构造出插件后把它投影进 manifest，键缺失才写）与**挂载点呈现**（`order` / `icon` / `hidden` / `root_access`，运行期读）。
3. **`plugin_version` 从「纯投影键」升级为「真身份键」**：投影与身份**同源**。`plugin_required` / `plugin_can_disable` 仍是**纯投影键**（没有 manifest 对应物），两者不混。
4. **保留键集合收口为 `RESERVED_KEYS`**，读写统一遍历它（剥离 / 保留各一处实现）——「哪些键属于装配方」从此只有一份清单。
5. **为第三方插件预留两个键**：`plugin_api`（要求的宿主插件 API 版本）与 `plugin_grants`（宿主授予的能力）。本期只**定义与解析、不强制**——授权校验是后续期次的事。

**理由**：**身份跟着目录走**（搬走目录 = 带走身份，连同一个改过的标题、一个本地化的描述；若身份只在代码里，同一个插件在所有智能体里只能叫同一个名字）；**停用不等于不存在**（插件管理列表**必须**能显示一个停用插件的名字，否则用户只看到一个目录名）；**装配期一次性投影不是「双源」**（出厂声明 → 落盘值是**单向**的，落盘后代码里的种子不再被读）；**挂载点呈现不投影**（它们是**挂载点**的属性、不是插件的属性——停用即无挂载点，`order` 随之无意义，这与「身份必须常在」正好相反）。

**后果与未完成项**：
- `PluginEntry` 的 `title` / `description` / `version` 改从 manifest 读 ⇒ **停用插件也有身份**；消费方按目录名兜底只留给**从未落位过**的目录。
- 装配期写盘只在键缺失时写，用户改过的值不被覆盖；写入时一律**保留**保留键，否则「在设置页保存一次配置」会把身份冲掉。
- **本期不删 `PluginMeta` 的身份字段**。删除它们需要「**不构造也能拿到出厂身份**」的能力——即一张按工厂 id 索引的静态自述注册表，那是 provider 两级解析的天然产物，留待后续期次一并迁移。**未完成的部分明确记录在此，不假装已解决。**
- **一个已知边界（不粉饰）**：身份只在**首次装配**时落位，因此**遗留工作区里「一直停用、从未构造过」的插件**在新版本下仍然没有身份——这是**现状等价而非退化**（改前同样拿不到）。

---

## ADR-033: 生命周期钩子 —— `start` **同步**、`stop` **异步**，停用与卸载各给理由

**状态**：已接受（**含未完成项**）

**背景**：`Plugin` trait 原先**没有生命周期钩子**，「停用」的语义因此只能是「**不构造**」：插件没有任何机会做清理——关监听、停后台任务、落盘，全都要靠 `Drop`。而 `Drop` 是同步的：不能 await、不能报错、也不区分「停用」「卸载」「退出」。对第三方插件（外部进程）这条是硬阻塞：`stop()` 要发 `shutdown` 帧 → 等 `ok` → 关 stdin → 等进程退出 → 超时则 kill，**没有钩子就没有地方放这段代码**。

**决策**：
1. trait 加两个**可选**钩子（有默认空实现 ⇒ 内置 impl 一行不改、行为零变化）：

   ```rust
   fn start(&self, ctx: Arc<dyn PluginInvokeRequest>) -> Result<(), PluginError>;      // 同步
   async fn stop(self: Arc<Self>, reason: PluginStopReason) -> Result<(), PluginError>; // 异步
   ```

2. **`start` 为什么是同步的**：装配路径是**同步**的（工厂签名本身非 async，改成 async 要波及注册表与全部工厂），**在那里 await 不可用**。需要异步初始化的插件走别的两步：**构造时同步 spawn** + **首次调用前惰性完成 init**，而不是把装配改 async。
3. **`stop` 为什么是异步的**：它的全部调用点都已在 async 上下文，且清理本身需要 await（发帧、等进程退出、flush 落盘）。**二者不对称是刻意的**，由各自的调用上下文决定。
4. **`PluginStopReason` 三态**：`Disabled`（可恢复停用：目录与数据都保留）/ `Uninstalled`（卸载：目录随后被删除）/ `Shutdown`（全局收尾）。区分它们是因为插件的**处置不同**——「卸载时是否保留自己的缓存 / 数据」是可恢复停用时不必做、卸载时必须当场决定的事；塞进一个布尔量就会在插件里长出一堆「我这次是为什么被停」的旁门判断。
5. **接线**：装配后逐个 `start`（身份投影之后、进实例表之前）；停用摘出实例表**之后立即** `stop(Disabled)`；卸载摘出实例表 → `stop(Uninstalled)` → **再**删目录。

**理由**：**钩子必须可选**——「加机制不改变现状」是这批改动的一贯口径，也是外部插件能与内置插件共存的前提；**`stop` 早于任何破坏性动作**——真正的截止线是「删目录」，顺序反了就等于让插件在数据已经没了之后再去做清理，卸载时尤其致命（那是插件**最后**一次能写盘的机会）；不把 `start` 做成 async 也避免了「装配期需要 runtime」。

**失败处置（两条相反的口径）**：
- **`start` 失败**：记 `plugin_error!` 并**保留挂载**（不摘掉）。启动时炸掉的插件同样是**要被看见**的——摘掉它，用户只看到「插件不见了」；留着它，每次调用会给出明确的错误。
- **`stop` 失败**：记 `plugin_warn!` 并**继续**（停用照常生效、卸载照常删目录）。与 `start` 相反——这里的用户意图是「**让它停下**」，用清理失败去否决它等于给插件一个「我停不掉」的否决权，而用户此刻已经没有别的手段了。

**未完成（明确记录，不假装已做）**：「进程退出」与「丢弃旧容器」两类**收尾**本期**不接**——仓库当前没有任何进程退出钩子，新增它是独立的一件事（要动两个入口、且要决定「谁负责遍历整棵树发 stop」），与本次「加钩子不改变现状」的目标冲突。它们与 `PluginStopReason::Shutdown` 一并留待接线时做。

---

## ADR-035: provider 化的判据 —— **三个条件与构造契约**

**状态**：已接受。补充 [ADR-007](#adr-007-静态注册-inventory)（**怎么注册**）与 [ADR-023](#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层)（**住哪**），回答**该是什么形态**。

**背景**：ADR-007 定了 `submit_object_creator!` + `inventory` 的静态注册，ADR-023 定了「依赖方 ≤ 1 就下沉回该模块」。两条都不回答这个问题：**一个被多个模块共享的功能，该做成 provider 契约，还是值对象 / 纯函数 / 全局单例？**

实测现有三个 provider 家族，**三种复用策略各不相同，而没有任何一处写着该怎么选**：

| 家族 | 构造代价 | 复用策略 | 策略写在哪 |
|---|---|---|---|
| `dyn Plugin` | 中等（持目录与配置） | **不复用**——每个挂载点一个独立实例 | `plugins/composite/registry.rs::mount_child` |
| `dyn ModelProtocol` | 零（无状态 unit struct） | 不需要 | 无处（也不需要） |
| `dyn EmbeddingService` | **昂贵**（ONNX 会话 + 341 MB 构建缓存） | 实现**自己** `LazyLock` 单例 | 只在 `providers/embedding/local.rs` 的一行注释里 |

第三个家族的策略只活在注释里——下一个写昂贵 provider 的人若照抄 `ModelProtocol` 的 `build` 写法，就会**每次调用重建**，且不会有任何编译期或门禁提示。

**决策**：

1. **provider 化需同时满足三个条件**：
   1. **调用方需要运行时多态**——≥2 个实现，且**编译期**不知道选哪个；
   2. **无状态，或状态可共享**——一个 id 对应一个对象语义；
   3. **功能是「按 id 装配对象」而不是「处理一段数据」**（理由见下）。
2. **构造契约：构造函数必须廉价。** 需要单例的实现在**自己内部**建（范本：`providers/embedding/local.rs` 的 `LazyLock`）；`creator_create_object` **不做缓存**。
3. **`ctx` 键与 `creator_create_object` 是两条互不替代的通道**：

   | | `ctx` 键（`SymbioKey`） | `creator_create_object` |
   |---|---|---|
   | 装什么 | **每次调用变化**的**值** | **按 id 装配**的**对象** |
   | 生命周期 | 一次 traverse / 一次工具调用 | 由持有者决定 |
   | 例 | `EVENT_SINK` · `ABORT_SIGNAL` · `CAPABILITY_ERRORS` · `PLUGIN_DIR` | `dyn Plugin` · `dyn ModelProtocol` · `dyn EmbeddingService` |
   | `ctx` 的角色 | 就是它本身 | 仅作**构造上下文**（参数袋） |

   判据一句话：**每次调用都不一样的值走 `ctx` 键；按 id 选一个实现走 `creator_create_object`。**

**理由**：

- 决策 1.3 有**签名级证据**：`type ObjectConstructor = fn(Arc<dyn PluginInvokeRequest>) -> Box<dyn Any + Send + Sync>`（`symbio_core/creator/mod.rs`）——**没有参数位**。机制表达得了「按 id 装配」，表达不了「让这个对象处理这段数据」；后者只能由返回对象自己的方法承担，数据经 `ctx` 键或方法参数进入。
- 决策 2 的「实现自持单例」不是权宜：**构造代价是实现的私事**。机制若代管，就必须知道「哪些实现昂贵」，而那正是 ADR-007「不针对任何具体类型做特殊化」要避免的知识。
- 决策 3 划清边界后，`ExecTranscriptWriter` 的形态才有解释：core 定义 trait（`ExecEventSink::Direct` 要调它，不能反向依赖 session 的 `Transcript`），但**不走 `creator_create_object`**——它的出口是「**这一次执行**的记录器」，是每次调用变化的值，故走 `ctx` 键（`ExecEventSinkKey`）。

**被否决的方案**：

- **给 `creator_create_object` 加统一缓存（按 id 缓存 `Arc<dyn Any>`）**：**会破坏分形挂载**。[`ASSEMBLY_SUB_AGENT_PLUGINS`](#adr-001-分形插件架构) 让**每一棵**子 Agent 子树都挂 `model` / `session` / `local`……同一个 provider id 因此在系统树与每棵子树下**各有一个挂载点**，各自的 `ctx` 带各自的 `PLUGIN_DIR`；缓存后所有子树会拿到**同一个**实例（且是第一个挂载点的 ctx），子智能体的模型服务与工作区记忆会全部串到父树上。收益仅是省一次 `Arc::clone`——而昂贵构造已由实现方用 `LazyLock` 解决。
- **把 core 的共享内核（`clock` / `text` / `logger`）硬抽 trait**：都不满足决策 1 的前两条——没有第二实现，`dyn` 只是把一次构造换成一次字符串查表（[ADR-011](./vdfs.md#adr-011-资源存储--vdfsprovider-的集中实现) 对 `VdfsProvider` 记的正是同一判据的反面：**不存在第二种实现时不套 `dyn`**）。记忆曾属同一家族，现已**整块**迁出 core（[ADR-037](#adr-037-实现可以离开-core--记忆整体迁往-providers)），其「不抽 trait」的论证随实现去了 `providers/memory/mod.rs`。
- **合并 `clock` 与 `text` 为「业务无关工具域」**：任何**有判别力**的合并判据都会把 `clock` 排除——`text` 是**纯函数**（输入决定输出），`clock` 是**非确定性来源**（无输入，输出随系统时钟变化）。要么判据松到能装下两者（那就成了杂物抽屉，`keys` 一度收下插件清单正是前车之鉴，见 [ADR-023](#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层) 的「依赖方数量」判据所修的那一类错放），要么判据有判别力（`clock` 随即被排除）。而合并的收益只是少一个目录 + 少一行 README——不足以换掉「一个域名自证内容」这条性质。

**后果与不变量**：

- 新增共享功能时**先过三条件**；三条件不全成立一律用值对象 / 纯函数 / 全局单例，**不硬抽 trait**。
- 新增 provider 时**构造必须廉价**；昂贵实现自持单例，范本指向 `providers/embedding/local.rs`。
- 不变量：`symbio_core` 里**每一处含具体逻辑的域都有 ≥2 个消费方**，且模块文档写明了「为什么在 core 而不是 provider」。此条可由 `grep` 复核。
- 反过来也成立：**「够底层」「被多个模块用」都不构成留在 core 的理由**——判据是 [ADR-023](#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层) 的**依赖方向**与 core README §0 的**契约 / 实现**之分。记忆的整块实现（**含文件名**）已据此迁往 `providers/memory`（[ADR-037](#adr-037-实现可以离开-core--记忆整体迁往-providers)）。

---

## ADR-036: 对象创建机制独立成域 —— 它是系统级反射机制，不是插件专属

**状态**：已接受。补充 [ADR-007](#adr-007-静态注册-inventory)（**怎么注册**）、[ADR-023](#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层)（**住哪**）与 [ADR-035](#adr-035-provider-化的判据--三个条件与构造契约)（**该是什么形态**），回答**归哪个域**。

**背景**：`create_object` / `has_creator` / `creator_ids` / `submit_object_creator!` 与注册表内部结构住在 `plugin` 域。三条已有 ADR 都没有决定这件事——ADR-007 定静态注册的手段、ADR-023 定「依赖方 ≤ 1 就下沉」、ADR-035 引用它作为「按 id 装配」的机制，**没有一条说它属于 `plugin`**。它住 `plugin` 只是因为最先服务的类型族恰好是 `dyn Plugin`。

而注册点实际服务**三个互不相关的类型族**：

| 类型族 | 注册点 | 所属域 / 层 |
|---|---|---|
| `dyn Plugin` | `plugins/*/plugin.rs` 的 `build`（16 个插件） | `plugin` 契约 |
| `dyn ModelProtocol` | `plugins/model/protocols/*.rs` | `model` 插件 |
| `dyn EmbeddingService` | `providers/embedding/local.rs` | `embedding` 域 |

**决策**：

1. **独立成 `creator` 域**：目录 `symbio_core/creator/`，公开面 `creator_create_object` / `creator_has` / `creator_ids`，加 `submit_object_creator!` 宏（`#[macro_export]`，导出到 crate 根）。
2. **归属判据是「它描述什么」，不是「谁先用了它」**（README §1.2「放置也是同一条规则的一部分」）。注册表描述的是「按 id 装配任意类型对象」，与「插件」这个具体类型族无关。
3. **前缀 `creator_`**；`plugin` 域不再有任何对象工厂符号。

**理由**：

- **签名里没有 `Plugin`**：`type ObjectConstructor = fn(Arc<dyn PluginInvokeRequest>) -> Box<dyn Any + Send + Sync>`（`symbio_core/creator/mod.rs`）——注册表按 `(id, TypeId)` 索引，对类型族**完全无知**。它对类型族的唯一了解是「可以被 `Box<dyn Any>` 装下」。
- **留在 `plugin` 会让 `embedding` / `model` 依赖 `plugin` 的一个与插件无关的符号**——`plugin` 于是成了它们的事实依赖，域间图多出一条没有语义的边。
- **「一域一前缀」的必然结果**：`PLUGIN_` 已属于 `plugin`（工厂 id `PLUGIN_ID_*`、清单键 `PLUGIN_KEY_*`、路由地址 `ROUTE_*`）。对象创建注册表若也叫 `PLUGIN_*`，「插件」这个词就同时指「插件」与「任意对象的装配」——正是 [ADR-023](#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层) 记的那类「一个前缀两种东西」。

**被否决的方案**：

- **留在 `plugin`，只改名 `PLUGIN_CREATE_OBJECT`**：前缀对了，但「`embedding` 的嵌入服务工厂住在 `plugin` 域」这件事仍然没有解释。改名只解决「名字读不出归属」，不解决「放错了域」。
- **并入 `keys`**：`keys` 只收「类型安全上下文键」（ADR-023），注册表是**行为**不是取值凭据。同类错放已被修过一次——插件清单曾收进 `keys`。
- **与 `keys` 合成一个 `registry` 域**：两者的判别力不同（「取值的凭据」vs「按 id 装配对象」），唯一能同时装下两者的判据是「都是查表」——那是杂物抽屉式判据，与 ADR-035 否决「合并 `clock` 与 `text`」的理由同源。

**后果与不变量**：

- 新增「按 id 装配任意类型对象」的需求一律进 `creator`；不新开域，也不挂回 `plugin`。
- 不变量：**`creator` 的公开签名里不得出现任何具体类型族**（`Plugin` / `ModelProtocol` / `EmbeddingService`）。`ObjectConstructor` 的签名即证据，可由 `grep` 复核。
- 域间图新增 `creator` 节点，依赖方向为 `creator → plugin`（只用 `PluginInvokeRequest` 作构造上下文），**无环**。

---

## ADR-037: **实现**可以离开 core —— 记忆整体迁往 providers

**状态**：已接受。补充 [ADR-023](#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层)（**住哪**）与 [ADR-035](#adr-035-provider-化的判据--三个条件与构造契约)（**什么形态**），回答**含具体逻辑的实现该不该留 core**。

**背景**：`symbio_core/memory` 一度是本层少数「含具体逻辑」的域：`MemoryFile` 的读写、两道容量闸门、片段排版、节点形状，外加一个三层共用的文件名常量 `MEMORY_AGENTS_FILE`。它留在 core 的理由是「三层都用」——work / session / agent 各构造一个 `MemoryFile` 指向自己的作用域。ADR-035 判的是**形态**（要不要 `dyn`），没有回答**住处**；于是「被多个模块共享」被当成了「留 core」的充分理由。

**决策**：

1. **整块迁往 `providers/memory`**（方式 B：具体类型直接组合，不套 `dyn`）。它不隶属任何单个插件——若住其中任何一个，另两个就得跨插件引用，违反「插件之间不直接相互引用」。
2. **core 里什么都不留——包括文件名**。文件名归各插件自己：`work::memory::WORK_MEMORY_FILE` / `session::memory::SESSION_MEMORY_FILE` / `agent::host::store::AGENT_MEMORY_FILE`。
3. **共享实现不认识文件名**：`MemoryFile::file_name()` 从**路径末段**推导，无兜底字面量（无作用域即无名，`Option<&str>`）。

**理由**：

- **「共享」不是「留 core」的判据，依赖方向才是**。core 是**契约层**（traits、协议类型、词表常量、纯工具函数）；记忆是**实现**，消费方是三个插件。按 ADR-023 的判据，它该住在**谁都够得着、又不属于任何人**的地方，即 `providers/`。
- **统一文件名缺乏约束力**：三层各写各的文件，改一层不影响另两层。把「都用同一个名字」登记成 core 契约，等于把**没有共享价值**的事写成契约——只会让「改一层」变成「改三层」。
- **`AGENTS.md` 的行业约定只对「目录」成立**：它说的是「工作区目录 / agent 目录里的指令与记忆」（`docs/design/agent-directory-spec.md` §6）。**会话目录不是 agent 目录**，用该名名不副实，故会话层改 `MEMORY.md`——这不是破例，是**没有约定可对齐**。

**被否决的方案**：

- **只搬实现、文件名留 core（作为「跨插件地址约定」）**：文件名是**各层的个性**，不是契约；且它会让 core 继续为「三层碰巧同名」背书——一旦有一层改名（会话正是如此），core 的常量立刻变成错的那个。
- **把 `MemoryFile` 改走 VDFS**（消掉 `std::fs`，复用统一资源访问面）：三个宿主的 VDFS provider 本身就是 `MemoryFile` 的**薄适配器**（`Read`/`Write` 直接委托 `store.read()` / `store.write()`）。让它再走 VDFS 会**递归**（`MemoryFile::write` → VDFS 路由 → 本插件 provider → `MemoryFile::write`）。写入通道本就只有一条，无需改。
- **给记忆硬抽 trait**：见 ADR-035 决策 1（无第二实现时不套 `dyn`）。

**后果与不变量**：

- 含具体逻辑的东西**默认住 `providers/`**；留在 core 的必须有「没有第二个住处」的理由——`event_bus` 的全局订阅表、`logger` 的进程级闸门属此类（**进程级单例**，不属任何插件）。
- 不变量：**core 的公开面里不得出现任何一层记忆的文件名**。此条可由 `grep MEMORY` 复核。
- 新增第四层记忆只需在自己的插件里定一个文件名常量，不触碰 core，也不触碰另三层。

---

## ADR-038: 帧与消息构造家族按依赖方数量下沉

**状态**：已接受。**取代 [ADR-034](./model.md#adr-034-sse-行解析契约随流循环迁入-model-插件) 决策 2 的位置条款**。**背景**：`turn.rs` 末尾的依赖方表一度以「本模块全部符号两侧共用」收尾——那是按**文件**数出来的：同一文件里既有两侧共用的 `llm_emit_message`，也有生产代码里只有 session 一个消费方的 `llm_build_assistant_messages`（唯一链路 `session/chat_loop/turn.rs` → `TurnOutput::into_messages`，model 那 9 处引用全在测试 fixture 里）。[ADR-023](#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层) 数的是**符号**的依赖方，文件不是依赖单位。

**决策**：①**落 session**：`llm_build_assistant_messages` / `llm_build_tool_message` / `TurnStreamChildIds` / `impl TurnOutput::{into_messages, is_reasoning_only, effective_text}` → `plugins/session/message_build.rs`；`llm_emit_state` / `llm_state_frame` / `llm_emit_removed` → `plugins/session/transcript/frames.rs`——两者生产消费方都只有 session。②**落 model**：`llm_emit_delta` → `plugins/model/stream.rs`，唯一调用点就是那里的流循环热路径。③**测试 fixture 随消费方走**：model 的 `message_builder.test.rs` 改为本地构造同形状的树，两侧各锁一半（落库形状在 `plugins/session/message_build.test.rs`，扁平化视图在 model 侧）；fixture 计入依赖方（ADR-023 不豁免测试），不迁就永远停在「1 生产 + 1 测试」。④**留守六个**：`llm_emit_message` / `llm_removed_frame` / `llm_short_id` / `TurnToolCallInfo` / `TurnOutput` 各有两个以上消费方；`llm_message_frame` 是登记在案的**例外**——外部只有 session，但它与 core 自己的 `llm_emit_message` 同进退，「完整消息必然带状态」只在一处实现。

**理由**：**住哪由调用链决定**——`into_messages` 若住 model，session 就得跨插件引用 model（E-009 禁止；固有实现不需 `use`，躲得过审计也仍是隐性依赖），唯一生产消费方是谁就住哪；**搬迁是一次编译期可检的移动**——固有 impl 可落在同 crate 任意模块，调用方不必 import 实现处，故调用点零改动。

**被否决的方案**：**只改文档、代码不动**（修得掉那句失实声明，修不掉「文件里躺着单消费方符号」——按文件豁免等于给 core 留一个「顺手放」的口子，[ADR-037](#adr-037-实现可以离开-core--记忆整体迁往-providers) 堵的正是同类）；**下沉 model**（生产消费方不是 model，且会新造一条 session→model 的边）。

**后果与不变量**：`symbio_core` 根平铺导出收窄为六个符号——去处写在 `symbio_core/llm/mod.rs` 的模块表，理由写在本条，两者不互相复述。判据不变：任一留守符号的依赖方降到 1 就同样下沉，`llm_message_frame` 不构成先例；跨插件引用构造器即 E-009 违例。

---

## ADR-042: 跨插件调用一律经容器 `route` + 路径常量，不持有对方类型

**决策**：插件 A 调用插件 B 的唯一合法形态是 `parent.route(ctx.set(PATH, ROUTE_X))`
（`parent` 是容器弱引用）。调用方**不得** `use` 被调方的任何符号；
`ctx` 里的载荷类型必须住在 `symbio_core`。

**理由**：`agent_run` → `session/chat/send` 已经是这个形态，但它是惯例而非规则。
本方案新增一条边，若不把规则写下来，下一次就会有人直接持有 `Arc<ChatPlugin>`。

**后果**：跨插件载荷类型自动获得 core 准入资格（依赖方 ≥ 2）；
`route` 常量进 `symbio_core::plugin::route`，由既有审计核对常量名 ↔ 值。

## ADR-043: v2 事件地基落地——契约居中于中性层，`store` / `projection` 按冻结形状进 `symbio_core`

**决策**：`symbio_core` 新增五个模块：`event`（事件契约：`Seq` / `Entity` × `Verb` 闭集 /
信封）/ `view`（`View` + `Budget`）/ `store`（`Store` trait + `MemoryStore`，F1）/
`projection`（`Projection<V>`，F2）/ `invariants`（N1 / N3 / N5 可执行断言）。
事件与视图类型住**中性层**（`event` / `view`），`store` 与 `projection` 只 import 它们，
彼此零依赖——这是 [plan/05 §3.1](../../docs/plan/05-模块架构.md)「六包互不依赖」的构造侧。

**理由**：纯净性与 append-only 不靠约定靠编译器：`Seq` 私有构造（伪造序号编译不过）、
`Projection::new` 泛型约束（句柄进不来，J3）、`Store` trait 无 update / delete（接口上
不存在 = 违规不可能）。契约先于实现冻结（F1 / F2），S4 的 `wal` 实现、S1 起的各投影
都从这里长出，接口零改动。

**被否决的方案**：事件名用自由字符串（网格之外的事件无法拒绝，J3 失守）——改为枚举
闭集 + `kind` 字符串作数据；投影返回 `Result`（错误路径会把「预算不够」升级成「对话中断」，
与 I3 相悖）——改为 `View { degraded }` 类型义务；把事件类型放进 `store` 模块（③ 与 ④
互不依赖的规则会破）——拆中性层。

**后果与不变量**：`EventEnvelope: Clone + Send + Sync`（事件是纯数据，快照跨线程）；
`invariants` 的每条检查必须带反向用例（证明它红得起来也静得下去）；形状对应关系由
[`docs/plan/verify/`](../../docs/plan/verify/) 的可执行夹具钉住，改冻结形状 = 架构变更
（[plan/03 §1](../../docs/plan/03-演进与验证.md)）。

## ADR-044: 实测与判据同源——成本、时延与兜底率是事件网格的一等数据，不用旁路遥测

**状态**：已接受

**决策**：SLO 的三列实测口径全部从事件网格统计，不设任何旁路计数器：

1. **成本/时延**：埋点在 adapter 边界——`LlmAdapter::generate_timed`（trait 默认
   方法，`Instant` 计时），实测值由调用方写进 `chat.assistant.final` 的 `cost_ms`；
   ③ `cost_ledger` 从事件切片累计，② 熔断判据（`CircuitBreaker` 的「已耗多少」）
   读台账，不读运行时计数器；
2. **档位是数据**：turn 装配进哪一时延档 = `user.message` 载荷 `tier`
   （`LatencyTier::name` / `from_name` 往返）；缺失/未知进 `unspecified` 桶
   （可观测，不静默归类）；
3. **兜底率**：③ `fallback_rate` 按 turn 号把 `chat.assistant.fallback` 归到
   本轮声明的档位；投影只报数，达标判定交给读方——**SLO 是承诺，投影是事实**。

**理由**：N1（同一事件序列两次计算逐字节相同）是这套数字可复核的全部前提——
旁路计数器不可重放、不可离线审计、与事实源必然漂移；漂移的判据就是静默失效的
入口（预算判据说「还有余量」而台账说「已耗尽」时，错的若是台账，熔断形同虚设）。
成本与档位走 I1 单通道落格，可观测的每一项都对应一格事件，不存在「日志里才有」
的数字。

**被否决的方案**：metrics 库 / 全局 `AtomicU64` 旁路遥测（快、零事件量，但
N1 不适用、判据与事实源漂移、append-only 审计拿不到它）；把档位做成事件信封的
枚举字段（档位是调度决定，属数据不属契约形状——信封形状已冻结，见
[ADR-043](./core.md#adr-043-v2-事件地基落地契约居中于中性层store--projection-按冻结形状进-symbio_core)）；
在 `CircuitBreaker` 里内联计时（闸门自己测自己，多主体聚合口径即破坏）。

**后果与不变量**：真实链路的 final 必带实测 `cost_ms`（桩与真实适配器共享
同一埋点面，零改动）；兜底归属跟随本轮用户消息的档位声明，`unspecified`
桶非空 = 口径诚实（漏声明被看见而不是被猜）；时延四层预算初值经系统开销首测
（P50 = 1ms）+ 真实端点复测（本地模型 P50 ≈ 4.1s，深度档）双重背书保留为
正式值，实测记录在
[slo-calibration.md](../../docs/plan/slo-calibration.md)（历史数字不删，
可回看漂移）；真实流量的兜底率与 P99 待阶段三出数——口径已定，数字只欠流量。

## ADR-045: v2 事实桥——生产流量经转写进事实源，不等整体切换

**状态**：已接受

**决策**：v1 会话链路（`chat_loop`）的每一轮收束时，把本轮事实（用户发言 /
助手答复 / 实测耗时）**转写**为 v2 事件，落进 per-session 的持久事实源
（`<会话目录>/v2-events.wal`）：

1. 转写点唯一——`finish_turn`（所有出口的收尾单点）；模型耗时在 LLM
   唯一发起处累计（含工具轮多次请求），随收束写进 `cost_ms`（ADR-044 同一口径）；
2. 转写纪律：**中止（Aborted）与无增量恢复（ResumeDone）不转写**——轮未
   收束，网格少一格是诚实的缺口，不是假象；**重试（resume 重跑同一用户
   消息）各成一格**（attempt 递增）——失败与成功都是真实发生的收束；
3. 桥的故障只记警告不冒泡——**桥不得拖垮 v1 对话**，但警告必须被看见，
   不允许静默吞。

**理由**：整体把 `chat_loop` 切到 v2 链路是重建级工程（流式管线、工具轮、
中止与 UI 桥接都要重接），不能等它完成才让 SLO 出数——口径已定（ADR-044）
而数字只欠流量，缺口可以也应该现在补上。桥的位置在「旁路遥测」的禁区之外：
ADR-044 反对的是**不进事件网格的计数器**；桥是把 v1 流量的既有事实转写为
网格事件（同一事实落入事实源），N1 / N2 / N5 在转写产物上照常成立、照常被
`check_all` 审计。

**被否决的方案**：等整体切换后再出数（口径空转期无限延长，阶段三的 P99 /
兜底率永远没有生产数据）；旁路遥测（ADR-044 已否，不因「过渡期」破例）；
由 v2 链路做主的双写（v2 会话链路尚未接线，本末倒置）。

**后果与不变量**：v1 管线行为零变化（桥是纯增量记录）；转写产物与 core 侧
事实源同构——不变量全绿、溯源与轮次编号靠构造成立、重开恢复不破 N2；
档位恒 `deep`（v1 每轮装配完整模型与工具，这是事实不是假设）；未来整体切换
完成后，本桥退役为兼容层再移除，期间转写产物始终是可审计的事实源。
