//! 事件契约 —— v2 事件溯源的共享类型层（[plan/01 §2 / §6](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! ## 为什么是独立模块
//!
//! 施工视图的六个包（`actors` / `projections` / `store` / `adapters` / `governance`）
//! 彼此**禁止互相依赖**，唯一例外是「事件与 View 的类型定义」——那是共享契约
//! （[plan/05 §3.1](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")，01 §13.5「契约从边上挪到中心」）。
//! 因此事件类型必须住在**中性层**：`store`（④）与 `projection`（③）都只 import 本模块，
//! 谁也拿不到谁的句柄。
//!
//! ## 冻结锚点（[plan/03 §1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）
//!
//! - **F5**：动词 5 个、能力 7 个、模式 3 个 —— 本模块冻结动词与实体的**闭集**
//!   （`Verb` / `Entity` 枚举）。新增格子 = `match` 加一臂，是数据变化不是机制变化；
//!   枚举而不是字符串，是为了让"网格之外的事件"**编译不过**（J3）。
//!   注意 J3 钉住的是**行与列**这两维：字符串确实写不出第 12 个实体。
//! - 事件**语法网格**（01 §6）：`实体 × 动词` 的格子上有若干**名字**（如
//!   `chat.assistant.final` 落在 `turn × closed`）。名字是数据，加名字不加枚举。
//!   于是「名字 ↔ 格位」这条对应关系本身要有个机器读得到的主人：[`grid`] 表。
//!   它的判据不在编译期（名字是字符串），而在**唯一写入口**——不在表上的名字、
//!   与自述格位不一致的名字，都落不进事实源（`AppendError::OffGrid`）。

use serde::{Deserialize, Serialize};

/// 名字 ↔ 格位的表：判据、理由与「为什么不是枚举」都写在它的模块头。
pub mod grid;
pub use grid::{event_cell, Cell};

/// 一个**事件名**的声明：名字与它在网格上的格位。
///
/// 这是事件词汇的登记口（DE-024 D-197）：`Entity` × `Verb` 是封闭语法，**名字不是**。
/// 加一个事件名 = 在 owner 插件里加一行 `EventDecl`，内核不认识这个名字。
///
/// ## 为什么住在 `event` 域而不是 `decl`
///
/// 它声明的是「这个名字落在哪一格」——回答的是事件词汇与网格的对应，不是载荷形状。
/// `decl` 域（`FactDecl` / `ViewDecl` / `ActorDecl`）答的是另一组问题：字段、读法、
/// 执行体收口。判据 7 的输入是 `FactDecl`，网格的输入是本类型，两者合并会逼其中
/// 一方说谎（边界分析见 [`crate::decl`]）。
///
/// 两表同名不冲突：一条事实若要进环，同时交 `EventDecl` 与 `FactDecl` 即可；`seal`
/// 只要求**插件声明的事实**都能在事件表上找到同名行（协议五格由内核预置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDecl {
    /// 事件名（线上 `kind` 的字面值）。
    pub name: &'static str,
    /// 名字落在哪一格。写闸按它拒绝「自述挪了一格」的事件。
    pub cell: Cell,
}

/// 不可伪造的单调序号（[plan/01 §2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 字段私有：**只有 `Store` 能构造**（`pub` 在 kernel crate 内，等价于拆分前的
/// `pub`），调用方只能拿到与比较——自己 `Seq(5)` 伪造一个序号编译不过。
/// 这就是「类型强制单调」的构造侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Seq(u64);

