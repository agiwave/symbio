use std::path::{Path, PathBuf};

mod policy_tracker;
mod policy_types;

pub use policy_tracker::ActionTracker;
pub use policy_types::AutonomyLevel;
// CapabilityRiskLevel 来自 core capability 域（统一风险定义，避免跨插件硬编码）
pub use symbio_plugin_api::CapabilityRiskLevel;

/// 规范化路径用于比较
pub fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// 检查路径是否以另一个路径为前缀
fn path_starts_with_normalized(base: &Path, prefix: &Path) -> bool {
    let normalized_base = normalize_path_for_comparison(base);
    let normalized_prefix = normalize_path_for_comparison(prefix);
    normalized_base.starts_with(&normalized_prefix)
}

/// 检查路径是否是安全的（不含 `..` 段——**两种分隔符都算**）
///
/// 规则本体在 [`symbio_core::vdfs::vdfs_has_parent_segment`]：shell 工具的
/// 路径守卫与 VDFS 物理层守卫必须是**同一条**规则，否则修了一处漏另一处。
/// 这里保留函数名（本模块的公开 API），实现转发过去。
pub fn is_safe_relative_path(path: &str) -> bool {
    !symbio_plugin_api::vdfs_has_parent_segment(path)
}

/// 策略规则（可运行期热更的部分；动作计数独立在 `tracker`）。
///
/// 字段与 local 插件的配置文档（`LocalConfig`）一一对应——配置文档是人类入口，
/// 这里是运行期真源。**默认值即「全放开」是裁决，不是默认没做**：安全边界在审批闸
/// （工具级风险 + 命令级 `risk_for`），这里管的是用户自己的执行偏好——分层与理由见
/// [D-101](../../../../docs/003-design/008-能力注册表与能力坐标系.md#d-101-内置插件的执行策略保留放开默认安全边界在审批闸不在用户偏好 "DE-008-能力注册表与能力坐标系")。
#[derive(Debug, Clone)]
pub struct PolicyRules {
    pub autonomy: AutonomyLevel,
    pub workspace_only: bool,
    /// 命令白名单；**空 = 不限制**（沿用 telegram `allowed_users`「空 = 不限制」
    /// 惯例）。结构性拒绝（命令替换 / 引号未闭合）不受此影响——那些在
    /// [`split_subcommands`] 里，与白名单无关。
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub allowed_roots: Vec<PathBuf>,
    /// 每小时动作上限；**0 = 不限流**。
    pub max_actions_per_hour: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
}

impl Default for PolicyRules {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Full,
            workspace_only: false,
            allowed_commands: Vec::new(),
            forbidden_paths: Vec::new(),
            allowed_roots: Vec::new(),
            max_actions_per_hour: 0,
            require_approval_for_medium_risk: false,
            block_high_risk_commands: false,
        }
    }
}

/// 工具执行安全策略
#[derive(Debug)]
pub struct SecurityPolicy {
    /// 规则本体（内部 `RwLock` 持有：配置文档写入即热更，无需重启）
    rules: std::sync::RwLock<PolicyRules>,
    pub tracker: ActionTracker,
}

impl Clone for SecurityPolicy {
    fn clone(&self) -> Self {
        Self {
            rules: std::sync::RwLock::new(self.rules().clone()),
            tracker: self.tracker.clone(),
        }
    }
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self::new(PolicyRules::default())
    }
}

impl SecurityPolicy {
    pub fn new(rules: PolicyRules) -> Self {
        Self {
            rules: std::sync::RwLock::new(rules),
            tracker: ActionTracker::new(),
        }
    }

    /// 短临界区读取；锁中毒视为可恢复（持锁方只在赋值瞬间持写锁）
    fn rules(&self) -> std::sync::RwLockReadGuard<'_, PolicyRules> {
        self.rules.read().unwrap_or_else(|p| p.into_inner())
    }

    /// 运行期热更规则（配置文档写入后调用；无需重启）
    pub fn update_rules(&self, rules: PolicyRules) {
        *self.rules.write().unwrap_or_else(|p| p.into_inner()) = rules;
    }
}

