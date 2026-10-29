//! 效果端口 —— 能力的**效果**由能力包交出，宿主只负责按序落格（DE-025 D-207）。
//!
//! ## 它补的是哪一格
//!
//! 「一次工具调用改变了什么」过去是宿主的判断：`loop` 里按工具名分流，`todo_write`
//! 记任务表、`agent_run` 记承诺、`vdfs_*` 记文件变更。每加一个带状态的能力就多一段
//! 宿主分支、一个 core 记录类型——那正是「加能力要改架构」的残余来源。
//!
//! 本模块把这条判断挪回能力包：
//!
//! ```text
//! 能力插件 ──submit_behavior!──▶ inventory（编译期随二进制生效）
//! 工具执行层 ──EffectInput（这次调用发生了什么）──▶ 收束出参
//! 轮末写口  ──apply(EffectCall)──▶ Vec<FactDraft> ──唯一写入口──▶ 事实源
//! ```
//!
//! ## 为什么效果在**写口**才求值
//!
//! `after` 需要轮锚（`t{turn}`）、溯源 seq、主体与来源主体——这些是**宿主在轮末
//! 才知道的**。若让工具执行层在调用当场求值，那四个值就得一路穿过批处理、并行收口
//! 与未执行收口五层签名；而它们与「这次调用发生了什么」无关。故工具执行层只交
//! [`EffectInput`]（纯值），轮末写口用 [`EffectCall`] 补齐宿主面再问行为。
//!
//! ## 选工具的依据是**声明**，不是名字
//!
//! 一个行为是否管辖某次调用，只看工具的 `CapabilityMeta.behaviors` 里有没有这个
//! 行为名（开放集合）。行为名住 [`crate::behaviors`]，实现住能力包——宿主
//! 两侧都不认识工具名。
//!
//! ## 前置判断：同一份声明，另一种出口
//!
//! [`ToolBehavior::before`] 与 `after` 的**选法相同**（按声明选、按 owner 序），出口不同：
//! `before` 只回答「这次调用能不能跑」，任一实现拒绝即拒绝（fail-closed，DE-025 D-219）。
//! 它拿的是调用身份 + 主体 + 控制面批首读好的结论（[`BeforeReadings`]），**没有句柄、
//! 没有轮锚**：判断发生在执行窗口之前，那时「这一轮」还没有可写的事实。
//!
//! `before` **不改写参数**：工具收到的就是模型发出的那一份。要变换入参的工具在自己的
//! `Capability.execute` 里做——那里才是入参的唯一解释者；执行层改写参数会让转写里记下
//! 的调用与真正跑的那一次不是同一件事。
//!
//! ## 写口唯一，读口也只读
//!
//! 行为只返回 [`FactDraft`]（尚未入库的事件），**不碰 WAL**：`Store::append` 仍在
//! 宿主的唯一写入口里，写闸（`kind` 与格位一致）照旧在那里校验（I1 不破）。
//!
//! 有些效果要**看得见已有事实**才判得出来——「这条任务已经开过没有」「这一格该开
//! 新节点还是记一次推进」。那类判断不能靠宿主代劳（代劳就等于宿主认识这种效果），
//! 所以 [`EffectEnv`] 交两片**只读**历史：[`EffectEnv::snapshot`]（调用方交来的那份）
//! 与 [`EffectEnv::applied`]（本轮已落格的草稿）。后者存在的理由很具体：同一轮里
//! 两个调用互相看不见时，第二条声明会把第一条刚开的格再开一次。

use serde_json::Value;
use symbio_core::{Entity, Event, Verb};

