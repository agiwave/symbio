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

use symbio_kernel::exec::ExecEventSink;
use symbio_kernel::plugin::Plugin;

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
    fn kick(&self, target: DriveTarget)
        -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>;

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