/// 归一化命令首词：去掉路径前缀（`C:\tools\npm.cmd` → `npm.cmd`）
/// 与 Windows 可执行扩展名（`npm.cmd` / `python.exe` / `run.bat` → `npm` / `python` / `run`）。
///
/// 模型在 Windows 上常写出带扩展名或路径前缀的命令，若不归一化会导致
/// 白名单匹配失败（"命令不在允许列表中"）与风险等级误判（`rm.exe` 被当成 Low 风险）。
fn normalize_base_command(base_cmd: &str) -> &str {
    let name = base_cmd.rsplit(['/', '\\']).next().unwrap_or(base_cmd);
    match name.rsplit_once('.') {
        Some((stem, "exe" | "cmd" | "bat")) => stem,
        _ => name,
    }
}

/// 代码执行器：真实命令藏在**参数里的脚本 / 表达式**中，白名单看不见内层。
///
/// 它们**保留在白名单**（Windows 下模型确实要靠它跑命令，砍掉能力是过度反应），
/// 但风险固定为 [`CapabilityRiskLevel::High`]——默认 Medium 阈值下必须用户审批，
/// 用户显式开到 High 阈值即代表自担风险。
///
/// 列表按**能力**而非「名字眼熟」收录：凡是能执行任意代码的解释器、shell、
/// 批量执行器都算。只收 `cmd` / `powershell` 会漏掉 `sh -c`、`node -e`、
/// `python -c`、`xargs` 等同样能跑任意命令的入口。
const CODE_RUNNERS: &[&str] = &[
    // shell / 包装器
    "powershell",
    "pwsh",
    "cmd",
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
    "ksh",
    "csh",
    "tcsh",
    "wsl",
    "runas",
    "start",
    "env",
    "nohup",
    "xargs",
    "eval",
    "source",
    // 语言解释器
    "node",
    "nodejs",
    "deno",
    "bun",
    "python",
    "python3",
    "py",
    "perl",
    "ruby",
    "php",
    "lua",
    "rscript",
    "julia",
    "java",
    "javaw",
    "awk",
    "gawk",
    "mawk",
];

/// 只读命令：不写文件、不改系统状态、不执行任意代码。
///
/// **这是唯一能判 `Low` 的集合**。不在集合里的命令一律按 [`CapabilityRiskLevel::High`]
/// 处理（见 `segment_risk_level` 的 fail-closed 注释）——「没见过 = 安全」正是
/// 审批闸门被绕过的原因：`del` / `rd` / `taskkill` 在 Windows 上都是系统自带命令，
/// 却因为不在旧的 Unix 风险表里被判成 `Low`，于是自动模式直接执行。
const READ_ONLY_COMMANDS: &[&str] = &[
    "ls",
    "dir",
    "pwd",
    "cd",
    "echo",
    "cat",
    "type",
    "more",
    "less",
    "head",
    "tail",
    "wc",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "fd",
    "find",
    "where",
    "which",
    "whoami",
    "hostname",
    "date",
    "printenv",
    "set",
    "ver",
    "uname",
    "systeminfo",
    "tree",
    "stat",
    "file",
    "du",
    "df",
    "ps",
    "tasklist",
    "netstat",
    "ipconfig",
    "ifconfig",
    "ping",
    "nslookup",
    "dig",
    "tracert",
    "traceroute",
    "diff",
    "cmp",
    "sort",
    "uniq",
    "cut",
    "tr",
    "nl",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "true",
    "false",
    "test",
];

