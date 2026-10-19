---
kind: adr
id: ADR-000-架构决策汇总
title: 架构决策总览
status: active
projection: true
lists: [adr-entries=owner]
created: 2026-10-14
prefix: ADR
label: 架构决策
desc: 为什么这样设计——决策、理由与被否决的方案

titleEqualsH1: false
serial: adr
required: [adr, impl, domain]
optional: [reason, supersedes]
sections: [决策]
closedStatus: active
requiresDepsDone: [active]
retiredStates: [superseded, withdrawn]
status.active:
status.active.forbid: [reason]
status.superseded.require: [reason]
status.withdrawn.require: [reason]
impl.planned: 未实施
impl.partial: 部分实施
impl.done: 已落地
impl.none: 未落地
domain.平台基座: 插件架构 / 路由 / 注册 / 执行接口 / symbio_core 准入 / 身份 / 生命周期 / 对象创建
domain.会话与执行: 编排 / 上下文 / 转写节点与实时面 / 输入 / 响应性
domain.资源与地址: VdfsProvider / 读侧成本 / 变更信封 / 新建与导入 / 地址归属
domain.模型与协议: 多协议适配 / TLS / 本地嵌入 / SSE 解析
domain.智能体与记忆: agent 目录宿主 / 记忆的两个作用域
summary.order: [active, superseded, withdrawn]
summary.active.title: 现行决策（active）
summary.active.columns: [id, title:决策, domain, impl, supersedes]
summary.superseded.title: 已被取代（superseded）
summary.superseded.columns: [id, title:决策, domain, reason, supersedes]
summary.withdrawn.title: 已回退 / 未落地（withdrawn）
summary.withdrawn.columns: [id, title:决策, reason]
summary.groupBy: domain
summary.groupByHeading: 按域分组
summary.groupBy.columns: [id, title:决策, status, impl]
---

# 架构决策总览

ADR 回答「为什么这样设计」：每条单文档，正文讲决策与被否决的方案。status 管「还算不算数」，impl 管「做到哪一步」，域（domain）管「约束谁」——三个正交字段，词表与闭集由本页 meta 声明。

