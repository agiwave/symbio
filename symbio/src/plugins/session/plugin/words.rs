//! 会话节点的**协议词表**（自 `symbio_core::vdfs_provider` 下沉，原样搬移不换语义）。
//!
//! 这些词是**会话域**的线上契约：`kind` / `ext` / `attributes.outcome` 的取值
//! 由前端（TS 侧 `schemas/` 有镜像）与 CLI 按字面量判读，谁都不许自己拼。
//! 消费者只有会话域自己，因此词表住在这里，不再占用内核词表。

// ==================== 场景类型（`kind`） ====================

/// 会话的**转写列表**（`<根>/session/<id>/<段>`）。
///
/// `kind` 是场景可自定义的（会话叶子自己就声明 `kind = "session"`），
/// 这里给转写列表一个**稳定的 ASCII 语义类型**：它的 `name` / `title` 是
/// 面向用户的展示名（可能随语言或文案调整），不能被消费者当成标识来认；
/// 而 `kind` 是**协议词**——消费者按它发现「哪个子目录是转写」，不必硬编码段名。
pub const KIND_MESSAGES: &str = "messages";

/// 会话的**收件箱**（`<根>/session/<id>/inbox`）。
///
/// 与 [`KIND_MESSAGES`] 同一手法：`name` / `title` 是展示名，`kind` 是协议词。
/// 收件箱里的一条是**还没被消费的用户消息**，因此条目的 `ext` 沿用
/// [`EXT_MESSAGE`]（它就是一条消息），靠 `kind` 与会话转写里的消息区分开：
/// 一个是"待发"，一个是"已发生"。
pub const KIND_INBOX: &str = "inbox";

// ==================== 上一轮结局（`attributes.outcome`） ====================
//
// 会话叶子用 `status` 表达「现在在不在跑」，用 `outcome` 表达「上一轮怎么结束的」。
// 两者是同一份运行态投影出的两个属性（`SessionRuntime`），所以词表必须住在一起
// ——消费方（前端、CLI、子智能体转播）读的是同一批字面量，谁都不许自己拼。
//
// 与内核的 `VDFS_STATUS_*` 同一约定：`outcome` 只在 `status != working` 时有
// 意义（`working` 时结局作废），两者一起读才构成完整的运行态。

/// 上一轮**正常结束**
pub const OUTCOME_COMPLETED: &str = "completed";
/// 上一轮**被用户中止**（与 `completed` 区分：提示音音色、收尾文案不同）
pub const OUTCOME_ABORTED: &str = "aborted";
/// 上一轮**以错误结束**——此时 `attributes.error` 带错误文案
pub const OUTCOME_FAILED: &str = "failed";

// ==================== 呈现扩展名（`ext`） ====================

/// 会话工作区（实时对话流）
pub const EXT_SESSION: &str = "session";
/// 单条对话消息（**列表项**：正文在内容里，结构在 `attributes` 里）
pub const EXT_MESSAGE: &str = "message";
