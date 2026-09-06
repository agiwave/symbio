//! 系统抽象层
//!
//! 提供跨平台的命令执行、字符编码处理和路径标准化逻辑

use encoding_rs::GBK;
use std::process::Stdio;
use tokio::process::Command;

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

/// 执行系统命令并返回解码后的输出（合并 stdout 和 stderr）
pub async fn run_command(
    cmd: &str,
    args: &[&str],
    workdir: Option<&str>,
) -> Result<String, String> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(path) = workdir {
        command.current_dir(path);
    }

    let output = command.output().await.map_err(|e| e.to_string())?;

    let mut combined = Vec::new();
    combined.extend_from_slice(&output.stdout);
    if !output.stderr.is_empty() {
        combined.extend_from_slice(b"\n--- stderr ---\n");
        combined.extend_from_slice(&output.stderr);
    }

    Ok(decode_output(&combined))
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
