//! 写族的产物富化：**声明选实现**、**成败定有无**、**名字只在本地**。

use super::FileChange;
use symbio_core::FileOp;
use symbio_plugin_api::behaviors;
use symbio_plugin_api::effects::{EffectInput, ToolBehavior};

fn input(tool: &str, success: bool) -> EffectInput {
    EffectInput {
        tool: tool.into(),
        call_id: "call-1".into(),
        args: serde_json::json!({ "path": "src/lib.rs" }),
        success,
        result: String::new(),
        why: String::new(),
        declared: &[behaviors::FILE_CHANGE],
    }
}

/// 写族四个工具各给一个动作，且**线格式与 core 的枚举同源**（不抄一份拼写）。
#[test]
fn write_family_maps_to_the_four_ops() {
    for (tool, op) in [
        ("vdfs_write", FileOp::Write),
        ("vdfs_edit", FileOp::Edit),
        ("vdfs_delete", FileOp::Delete),
        ("vdfs_mkdir", FileOp::Mkdir),
    ] {
        let got = FileChange.annotate(&input(tool, true));
        assert_eq!(got.len(), 1, "{tool} 要补一个字段");
        assert_eq!(got[0].0, "file_op");
        assert_eq!(got[0].1, serde_json::json!(op.as_str()));
    }
}

/// 挂载点前缀不影响判据（线上名是 `local/vdfs_write` 这种形态）。
#[test]
fn a_mount_prefix_is_tolerated() {
    let got = FileChange.annotate(&input("local/vdfs_write", true));
    assert_eq!(got[0].1, serde_json::json!("write"));
}

/// 读 / 统计族与未知工具：**不补**——它们不是工作目录的变更。
#[test]
fn read_family_annotates_nothing() {
    for tool in [
        "vdfs_read",
        "vdfs_list",
        "vdfs_stat",
        "vdfs_search",
        "vdfs_tree",
        "shell",
        "mcp__srv__vdfs_write",
    ] {
        assert!(
            FileChange.annotate(&input(tool, true)).is_empty(),
            "{tool} 不是写族"
        );
    }
}

/// 失败的写不补：一条失败产物的载荷里出现 `file_op` 本身就是谎。
#[test]
fn a_failed_write_annotates_nothing() {
    assert!(FileChange.annotate(&input("vdfs_write", false)).is_empty());
}
