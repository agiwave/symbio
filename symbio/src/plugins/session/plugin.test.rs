//! `plugin.rs` 模块根的单元测试（结构体 / `impl Plugin` / 配置定义）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：测试跟着
//! **被测试的实现文件**走——`plugin/nodes.rs` 与 `plugin/vdfs_provider.rs` 的测试
//! 分别在 `plugin/nodes.test.rs`、`plugin/vdfs_provider.test.rs`。

use super::*;

/// 验证 session 存储目录**只**从 HomedirRegistry 派生，不依赖 config；
/// 且它就是宿主层的资源类别根（`category_dir(PLUGIN_SESSION)`）——
/// 会话因此不再手拼一份 `<homedir>/plugins/<类别>` 布局。
#[test]
fn test_session_storage_dir_from_homedir() {
    let dir = SessionPlugin::session_storage_dir();
    let expected = crate::symbio_core::HomedirRegistry::get()
        .join("plugins")
        .join("session");
    assert_eq!(
        dir, expected,
        "session_storage_dir 必须等于 <homedir>/plugins/session"
    );
    assert_eq!(
        dir,
        crate::providers::vdfs_service::entry::category_dir(PLUGIN_SESSION),
        "会话存储根必须与 VDFS 资源类别根同一条构造式"
    );
    assert!(
        dir.is_absolute(),
        "session_storage_dir 必须是绝对路径: {}",
        dir.display()
    );
}

/// 验证 SessionConfig 不再包含已删除的死字段：
/// - `storage_dir`（存储根由 HomedirRegistry 统一决定）
/// - `session_id`（零消费者；id 由会话目录名决定，配置内自指冗余）
#[test]
fn test_session_config_has_no_dead_fields() {
    let cfg = SessionConfig::default();
    let json = serde_json::to_value(&cfg).unwrap();
    for key in ["storage_dir", "session_id"] {
        assert!(
            json.get(key).is_none(),
            "SessionConfig 不应再包含 {key} 字段, got: {json}"
        );
    }
}

/// 验证从含旧字段的配置反序列化时，未知键被静默忽略（旧 session_config.json
/// 无需迁移即可继续加载）。
#[test]
fn test_session_config_deserialize_ignores_legacy_keys() {
    let json = serde_json::json!({
        "storage_dir": "/tmp/should_be_ignored",
        "session_id": "stale-id-should-be-ignored",
        "max_messages": 42,
    });
    let cfg: SessionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(cfg.max_messages, 42, "max_messages 应被正确反序列化");
}

// ==================== 配置契约一致性（复杂度审计 P0-1 / P1-①）====================

/// `max_tool_rounds` 的契约翻译：0 → `None`（不限制），>0 → `Some(n)`（软上限）。
/// 修复前编排层三处无条件 `Some(...)`，使 chat_loop 的 `None` = 无限语义在主路径不可达。
#[test]
fn max_tool_rounds_zero_maps_to_unlimited() {
    let cfg = SessionConfig {
        max_tool_rounds: 15,
        ..Default::default()
    };
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        Some(15),
        "显式 >0 的值应作为软上限下发"
    );

    let cfg = SessionConfig {
        max_tool_rounds: 0,
        ..Default::default()
    };
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        None,
        "0 必须翻译为 None（不限制），而非 Some(0)（0 轮即熔断）"
    );
}

/// 默认配置的产品意图锁定：默认**不设硬性轮次上限**，以显式语义 `0`（不限制）
/// 表达，而非魔法数 65535（旧默认，行为等价但语义含糊）。
#[test]
fn default_max_tool_rounds_is_effectively_unlimited() {
    let cfg = SessionConfig::default();
    assert_eq!(
        cfg.max_tool_rounds, 0,
        "默认轮次上限必须是 0 = 不限制（用户明确要求不设硬上限）"
    );
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        None,
        "默认配置翻译到 Request 层必须是 None（无限轮次）"
    );
}

// ==================== 配置文档（`.vdfs/session/PLUGIN.yml`） ====================

/// **定义与配置同源**：面板字段的默认值一律来自 `SessionConfig::default()`，
/// 且 serde 默认值函数与 `Default` impl 不漂移（两处各自书写必然漂移）。
#[test]
fn config_definition_defaults_come_from_session_config() {
    let defaults = serde_json::to_value(SessionConfig::default()).unwrap();
    let def = config_definition();
    let fields = &def.sections[0].fields;
    assert!(!fields.is_empty(), "会话配置必须有字段");
    for f in fields {
        let declared = f
            .default
            .clone()
            .unwrap_or_else(|| panic!("字段 {} 缺少 default", f.key));
        let actual = defaults
            .get(&f.key)
            .unwrap_or_else(|| panic!("SessionConfig 不存在字段 {}", f.key));
        assert_eq!(
            &declared, actual,
            "面板 {}.default={declared} 与 SessionConfig::default().{}={actual} 不一致",
            f.key, f.key
        );
    }

    let from_empty: SessionConfig =
        serde_json::from_str("{}").expect("空对象应能反序列化出默认配置");
    assert_eq!(
        serde_json::to_value(&from_empty).unwrap(),
        defaults,
        "SessionConfig 的 serde 默认值与 Default impl 漂移了（两处各自书写）"
    );
}