/// 系统性破坏 / 不可逆命令（内建命令与系统管理入口）。
///
/// 与 `CODE_RUNNERS` 分开：这些命令的**首个词本身就是动作**，不需要靠参数里的脚本
/// 才能造成后果。跨平台收录——风险表按命令名而不是按 host OS 分支，Windows 命令在
/// Unix 上不存在时多判一档无害，漏判则是一条真实的绕过路径。
const DESTRUCTIVE_COMMANDS: &[&str] = &[
    // 删除 / 覆盖
    "rm",
    "rmdir",
    "rd",
    "del",
    "erase",
    "deltree",
    "unlink",
    "shred",
    "truncate",
    "wipe",
    "dd",
    // 权限 / 属主
    "chmod",
    "chown",
    "chgrp",
    "setfacl",
    "icacls",
    "takeown",
    "cacls",
    // 磁盘 / 分区 / 文件系统
    "mkfs",
    "mke2fs",
    "fdisk",
    "parted",
    "diskpart",
    "format",
    "mount",
    "umount",
    "cipher",
    "vssadmin",
    "bcdedit",
    // 进程 / 电源
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "logoff",
    "kill",
    "killall",
    "pkill",
    "taskkill",
    "tskill",
    "systemctl",
    "service",
    "launchctl",
    // 账户 / 注册表 / 计划任务 / 网络配置
    "reg",
    "regedit",
    "regedt32",
    "sc",
    "net",
    "netsh",
    "schtasks",
    "at",
    "wmic",
    "runas",
    "sudo",
    "su",
    "doas",
    // 外发 / 远程执行（数据外带与远程通道）
    "curl",
    "wget",
    "ftp",
    "scp",
    "ssh",
    "nc",
    "ncat",
    "socat",
    "docker",
    "podman",
    "kubectl",
    "helm",
    // PowerShell 动词（包装器之外的直达形态）
    "remove-item",
    "ri",
    "clear-content",
    "clc",
    "set-executionpolicy",
    "invoke-expression",
    "iex",
    "start-process",
    "stop-process",
    "stop-computer",
    "restart-computer",
    "format-volume",
    "clear-disk",
    "remove-partition",
];

/// 有副作用但可回滚的命令：写入 / 移动 / 建立链接。
const WRITE_COMMANDS: &[&str] = &[
    "touch", "mkdir", "md", "mv", "move", "cp", "copy", "xcopy", "robocopy", "ren", "rename", "ln",
    "link", "mklink", "tee", "sed", "rsync", "install",
];

/// 把命令行按**引号外的**命令分隔符切成子命令；遇不可枚举的构造则报错。
///
/// ## 为什么必须逐段检查
///
/// 白名单若只比整条命令的首词，`git status; rm -rf D:\` 的首词是 `git`
/// （在白名单、判 Low 风险），**第二个命令完全不可见**——而危害最大的恰恰
/// 是这种「尾随命令」。切成子命令后每一段都要过白名单、都要算风险。
///
/// ## 分隔符与引号
///
/// - 分隔符：`;` `&` `|` 与换行（`&&` / `||` 会被折叠成一次切分）
/// - 引号内的一切按字面量处理：`grep 'a|b'` 里的 `|` **不是**管道，
///   不切分才不会把命令切碎导致误伤合法命令
///
/// ## 被拒绝的构造
///
/// 命令替换 `` ` `` / `$(` 会执行**任意**子命令，其内容无法用白名单枚举，
/// 因此直接拒绝（双引号内同样生效——`sh` 在双引号里照样执行替换）。
/// 变量展开（`$VAR` / `%VAR%`）只替换文本、不执行命令，故不在此列。
fn split_subcommands(command: &str) -> Result<Vec<String>, String> {
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '`' || (c == '$' && chars.peek() == Some(&'(')) {
                    return Err(format!("不允许命令替换：{command}"));
                }
                current.push(c);
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    current.push(c);
                }
                '`' => return Err(format!("不允许命令替换：{command}")),
                '$' if chars.peek() == Some(&'(') => {
                    return Err(format!("不允许命令替换：{command}"))
                }
                // fd 重定向（`2>&1` / `1>&2`）：`&` 是重定向语法的一部分，不是
                // 分隔符——当分隔符会把后面的 `1` 切成独立「命令」，整条命令被
                // 误拒（实测会话 `09d74431` 有 43 次这样的假阳性拒绝）。
                '&' if current.ends_with('>') => current.push(c),
                ';' | '&' | '|' | '\n' | '\r' => {
                    let seg = current.trim();
                    if !seg.is_empty() {
                        segments.push(seg.to_string());
                    }
                    current.clear();
                }
                _ => current.push(c),
            },
        }
    }

    if quote.is_some() {
        return Err(format!("引号未闭合：{command}"));
    }
    let seg = current.trim();
    if !seg.is_empty() {
        segments.push(seg.to_string());
    }
    if segments.is_empty() {
        return Err("空命令".to_string());
    }
    Ok(segments)
}

