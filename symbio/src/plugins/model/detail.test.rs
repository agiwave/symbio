//! `symbio/src/plugins/model/detail.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn definition_covers_model_form_surface() {
    let def = model_detail_definition();
    assert_eq!(def.binding, "upload");
    // 基本分区 6 字段 + 高级折叠分区 6 字段
    assert_eq!(def.sections[0].fields.len(), 6);
    assert_eq!(def.sections[1].title.as_deref(), Some("高级设置"));
    assert!(def.sections[1].collapsed);
    // 高级设置包含最大上下文（max_context_tokens，默认 256k）
    let ctx_field = def.sections[1]
        .fields
        .iter()
        .find(|f| f.key == "max_context_tokens")
        .expect("max_context_tokens field should exist");
    assert_eq!(ctx_field.default, Some(serde_json::json!(262_144)));
    // 预设联动：provider 字段触发，预设含模型候选与协议校正
    let spec = def.presets.as_ref().unwrap();
    assert_eq!(spec.field, "provider");
    assert!(spec.presets.len() >= 20);
    let openai = spec.presets.iter().find(|p| p.value == "openai").unwrap();
    assert_eq!(openai.set["api_base"], "https://api.openai.com/v1");
    assert!(openai.options["api_protocol"].contains(&"openai_responses".to_string()));
    // 动作：test/save/divider/set-default/delete 全齐
    let ids: Vec<&str> = def.actions.iter().map(|a| a.id.as_str()).collect();
    assert!(
        ids.contains(&"test")
            && ids.contains(&"save")
            && ids.contains(&"delete")
            && ids.contains(&"set-default")
    );
}
