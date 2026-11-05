//! 「事实落账后踢一次驱动」的宿主契约。
//!
//! 挂点是**事实写入的唯一收口**（事实源 `append` 成功之后），不是任何 API 的返回处：
//! 每个入口各自记得踢一脚，第五个入口必忘；事实只有本进程写，写口唯一，挂在这里
//! 所有写方都受益。
//!
//! 写口（会话事实源）只认本 trait；**实现只有装配层一份**（经 `inventory` 登记，
//! 与事实源 opener 同一手法）。判据：**一个实现、多处调用**。
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

/// 事实写入成功之后踢一次驱动。
pub trait DriveEntry: Send + Sync + 'static {
    /// 踢一次本会话的驱动。
    ///
    /// `None` = 本会话已在跑：调用方无需动作（dirty 位已置，当前那轮退出前会再跑）。
    /// 返回的 future 由调用方 spawn；它**只调用一次**驱动入口，不在外面包
    /// `loop { drive }`——环内部的轮次推进归环自己。
    ///
    /// `workdir` 是会话 metadata 的工作目录：`append` 不携带运行选项，由写口
    /// （会话事实源）在打开时从会话 metadata 取来带上；`None` = 由实现回落。
    fn kick(
        &self,
        session_id: &str,
        workdir: Option<&str>,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>;

    /// 等本会话驱动**空闲**（`running == false && dirty == false`）。
    ///
    /// **这是等，不是推**：不要拿它当第二个 kick。`NoDebt` 与真等待（模型未回、
    /// 答复未到）都算停——驱动已返回、`running` 已清。
    ///
    /// 严禁在驱动体内调用：实现检测到「由驱动自身调用」时直接返回，不自锁。
    fn settled(&self, session_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
}

/// 装配层登记的驱动实现（进程内唯一）。
pub struct DriveEntryDecl {
    pub entry: &'static dyn DriveEntry,
}

inventory::collect!(DriveEntryDecl);

/// 取装配层登记的驱动实现；未装配（如单测）返回 `None`，写口静默跳过。
pub fn declared_drive_entry() -> Option<&'static dyn DriveEntry> {
    inventory::iter::<DriveEntryDecl>
        .into_iter()
        .next()
        .map(|decl| decl.entry)
}