impl Seq {
    /// 仅事件源实现可构造（kernel crate 内；对 `symbio_core` 是同一份共享语法）。
    pub fn new(v: u64) -> Self {
        Seq(v)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

/// 事件时间戳：Unix 毫秒（UTC），口径同 `clock_now_ms`。
pub type Timestamp = i64;

// ── 事件名字表（S01：turn/* 全部，[roadmap/S01 §3](../../../../docs/003-design/031-最小闭环.md "DE-031-最小闭环")）────
//
// 名字是**数据**（加名字不加枚举，01 §6）；但名字必须**单点定义**——散在调用点的
// 字符串字面量必然漂移（同 merge_supplements 的单点判据）。
//
// **归属（DE-024 D-197）**：这里只留内核自己持有的名字（协议五格与尚未迁出的域账本）。
// 插件持有的事件名住在它自己的 `EventDecl` 上，格位随名字一起声明，内核不再认识它们。
// 仍在 [`grid`] 表上的名字以那张表为判据；已迁出的名字以 owner 的声明为判据。

/// 用户发言。落在 `turn × opened`。
pub const EVENT_USER_MESSAGE: &str = "user.message";
/// **本轮的派发档**（载荷 `{ tier, budget_ms, principal }`）。落在 `turn × asserted`。
///
/// ## 为什么档位从 `user.message` 里分出来
///
/// 用户格必须在**任何判决之前**落（`owed` 要拿它匹配"这一轮开始了"），而档位要到
/// 召回 / 技能命中之后才知道（反射档 vs 深度档）。把档位留在用户格上，就只能要么
/// 提前写一个还没定的值（SLO / 兜底率按它分组 ⇒ 统计说谎），要么把用户格推回判决
/// 之后（输入闭包重新破掉）。本格是第三条路：开轮格只说"用户说了什么"，本格说
/// "这一轮按哪一档跑"——与 [D-044] 的「档位是数据」同一条事实，只是分两次落。
///
/// 读方（`projection::fallback::declared_tier` / `slo` / `invariants::declared_budget_ms`）
/// 优先读本格；历史 WAL 没有本格时回退读 `user.message` 的 `tier` / `budget_ms`。
///
/// 决策 D-044 见 [D-044](../../../../docs/003-design/016-工程落地与验证.md#d-044-实测与判据同源成本--时延--兜底率是事件网格一等数据 "D-044")
pub const EVENT_TURN_DISPATCH: &str = "turn.dispatch";

/// **轮边界折进的补充**（载荷 `{ text, count }`）。落在 `turn × asserted`。
///
/// ## 为什么它必须是一格事实（缺口 3）
///
/// 用户在助手干活途中补了一句话，那句话被折进 `context.messages` —— 但 `TurnInput.text`
/// 在**轮首就定死了**，于是这句话：
///
/// - 进了消息列表（会落库、会进 `messages.json`），
/// - **没进事实网格**，而 prompt 是从事实网格投影的 ⇒ **模型看不见它**，
/// - 且**完全静默**：不报错、不告警、既有断言全绿。
///
/// 「会话接受了一句用户话，但它从未成为事实」——补一句成为事实，最小的格子就是它。
///
/// ## 为什么不是 `user.message` 的第二次
///
/// `user.message` 落在 `turn × opened`，语义是「开轮」。轮已经开了（补充发生在
/// 干活途中），再落一条 `opened` 会让「一个 turn 一个开轮格」这条不变量失效。
///
/// ## `count`：一次可能折进多条
///
/// 抽干是**批量**的（`merge_supplements` 把同窗口内的多条合并成一条消息）。
/// `text` 是合并后的正文，`count` 是**原条数**——记下来是因为「模型收到一大段」
/// 与「用户连说了 5 句」对模型是不同的输入，而网格不该只保留前者。
pub const EVENT_TURN_SUPPLEMENTED: &str = "turn.supplemented";

/// **重试开出另一支**（载荷 `{ of: <被重试的 Turn 根 id>, into: <新支的 Turn 根 id> }`）。
/// 落在 `turn × asserted`。
///
/// ## 它说的是「不删那一支」
///
/// 删除式重试（把失败轮连子树一起 `remove-nodes`）会把两件事混成一件：**这一次不再
/// 读了**（呈现）与**这一次没发生过**（事实）。前者不需要动存储——它由选中态说；后者
/// 是假的，那次往返确实发生、确实产出过半截答案与真跑过的工具。本格因此不删：旧支
/// 留在树里，只是不再是当前读的那一支。裁决见
/// [D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")。
///
/// ## 为什么载荷是**节点 id** 而不是轮号
///
/// 轮号与 Turn 根**不是一对一**：一幕起一个新根，幕间续写（被长度上限掐断的那条路）
/// 让同一轮开出多个根。按轮号归组会把「同一轮的第二幕」误读成「另一支」，而按根 id
/// 归组只需要问「这一次答复与那一次答复是不是同一个问题的两个候选」。
/// 节点身份因此又一次是事实的一部分（与 `turn_id` / `result_id` 同一条纪律）。
pub const EVENT_TURN_BRANCH: &str = "turn.branch";

/// **用户读的是这一支**（载荷 `{ selected: <Turn 根 id> }`）。落在 `turn × asserted`。
///
/// ## 为什么选中要落事实而不是界面状态
///
/// 「此刻看的是哪一支」是**用户做过的一件事**，不是渲染细节：它决定下一轮请求里
/// 模型看到的那段历史。前端自己记（一份内存态）等于把同一个事实交回进程寿命——
/// 重启就没了，而分支还在。与 `authz.granted` 同一条判据：**做了选择就留下一笔账**。
///
/// 组内**最晚**的一条胜出；一条都没有时默认组里最晚出生的那一支（「重试完什么都不点」
/// 读到的必须是新的一次，否则用户以为重试没发生）。
pub const EVENT_BRANCH_SELECTED: &str = "turn.branch_selected";

/// **轮边界汇报**（载荷 `{ text, tool_rounds, quiet_ms }`）。落在 `turn × asserted`。
///
/// ## 它为什么是事实（缺口 4）
///
/// 「干到一半也要能说一句话」。那句话由 `compose` 的模板产线生成（零 LLM 往返），
/// 发点在**工具批次边界**（`turn_exec` 的汇报闭包）而不是 `chat_loop` 的轮边界——后者每个
/// 用户轮只经过一次请求，而工具循环在运行器的环体里，真实的工具轮数只在那里
/// （载荷里的 `tool_rounds` 就是它；计数口径见 [D-074](../../../../docs/003-design/014-单环一表.md#d-074-恒-0-的轮内计数退出主循环工具轮软上限整条退役抽干点搬到批次边界 "DE-014-单环一表")）。
/// 该不该说是 `policy.report` 行的四条与关系：开关开着 ∧ 沉默够久 ∧
/// `tool_rounds >= min_rounds` ∧ 未超本轮配额（判据在 `report_due` 视图 + 行表）。
///
/// 汇报不是轮次事实（一轮可以报零到多次），所以它与 `turn.supplemented` 同格
/// （`turn × asserted`），不占 `chat.assistant.final` 那格 N3 的「每轮至多一条」。
///
/// ## 为什么标注「不进请求包」
///
/// 汇报是**界面文本**：`exclude_from_context`（与 `Escalate` 首响同一条纪律）。
/// 投影仍然收它 —— 用户看得见、落库要留住；只是 `to_messages()` 把它滤掉。
/// 滤掉这件事由 `prompt_fidelity` 反向盯着：它一旦漏进请求包就报缺口。
pub const EVENT_ASSISTANT_REPORTED: &str = "chat.assistant.reported";

/// **对话面开场白**（载荷 `{ text, id, reason, exclude_from_context }`）。落在 `turn × asserted`。
///
/// ## 它为什么是事实
///
/// `Escalate` 判决在轮首产出一句首响（「好，我来处理」），随后本轮**继续**进工具循环
/// ——它与 `chat.assistant.final` 不是同一条：收束格是干活干完之后的答复，本格干活
/// **开始前**的开场白。这句是事实，随本格入格（WAL 为唯一事实源）——
/// 不入格则重建折不出这条根级对话面节点——实测 t20 / t22 的「恰有一条对话面
/// 节点」全部落空（`meta.surface = "reply"` / `meta.reason = needs_work`）。
/// 节点形状是事实的一部分（原则），故单立一格。
///
/// ## 为什么与 `chat.assistant.reported` 分格而不是复用
///
/// 汇报的理由码恒为 `progress`（界面进度），本格的理由码是上游判决原样透传
/// （`needs_work` 等）；两者的 `exclude_from_context` 虽同为真（都是界面文本、
/// 不进模型请求包），但**理由码是数据**，把「开场白」塞进「汇报」那格会让
/// `session/stats` / 前端面板把开场白读成进度汇报。名字是数据，加名字不加枚举。
///
/// ## 为什么标注「不进请求包」
///
/// 与 [`EVENT_ASSISTANT_REPORTED`] 同一条纪律：开场白是**界面文本**，
/// `exclude_from_context = true`。投影仍然收它（用户看得见、落库要留住），
/// 只是 `to_messages()` 把它滤掉——由 `prompt_fidelity` 反向盯着。
pub const EVENT_ASSISTANT_OPENING: &str = "chat.assistant.opening";

/// 助手最终答复（每 turn 至多 1 条——N3）。落在 `turn × closed`。
pub const EVENT_ASSISTANT_FINAL: &str = "chat.assistant.final";
/// 兜底话术——**普通事件，不是特殊通道**（生成失败也要有输出）。落在 `turn × closed`。
pub const EVENT_ASSISTANT_FALLBACK: &str = "chat.assistant.fallback";

/// 等待用户（本轮尚未了结，如工具报了 confirm / ask_user）。落在 `turn × progressed`。
///
/// ## 为什么它是事实而不是「什么都没发生」
///
/// 思考 / 正文 / 工具是**事实**；完成 / 终止 / 等待是事实的**状态**。等待用户时模型
/// 已经说过话（"需要你确认一下"）——那句话是事实，必须入格（载荷 `status` 记
/// `pending`），否则恢复续写后模型看不见自己问过什么，会把同一句再问一遍。
/// **不为「等待」新增收束语义**：本格不收束轮次（`turn × progressed` 不是
/// `closed`），恢复后照常以收束格了结，「一轮至多一条收束格」不变。
///
/// ## 为什么不是 `chat.assistant.reported` 的第二次
///
/// 汇报是**界面文本**（`exclude_from_context`，不进请求包），而等待用户那句话是
/// **对话事实**——恢复续写时模型必须看得见它。两者角色相同（assistant）、
/// 进不进请求包相反，混在一处必然让其中一半被静默滤掉。
pub const EVENT_ASSISTANT_PROGRESS: &str = "chat.assistant.progress";

// ── 协议族（port protocol）：**对外调用落成事实**──
//
// 这五个名字是**新增**，不是替换：现网的事实名全是业务词汇（`memory.recalled` /
// `task.rework_created` / `chat.assistant.final` / `commitment.broken` …），
// 有各自真实的读方，**一个不动**。把它们压成几个协议名不是简化，是销毁语义。
//
// 补上的是**一类从来不存在的事实**：今天「我要调模型」「我要调工具」只活在内存里
// （DE-014 §2.1 的「意图」）。落成事实之后，每一次对外调用都可回放、可恢复、可审计。
//
// **端口是载荷，不是事实种类**：只有这五个名字带 `port` 字段（DE-015 §2.4 的裁决），
// 端口的取值（`turn` / `model` / `tool` / …）写在载荷里。**加一个端口 = 0 个新事实名。**
//
// 落格：**五个都在 `turn × asserted`**，逐条理由写在各自的文档与 `grid.rs` 的对应行注释里。

/// **该调了**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 决策者——写下「这一轮要发起这次调用」的那一方。
///
/// ## 为什么落 `turn × asserted`
/// 它是一条**关于这一轮的断言**（「这一轮欠一次 `port` 调用」），不是状态迁移：
/// `turn × opened` 是轮次的**开始**（一个 turn 只能有一次），`turn × closed` 是轮次的
/// **结束**（收束格），`turn × progressed` 是**用户看得见**的动静（`chat.assistant.progress`）。
/// 协议事实是**机器对机器**的账：不进对话面、不产节点、不进请求包。
///
/// ## 载荷
/// `port` = 端口名；`id` = **逻辑 id**（同一笔调用的各次尝试共用它，幂等键按它去重）；
/// `attempt` = 尝试号（重试时 +1）。
pub const EVENT_REQUEST: &str = "request";

/// **已经派出去了**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// **引擎**（`before` 标记位），不是搬运者——搬运者的 `run` 可能不产生事实就返回。
/// 若由它写 `sent`，「派出去了」就依赖对方自觉：它一忘，`dispatch` 行永远不还清，
/// 而**没有任何东西会红**（DE-015 §2.3； 判据 2）。
///
/// ## 为什么落 `turn × asserted`
/// 与 `request` 同格同族：一次调用的两个阶段是同一条账的两个断言。
/// 「已派出」不是用户看得见的动静，故不占 `turn × progressed`。
pub const EVENT_SENT: &str = "sent";

/// **回来了**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 搬运者（拿到应答的一方）。
///
/// ## 为什么落 `turn × asserted`
/// 「有应答」是一条关于这一轮的断言。它**不是**收束格：轮子由
/// `chat.assistant.final` / `fallback` 收束，一次端口调用的应答不结束一轮。
pub const EVENT_RESPONSE: &str = "response";

/// **没成功**（载荷 `{ port, id, why }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 搬运者；引擎在执行体 `run` 返 `Err` 时兜底写它——**失败也是事实**，
/// 义务环里没有异常分支（DE-015 §2.6 契约 3）。
///
/// ## 为什么落 `turn × asserted`（而不是 `closed`）
/// 与 `authz.denied` 同一条纪律：一次端口调用失败**不关轮**——轮还可能重试、
/// 走降级、或以兜底话术收束。把它算成收束会让 `unresolved_turns` 少报一轮
/// 「静默中断」，而那正是**最该被发现的失效**。
///
/// ## 载荷
/// 没有 `attempt`：`failed` 关的是**逻辑 id**（所有尝试），不是「某一次尝试失败」。
pub const EVENT_FAILED: &str = "failed";

/// **这次尝试作废，换一次**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 租约看护者（`lease_keeper`）——`sent` 之后长时间无应答时，它按
/// `PortDecl.max_attempts` 决定重试（落 `retry` + 新一轮 `request`）或判死
/// （落 `failed`）。
///
/// ## 为什么需要它
/// 没有它，`sent` 之后崩溃 ⇒ `await` 行永远 blocked ⇒ 那笔账永远挂着，
/// 而日志上「发出去了」与「卡住了」长得一样（DE-015 §2.6 契约 7）。
pub const EVENT_RETRY: &str = "retry";

// ── 授权（authz）：**拒绝也是一条事实**──

/// 写侧闸拒绝落下的**否定事实**。落在 `turn × asserted`。
///
/// ## 它补的是什么
///
/// 收束闸拒绝时，此前只有一条 `warn`：日志里留下的是「该轮没收束」——与
/// 「根本没跑完」**长得一样**。于是「谁在什么时候因为什么被拒了」答不出来，
/// 回放也重现不出这次拒绝。**全项目唯一一处用「没写」表达「不许写」的地方。**
///
/// 它违反的是 I1（单通道），不是权限语义：一切状态变更只经事实源，
/// 那么「被拒绝」这个状态变更也必须是一条事实。
///
/// ## 载荷
///
/// `{ key, kind, reason, detail }`：`key` = 被拒的**目标格名**
/// （`chat.assistant.final` / `chat.assistant.fallback` / `artifact.added`）、
/// `kind` = `speech` / `artifact`（与写侧闸按种类分两臂的划分同源）、
/// `reason` = [`ClosureReason::Denied`]（与收束格同一个字段、同一个闭集）、
/// `detail` = 人话原因（缺哪一项能力，或哪一道作用域上限）。作者（`actor`）
/// 就是被拒的主体——「谁」不再单独记一份。
///
/// 义务环落地后 `key` 会换成 `Pattern` 的键（DE-015 §4.2 的 `denied(key)`）；
/// 今天没有 `Decl`，先用格名——它是同一件事在日志里的名字。
///
/// ## 三条边界
///
/// 1. **不是收束格**（`asserted` 不是 `closed`）：该轮仍然未收束，
///    `unresolved_turns` 照报。把 `denied` 算成收束等于把「静默中断」
///    这条最该被发现的失效一起掩盖掉。
/// 2. **幂等**：同一轮同一 `kind` 只落一条（`deny-{turn}-{kind}`）。产物臂在
///    工具结果的循环里，多轮 / 多个结果会重复走到闸前，而它们说的是同一件事。
/// 3. **不影响判定**：落不进去也照原样返回拒绝结果——闸门的结论是「不许写」，
///    不能因为记账失败就放行。
pub const EVENT_AUTHZ_DENIED: &str = "authz.denied";

// ── 停机原因：**一个字段、一个闭集**──

/// 停机原因——「这一轮为什么停」的唯一答案，取值是**闭集**。
///
/// ## 它收掉的是三个形状
///
/// 此前同一个问题有三个载体，语义互相重叠且都不够用：
///
/// | 旧形状 | 问题 |
/// |---|---|
/// | `payload.status` ∈ `completed` / `failed` / `aborted` | 结果与原因混在一个字段里 |
/// | `payload.terminated.reason` | 恒为 `"aborted"`，等于没有信息 |
/// | `payload.why` | 自由文本，不可判 |
///
/// 于是「这一轮为什么停」只能靠读三个键 + 猜。而它正是重启后最需要回答的问题：
/// 义务环里没有 `Halt` 这个值——"停"只有一种表达，就是"没有债了"，那么
/// "为什么没有债了"必须写在事实里（DE-014 需要一个运行期分支
/// `None => halt(NoRule)`；义务环里那个分支不存在——装载期"每行收口"已拦住，
/// 所以闭集里**没有 `no_rule`**，这不是遗漏）。
///
/// ## 与 `status` 的分工
///
/// `status` 留下，但只留**结果面**三值（`completed` / `failed` / `aborted`）：
/// 结果与原因是两个维度，合成一个字段会让「截断但答出来了」「失败且被拒」
/// 这类组合表达不出来。
///
/// ## `detail`：人话细节，唯一不可判的字段
///
/// 闭集回答"是哪一类"，`detail` 回答"具体是什么"（如适配器原始错误文本）。
/// 二者分开是因为：把细节塞进闭集会让它不可判，把闭集市写成自由文本会让它
/// 不可统计。**只许对 `reason` 做分支，`detail` 只给人读。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureReason {
    /// 说完了（自然结束）。
    Completed,
    /// 用户打断（或首请求前到达的中止）。
    Interrupted,
    /// 上界耗尽——`finish_reason = length`：步数 / 预算 / max_tokens 掐断。
    Exhausted,
    /// 主路径失败，兜底顶上。
    Fallback,
    /// 收束闸拒绝。**不产生收束格**——被拒的一轮仍然未收束，
    /// 这个值只落在 `authz.denied` 上。
    Denied,
}

impl ClosureReason {
    /// 全部取值——审计判据的闭集由这里派生，**不另写一份**。
    pub const ALL: [ClosureReason; 5] = [
        ClosureReason::Completed,
        ClosureReason::Interrupted,
        ClosureReason::Exhausted,
        ClosureReason::Fallback,
        ClosureReason::Denied,
    ];