impl SecurityPolicy {
    pub async fn is_path_allowed_for_read<P: AsRef<Path>>(
        &self,
        path: P,
        workspace_dir: &Path,
    ) -> bool {
        let r = self.rules();
        let path = path.as_ref();
        let path_str = path.to_string_lossy();
        if !is_safe_relative_path(&path_str) {
            return false;
        }
        for forbidden in &r.forbidden_paths {
            let expanded = shellexpand::tilde(forbidden);
            if symbio_plugin_api::vdfs_path_within(&path_str, expanded.as_ref()) {
                return false;
            }
        }
        if !r.workspace_only {
            return true;
        }
        if !path.is_absolute() {
            return true;
        }
        path_starts_with_normalized(path, workspace_dir)
            || r.allowed_roots
                .iter()
                .any(|root| path_starts_with_normalized(path, root))
    }

    /// 白名单判定：**每一个子命令**的首词都必须命中允许列表；**空白名单 = 不限制**。
    ///
    /// `_threshold` 刻意不参与判定——白名单管「能跑什么」，风险阈值管「跑之前
    /// 要不要审批」，两者正交。早先这里有 `threshold == High ⇒ 放行一切` 的
    /// 旁路，它让白名单在高危模式下**整体失效**（连命令替换都被放过）。
    pub fn is_command_allowed(&self, command: &str, _threshold: CapabilityRiskLevel) -> bool {
        if self.rules().autonomy == AutonomyLevel::ReadOnly {
            return false;
        }
        match split_subcommands(command) {
            // 结构非法（命令替换 / 引号未闭合）⇒ 拒；合法则逐段过白名单
            Ok(segments) => segments.iter().all(|seg| self.segment_is_allowed(seg)),
            Err(_) => false,
        }
    }

    /// 单个子命令是否命中白名单（比归一化后的首词）；**空白名单 = 不限制**
    fn segment_is_allowed(&self, segment: &str) -> bool {
        let r = self.rules();
        if r.allowed_commands.is_empty() {
            return true;
        }
        let base_cmd = segment.split_whitespace().next().unwrap_or("");
        let cmd_name = normalize_base_command(base_cmd);
        r.allowed_commands
            .iter()
            .any(|allowed| allowed == cmd_name || allowed == base_cmd)
    }

    pub fn is_rate_limited(&self) -> bool {
        let max = self.rules().max_actions_per_hour;
        self.tracker.is_at_limit(max)
    }

    pub fn record_action(&self) {
        self.tracker.record();
    }

    /// 整条命令的风险等级 = **各子命令风险的最大值**。
    ///
    /// 只看首词会让 `git status; rm -rf D:\` 判成 Low（首词是 git），
    /// 尾随的破坏性命令因此绕开审批门槛。
    pub fn command_risk_level(&self, command: &str) -> CapabilityRiskLevel {
        match split_subcommands(command) {
            Ok(segments) => segments
                .iter()
                .map(|s| self.segment_risk_level(s))
                .max()
                .unwrap_or(CapabilityRiskLevel::High),
            // 结构非法：fail-closed，按最高风险处理，交给白名单 / 审批拦
            Err(_) => CapabilityRiskLevel::High,
        }
    }

