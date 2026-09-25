//! `symbio/src/plugins/vdfs/protocol.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn ops_are_unique_and_prefixed() {
    assert_eq!(VDFS_OPS.len(), 13, "新增协议操作请同步本计数与文档");
    for op in VDFS_OPS {
        assert!(op.starts_with("vdfs/"), "协议路径必须以 vdfs/ 开头：{op}");
    }
    let mut sorted = VDFS_OPS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), VDFS_OPS.len(), "协议路径不得重复");
}

#[test]
fn write_request_to_content_text_and_binary() {
    let c = VdfsWriteRequest {
        path: "/setting/local".into(),
        text: Some("{\"a\":1}".into()),
        ..Default::default()
    }
    .to_content();
    assert_eq!(c.text.as_deref(), Some("{\"a\":1}"));
    assert!(!c.binary);
    assert_eq!(c.size, 7);

    let b = VdfsWriteRequest {
        path: "/x/y".into(),
        b64: Some("AAEC".into()),
        ..Default::default()
    }
    .to_content();
    assert!(b.binary);
    assert_eq!(b.b64.as_deref(), Some("AAEC"));
}

#[test]
fn write_request_carries_create_intent() {
    // create 位随线路信封进入域内容体（provider 据此区分新建 / 覆盖）
    let c = VdfsWriteRequest {
        path: "/session/abc.session".into(),
        text: Some(String::new()),
        create: true,
        ..Default::default()
    }
    .to_content();
    assert!(c.create, "create 位必须透传到 VdfsContent");
    assert_eq!(c.text.as_deref(), Some(""));

    // 缺省为覆盖（create = false），且 false 不污染线上形状
    let plain = VdfsWriteRequest {
        path: "/a".into(),
        text: Some("x".into()),
        ..Default::default()
    }
    .to_content();
    assert!(!plain.create);
    let v = serde_json::to_value(&plain).unwrap();
    assert!(v.get("create").is_none(), "false 不序列化");
}

/// 线格式不变：条目 = `{path, …节点字段}`（[`VdfsItem`] 用 `#[serde(flatten)]`）
///
/// 地址从**节点**移到**条目**上，线上形状一个字节没变——这是这次收敛能安全落地的
/// 前提：所有既有消费者读到的仍是同一个 `{path, name, title, …}`。
#[test]
fn list_item_keeps_the_wire_shape() {
    let resp = VdfsListResponse {
        path: "/mem".into(),
        node: VdfsNode::dir("mem", "内存", crate::symbio_core::VdfsAccess::LIST),
        items: vec![VdfsItem::new(VdfsNode::file(
            "a.txt",
            "A",
            crate::symbio_core::VdfsAccess::READ,
        ))
        .with_path("mem/a.txt")],
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["items"][0]["name"], serde_json::json!("a.txt"));
    assert_eq!(v["items"][0]["path"], serde_json::json!("mem/a.txt"));
    let back: VdfsListResponse = serde_json::from_value(v).unwrap();
    assert_eq!(back.items.len(), 1);
    assert_eq!(back.items[0].path, "mem/a.txt");
}