    /// 线上拼写（载荷 `reason` 的值）。
    pub fn as_str(self) -> &'static str {
        match self {
            ClosureReason::Completed => "completed",
            ClosureReason::Interrupted => "interrupted",
            ClosureReason::Exhausted => "exhausted",
            ClosureReason::Fallback => "fallback",
            ClosureReason::Denied => "denied",
        }
    }

    /// 从载荷字符串解析（读侧 / 审计用）。`None` = 越界——这正是判据要抓的。
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == raw)
    }
}

// ── 产物格子（S02 §3，[roadmap/002-工具调用与产物.md](../../../../docs/003-design/032-工具调用与产物.md "DE-032-工具调用与产物")）──
//
// 工具**结果**是产物事实：模型请求了工具、工具产出了这段东西。工具调用**本身**
// 不单独占格（同一件事两格必然漂移）；`produced_by` 指向**本轮用户格**，即
// 「这个产物是本轮用户请求的后果」——S02 §3 的验收断言正是 `caused_by` 指向发起
// 它的 `task/turn`。

/// 产物产出：一次工具调用的**结果**（载荷 `{ tool, text }`）。
/// 落在 `artifact × asserted`（网格坐标见 [plan/01 §6](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
pub const EVENT_ARTIFACT_ADDED: &str = "artifact.added";

