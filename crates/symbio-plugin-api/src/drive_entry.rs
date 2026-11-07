//! 「事实落账后踢一次驱动」的宿主契约。
//!
//! 挂点是**事实写入的唯一收口**（事实源 `append` 成功之后），不是任何 API 的返回处：
//! 每个入口各自记得踢一脚，第五个入口必忘；事实只有本进程写，写口唯一，挂在这里
//! 所有写方都受益。
//!
//! 写口（会话事实源）只认本 trait；**实现只有装配层一份**（经 `inventory` 登记，
//! 与事实源 opener 同一手法）。判据：**一个实现、多处调用**。
//!
//! ## 驱动绑定「根」，不按入口反查
//!
//! 装配层按**部署根**（`Home` / 子智能体的容器树 / 测试装配）构造实例：
//! [`DriveBinding::bind`] 收根插件、交还该根的唯一实例。事实源在打开那一刻
//! 就把这份句柄连同会话身份、工作目录一起绑在自己身上，[`DriveTarget`] 直接
//! 带着事实源句柄交回实现——写口**不再**用「进程级入口 + 按会话 id 反查事实源」，
//! 子空间会话因此不会落到主根的 store 上。
//!
//! ## 为什么返回 future 而不是自己 spawn
//!
//! 装配层不持有运行时（`tokio` 只是 dev 依赖）；写口已经在异步环境里，由它把
//! future 交给自己运行时。装配层只提供「驱动一次」的唯一实现。
//!
//! ## 重入
//!
//! 实现按会话维护「已在跑」标记与 dirty 位：跑着期间再来的写入只置 dirty、不嵌套；
//! 退出前 dirty 为真就再跑一轮。不加锁、不加线程、不加定时器。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::Plugin;
use symbio_sdk::agent::AgentScope;

use crate::fact_source::FactSource;

/// 一次踢驱动要交回的全部输入：**根已绑在实现实例上**，这里只带本会话的事实。
///
/// 事实源句柄由写口在打开时留下，踢的时候原样交回——实现不再按 `session_id`
/// 去开面反查，子空间会话的事实源不会被解析成主根的 store。
pub struct DriveTarget {
    /// 本会话身份（驱动上下文里的 `SESSION_ID`）。
    pub session_id: String,
    /// 会话 metadata 的工作目录：`append` 不携带运行选项，由写口在打开时取来。
    pub workdir: Option<String>,
    /// 会话主体来源：metadata 的 `agent_id`（空串 = 无绑定主体 ⇒ `session/main`）。
    ///
    /// 由写口在打开会话事实源时取来，`drive_on_new_fact` 据此设 `AGENT_ID`——
    /// **全链路只有这一处设它**，其余执行体只读。`None` = 取不到（拒绝回落主体）。
    pub agent_id: Option<String>,
    /// 会话运行参数：已按 `req > session.metadata > 默认` 解析完的**结果值**。
    ///
    /// 与 `workdir` / `agent_id` 同一来源、同一理由：`append` 不携带运行选项，由写口
    /// 在打开会话事实源时从 metadata 取来；`drive_on_new_fact` 把它们挂进本轮 ctx
    /// （`MODE` / `RISK_LEVEL` / `PROVIDER_ID`），审批闸门与模型端口据此取值。
    /// 解析链的**唯一实现**在会话侧（`store/layout.rs::resolve_session_params`）。
    pub mode: String,
    pub risk_level: String,
    /// 显式指定的 provider；`None` = 由模型侧选默认（本契约不猜）。
    pub provider_id: Option<String>,
    /// 本会话的 Agent 权限作用域（D-153）：`agent_id` 经 agent 插件解析后的运行期
    /// 事实（目录根 + 声明），由**收集方**在 `collect_capabilities` 之后发布到事实源；
    /// `None` = 本会话没有选中 Agent（宿主态，工具侧按宿主权限处理）。
    ///
    /// 载在驱动目标上而不是让执行体回头解析：工具执行跑在驱动新建的 ctx 里，
    /// 与收集期的轮 ctx 不是同一个；重新读 manifest 就是第二个解析器。
    pub agent_scope: Option<AgentScope>,
    /// 本轮入站来源主体（D-067）：请求 `metadata.source_principal` 声明的数据，
    /// 由开轮写方从收件箱条目发布、事实源随目标带过来。`None` = 本机用户发起。
    pub source_principal: Option<String>,
    /// 本轮的中止信号（`DE-047` 新增槽位）：由会话状态**唯一持有**，事实源只引用
    /// 同一个 `Arc` 的当前值。`None` = 没有在途轮次（读方按「不会中止」处理）。
    ///
    /// 它是**驱动级信号**（`DE-047`：`abort` 留环），不是端口私有值：环把它挂进本轮
    /// ctx 的 `ABORT_SIGNAL`，模型 / 工具的读取面因此看到同一个对象。
    pub abort: Option<ExecAbortSignal>,
    /// 本会话的事实源句柄（写口打开的那一份，唯一写口）。
    pub facts: Arc<dyn FactSource>,
    /// 执行期出口：由装配层按根提供真实转写句柄；内部请求显式静默。
    pub sink: ExecEventSink,
}