    /// 单个子命令的风险等级（比首词 + 危险模式扫描）。
    ///
    /// **fail-closed**：只有 [`READ_ONLY_COMMANDS`] 与已知只读的 `git` 子命令判
    /// `Low`；解释器判 `High`；破坏性命令判 `High`；写入判 `Medium`；**其余一律
    /// `High`**。最后一条是安全边界——「未知 = Low」会让任何未收录的系统命令
    /// （`del` / `rd` / `format` / `taskkill`）在自动模式下免审批执行，而审批闸门
    /// 正是本项目声明的安全边界（[D-101](../../../../docs/003-design/008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系")）。
    fn segment_risk_level(&self, segment: &str) -> CapabilityRiskLevel {
        let command_lower = segment.to_lowercase();
        let base_cmd = command_lower.split_whitespace().next().unwrap_or("");
        let base_cmd = normalize_base_command(base_cmd);

        // 危险模式优先：即使首词是只读命令，参数里也可能藏着执行 / 删除。
        let high_risk_patterns = [
            "rm -rf /",
            "rm -fr /",
            "mkfs",
            "dd if=",
            "sudo",
            "git reset --hard",
            "git clean -",
            "find -delete",
            "-exec ",
            "-execdir ",
            "remove-item",
            "invoke-expression",
            "set-executionpolicy",
            "| sh",
            "| bash",
            "| powershell",
            "| cmd",
        ];
        if high_risk_patterns
            .iter()
            .any(|pattern| command_lower.contains(pattern))
        {
            return CapabilityRiskLevel::High;
        }

        if CODE_RUNNERS.contains(&base_cmd) || DESTRUCTIVE_COMMANDS.contains(&base_cmd) {
            return CapabilityRiskLevel::High;
        }
        if WRITE_COMMANDS.contains(&base_cmd) {
            return CapabilityRiskLevel::Medium;
        }
        if base_cmd == "git" {
            // `git -c alias.x='!sh'` 能把任意命令挂到别名上；`-C` 只切目录，
            // 两者都以 `-` 开头，必须分开判。
            // 大小写敏感：`git -c` 是配置注入，`git -C` 只是切目录。
            let second_raw = segment.split_whitespace().nth(1).unwrap_or("");
            if second_raw == "-c" {
                return CapabilityRiskLevel::High;
            }
            let second = command_lower.split_whitespace().nth(1).unwrap_or("");
            // `git reset --hard` / `git clean -` 已由上面的模式拦成 High；
            // 其余写操作按「可回滚的写入」处理，只读子命令仍是 Low。
            let git_ops = [
                "commit",
                "push",
                "merge",
                "rebase",
                "cherry-pick",
                "revert",
                "checkout",
                "restore",
                "switch",
                "stash",
                "apply",
                "am",
                "reset",
                "clean",
            ];
            if git_ops.contains(&second) {
                return CapabilityRiskLevel::Medium;
            }
            return CapabilityRiskLevel::Low;
        }
        if READ_ONLY_COMMANDS.contains(&base_cmd) {
            return CapabilityRiskLevel::Low;
        }
        // 没见过 = 不知道会做什么 = 最高档，交给审批闸门。
        CapabilityRiskLevel::High
    }

    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
        threshold: CapabilityRiskLevel,
    ) -> Result<CapabilityRiskLevel, String> {
        if !self.is_command_allowed(command, threshold) {
            return Err(format!("命令不在允许列表中: {command}"));
        }
        let risk = self.command_risk_level(command);
        // 阈值 High = 用户已确认承担高风险 ⇒ 自动批准，不再逐级要审批。
        // 注意：白名单与结构检查在上面已经执行过，**不因此旁路**。
        if threshold == CapabilityRiskLevel::High {
            return Ok(risk);
        }
        let r = self.rules();
        match risk {
            CapabilityRiskLevel::High => {
                if r.block_high_risk_commands {
                    return Err("高风险命令被策略阻止".into());
                }
                if r.autonomy == AutonomyLevel::Supervised && !approved {
                    return Err("高风险命令需要显式批准".into());
                }
            }
            CapabilityRiskLevel::Medium => {
                if r.autonomy == AutonomyLevel::Supervised
                    && r.require_approval_for_medium_risk
                    && !approved
                {
                    return Err("中等风险命令需要批准".into());
                }
            }
            CapabilityRiskLevel::Low => {}
        }
        Ok(risk)
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