// ── 消息列表变更的格子（D-053 / D-054）──
//
// 「消息列表被改过」也是**发生过什么**。在轨 S 之前，编辑 / 截断只重写
// `messages.json`，事实源里一个字都没有 ⇒ 主读路径一旦切到「从 WAL 重建」，
// **被截断的消息会复活、编辑过的内容会回退**。
//
// 这一族是**闭集**（D-054 决策 1）——列表的改动只有四种，三个节点级 + 一个列表级：
// 诞生（本格）/ 断言（`message.edited`）/ 关闭（`message.removed`）/ 取代
// （`session.consolidated`）。任何场景都是它们的组合；**新场景若需要第五种 ⇒ 模型错了**，
// 应当改模型而不是加一格。
//
// ## 为什么是 `message` 实体而不是 `turn`
//
// §6 已预留 `user.message.edited`（`turn × opened`），那是「用户改刚发的那句话」
// 这一**窄义**。而 CRUD 是**列表级、跨轮**的：一次截断可以横跨多轮，归 `turn`
// 会把「一轮开始」与「一段历史被删」压进同一格（准入三问「能归入已有实体？」的
// 否定一侧）。

/// 一条消息**诞生**（载荷 `{ messages: [<ChatNode JSON>…] }`）。落在 `message × opened`。
///
/// ## 为什么需要它（`turn` / `artifact` 不是已经承载了节点吗）
///
/// 大部分节点的诞生**由别的事实承载**：用户发言归 `turn × opened`、助手正文归
/// `turn × closed|progressed`、工具结果归 `artifact × asserted`（重建侧的
/// `nodes_of` 就是从这些格取出节点的）。但有一类节点**没有别的格可搭车**——
/// 压缩记录节点（「已压缩上下文」那张卡）是最典型的一个：它是一次压缩的**记录**，
/// 不是任何一轮的产出，也不属于任何工具调用。
///
/// 少了这一格，那类节点在事实源里**从来没有出生过** ⇒ 切主读路径之后它们整条消失
/// （D-054 决策 1 的「诞生」原语）。
///
/// ## 边界：它是「没有语义承载」的那一格
///
/// 判据是**否定的**——只有当一条节点的诞生**找不到别的格**时才用它。承载着对话内容的
/// 节点（用户发言 / 助手正文 / 工具结果）必须由它们自己的事实落，否则同一条节点
/// 有两个 owner（D-049 理由 1）。
/// 因此本格的节点是**界面文本**类（`prompt_fidelity` 的 `excluded_from_prompt`），
/// 不进模型请求包。
///
/// ## 为什么载荷是列表而不是单条
///
/// 与 `message.removed` 的 `ids` 同一条判据：**一个动作一条事实**。一次「诞生若干节点」
/// 的操作（压缩收口时把记录节点连同收口节点一起写回）落一条，而不是 n 条。
pub const EVENT_MESSAGE_ADDED: &str = "message.added";
/// 编辑一条消息（载荷 `{ message: <ChatNode 的 JSON> }`——补丁**应用后**的整条）。
/// 落在 `message × asserted`。
///
/// ## 为什么载荷是整条而不是字段子集
///
/// 重建与转写都按 **id 替换**：拿到整条就不必再猜「这次改的是哪几个字段」——
/// 补丁语义本来就是「未提供的保持不变」，而应用后的整条**就是**那个结果。
/// 只记子集等于把「合并」这件事留一份实现给重建，两份必然漂移。
///
/// ## 为什么它必须带 `produced_by`
///
/// 它是断言类（I2 无溯源不声明），且溯源有确切的答案：**被改的那条事实**
/// （`produced_by` = 承载该节点的那条事件的 seq）。「这次编辑源自那条发言」——
/// 而不是拿一个「此刻」的号凑数。
pub const EVENT_MESSAGE_EDITED: &str = "message.edited";
/// 一段消息没了（载荷 `{ ids: [...] }`）。落在 `message × closed`。
///
/// ## 为什么一次操作只落一条（区间用 id 列表表达）
///
/// 截断的语义是「从 X 起全没了」——**一个动作**。逐条发事件会把一个动作拆成 n 条
/// 事实，n 随消息量增长，而事实网格的价值正在于「一件事一个键」。与
/// `memory.forgotten` 同形：收束格表达「没了」，载荷说清「哪些」。
///
/// `ids` 是**被删节点 id 的完整列表**（与 `truncate_messages` 回执里那份同源）——
/// 重建与转写据此丢弃节点，不做「从某条起」的区间推断（区间推断要求重建知道
/// 列表顺序，而顺序是呈现细节，不是事实）。
pub const EVENT_MESSAGE_REMOVED: &str = "message.removed";

