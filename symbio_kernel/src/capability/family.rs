//! 内置六族——**跨插件共享的能力行词汇**。
//!
//! 族是值（[`CapabilityFamily`]），不是内核枚举：新族不必在这里
//! 登记。在声明它的插件里写
//! `const MY_FAMILY: CapabilityFamily = CapabilityFamily::multi("my_family");` 与一个
//! 载荷类型即可，内核零改动（[D-175](../../../../docs/003-design/021-横向扩充与插件切分.md "DE-021-横向扩充与插件切分")）。
//!
//! 六个常量留在这里的判据是**词汇归属**而不是形状：它们各自有 ≥2 个消费方在共享地址上
//! 看见（工具面、模型、提示词、挂载、选项、可配置），故名字必须有一份单点定义。
//! 新族先住声明方；长到第二个消费者时，同样搬到共享处——那是词汇问题，不是内核形状问题。

use super::CapabilityFamily;

/// 能力行：LLM 可见工具（可调用对象）。
pub const TOOL: CapabilityFamily = CapabilityFamily::multi("tool");
/// 模型服务：生效者占槽位，其余是已启用目录。
pub const MODEL: CapabilityFamily = CapabilityFamily::single("model");
/// 系统提示词：一段文字，行名就是段名（覆盖键）。
pub const PROMPT: CapabilityFamily = CapabilityFamily::multi("prompt");
/// VDFS 挂载：根是唯一占位的行（保留行名 `<root>`）。
pub const MOUNT: CapabilityFamily = CapabilityFamily::single("mount");
/// 选项字段：会话配置表单的一个字段（行名 = `field.key`）。
pub const OPTION: CapabilityFamily = CapabilityFamily::multi("option");
/// 可配置声明：设置页里的一份配置条目（行名 = 目录名）。
pub const CONFIGURABLE: CapabilityFamily = CapabilityFamily::multi("configurable");