/// 事实写入成功之后踢一次驱动。
pub trait DriveEntry: Send + Sync + 'static {
    /// 踢一次本会话的驱动。
    ///
    /// `None` = 本会话已在跑：调用方无需动作（dirty 位已置，当前那轮退出前会再跑）。
    /// 返回的 future 由调用方 spawn；它**只调用一次**驱动入口，不在外面包
    /// `loop { drive }`——环内部的轮次推进归环自己。
    fn kick(
        &self,
        target: DriveTarget,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>;

    /// 等本会话驱动**空闲**（`running == false && dirty == false`）。
    ///
    /// **这是等，不是推**：不要拿它当第二个 kick。`NoDebt` 与真等待（模型未回、
    /// 答复未到）都算停——驱动已返回、`running` 已清。
    ///
    /// 严禁在驱动体内调用：实现检测到「由驱动自身调用」时直接返回，不自锁。
    fn settled(&self, session_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
}

/// 按**部署根**造驱动实例的工厂。
///
/// 装配层登记唯一实现；每个根（主根 / 子空间容器树 / 测试装配）各持一份实例。
/// 「一个实现」指实现类型只有一份，不是「一个进程只能有一个实例」——按入口
/// 共用实例会把子空间会话的驱动落回主根。
pub trait DriveBinding: Send + Sync + 'static {
    /// 为 `root` 绑定一份驱动实例（同一根反复绑定应得到可用的会话状态共享）。
    fn bind(&self, root: Arc<dyn Plugin>) -> Arc<dyn DriveEntry>;
}

/// 装配层登记的按根绑定工厂（进程内唯一实现）。
pub struct DriveBindingDecl {
    pub binding: &'static dyn DriveBinding,
}

inventory::collect!(DriveBindingDecl);

/// 取装配层登记的按根绑定工厂；未装配（如单测）返回 `None`，写口静默跳过。
pub fn declared_drive_binding() -> Option<&'static dyn DriveBinding> {
    inventory::iter::<DriveBindingDecl>()
        .next()
        .map(|decl| decl.binding)
}

/// 装配层登记的驱动实现（主根的那一份，`settled` 等入口用）。
pub struct DriveEntryDecl {
    pub entry: &'static dyn DriveEntry,
}

inventory::collect!(DriveEntryDecl);

/// 取装配层登记的主根驱动实现；未装配（如单测）返回 `None`，写口静默跳过。
pub fn declared_drive_entry() -> Option<&'static dyn DriveEntry> {
    inventory::iter::<DriveEntryDecl>()
        .next()
        .map(|decl| decl.entry)
}