/// 会话历史的**压缩快照**（载荷 `{ messages: [<ChatNode JSON>…] }`——压缩后的整份列表）。
/// 落在 `message × progressed`。
///
/// ## 为什么是 `message × progressed` 而不是 `system`
///
/// 用 D-053 自己的判据：双源对账
/// （`session.reconciled`，已随 退役）曾归 `system`，理由是「它说的不是
/// 『这个会话发生了什么』，而是『两个读源此刻是否一致』」——诊断。而压缩**确实发生过**：
/// 一段历史被摘要取代、列表从此读作快照那一份。那是
/// **会话事实**，故归 `message`（本实体正是「消息列表上的变更」）。动词取 `progressed`
/// 与 `memory.consolidated` 同——两者都是「巩固」。
///
/// ## 它取代前缀：取代发生在折叠侧，不在读口
///
/// 一条 `session.consolidated` **取代**它之前的所有消息事实：此后所有读侧
/// （`rebuild` 折节点树、`transcript` 折对话行、`stats` 扫水位…）看到的「历史」
/// 都是「最近快照 + 其后事实」——那是**折叠**从最近一份快照起折出来的，不是读口
/// 只给了那一段。被取代的那一段**不删**——持久化实现把它移入归档段，
/// 留作审计与回溯，而 `range` 的口径恒是完整史
/// （[plan/14 §2.2](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 推论 1）。
/// **不删是因为事实不可凭空消失**；移入归档段是因为「压缩」的语义就是「那段历史已被
/// 摘要取代」，它不再需要留在增长优化的窗口里。
///
/// 于是「WAL 无界增长」与「上下文压缩」由**同一个机制**解决（D-053 决策 6）：
/// 一次压缩 = 多一条快照事件 + 把被取代的前缀移入归档段。
pub const EVENT_SESSION_CONSOLIDATED: &str = "session.consolidated";

// ── 记忆格子（S5，[roadmap/S06 §3](../../../../docs/003-design/036-长期记忆与语义检索.md "DE-036-长期记忆与语义检索")）────

/// 编码：学到的语义内容（`payload: { content, tag, generation, vis_scope }`）。
/// 落在 `memory × opened`。**记忆必带溯源**（I2 扩展：溯源覆盖 100%）。
/// **载荷里没有 `vec` 键**：向量索引落成时按 `content` 回填，恒空的预留键不算
/// 承诺（[D-110](../../../symbio-plugin-memory/README.md#d-110-恒空预留键不进事实形状记忆的向量键与它的写方同批落格 "crates/symbio-plugin-memory/src/README")；写方 `plugins/loop/memory_events.rs`）。
pub const EVENT_MEMORY_ENCODED: &str = "memory.encoded";
/// 巩固：压缩 + 反事实（不是逐帧回放）。落在 `memory × progressed`。
pub const EVENT_MEMORY_CONSOLIDATED: &str = "memory.consolidated";
/// 遗忘：**不是物理删除**——Log 永不删，只是投影不再包含（可撤销、可审计）。
/// 落在 `memory × closed`。
pub const EVENT_MEMORY_FORGOTTEN: &str = "memory.forgotten";
/// 校准断言：技能**被用过之后**的一次观测（命中 / 回退），落在 `memory × asserted`。
///
/// ## 为什么它不能继续复用 `memory.recalled`
///
/// 复用是 S11 §3 的省格决定，但代价落在**读侧**：校准投影于是只能靠**载荷有没有
/// `skill_id` + `fallback`** 来认自己的输入——同一张格子被两种语义共用，判别退化成
/// 「谁恰好带了这两个字段」。那不是闭集：**任何将来带这两个字段的事件都会被算进置信度**，
/// 而唯一的防线是「编译事件恰好不带 `fallback`」（`calibration.rs` 的注释写着
/// 「编译事件不计」，读起来像判据，其实是巧合）。
///
/// 给它一格之后，读侧按 `kind` 过滤，判别回到闭集。写侧不变（仍是轮末 `derived_facts`
/// 收束族，**不开第二写通道**——I1）。
pub const EVENT_MEMORY_CALIBRATED: &str = "memory.calibrated";

