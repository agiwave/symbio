//! `symbio/src/plugins/skill/detail.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn definition_covers_skill_form_surface() {
    let def = skill_detail_definition();
    assert_eq!(def.binding, "upload");
    // 基本 2 字段 + 触发与调用折叠分区 5 字段 + 正文 1 字段
    assert_eq!(def.sections[0].fields.len(), 2);
    assert!(def.sections[1].collapsed);
    assert_eq!(def.sections[2].fields.len(), 1);
    let ids: Vec<&str> = def.actions.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"save") && ids.contains(&"delete"));

    // 导入整包是**详情页动作**（不是 `root_new_type` 上的字段）：只在草稿态出现，
    // 载荷声明为一个本地 zip——使用方据此先取文件再执行。
    let import = def
        .actions
        .iter()
        .find(|a| a.id == crate::symbio_core::vdfs_provider::VDFS_ACTION_IMPORT)
        .expect("详情页要给出导入入口，否则草稿上无从导入");
    assert_eq!(
        import.pack.as_deref(),
        Some(crate::symbio_core::vdfs_provider::VDFS_EXT_ZIP)
    );
    let when = import.when.as_ref().expect("导入只在草稿态");
    assert!(when.holds(&serde_json::json!({ "is_existing": false })));
    assert!(!when.holds(&serde_json::json!({ "is_existing": true })));
}

#[test]
fn manifest_roundtrip_preserves_fields() {
    let manifest = serde_json::json!({
        "name": "pdf-extract",
        "description": "Extract text and tables from PDF files.",
        "when_to_use": "When the user asks to parse a PDF",
        "argument_hint": "<pdf_path>",
        "allowed_tools": ["read_file", "bash"],
        "model": "claude-sonnet-4-5",
        "disable_model_invocation": true,
        "body": "Read the PDF at ${input}.",
    });
    let md = manifest_to_skill_md("pdf-extract", &manifest).unwrap();
    let cfg = skill_md_to_config(&md).unwrap();
    assert_eq!(cfg["name"], "pdf-extract");
    assert_eq!(cfg["allowed_tools"][0], "read_file");
    assert_eq!(cfg["disable_model_invocation"], true);
    assert_eq!(cfg["body"], "Read the PDF at ${input}.");
    // 生成的 SKILL.md 能被 loader 同规则解析（frontmatter 闭合）
    assert!(md.starts_with("---\nname: pdf-extract\n"));
    assert!(md.contains("\n---\n"));
}

#[test]
fn manifest_rejects_name_mismatch_and_short_description() {
    let ok_desc = "A long enough description.";
    let manifest = serde_json::json!({ "name": "other", "description": ok_desc });
    let err = manifest_to_skill_md("pdf-extract", &manifest).unwrap_err();
    assert!(err.to_string().contains("必须与目录名"));

    let manifest = serde_json::json!({ "name": "pdf-extract", "description": "short" });
    let err = manifest_to_skill_md("pdf-extract", &manifest).unwrap_err();
    assert!(err.to_string().contains("10 字符"));
}
