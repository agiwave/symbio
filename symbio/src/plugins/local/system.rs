//! 系统工具函数层
//!
//! 提供跨平台的字符编码处理和参数校验逻辑。
//!
//! Phase sink：自 `symbio_core/system.rs` 下沉至 local 插件——唯一消费者
//! （shell / content_search）均在本插件内，属模块私有设施，
//! 不再置于 core 共享层。
//!
//! 体检备注（audit-4）：原 `run_command` 自 core 时代起即无任何消费者，
//! 已删除；shell.rs 的命令执行有其自身的完整实现。

use encoding_rs::GBK;

/// 自动探测并解码字节流（支持 Windows GBK 回退）
pub fn decode_output(bytes: &[u8]) -> String {
    let (res, _, has_errors) = encoding_rs::UTF_8.decode(bytes);
    if !has_errors {
        return res.into_owned();
    }
    // 如果 UTF-8 解码失败，回退到 GBK (Windows)
    let (res_gbk, _, _) = GBK.decode(bytes);
    res_gbk.into_owned()
}

/// 验证输入参数是否符合要求的字段
pub fn validate_params(input: &serde_json::Value, required: &[&str]) -> Result<(), String> {
    for field in required {
        if input.get(*field).is_none() {
            return Err(format!("缺少必填参数: {field}"));
        }
    }
    Ok(())
}
