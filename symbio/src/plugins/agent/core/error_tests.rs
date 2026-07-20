//! AgentError 单元测试
//!
//! 对应源文件: `error.rs`

use super::*;

#[test]
fn test_validation_maps_to_validation_error() {
    let err = AgentError::validation("missing field");
    let pe: PluginError = err.into();
    assert!(matches!(pe, PluginError::ValidationError(_)));
}

#[test]
fn test_config_maps_to_validation_error() {
    let err = AgentError::Config("bad backend".to_string());
    let pe: PluginError = err.into();
    assert!(matches!(pe, PluginError::ValidationError(_)));
}

#[test]
fn test_not_found_maps_to_validation_error() {
    // I-048: NotFound 变体直接映射，不再依赖字符串匹配
    let err = AgentError::NotFound("Agent 'foo' not found".to_string());
    let pe: PluginError = err.into();
    assert!(matches!(pe, PluginError::ValidationError(_)));
}

#[test]
fn test_already_exists_maps_to_validation_error() {
    // I-048: AlreadyExists 变体直接映射
    let err = AgentError::AlreadyExists("id 'x' 已存在".to_string());
    let pe: PluginError = err.into();
    assert!(matches!(pe, PluginError::ValidationError(_)));
}

#[test]
fn test_storage_not_found_string_no_longer_detected() {
    // I-048: Storage 变体不再做字符串匹配，统一映射为 InternalError
    let err = AgentError::Storage("NotFound: 单元不存在".to_string());
    let pe: PluginError = err.into();
    assert!(matches!(pe, PluginError::InternalError(_)));
}

#[test]
fn test_io_maps_to_internal() {
    let io_err = std::io::Error::other("boom");
    let err: AgentError = io_err.into();
    let pe: PluginError = err.into();
    assert!(matches!(pe, PluginError::InternalError(_)));
}

#[test]
fn test_store_error_bridge() {
    // I-048: StoreError::NotFound 直接映射到 AgentError::NotFound
    let s_err = StoreError::NotFound("unit".to_string());
    let a_err: AgentError = s_err.into();
    assert!(matches!(a_err, AgentError::NotFound(_)));
    assert!(a_err.is_not_found());

    // StoreError::AlreadyExists 直接映射到 AgentError::AlreadyExists
    let s_err = StoreError::AlreadyExists("id".to_string());
    let a_err: AgentError = s_err.into();
    assert!(matches!(a_err, AgentError::AlreadyExists(_)));

    // StoreError::InvalidInput 映射到 AgentError::Validation
    let s_err = StoreError::InvalidInput("bad".to_string());
    let a_err: AgentError = s_err.into();
    assert!(matches!(a_err, AgentError::Validation(_)));
}