// ── 承诺格子（S6 第 15 步，[roadmap/S08 §3](../../../../docs/003-design/038-多主体与对等承诺.md "DE-038-多主体与对等承诺")）──
//
// 立约 = `commitment` 实体的一格事件（**加格子，不加机制**——对等协作是图，
// 子智能体递归的树表达不了，才需要这组格子）。声誉是它的投影（`projection = reputation`）。

/// 立约：`from` 向 `to` 承诺交付什么（载荷 `{ id, from, to, promise }`）。
/// 落在 `commitment × opened`。
pub const EVENT_COMMITMENT_OFFERED: &str = "commitment.opened";
/// 守约收束：承诺按约履行。落在 `commitment × closed`。
pub const EVENT_COMMITMENT_RELEASED: &str = "commitment.released";
/// 违约收束：承诺未履行（载荷带 `why`——违约必须可观测）。落在 `commitment × closed`。
pub const EVENT_COMMITMENT_BROKEN: &str = "commitment.broken";
/// 对等声明：把承诺状态对等宣告给协作方（不是新通道，是普通事件）。
/// 落在 `commitment × asserted`。
pub const EVENT_COMMITMENT_ASSERTED: &str = "commitment.asserted";

// ── 任务格子（S7 第 16–18 步，[roadmap/S03 §3](../../../../docs/003-design/033-多步任务与返工.md "DE-033-多步任务与返工")）──
//
// 任务图（DAG）= 事件的 `depends_on` 载荷字段——**数据，不是机制**；
// 返工 = 新增一条事件（`task.rework_created`），不是修改历史（append-only
// 已经提供了撤销语义）。落在 `task × opened / progressed / held / asserted`
// （`task.ready` 也落 `progressed`，`task.assigned` 也落 `asserted`——两格是
// 调度者的判断与发放，不新增动词）。
//
// 写方 = `plugins/session/task_events`（会话任务清单 `todo_write` 的转写，
// 04 §3.1 批⑨）；`task.held` 的写方是插话抢占（批⑩）；`task.ready` /
// `task.assigned` 的写方是调度者（`actors::schedule`，S04 `D-139`）。

/// 开任务：载荷 `{ task_id, depends_on, goal }`。落在 `task × opened`。
pub const EVENT_TASK_OPENED: &str = "task.opened";
/// 推进：执行中的普通事实。落在 `task × progressed`。
pub const EVENT_TASK_PROGRESS: &str = "task.progress";
/// 暂挂（插话抢占 / 等外部资源；[roadmap/S07](../../../../docs/003-design/037-插话与实时打断.md "DE-037-插话与实时打断")
/// 里这一格的事件名叫 `task.blocked`，本仓库以 `task.held` 单点定义）。
/// 落在 `task × held`。
pub const EVENT_TASK_HELD: &str = "task.held";
/// 终态：验证通过（验收通过才终态）。落在 `task × asserted`。
pub const EVENT_TASK_ASSERTED: &str = "task.asserted";
/// 返工：判定不合格 ⇒ **新增一条事件**（重开一个返工节点），不是回滚。
/// 落在 `task × asserted`（返工本身是一次质量判定的事实）。
pub const EVENT_TASK_REWORK_CREATED: &str = "task.rework_created";
/// 发放：调度者把一笔就绪任务派给一个持有者，并附**租约**。载荷
/// `{ task_id, owner, lease_ms, until_ms }`。落在 `task × asserted`。
///
/// ## 唯一写方
///
/// 只有调度者写它（`actors::schedule::ScheduleDecider`），且写侧闸读
/// `assign.work`（`ActorSpec::can_assign_work`）——没有该能力的主体零事件
/// （fail-closed）。租约不是锁原语：持有状态由投影从这格事件算出，到期由
/// 调度者判死（见 `task.ready` 的 `lease_expired`）。
pub const EVENT_TASK_ASSIGNED: &str = "task.assigned";

// ── 控制格子（S8 第 19–20 步，[roadmap/S07 §3](../../../../docs/003-design/037-插话与实时打断.md "DE-037-插话与实时打断")、
//    [roadmap/S09 §3](../../../../docs/003-design/039-外部执行与熔断.md "DE-039-外部执行与熔断")）────
//
// 打断信号与熔断**共用一格**：`control/opened`（kinds 都是 `task.controlled`，
// 以载荷 `reason` 区分）——插话的"停/继续/改道"与外部的"熔断"是同一类事实：
// 一个更快的判定者决定了对在跑事务的处置。不加新机制，只加一格。

/// 控制判定产出（打断处置 / 熔断）。载荷 `{ task_id?, reason, ... }`。
/// 落在 `control × opened`。**抢占判定者只持 JudgeIntent**——无 `reply.*` 写权，
/// 判定者不得直接发言（S07 §5）。
pub const EVENT_CONTROL_OPENED: &str = "task.controlled";

/// 「这是一次自主开轮」在**地址写入体**上的判别键（`ChatNode.meta` 的键名）。
///
/// ## 为什么一个写体的键名住在事件词汇表
///
/// 它不是事件名，而是**同一个事实的另一种写法**：写体 `meta.trigger` 声明的是
/// 「这一轮由系统自己发起」，落进事实源就是 `system.triggered` 那一格。
/// 两侧必须同源——`meta` 是自由 JSON，键名拼错编译器管不到，生产方（调度侧）与
/// 消费方（存储所有者的写臂）各写一份就是两条漂移的口径（[D-077](../../../symbio-plugin-heartbeat/README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README") 裁决二）。
///
/// 形状：`meta.trigger = { "kind": <EVENT_TRIGGER_KIND_*>, "idle_ms": N }`，
/// **在写臂就地消费**，不要求它活到转写。
pub const EVENT_TRIGGER_META_KEY: &str = "trigger";
/// 触发方式的取值：**定时触发**（心跳一族的调度器）。落在 `system.triggered`
/// 的载荷 `kind` 上，也落在写体 `meta.trigger.kind` 上——同一个名字说同一件事。
pub const EVENT_TRIGGER_KIND_SCHEDULED: &str = "scheduled";

// ── 裁决格子（S13 §3，[roadmap/S13](../../../../docs/003-design/043-多智能体社会.md "DE-043-多智能体社会")）──
//
// 集体决策 = 「多个主体对同一 verdict 的竞争 + 仲裁者」：`verdict` 实体（F5 闭集
// 里一直有这一行）在 S13 取的第一个值就是**淘汰裁决**——声誉投影低于阈值的主体
// 被收回 `assign.work`（§2「淘汰」行）。**不加新格子**：`verdict × asserted`
// 正是「对一次判定盖章」，与承诺族的 `commitment.asserted` 同形。