/// 一次工具调用的**效果输入**：工具执行层交出的纯值。
///
/// 不含轮锚 / 主体 / 溯源——那几样是宿主在写口盖的章（见模块文档）。工具执行层
/// 收集它，`DerivedFacts` 运送它，轮末写口求值。
#[derive(Debug, Clone)]
pub struct EffectInput {
    /// 线上全名（`local/todo_write` 这种形态）。
    pub tool: String,
    /// 本次调用在转写里的 ToolCall 节点 id（承诺号据此成格）。
    pub call_id: String,
    /// 定稿参数（工具收到什么就是什么，宿主不重解释）。
    pub args: Value,
    /// 是否成功。**未执行 / 被拒 / 协议失败**同样是 `false`——「没跑」与「跑失败」
    /// 对效果来说都是没发生，区别只在 [`Self::why`]。
    pub success: bool,
    /// 结果正文（成功时是工具交回的文本，失败时是错误文本）。
    pub result: String,
    /// 没成功的原因（成功时为空串）。行为据此决定违约理由，不自己编。
    pub why: String,
    /// 本工具声明的行为名（`CapabilityMeta.behaviors` 的原样切片）。
    ///
    /// 收集期由 `ToolDecls` 从注册表元数据抄进来：写口据此决定跑哪些行为，
    /// 不必再回读注册表（那一读在批首已经发生过一次）。
    pub declared: &'static [&'static str],
}

/// 写族工具的四种动作——`file_op` 的线格式判别词（DE-025 D-207）。
///
/// 生产方是写族工具的效果（`ToolBehavior::annotate` 交 `as_str`），消费方是变更账
/// 投影（`from_wire` 读回）。**两边都只依赖本类型**：动作词表是插件间的契约（住
/// plugin-api），不是某一侧的私有拼写。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Write,
    Edit,
    Delete,
    Mkdir,
}

impl FileOp {
    /// 线格式判别词 → 动作。不是四个判别词之一 ⇒ 不是变更。
    ///
    /// 与 [`Self::as_str`] 是同一份拼写的两个方向：生产方用 `as_str` 交值，
    /// 读方用这个读回来——两处各写一份字面量就会漂移。
    pub fn from_wire(op: &str) -> Option<Self> {
        match op {
            "write" => Some(Self::Write),
            "edit" => Some(Self::Edit),
            "delete" => Some(Self::Delete),
            "mkdir" => Some(Self::Mkdir),
            _ => None,
        }
    }

    /// 线格式的判别词——与 `#[serde(rename_all = "snake_case")]` 逐字一致。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Mkdir => "mkdir",
        }
    }
}

/// 求值一次调用效果所需的**宿主面**：轮锚、两个主体与两片只读历史。
///
/// 与 [`EffectInput`] 分开的理由见模块文档——前者是「发生了什么」，本结构是
/// 「宿主在哪一刻、以谁的名义、看得见哪些事实」。
#[derive(Debug, Clone, Copy)]
pub struct EffectEnv<'a> {
    /// 本轮序号（承诺号 `v2c-t{turn}-…` 的词干来源）。
    pub turn: u64,
    /// 本轮开口格的 seq——派生事实的溯源锚（I2）。
    pub user_seq: u64,
    /// 本会话主体（写事件的 `actor`）。
    pub principal: &'a str,
    /// 来源主体（D-067：本主体向「这条消息的出处」立约）。
    pub source: &'a str,
    /// 轮初那份事实史（调用方交来的切片）。
    pub snapshot: &'a [Event],
    /// 本轮**已落格**的草稿（按落笔顺序）：同轮的后一个效果要看得见前一个。
    ///
    /// 空切片是常态（一个效果一轮只有一条）。它的存在只为一件事：让「按历史
    /// 折叠」的效果在同一个轮次里保持与串行声明相同的判据。
    pub applied: &'a [Event],
}

impl EffectEnv<'_> {
    /// 按发生顺序遍历两片历史：轮初的在前，本轮刚落格的在后。
    pub fn facts(&self) -> impl Iterator<Item = &Event> {
        self.snapshot.iter().chain(self.applied.iter())
    }
}

/// 一次调用的完整效果输入：工具面 + 宿主面。`after` 拿到的就是它。
#[derive(Debug, Clone, Copy)]
pub struct EffectCall<'a> {
    pub tool: &'a str,
    pub call_id: &'a str,
    pub args: &'a Value,
    pub success: bool,
    pub result: &'a str,
    pub why: &'a str,
    /// 本工具声明的行为名（选行为用，不是给行为读的）。
    pub declared: &'static [&'static str],
    pub turn: u64,
    pub user_seq: u64,
    pub principal: &'a str,
    pub source: &'a str,
    /// 见 [`EffectEnv::snapshot`]。
    pub snapshot: &'a [Event],
    /// 见 [`EffectEnv::applied`]。
    pub applied: &'a [Event],
}

