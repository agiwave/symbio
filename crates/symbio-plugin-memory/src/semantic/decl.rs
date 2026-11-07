//! 语义记忆的**词汇**：`memory.recalled` 的名字（DE-026 D-264 第 4 条）。
//!
//! ## 名字在这里，格位在 `crate::decl`
//!
//! 一条事实的「名字 + 格位」是它的**词汇**。本包交名字（本文件），格位随记忆四格
//! 一起在 [`crate::decl::EVENTS`] 提交——词汇的 owner 是记忆插件，提交口只有一个，
//! 免得「同一族名字分两处声明」。
//!
//! ## 与 `shared_events` 的分界
//!
//! 名字**没有**登记进 `PLUGIN.yml` 的 `shared_events`：那份清单是给「别的插件按名字
//! 取用」的（生成 `symbio_plugin_api::events`），而 `memory.recalled` 今天没有跨插件
//! 读方。等真有第二个读方时，动作是把它登记进清单、改从生成物取——而不是让读方
//! 抄一份字面量（DE-024 D-204）。

/// 检索事实的名字。幂等键的**词干**由宿主算（`recalled-{user_seq}`，D-252）。
pub(crate) const EVENT_MEMORY_RECALLED: &str = "memory.recalled";