/// 淘汰裁决：仲裁者对「某主体不再被授予 `assign.work`」的盖章
/// （载荷 `{ subject, capability, change, score, threshold, why }`，
/// 见 `actors::EliminationDecider`）。落在 `verdict × asserted`。
///
/// ## 为什么它是事件而不是表改动
///
/// 授权表（`authz::ROWS`）是装配期常量——运行期改表就把「表授予 A、闸门判 B」
/// 的漂移面请回来了。淘汰于是走**事件通道**：一条 `verdict.asserted` 落盘，
/// 读侧（闸门）折叠成 `governance::GrantsOverlay` 的撤销，`Restore` 是同格的
/// 逆操作。可审计（流水在 Log 里）、可撤销（再写一条恢复）——S13 §6 断言 3。
pub const EVENT_VERDICT_ASSERTED: &str = "verdict.asserted";

/// 事件实体 —— 语法网格的**行**，闭集（F5，11 个）。
///
/// 新增实体是 L-schema 变化（[plan/03 §2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
/// 加枚举臂 + ADR 登记，不是机制变更。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Entity {
    Turn,
    Task,
    Artifact,
    Verdict,
    Memory,
    Commitment,
    Conation,
    Thread,
    Control,
    System,
    /// 消息列表上的一次**变更**（编辑 / 删除 / 压缩快照）。见 [`EVENT_MESSAGE_EDITED`]。
    ///
    /// 与 `Turn` 的分界：`turn` 记的是「这一轮说了什么」，本实体记的是
    /// 「列表里的某条被改成 / 去掉了 / 整段被摘要取代了」——它跨轮、不开始任何一轮。
    Message,
}

/// 事件动词 —— 语法网格的**列**，闭集（F5，5 个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Verb {
    Opened,
    Progressed,
    Held,
    Closed,
    Asserted,
}

/// 事件信封：一 条已发生事实的全部元数据。
///
/// **载荷与元数据分两层**：`payload` 是格子自己的数据（如发言内容）；
/// 其余字段是**所有事件共有**的溯源 / 预算元数据（I2 无溯源不声明、I3 到点必答）。
///
/// `seq` 入库前为 `None`——它由 [`crate::store::Store::append`]
/// 在唯一写入口内**分配**，调用方无权指定（append-only + 单调的构造侧保证）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// 幂等键：重复 `append` 同一 `event_id` ⇒ [`crate::store::AppendError::Duplicate`]。
    pub event_id: String,
    /// 网格名（数据），如 `"chat.assistant.final"`；落在 `entity × verb` 格子上。
    pub kind: String,
    pub entity: Entity,
    pub verb: Verb,
    /// 所属回合。N3（每 turn 至多一条 final）按它分桶。
    pub turn: u64,
    /// 产生者（Principal，数据：字符串 ID）。
    pub actor: String,
    /// 溯源：产生本事件的上游事件 seq（I2——断言类事件必须非空）。
    pub produced_by: Option<u64>,
    /// 产生本事件花费的毫秒数（I3 记账）。
    pub cost_ms: u64,
    /// 入库时间（Unix 毫秒）。
    pub ts: Timestamp,
    /// 格子自己的数据。
    pub payload: serde_json::Value,
    /// 由 `Store` 分配的全局单调序号；入库前 `None`。
    pub seq: Option<Seq>,
}

impl Event {
    /// 造一条**未入库**事件（seq 待 `Store` 分配）。
    pub fn pending(
        event_id: impl Into<String>,
        kind: impl Into<String>,
        entity: Entity,
        verb: Verb,
        turn: u64,
        actor: impl Into<String>,
    ) -> Self {
        Event {
            event_id: event_id.into(),
            kind: kind.into(),
            entity,
            verb,
            turn,
            actor: actor.into(),
            produced_by: None,
            cost_ms: 0,
            ts: 0,
            payload: serde_json::Value::Null,
            seq: None,
        }
    }

    /// 设置溯源（I2）——链式写法，方便构造。
    pub fn with_produced_by(mut self, seq: u64) -> Self {
        self.produced_by = Some(seq);
        self
    }

    /// 设置预算记账（I3）。
    pub fn with_cost_ms(mut self, cost_ms: u64) -> Self {
        self.cost_ms = cost_ms;
        self
    }

    /// 设置时间戳。
    pub fn with_ts(mut self, ts: Timestamp) -> Self {
        self.ts = ts;
        self
    }

    /// 设置载荷。
    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }

    /// 本事件的名字在名字表上的**格位**（`None` = 名字不在表上）。
    ///
    /// 它判的是 `kind`，**不读** `self.entity` / `self.verb`——两者的分歧正是
    /// [`crate::store::AppendError::OffGrid`] 要拒绝的那件事。
    pub fn grid_cell(&self) -> Option<Cell> {
        grid::event_cell(&self.kind)
    }
}

// ── 载荷里的节点身份（读侧共享）────────────────────────────────────────────
//
// 「一条事件承载哪个消息节点」这件事，**写侧与读侧必须给出同一个答案**：写侧落格
// 时把节点 id 记进载荷，读侧（`rebuild` 折节点树 / `transcript` 折对话行）按同一
// 规则取出来。三个消费方各写一遍的后果是**静默**的——`message.edited` 定位不到它
// 要改的那条节点，表现为「编辑没生效」，而没有任何东西会红。
//
// 规则本体因此住在这里（core 的中性契约层），而不是住在任一读侧实现里。它们只读
// 载荷、不碰任何状态，是纯函数。

/// 载荷里的节点身份：`payload[key]` 优先；缺失（该格不记身份）⇒ 按事件 id 稳定合成。
///
/// ## 「稳定」是硬要求
///
/// 合成 id 用**事件 id**（写方按「一件事一个键」给出，如 `s-{turn}-{n}`）——
/// 同一条事件每次读都得同一个值。若用随机值，每次重开会话都会多出一批「新」节点
/// （前端按 id 合并，id 一变同一句话就是两个身份）。
pub fn event_payload_node_id(e: &Event, key: &str) -> String {
    payload_str(e, key).unwrap_or_else(|| e.event_id.clone())
}