impl EffectCall<'_> {
    /// 轮锚词干（`t{turn}`）——事件号的跨轮唯一性从这里来。
    pub fn anchor(&self) -> String {
        format!("t{}", self.turn)
    }

    /// 按发生顺序遍历两片历史（见 [`EffectEnv::facts`]）。
    pub fn facts(&self) -> impl Iterator<Item = &Event> {
        self.snapshot.iter().chain(self.applied.iter())
    }
}

impl EffectInput {
    /// 补上宿主面，得到 `after` 的入参。
    pub fn call<'a>(&'a self, env: &'a EffectEnv<'a>) -> EffectCall<'a> {
        EffectCall {
            tool: &self.tool,
            call_id: &self.call_id,
            args: &self.args,
            success: self.success,
            result: &self.result,
            why: &self.why,
            declared: self.declared,
            turn: env.turn,
            user_seq: env.user_seq,
            principal: env.principal,
            source: env.source,
            snapshot: env.snapshot,
            applied: env.applied,
        }
    }
}

// ── 前置判断（DE-025 D-219）────────────────────────────────────────────

/// 一次工具的**前置判断输入**：纯值。
///
/// 与 [`EffectCall`] 的分界是时机：`before` 在**执行窗口之前**求值，那时轮锚、溯源与
/// 「本轮已落格的草稿」都还不存在；它要看的是「这次调用以谁的名义、在什么结论下发起」。
#[derive(Debug, Clone, Copy)]
pub struct BeforeRequest<'a> {
    /// 线上全名（`local/todo_write` 这种形态）。
    pub tool: &'a str,
    /// 本次调用在转写里的 ToolCall 节点 id。
    pub call_id: &'a str,
    /// 定稿参数（模型收到什么就是什么；`before` 无权改写）。
    pub args: &'a Value,
    /// 发起这次调用的主体（写事件的 `actor` 与它同一取值点）。
    pub principal: &'a str,
    /// 本工具声明的行为名（选实现用，不是给实现读的）。
    pub declared: &'static [&'static str],
    /// 本会话的事实切片（只读）：按历史判的前提（「这句话记过没有」）需要它。
    pub facts: &'a [Event],
    /// 控制面在批首判好的结论（见 [`BeforeReadings`]）。
    pub readings: BeforeReadings,
}

/// 控制面交给前置判断的**读数**——判断的输入开放、判断闭集（D-208）。
///
/// 这里只放宿主才能读到的结论：能力实现拿得到自己的声明面与调用参数，却拿不到
/// 「部署级覆盖层收回了哪些能力」这类跨会话的折叠值。**加一条读数 = 新增一种判断的
/// 输入，走决策**（DE-025 D-219）——它是插件认识的宿主词汇，不是自由形状。
#[derive(Debug, Clone, Copy, Default)]
pub struct BeforeReadings {
    /// 主体的 `assign.work` 是否已被淘汰回路收回（部署级覆盖层 last-wins，宿主批首读一次）。
    pub assign_work_revoked: bool,
}

/// 一次拒绝：判断归属的名字 + 回灌模型的拒因 + 退出语义。
///
/// 与后置效果不同，拒绝**必须**落到调用结果上——否则模型会看到一次「凭空消失」的
/// 调用。宿主据此产生结果子节点、收敛父节点，并按 [`Self::notes_delegation`] 决定
/// 要不要把这次调用记成没履约的承诺。
#[derive(Debug, Clone)]
pub struct BeforeRefusal {
    /// 判断归属的稳定名字（日志 / 测试 / 审计；改名 = 改事实）。
    pub gate: &'static str,
    /// 回灌模型的完整拒因。
    pub reason: String,
    /// `meta.failure_kind` 的取值（约定字段，控制流不靠猜正文）。
    pub kind: &'static str,
    /// 要不要记违约承诺。`true` = 这次调用本来会产出效果，拒止同样是一次没履约；
    /// `false` = 工具根本没跑，「声明过什么」无从说起（如任务表写侧）。
    pub notes_delegation: bool,
}

