//! 系统工具函数层
//!
//! 提供跨平台的字符编码处理和参数校验逻辑。
//!
//! 归属规则：只被单一插件消费的设施定义在该插件内部，core 不承载单模块内部
//! 定义——本模块的唯一消费者（shell / content_search）均在本插件内。
//!
//! 命令执行不在本模块：`shell.rs` 自带完整实现，本模块只提供编码解码与参数校验。

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