<!-- registry:summary begin -->
<!-- list:adr-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../008-spec/001.md)，机制见 [PL-011 · 登记与汇总机制](../004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")。

**计数**：active 83 · superseded 7 · withdrawn 3（共 93）

## 现行决策（active）

| 编号 | 决策 | 域 | 落地 | 取代 |
|---|---|---|---|---|
| `ADR-001` | 分形插件架构：容器与叶子同接口 | 平台基座 | 已落地 | — |
| `ADR-002` | 路径即路由 | 平台基座 | 已落地 | — |
| `ADR-003` | Session 作为编排入口 | 平台基座 | 已落地 | — |
| `ADR-004` | 多协议 LLM 适配（4 套适配器） | 模型与协议 | 已落地 | — |
| `ADR-005` | Agent 目录宿主（agent-dir/v2） | 智能体与记忆 | 已落地 | — |
| `ADR-006` | 薄宿主层：Tauri 仅 3 个 command | 平台基座 | 已落地 | — |
| `ADR-007` | 静态注册（inventory） | 平台基座 | 已落地 | — |
| `ADR-011` | 资源存储 = VdfsProvider 的三个集中实现 | 资源与地址 | 已落地 | — |
| `ADR-012` | 读侧成本是设计约束——现在是什么必须有一张可核对的表 | 资源与地址 | 已落地 | — |
| `ADR-013` | TLS 后端 = 平台原生栈（native-tls），不做纯 Rust 密码学 | 模型与协议 | 已落地 | — |
| `ADR-015` | 前端显示由节点状态驱动，不由事件顺序驱动 | 会话与执行 | 已落地 | — |
| `ADR-016` | 本地嵌入改用 ort（ONNX Runtime）——推翻 ADR-014 的性能前提 | 模型与协议 | 已落地 | ADR-014 |
| `ADR-018` | 压缩失败不得裁剪历史——失败必须是可见、可重试、可持久化的状态 | 会话与执行 | 已落地 | — |
| `ADR-019` | 跨栈契约手工镜像 + 审计守卫；G3 前端面板去语义否决 | 平台基座 | 已落地 | — |
| `ADR-020` | EventSink + AbortSignal 取代 PluginChannel 的双职责 | 平台基座 | 已落地 | — |
| `ADR-021` | 两个执行接口同形（ExecEnv） | 平台基座 | 已落地 | — |
| `ADR-022` | SSE 增量解析：契约拆两个方法，字段名留协议层 | 模型与协议 | 已落地 | — |
| `ADR-023` | symbio_core 的准入规则 = 依赖方数量 | 平台基座 | 已落地 | ADR-037、ADR-038 |
| `ADR-024` | 会话选项并入详情方言——选项行是配置表单的字段，不是独立协议 | 会话与执行 | 已落地 | — |
| `ADR-025` | 顺序是节点属性；delta 是 updated 的传输形态 | 会话与执行 | 已落地 | — |
| `ADR-026` | 子智能体的完整会话在 VDFS 空间内自驱动（inbox） | 会话与执行 | 已落地 | — |
| `ADR-027` | 可新建的东西至多一种（new_type 清单退役） | 资源与地址 | 已落地 | — |
| `ADR-028` | VdfsRequest::Move 不收——移动是外层组合，不是核心原语 | 资源与地址 | 已落地 | — |
| `ADR-029` | 导入是详情页动作，不是类型入口——VdfsNewType 收成纯呈现定义 | 资源与地址 | 已落地 | — |
| `ADR-030` | 地址属于条目，不属于节点——VdfsItem 拆出，写回执只给名字 | 资源与地址 | 已落地 | — |
| `ADR-031` | 会话的输入 = 地址上的写入 / 动作（chat/send 与 chat/abort 退役） | 会话与执行 | 已落地 | — |
| `ADR-032` | 插件身份归 PLUGIN.yml——PluginMeta 从元信息降为出厂自述 | 平台基座 | 部分实施 | — |
| `ADR-033` | 生命周期钩子：start 同步、stop 异步，停用与卸载各给理由 | 平台基座 | 已落地 | — |
| `ADR-035` | provider 化的三个条件与构造契约 | 平台基座 | 已落地 | — |
| `ADR-036` | 对象创建机制独立成 creator 域——它是系统级反射机制，不是插件专属 | 平台基座 | 已落地 | — |
| `ADR-039` | session 按域重组：一域一目录，编排层与领域层分离 | 会话与执行 | 已落地 | ADR-017 |
| `ADR-040` | work 并入 memory：记忆的两个作用域同属一个所有者 | 智能体与记忆 | 已落地 | — |
| `ADR-041` | 会话的响应性由编排层调度 + 无副作用对话服务承担 | 会话与执行 | 已落地 | — |
| `ADR-042` | 跨插件调用一律经容器 route + 路径常量，不持有对方类型 | 平台基座 | 已落地 | — |
| `ADR-043` | v2 事件地基：契约居中，store / projection 按冻结形状进 core | 平台基座 | 已落地 | — |
| `ADR-044` | 实测与判据同源：成本 / 时延 / 兜底率是事件网格一等数据 | 平台基座 | 已落地 | ADR-045 |
| `ADR-046` | 事实能力进 symbio_core：不以「可选插件」承载必须有值的机制 | 平台基座 | 已落地 | — |
| `ADR-047` | 委派者三项真源：判定回 reason、能力走注入、进展读磁盘事实 | 会话与执行 | 已落地 | — |
| `ADR-048` | 出厂档位改 full——v2 链路是出厂路径，bridge 退役 | 会话与执行 | 已落地 | — |
| `ADR-048a` | v2 的 prompt 形状必须回到结构化消息（不把工具结果拍平成纯文本） | 会话与执行 | 已落地 | — |
| `ADR-049` | WAL 是会话唯一事实源，SessionStore 降为缓存——轨 S 的起点 | 会话与执行 | 已落地 | — |
| `ADR-051` | v1 执行档与 V2Mode 旋钮退役——v2 是唯一执行路径 | 会话与执行 | 已落地 | — |
| `ADR-052` | 事实的边界是「发生过什么」：思考与半截产出入格——真源切换的前置 | 会话与执行 | 已落地 | — |
| `ADR-053` | 消息的变更即事实：编辑 / 删除入格，压缩走快照——T-056 的 CRUD 前置 | 会话与执行 | 已落地 | — |
| `ADR-054` | 消息列表的变更收成闭集原语 + 顺序反转 + 类型强制——b3 的纪律与强制 | 会话与执行 | 已落地 | — |
| `ADR-055` | 核心形状棘轮：symbio_core 的公共面只许收窄，放宽必须挂得住一条决策 | 平台基座 | 已落地 | — |
| `ADR-056` | 登记条目库迁入 docs/——位置不再决定判据适用范围 | 平台基座 | 已落地 | — |
| `ADR-057` | prompt 形状基线钉「装配决定的结构」，规模只作观测 | 会话与执行 | 已落地 | — |
| `ADR-058` | 「跑过真实会话」的落地形态 = 门内有界压测，判据读生产出口——换基边界由此归出一条纪律三个出口 | 会话与执行 | 已落地 | — |
| `ADR-059` | 能力坐标系的科学化：轴要有刻度、天梯不再另立语义、每条判断写明反证现象 | 平台基座 | 已落地 | — |
| `ADR-060` | 覆盖率阈值不进门禁：不做逐 crate 的 `cargo-llvm-cov` floor，理由与重开条件写在这里 | 平台基座 | 已落地 | — |
| `ADR-061` | store = distributed 从值域退役：多副本不在计划内，留一格「将来也许能用」比删掉更脏 | 平台基座 | 已落地 | — |
| `ADR-062` | 事实完整性收口：actor 在写入方按真实来源派生，ts 兜底在 WalStore 唯一写入口（MemoryStore 不兜底） | 平台基座 | 已落地 | — |
| `ADR-063` | 一个会话线程对应一个显示区域：「对话 / 工作」分栏呈现退役，工作过程并入同一窗口 | 会话与执行 | 已落地 | — |
| `ADR-064` | T-124 的「调度器 / 租约 / cap」不是已落地的数据行——本阶真实增量收窄为「单写者 FIFO + 借来的 held」 | 会话与执行 | 已落地 | — |
| `ADR-065` | 对等侧收束：`commitment.asserted` 缺的不是「写方」而是「对等 offer/assert」——② 与 ③ 同一批，收束格按种类判 | 智能体与记忆 | 已落地 | — |
| `ADR-066` | 输入面退役的前置条件解除：地址写入不承载运行选项——三项运行参数已有会话 metadata 回退，协议面不必变宽 | 会话与执行 | 已落地 | — |
| `ADR-067` | 产物收束的不变量面：C4 认产物格、`closure_denied` 数产物、恢复锚刻意不认——三处口径一次说清 | 智能体与记忆 | 已落地 | — |
| `ADR-068` | 审批闸门住在能力注册的收口点（`DefaultToolVisitor::register`），判据一份、入口两处 | 会话与执行 | 已落地 | — |
| `ADR-069` | Principal 三字段——kind 与 id 分家，id 是可路由地址 | 平台基座 | 已落地 | — |
| `ADR-070` | NodeKind 是唯一前端分派轴——style_tag 降级为后端注记，呈现靠 title/icon/actions 数据 | 会话与执行 | 部分实施 | — |
| `ADR-071` | RowKind 封闭集 6 值 → 7 值（+Configurable）——4e 收集口收敛的裁决 | 平台基座 | 已落地 | — |
| `ADR-072` | `context_compact` 工具与 55% 水位提醒一并退役；压缩只剩一个入口 | 会话与执行 | 已落地 | — |
| `ADR-073` | `TurnState` 的轮内簿记与闸门「等工具齐」一并退场；闸门只剩退出条件 | 会话与执行 | 已落地 | — |
| `ADR-074` | 恒 0 的轮内计数退出主循环：工具轮软上限整条退役，抽干点搬到批次边界 | 会话与执行 | 已落地 | — |
| `ADR-075` | 卸载能力先于目录分离：session 切分立三条前置，记忆两腿与存储留守 | 平台基座 | 部分实施 | — |
| `ADR-076` | 心跳片的八个宿主触点逐条落位：调度随片走，触发事实落进收件箱写臂 | 平台基座 | 已落地 | — |
| `ADR-077` | 心跳片的落点定为一层的插件目录：卸载单位与第二个消费单位都由它给出 | 平台基座 | 已落地 | — |
| `ADR-078` | 请求视图三层淡化退役：请求体是事实网格的纯投影，注入段只追加不改写 | 会话与执行 | 已落地 | — |
| `ADR-079` | 同名两物：core 的 ActorSpec.pattern 是主体分类，S11 §4 的 actor.pattern 是开关参数——前者不该做分派点 | 会话与执行 | 已落地 | — |
| `ADR-080` | 内部请求（不带工具、不产生对话轮次帧）不受档位闸门约束——三条已登记，classify 的去留归 T-162 | 模型与协议 | 已落地 | — |
| `ADR-081` | P4：驱动器不解存储、不把存储内部对象拿回家、构造只住在收敛手柄——批次 6 分 6a / 6b | 平台基座 | 已落地 | — |
| `ADR-082` | 6b：驱动器成片落为独立插件 loop——触发走地址面、服务走端口、词表先上移，分四刀 | 平台基座 | 已落地 | — |
| `ADR-083` | 自上而下再推导：废除「服务端口进 core」——跨插件通货闭族在既有机制词汇，切片补立前置 P5「运行态归源」 | 平台基座 | 已落地 | — |
| `ADR-084` | 能力模块公用基础协议：上下文只递值，事实只经引擎——A/B/C/D/E 五类需要的唯一取得方式 | 平台基座 | 已落地 | — |
| `ADR-085` | 压缩取代的是消息列表，不是轮次生命周期——恢复锚读完整史，归档刀口不动 | 会话与执行 | 已落地 | — |
| `ADR-086` | loop↔session 切片契约：TurnDispatch 进程内交付、TurnSpec 纯值面、内部动作走地址面 | 平台基座 | 已落地 | — |
| `ADR-087` | 契约词按「它描述什么」分域：取消 schemas 目录，机制只向下依赖词汇 | 平台基座 | 已落地 | — |
| `ADR-088` | 环的指令集是闭集：Capsule 收成枚举，Step 五值与 drive 不成立 | 会话与执行 | 已落地 | — |
| `ADR-089` | 「一表」的准确表述：一个行形状 + 两个目录（按消费方分），删除 Policy 死变体 | 平台基座 | 已落地 | — |
| `ADR-090` | 三套序各有唯一 owner：依赖序是硬约束、价值序是权重、施工序只在可行域内 | 平台基座 | 已落地 | — |
| `ADR-091` | 事件不做 schema 版本化：用「只增不改」换掉一个没有读方的版本号 | 平台基座 | 已落地 | — |
| `ADR-092` | 本阶不做 A2A 对外互操作：定位是本地平台，跨平台协商等有第二个真实对端再说 | 平台基座 | 已落地 | — |

## 已被取代（superseded）

| 编号 | 决策 | 域 | 理由 | 取代 |
|---|---|---|---|---|
| `ADR-010` | 统一实体管理：entities/* 契约 + EntityCapabilities | 资源与地址 | entities/*、能力开关、EntityProvider trait + VdfsAdapter、StorageService 已逐层删除；资源访问统一经 VDFS（ADR-011） | — |
| `ADR-014` | 本地嵌入 = tract-onnx 纯 Rust 推理（弃 fastembed / ORT） | 模型与协议 | tract 已完全退出依赖树；取代者：ADR-016 | — |
| `ADR-017` | session 规模债暂不拆分（带触发条件） | 会话与执行 | 三条触发条件全部命中；取代者：ADR-039 | — |
| `ADR-034` | SSE 契约的位置条款（契约在 core，字段名在协议层） | 模型与协议 | 契约随流循环下沉迁入 plugins/model/protocols/sse.rs，位置条款失效；取代者：ADR-022 | — |
| `ADR-037` | 共享 ≠ 契约：判据是依赖方向，含具体逻辑的实现默认住 providers/ | 平台基座 | 判据即 ADR-023 的准入规则，无独立内容；取代者：ADR-023 | — |
| `ADR-038` | symbio_core 准入规则的逐条执行（数的是符号不是文件） | 平台基座 | 唯一登记例外 llm_message_frame；取代者：ADR-023 | — |
| `ADR-045` | v2 事实桥：生产流量经转写进事实源，不等整体切换 | 平台基座 | 转写纪律并入 ADR-044；桥退役见 ADR-048；机制见 symbio/src/plugins/session/docs/017-module-layout.md；取代者：ADR-044 | — |

## 已回退 / 未落地（withdrawn）

| 编号 | 决策 | 理由 |
|---|---|---|
| `ADR-008` | 多存储后端：会话存储可选 file / sqlite / memory | 三后端与 store_kind 已删除，会话存储收为单一 SessionStore（持久 = 磁盘布局 / 临时 = 进程内驻留）。与本条无关的资源存储见 ADR-011 |
| `ADR-009` | 机制化认知 v9：CU + prop 驱动的认知层 | CU 与认知层已从代码移除（分类残余亦已清理） |
| `ADR-050` | TurnRunner 族按依赖方数量下沉 plugins/session | 非架构决策 / 特例——下沉即应用 ADR-023 |

## 按域分组

### 平台基座（42）

> 插件架构 / 路由 / 注册 / 执行接口 / symbio_core 准入 / 身份 / 生命周期 / 对象创建

| 编号 | 决策 | 状态 | 落地 |
|---|---|---|---|
| `ADR-001` | 分形插件架构：容器与叶子同接口 | active | 已落地 |
| `ADR-002` | 路径即路由 | active | 已落地 |
| `ADR-003` | Session 作为编排入口 | active | 已落地 |
| `ADR-006` | 薄宿主层：Tauri 仅 3 个 command | active | 已落地 |
| `ADR-007` | 静态注册（inventory） | active | 已落地 |
| `ADR-009` | 机制化认知 v9：CU + prop 驱动的认知层 | withdrawn | 未落地 |
| `ADR-019` | 跨栈契约手工镜像 + 审计守卫；G3 前端面板去语义否决 | active | 已落地 |
| `ADR-020` | EventSink + AbortSignal 取代 PluginChannel 的双职责 | active | 已落地 |
| `ADR-021` | 两个执行接口同形（ExecEnv） | active | 已落地 |
| `ADR-023` | symbio_core 的准入规则 = 依赖方数量 | active | 已落地 |
| `ADR-032` | 插件身份归 PLUGIN.yml——PluginMeta 从元信息降为出厂自述 | active | 部分实施 |
| `ADR-033` | 生命周期钩子：start 同步、stop 异步，停用与卸载各给理由 | active | 已落地 |
| `ADR-035` | provider 化的三个条件与构造契约 | active | 已落地 |
| `ADR-036` | 对象创建机制独立成 creator 域——它是系统级反射机制，不是插件专属 | active | 已落地 |
| `ADR-037` | 共享 ≠ 契约：判据是依赖方向，含具体逻辑的实现默认住 providers/ | superseded | 已落地 |
| `ADR-038` | symbio_core 准入规则的逐条执行（数的是符号不是文件） | superseded | 已落地 |
| `ADR-042` | 跨插件调用一律经容器 route + 路径常量，不持有对方类型 | active | 已落地 |
| `ADR-043` | v2 事件地基：契约居中，store / projection 按冻结形状进 core | active | 已落地 |
| `ADR-044` | 实测与判据同源：成本 / 时延 / 兜底率是事件网格一等数据 | active | 已落地 |
| `ADR-045` | v2 事实桥：生产流量经转写进事实源，不等整体切换 | superseded | 已落地 |
| `ADR-046` | 事实能力进 symbio_core：不以「可选插件」承载必须有值的机制 | active | 已落地 |
| `ADR-055` | 核心形状棘轮：symbio_core 的公共面只许收窄，放宽必须挂得住一条决策 | active | 已落地 |
| `ADR-056` | 登记条目库迁入 docs/——位置不再决定判据适用范围 | active | 已落地 |
| `ADR-059` | 能力坐标系的科学化：轴要有刻度、天梯不再另立语义、每条判断写明反证现象 | active | 已落地 |
| `ADR-060` | 覆盖率阈值不进门禁：不做逐 crate 的 `cargo-llvm-cov` floor，理由与重开条件写在这里 | active | 已落地 |
| `ADR-061` | store = distributed 从值域退役：多副本不在计划内，留一格「将来也许能用」比删掉更脏 | active | 已落地 |
| `ADR-062` | 事实完整性收口：actor 在写入方按真实来源派生，ts 兜底在 WalStore 唯一写入口（MemoryStore 不兜底） | active | 已落地 |
| `ADR-069` | Principal 三字段——kind 与 id 分家，id 是可路由地址 | active | 已落地 |
| `ADR-071` | RowKind 封闭集 6 值 → 7 值（+Configurable）——4e 收集口收敛的裁决 | active | 已落地 |
| `ADR-075` | 卸载能力先于目录分离：session 切分立三条前置，记忆两腿与存储留守 | active | 部分实施 |
| `ADR-076` | 心跳片的八个宿主触点逐条落位：调度随片走，触发事实落进收件箱写臂 | active | 已落地 |
| `ADR-077` | 心跳片的落点定为一层的插件目录：卸载单位与第二个消费单位都由它给出 | active | 已落地 |
| `ADR-081` | P4：驱动器不解存储、不把存储内部对象拿回家、构造只住在收敛手柄——批次 6 分 6a / 6b | active | 已落地 |
| `ADR-082` | 6b：驱动器成片落为独立插件 loop——触发走地址面、服务走端口、词表先上移，分四刀 | active | 已落地 |
| `ADR-083` | 自上而下再推导：废除「服务端口进 core」——跨插件通货闭族在既有机制词汇，切片补立前置 P5「运行态归源」 | active | 已落地 |
| `ADR-084` | 能力模块公用基础协议：上下文只递值，事实只经引擎——A/B/C/D/E 五类需要的唯一取得方式 | active | 已落地 |
| `ADR-086` | loop↔session 切片契约：TurnDispatch 进程内交付、TurnSpec 纯值面、内部动作走地址面 | active | 已落地 |
| `ADR-087` | 契约词按「它描述什么」分域：取消 schemas 目录，机制只向下依赖词汇 | active | 已落地 |
| `ADR-089` | 「一表」的准确表述：一个行形状 + 两个目录（按消费方分），删除 Policy 死变体 | active | 已落地 |
| `ADR-090` | 三套序各有唯一 owner：依赖序是硬约束、价值序是权重、施工序只在可行域内 | active | 已落地 |
| `ADR-091` | 事件不做 schema 版本化：用「只增不改」换掉一个没有读方的版本号 | active | 已落地 |
| `ADR-092` | 本阶不做 A2A 对外互操作：定位是本地平台，跨平台协商等有第二个真实对端再说 | active | 已落地 |

### 会话与执行（33）

> 编排 / 上下文 / 转写节点与实时面 / 输入 / 响应性

| 编号 | 决策 | 状态 | 落地 |
|---|---|---|---|
| `ADR-008` | 多存储后端：会话存储可选 file / sqlite / memory | withdrawn | 未落地 |
| `ADR-015` | 前端显示由节点状态驱动，不由事件顺序驱动 | active | 已落地 |
| `ADR-017` | session 规模债暂不拆分（带触发条件） | superseded | 已落地 |
| `ADR-018` | 压缩失败不得裁剪历史——失败必须是可见、可重试、可持久化的状态 | active | 已落地 |
| `ADR-024` | 会话选项并入详情方言——选项行是配置表单的字段，不是独立协议 | active | 已落地 |
| `ADR-025` | 顺序是节点属性；delta 是 updated 的传输形态 | active | 已落地 |
| `ADR-026` | 子智能体的完整会话在 VDFS 空间内自驱动（inbox） | active | 已落地 |
| `ADR-031` | 会话的输入 = 地址上的写入 / 动作（chat/send 与 chat/abort 退役） | active | 已落地 |
| `ADR-039` | session 按域重组：一域一目录，编排层与领域层分离 | active | 已落地 |
| `ADR-041` | 会话的响应性由编排层调度 + 无副作用对话服务承担 | active | 已落地 |
| `ADR-047` | 委派者三项真源：判定回 reason、能力走注入、进展读磁盘事实 | active | 已落地 |
| `ADR-048` | 出厂档位改 full——v2 链路是出厂路径，bridge 退役 | active | 已落地 |
| `ADR-048a` | v2 的 prompt 形状必须回到结构化消息（不把工具结果拍平成纯文本） | active | 已落地 |
| `ADR-049` | WAL 是会话唯一事实源，SessionStore 降为缓存——轨 S 的起点 | active | 已落地 |
| `ADR-050` | TurnRunner 族按依赖方数量下沉 plugins/session | withdrawn | 已落地 |
| `ADR-051` | v1 执行档与 V2Mode 旋钮退役——v2 是唯一执行路径 | active | 已落地 |
| `ADR-052` | 事实的边界是「发生过什么」：思考与半截产出入格——真源切换的前置 | active | 已落地 |
| `ADR-053` | 消息的变更即事实：编辑 / 删除入格，压缩走快照——T-056 的 CRUD 前置 | active | 已落地 |
| `ADR-054` | 消息列表的变更收成闭集原语 + 顺序反转 + 类型强制——b3 的纪律与强制 | active | 已落地 |
| `ADR-057` | prompt 形状基线钉「装配决定的结构」，规模只作观测 | active | 已落地 |
| `ADR-058` | 「跑过真实会话」的落地形态 = 门内有界压测，判据读生产出口——换基边界由此归出一条纪律三个出口 | active | 已落地 |
| `ADR-063` | 一个会话线程对应一个显示区域：「对话 / 工作」分栏呈现退役，工作过程并入同一窗口 | active | 已落地 |
| `ADR-064` | T-124 的「调度器 / 租约 / cap」不是已落地的数据行——本阶真实增量收窄为「单写者 FIFO + 借来的 held」 | active | 已落地 |
| `ADR-066` | 输入面退役的前置条件解除：地址写入不承载运行选项——三项运行参数已有会话 metadata 回退，协议面不必变宽 | active | 已落地 |
| `ADR-068` | 审批闸门住在能力注册的收口点（`DefaultToolVisitor::register`），判据一份、入口两处 | active | 已落地 |
| `ADR-070` | NodeKind 是唯一前端分派轴——style_tag 降级为后端注记，呈现靠 title/icon/actions 数据 | active | 部分实施 |
| `ADR-072` | `context_compact` 工具与 55% 水位提醒一并退役；压缩只剩一个入口 | active | 已落地 |
| `ADR-073` | `TurnState` 的轮内簿记与闸门「等工具齐」一并退场；闸门只剩退出条件 | active | 已落地 |
| `ADR-074` | 恒 0 的轮内计数退出主循环：工具轮软上限整条退役，抽干点搬到批次边界 | active | 已落地 |
| `ADR-078` | 请求视图三层淡化退役：请求体是事实网格的纯投影，注入段只追加不改写 | active | 已落地 |
| `ADR-079` | 同名两物：core 的 ActorSpec.pattern 是主体分类，S11 §4 的 actor.pattern 是开关参数——前者不该做分派点 | active | 已落地 |
| `ADR-085` | 压缩取代的是消息列表，不是轮次生命周期——恢复锚读完整史，归档刀口不动 | active | 已落地 |
| `ADR-088` | 环的指令集是闭集：Capsule 收成枚举，Step 五值与 drive 不成立 | active | 已落地 |

### 资源与地址（7）

> VdfsProvider / 读侧成本 / 变更信封 / 新建与导入 / 地址归属

| 编号 | 决策 | 状态 | 落地 |
|---|---|---|---|
| `ADR-010` | 统一实体管理：entities/* 契约 + EntityCapabilities | superseded | 未落地 |
| `ADR-011` | 资源存储 = VdfsProvider 的三个集中实现 | active | 已落地 |
| `ADR-012` | 读侧成本是设计约束——现在是什么必须有一张可核对的表 | active | 已落地 |
| `ADR-027` | 可新建的东西至多一种（new_type 清单退役） | active | 已落地 |
| `ADR-028` | VdfsRequest::Move 不收——移动是外层组合，不是核心原语 | active | 已落地 |
| `ADR-029` | 导入是详情页动作，不是类型入口——VdfsNewType 收成纯呈现定义 | active | 已落地 |
| `ADR-030` | 地址属于条目，不属于节点——VdfsItem 拆出，写回执只给名字 | active | 已落地 |

### 模型与协议（7）

> 多协议适配 / TLS / 本地嵌入 / SSE 解析

| 编号 | 决策 | 状态 | 落地 |
|---|---|---|---|
| `ADR-004` | 多协议 LLM 适配（4 套适配器） | active | 已落地 |
| `ADR-013` | TLS 后端 = 平台原生栈（native-tls），不做纯 Rust 密码学 | active | 已落地 |
| `ADR-014` | 本地嵌入 = tract-onnx 纯 Rust 推理（弃 fastembed / ORT） | superseded | 未落地 |
| `ADR-016` | 本地嵌入改用 ort（ONNX Runtime）——推翻 ADR-014 的性能前提 | active | 已落地 |
| `ADR-022` | SSE 增量解析：契约拆两个方法，字段名留协议层 | active | 已落地 |
| `ADR-034` | SSE 契约的位置条款（契约在 core，字段名在协议层） | superseded | 未落地 |
| `ADR-080` | 内部请求（不带工具、不产生对话轮次帧）不受档位闸门约束——三条已登记，classify 的去留归 T-162 | active | 已落地 |

### 智能体与记忆（4）

> agent 目录宿主 / 记忆的两个作用域

| 编号 | 决策 | 状态 | 落地 |
|---|---|---|---|
| `ADR-005` | Agent 目录宿主（agent-dir/v2） | active | 已落地 |
| `ADR-040` | work 并入 memory：记忆的两个作用域同属一个所有者 | active | 已落地 |
| `ADR-065` | 对等侧收束：`commitment.asserted` 缺的不是「写方」而是「对等 offer/assert」——② 与 ③ 同一批，收束格按种类判 | active | 已落地 |
| `ADR-067` | 产物收束的不变量面：C4 认产物格、`closure_denied` 数产物、恢复锚刻意不认——三处口径一次说清 | active | 已落地 |

<!-- registry:summary end -->