impl BeforeRefusal {
    pub fn new(gate: &'static str, reason: impl Into<String>, kind: &'static str) -> Self {
        Self {
            gate,
            reason: reason.into(),
            kind,
            notes_delegation: true,
        }
    }

    /// 工具没跑 ⇒ 不记违约承诺（只影响记账面，不影响结果正文与父节点终态）。
    pub fn without_delegation_note(mut self) -> Self {
        self.notes_delegation = false;
        self
    }
}

/// 一次前置判断的结论。
#[derive(Debug, Clone)]
pub enum BeforeDecision {
    /// 放行：交给下一个行为判，全部放行才执行。
    Allow,
    /// 拒绝：整批到这里为止（fail-closed），后面的行为不再被问。
    Refuse(BeforeRefusal),
}

/// 一条**待落格**的事实：行为交出的草稿，宿主盖章入库。
///
/// 字段是「宿主不解释的那部分」：`kind` 由 owner 的 `EventDecl` 登记、`payload`
/// 由 owner 决定、`key` 是幂等键词干。`turn` / `actor` / `ts` 由写口填——行为
/// 无权指定「这笔账记在哪一轮、以谁的名义」，那是宿主的事实。
#[derive(Debug, Clone)]
pub struct FactDraft {
    /// 幂等键（= 入库事件的 `event_id`）。跨轮唯一性由行为用 [`EffectCall::anchor`]
    /// 构造，宿主不重写——重写会让「同一笔效果两次入库」变成两笔。
    pub key: String,
    /// 事件名（写闸按它校验格位）。
    pub kind: String,
    pub entity: Entity,
    pub verb: Verb,
    pub payload: Value,
    /// 溯源锚（I2）。`None` 只在行为确实无上游时使用。
    pub produced_by: Option<u64>,
}

impl FactDraft {
    /// 从一条**领域成格函数**的产物取草稿。
    ///
    /// 存在的理由：承诺这类判断种类在 core 有自己的成格函数（`commitment_events`），
    /// 行为不该把它抄一遍。宿主面（`turn` / `actor` / `ts`）在这里被**丢掉**，
    /// 由写口按本轮的章重填。
    pub fn from_event(e: Event) -> Self {
        Self {
            key: e.event_id,
            kind: e.kind,
            entity: e.entity,
            verb: e.verb,
            payload: e.payload,
            produced_by: e.produced_by,
        }
    }

    /// 盖章入库：`turn` 与 `actor` 由宿主给。
    pub fn into_event(self, turn: u64, actor: &str) -> Event {
        let mut event = Event::pending(self.key, self.kind, self.entity, self.verb, turn, actor);
        event.payload = self.payload;
        event.produced_by = self.produced_by;
        event
    }
}

/// 能力对**自己的效果**的实现。
///
/// 一个行为实例 = 一个实现 + 一次 `submit_behavior!`；加一个实例只改能力包。
/// 新增一种行为**语义**（框架第一次出现这种判断）才是架构变更（DE-025 D-210）。
pub trait ToolBehavior: Sync {
    /// 装配顺序的依据：同一次调用上多个行为按 owner 字典序执行（DE-025 §3 第 4 条）。
    fn owner(&self) -> &'static str;

    /// 本实现管辖的行为名，必须是 [`crate::behaviors`] 里的已知名字。
    ///
    /// 与工具声明同名才生效：工具说「我受这项管辖」，实现说「我来记这笔账」——
    /// 两侧各自声明，中间没有一张中心映射表。
    fn behavior(&self) -> &'static str;

    /// 一次调用的终态效果。返回空 = 这次调用对本能力没有效果（**默认**：
    /// 大多数行为只在部分终态上记账）。
    ///
    /// 只产出草稿，**不写事实源**：写口唯一（I1）。
    fn after(&self, _call: &EffectCall<'_>) -> Vec<FactDraft> {
        Vec::new()
    }

    /// 一次调用**执行之前**的判断。返回 [`BeforeDecision::Allow`] = 不拦（**默认**）。
    ///
    /// 与 [`Self::after`] 同一份声明面：工具说「我受这项管辖」，本实现说「我来判这一关」。
    /// 任一实现拒绝即拒绝（DE-025 D-219）；判断是纯函数——不写事实、不改参数、不取句柄。
    fn before(&self, _req: &BeforeRequest<'_>) -> BeforeDecision {
        BeforeDecision::Allow
    }

