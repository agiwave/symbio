//! 段判据：路径语义只有一份、经端口可收集、节奏是请求级。

use super::{VdfsPathProvider, SLOT};
use crate::tools::PATH_DESC;
use symbio_plugin_api::{Cadence, ContextProvider, ContextRequest};

fn request<'a>() -> ContextRequest<'a> {
    ContextRequest {
        request_round: 0,
        principal: "session/a",
        query: None,
        facts: &[],
        recall: None,
        live: None,
    }
}

/// provider 交出的正文与 `PATH_DESC` 逐字相同（不另写第二份文案）。
#[test]
fn the_section_carries_the_same_text_as_path_desc() {
    let sections = VdfsPathProvider.provide(&request());
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].slot, SLOT);
    assert_eq!(sections[0].text, PATH_DESC);
}

/// 请求级节奏：段随每次模型请求重新拼接，而不是只在首幕出现。
#[test]
fn the_section_is_request_level() {
    assert_eq!(VdfsPathProvider.cadence(), Cadence::Request);
}

/// 收集面按 owner 找到这条 provider（同一份提交，不另写注册）。
#[test]
fn the_section_is_collected_through_the_port() {
    let sections = symbio_plugin_api::context_sections(&request(), Cadence::Request);
    let own: Vec<_> = sections.iter().filter(|s| s.slot == SLOT).collect();
    assert_eq!(own.len(), 1, "{sections:?}");
}