/// 收束 / 进展 / 兜底格的**正文子节点** id。
///
/// ## 为什么不是「读 `text_id` 就完了」
///
/// 落库侧（`message_build::llm_build_assistant_messages`）有一条 **reasoning-only**
/// 规则：思考与正文是同一段内容且没有独立文本回复时，那段内容归 **Text 子节点**、
/// 并**复用 reasoning 的流式 id**（否则同一段内容在存储层出现两份）。重建若只认
/// `text_id`，reasoning-only 那一轮会得到 `{turn_id}-text` 这个**落库侧并不存在**的
/// id——于是那条正文在实时面与重建面是两个身份，且 `message.edited` 永远定位不上它。
///
/// 判据与落库侧**逐字相同**（同一段文本比较），因为它就是同一个问题的答案。
pub fn event_produced_text_node_id(e: &Event) -> String {
    event_output_text_node_id(
        payload_str(e, "text").as_deref().unwrap_or_default(),
        payload_str(e, "reasoning").as_deref().unwrap_or_default(),
        payload_str(e, "text_id").as_deref(),
        payload_str(e, "reasoning_id").as_deref(),
        payload_str(e, "turn_id").as_deref(),
    )
}

/// [`event_produced_text_node_id`] 的**字段版**：同一条规则，供写侧在落格前算好后写进
/// `answer_id`（模型调用的事实自带它那一轮的正文身份）。
///
/// 写侧与读侧共用这一份实现：两处各算一遍的后果是静默的——`response` 记的 `answer_id`
/// 与收束格/转写行算出来的值不同，表现就是「编辑定位不上那条正文」。
pub fn event_output_text_node_id(
    text: &str,
    reasoning: &str,
    text_id: Option<&str>,
    reasoning_id: Option<&str>,
    turn_id: Option<&str>,
) -> String {
    let reasoning_only = !reasoning.trim().is_empty() && reasoning.trim() == text.trim();
    let id = if reasoning_only {
        reasoning_id.or(text_id)
    } else {
        text_id
    };
    id.map(str::to_string).unwrap_or_else(|| {
        // 载荷连 `turn_id` 都没有（答话轮 / 反射档：没有流式节点）⇒ 与落库侧同一条
        // 回退：正文子节点的名字由 Turn 根派生。
        format!("{}-text", turn_id.unwrap_or_default())
    })
}

/// 工具**结果**节点的 id（`artifact.added` 载荷 `result_id`）。
///
/// 恢复路记 `null`（那一轮没有流式结果节点）⇒ 按**事件 id** 合成。合成用的**不是**
/// `tool_call_id`：结果与调用是两张卡，共用 id 会让前端把它们合成一条。
pub fn event_tool_result_node_id(e: &Event) -> String {
    payload_str(e, "result_id").unwrap_or_else(|| format!("{}-result", e.event_id))
}

/// 一条事件在**读侧**承载的节点身份（`None` = 该格不产节点）。
///
/// ## 它是「哪一格对应哪个节点」的单点答案
///
/// 三个消费方都按它认节点：`rebuild`（节点树）、`transcript`（对话行）、
/// `prompt_fidelity`（变更索引的配对）。各写一遍的后果是**静默**的——
/// `message.edited` 定位不上要改的那条节点，表现为「编辑没生效」。
///
/// ## 只给「内容节点」，不给全部节点
///
/// 一次模型往返的正文与思考是两个节点（`response` / `failed` 各自承载），收束格只
/// **引用**其中正文那个的身份（`answer_id`）。对话读侧（转写行 / prompt 复核）只需要
/// 它；节点树（`rebuild`）要的是全部，所以它自己按节点逐个取。
pub fn event_node_of(e: &Event) -> Option<String> {
    match (e.entity, e.verb, e.kind.as_str()) {
        (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => Some(event_payload_node_id(e, "id")),
        (Entity::Turn, Verb::Asserted, EVENT_TURN_SUPPLEMENTED) => {
            Some(event_payload_node_id(e, "id"))
        }
        (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_REPORTED) => {
            Some(event_payload_node_id(e, "id"))
        }
        (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_OPENING) => {
            Some(event_payload_node_id(e, "id"))
        }
        (
            Entity::Turn,
            Verb::Closed | Verb::Progressed,
            EVENT_ASSISTANT_FINAL | EVENT_ASSISTANT_FALLBACK | EVENT_ASSISTANT_PROGRESS,
        ) => Some(payload_str(e, "answer_id").unwrap_or_else(|| event_produced_text_node_id(e))),
        (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => {
            Some(event_tool_result_node_id(e))
        }
        _ => None,
    }
}

/// 取载荷里的**非空**字符串字段（空串读出来是「这个字段没记」，不是「记了空」）。
fn payload_str(e: &Event, key: &str) -> Option<String> {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 信封的最小契约（[plan/01 §2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的 `type Event: EventEnvelope`）。
///
/// `Store` 只需要知道三件事：幂等键是什么、seq 在不在、seq 怎么赋值。
/// 剩余字段对存储层不可见——将来 WAL / 分片实现携带更富的信封也不动 trait（F1）。
///
/// `Send + Sync` 是事件作为**纯数据**的本性（快照要跨线程交给投影），
/// 放在超 trait 上，任何实现都无法造出不能跨线程的事件。
pub trait EventEnvelope: Clone + Send + Sync {
    /// 幂等键。
    fn event_id(&self) -> &str;
    /// 入库前 `None`。
    fn seq(&self) -> Option<Seq>;
    /// **仅由 `Store` 在唯一写入口内调用。**
    fn assign_seq(&mut self, seq: Seq);
    /// 落格前给「没带时刻」的信封兜底——**仅由 `Store` 在唯一写入口内调用**，
    /// 与 [`EventEnvelope::assign_seq`] 同一条纪律（入库形态由存储层说了算）。
    ///
    /// 为什么必须存在：`d781a6c7` 号会话 548 个事件的 `ts` 全为 0——上游十余处
    /// `Event::pending` 构造点，填了 `timestamp` 的是少数，`0` 便一路漏进事实源。
    /// `0` 不是「未知」而是**谎报**（假装都发生在纪元起点），时序投影因此整体失真。
    /// 逐构造点手改必然再漏（同一件事的手写实现已证明会漂移），故收口在入库处：
    /// 没带时刻的事件以**落格时刻**为准，事实源里不再有 `ts = 0` 的事件。
    /// 默认实现不动——没有时刻概念的信封保持原样。
    fn fallback_timestamp(&mut self, _now_ms: i64) {}
}

impl EventEnvelope for Event {
    fn event_id(&self) -> &str {
        &self.event_id
    }

    fn seq(&self) -> Option<Seq> {
        self.seq
    }

    fn assign_seq(&mut self, seq: Seq) {
        self.seq = Some(seq);
    }

    fn fallback_timestamp(&mut self, now_ms: i64) {
        // `0` 是 [`Event::pending`] 的「没填」哨兵，不是事实；带了时刻的不碰。
        if self.ts == 0 {
            self.ts = now_ms;
        }
    }
}