    /// 一次调用对**既有事实**的补充字段（载荷富化）。返回空 = 不补（**默认**）。
    ///
    /// 与 [`Self::after`] 的分界是「有没有新格」：`after` 造新事实，本方法只给
    /// 已经要落的那条事实补字段。今天唯一的用法是工作目录变更账——`artifact.added`
    /// 早就记下了 `tool` / `args` / `success`（[D-169]），缺的只是「这次写属于哪种
    /// 动作」，而那个动作是工具自己的身份，不该由读侧按名字猜。
    ///
    /// 富化的对象是**产物格**（工具结果那一格），所以键名与取值都进载荷、跟着事实
    /// 一起走：读侧不再需要认识任何工具名。
    ///
    /// [D-169]: ../../../../docs/003-design/001-核心架构.md "DE-001-核心架构"
    fn annotate(&self, _input: &EffectInput) -> Vec<(&'static str, Value)> {
        Vec::new()
    }
}

/// 一条行为实现的链接期提交。
pub struct BehaviorDecl {
    pub behavior: &'static dyn ToolBehavior,
}

inventory::collect!(BehaviorDecl);

/// 提交一条行为实现。在插件 crate 的模块作用域调用一次即可。
///
/// ```ignore
/// symbio_plugin_api::submit_behavior!(DelegationPromise);
/// ```
#[macro_export]
macro_rules! submit_behavior {
    ($b:expr) => {
        $crate::inventory::submit! {
            $crate::effects::BehaviorDecl { behavior: &$b }
        }
    };
}

/// 已链接进来的全部行为实现，**按 owner 字典序**（同 owner 保持提交序）。
///
/// 排序在读取侧做而不是装配期做一次：表是 `inventory` 的编译期集合，本仓不持有
/// 第二个可变注册表（与 `declarations` 同一条纪律）。
pub fn handlers() -> Vec<&'static dyn ToolBehavior> {
    let mut all: Vec<&'static dyn ToolBehavior> = inventory::iter::<BehaviorDecl>()
        .map(|d| d.behavior)
        .collect();
    all.sort_by_key(|b| b.owner());
    all
}

/// 求一次调用的全部效果草稿：按 owner 序，逐个跑「工具声明了这个行为」的实现。
///
/// 顺序确定（DE-025 §3 第 4 条）：效果按同序追加，于是同一批事实的落笔次序
/// 与装配顺序无关、只与 owner 字典序有关。
pub fn apply(call: &EffectCall<'_>) -> Vec<FactDraft> {
    let mut out = Vec::new();
    for behavior in handlers() {
        if !call.declared.contains(&behavior.behavior()) {
            continue;
        }
        out.extend(behavior.after(call));
    }
    out
}

/// 求一次调用对**产物格**的补充字段：口径同 [`apply`]（按声明选、按 owner 序）。
///
/// 与 [`apply`] 分开的理由是时机不同：本函数只要「发生了什么」（[`EffectInput`]），
/// 在调用收口时就能算；`apply` 要轮锚与主体，只能在轮末写口算。两件事各有各的
/// 最早可判时刻，合成一次求值只会把其中一件拖到它不需要的时机。
pub fn annotations(input: &EffectInput) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    for behavior in handlers() {
        if !input.declared.contains(&behavior.behavior()) {
            continue;
        }
        out.extend(behavior.annotate(input));
    }
    out
}

/// 判一次调用：按 owner 序问「工具声明了这个行为」的实现，**先拒绝者胜**。
///
/// fail-closed 的形状就是这一行 `Some` 的早退：任一道判断拒绝，后面的判断不再被问
/// （它们可能依赖前者放行后的事实），调用也不进执行窗口（DE-025 D-219）。
pub fn apply_before(req: &BeforeRequest<'_>) -> Option<BeforeRefusal> {
    for behavior in handlers() {
        if !req.declared.contains(&behavior.behavior()) {
            continue;
        }
        if let BeforeDecision::Refuse(refusal) = behavior.before(req) {
            return Some(refusal);
        }
    }
    None
}

#[cfg(test)]
#[path = "effects.test.rs"]
mod tests;
