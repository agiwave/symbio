//! 上下文贡献：把**路径语义**作为一段静态上下文注入。
//!
//! 八个 VDFS 工具的 `path` 参数共用同一套语义。这段文案此前逐份复制在八份 schema 里，
//! 每次请求都要把同一段文字发八遍——它属于「模型一开始就该知道的环境事实」，不属于
//! 任何一个工具的参数说明。因此它在这里以**一段**上下文注入，schema 里只留短句
//! （`tools::PATH_SHORT`）。
//!
//! 段内容与 [`crate::tools::PATH_DESC`] 同源，不另写第二份文案；provider 是纯函数，
//! 不读事实、不写事实——语义本身不随轮次变化。
//!
//! 节奏是**请求级**：段随每次模型请求重新拼接（发言级只在首幕出现，而工具每一轮都在，
//! 第二轮之后模型就不能再靠首幕那段理解路径了）。

use symbio_plugin_api::{Cadence, ContextProvider, ContextRequest, ContextSection};

/// owner 决定拼接顺序（按 `owner` 字典序）。
const OWNER: &str = "vdfs";

/// 段身份（`ChatNode.meta.kind`）。
pub(crate) const SLOT: &str = "vdfs_path";

/// 段优先级：路径语义是**必须一直看得见**的环境事实，排在参考类段之上、宿主三类段之下。
const PRIO: u8 = 60;

pub(crate) struct VdfsPathProvider;

impl ContextProvider for VdfsPathProvider {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn cadence(&self) -> Cadence {
        Cadence::Request
    }

    fn provide(&self, _req: &ContextRequest<'_>) -> Vec<ContextSection> {
        vec![ContextSection::new(
            SLOT,
            crate::tools::PATH_DESC.to_string(),
            PRIO,
        )]
    }
}

symbio_plugin_api::submit_context_provider!(VdfsPathProvider);

#[cfg(test)]
#[path = "context.test.rs"]
mod tests;
