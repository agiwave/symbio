use std::path::{Path, PathBuf};

mod policy_tracker;
mod policy_types;

pub use policy_tracker::ActionTracker;
pub use policy_types::*;

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
/// 规则本体在 [`crate::symbio_core::vdfs::has_parent_segment`]：shell 工具的
/// 路径守卫与 VDFS 物理层守卫必须是**同一条**规则，否则修了一处漏另一处。
/// 这里保留函数名（本模块的公开 API），实现转发过去。
pub fn is_safe_relative_path(path: &str) -> bool {
    !crate::symbio_core::has_parent_segment(path)
}

/// 策略规则（可运行期热更的部分；动作计数独立在 `tracker`）。
///
/// 字段与 local 插件的配置文档（`LocalConfig`）一一对应——配置文档是人类入口，
/// 这里是运行期真源。默认值即「全放开」：这是内置插件自己工具的执行策略，不是
/// 宿主对插件的授权（那个分层见 third-party-plugin-spec）——限制的价值在「用户
/// 想收紧时有的收」，而非默认替用户做主。实测会话 `09d74431`：默认限流 100 次/
/// 小时把 cmd 通道锁死近一小时、325 次调用被拒，全是自伤。
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

/// 命令包装器：真实命令藏在**参数里的脚本**中，白名单看不见内层。
///
/// 它们**保留在白名单**（Windows 下模型确实要靠它跑命令，砍掉能力是过度反应），
/// 但风险固定为 [`RiskLevel::High`]——默认 Medium 阈值下必须用户审批，
/// 用户显式开到 High 阈值即代表自担风险。
const SHELL_WRAPPERS: [&str; 3] = ["powershell", "pwsh", "cmd"];

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
            if crate::symbio_core::path_within(&path_str, expanded.as_ref()) {
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
    pub fn is_command_allowed(&self, command: &str, _threshold: RiskLevel) -> bool {
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
    pub fn command_risk_level(&self, command: &str) -> RiskLevel {
        match split_subcommands(command) {
            Ok(segments) => segments
                .iter()
                .map(|s| self.segment_risk_level(s))
                .max()
                .unwrap_or(RiskLevel::High),
            // 结构非法：fail-closed，按最高风险处理，交给白名单 / 审批拦
            Err(_) => RiskLevel::High,
        }
    }

    /// 单个子命令的风险等级（比首词 + 危险模式扫描）
    fn segment_risk_level(&self, segment: &str) -> RiskLevel {
        let command_lower = segment.to_lowercase();
        let base_cmd = command_lower.split_whitespace().next().unwrap_or("");
        let base_cmd = normalize_base_command(base_cmd);
        // 命令包装器：内层脚本对白名单不可见 ⇒ 一律高风险（默认阈值下需审批）
        if SHELL_WRAPPERS.contains(&base_cmd) {
            return RiskLevel::High;
        }
        let high_risk = [
            "rm", "sudo", "su", "chmod", "chown", "shutdown", "reboot", "mkfs", "dd", "mount",
            "umount", "curl", "wget",
        ];
        if high_risk.contains(&base_cmd) {
            return RiskLevel::High;
        }
        let high_risk_patterns = ["rm -rf /", "rm -fr /", "mkfs", "dd if=", "sudo"];
        for pattern in &high_risk_patterns {
            if command_lower.contains(pattern) {
                return RiskLevel::High;
            }
        }
        let medium_risk = ["touch", "mkdir", "mv", "cp", "ln"];
        if medium_risk.contains(&base_cmd) {
            return RiskLevel::Medium;
        }
        if base_cmd == "git" {
            let git_ops = ["commit", "push", "reset", "clean", "rebase", "merge"];
            let second = command_lower.split_whitespace().nth(1).unwrap_or("");
            if git_ops.contains(&second) {
                return RiskLevel::Medium;
            }
        }
        RiskLevel::Low
    }

    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
        threshold: RiskLevel,
    ) -> Result<RiskLevel, String> {
        if !self.is_command_allowed(command, threshold) {
            return Err(format!("命令不在允许列表中: {command}"));
        }
        let risk = self.command_risk_level(command);
        // 阈值 High = 用户已确认承担高风险 ⇒ 自动批准，不再逐级要审批。
        // 注意：白名单与结构检查在上面已经执行过，**不因此旁路**。
        if threshold == RiskLevel::High {
            return Ok(risk);
        }
        let r = self.rules();
        match risk {
            RiskLevel::High => {
                if r.block_high_risk_commands {
                    return Err("高风险命令被策略阻止".into());
                }
                if r.autonomy == AutonomyLevel::Supervised && !approved {
                    return Err("高风险命令需要显式批准".into());
                }
            }
            RiskLevel::Medium => {
                if r.autonomy == AutonomyLevel::Supervised
                    && r.require_approval_for_medium_risk
                    && !approved
                {
                    return Err("中等风险命令需要批准".into());
                }
            }
            RiskLevel::Low => {}
        }
        Ok(risk)
    }

    /// 检查工具是否需要审批建议（基于「执行风险等级」阈值）
    ///
    /// 返回：(建议是否需要审批, 风险等级)
    ///
    /// 规则与前端「执行风险等级」保持一致：
    /// - 工具风险等级 **>** 执行风险阈值 → 需要用户审批
    /// - 工具风险等级 **≤** 执行风险阈值 → 直接执行（自动批准）
    ///
    /// `threshold` 来自 ctx[RISK_LEVEL]（per-session，由 orchestrator 从
    /// `session.metadata.risk_level` 或 `chat_send.risk_level` 写入，默认 medium）。
    pub fn check_tool_approval_needed(
        &self,
        _tool_name: &str,
        tool_risk_level: RiskLevel,
        threshold: RiskLevel,
    ) -> (bool, RiskLevel) {
        let needs_approval = tool_risk_level > threshold;
        (needs_approval, tool_risk_level)
    }

    pub fn get_tool_risk_level(
        &self,
        tool_name: &str,
        args: Option<&serde_json::Value>,
    ) -> RiskLevel {
        match tool_name {
            // `ask_user` 只是向用户提问，不触碰任何资源；若不显式归为 Low，
            // 它会落到默认 Medium —— 当会话把执行风险阈值设为 low 时，
            // 提问本身反而要先过一次审批，属荒谬路径。
            "read_file" | "web_fetch" | "web_search" | "glob_search" | "content_search"
            | "vdfs_read" | "vdfs_search" | "ask_user" => RiskLevel::Low,
            "shell" => {
                if let Some(cmd) = args.and_then(|a| a.get("command")).and_then(|c| c.as_str()) {
                    self.command_risk_level(cmd)
                } else {
                    RiskLevel::High
                }
            }
            "http_request" => RiskLevel::High,
            "write_file" | "file_edit" | "vdfs_write" | "vdfs_edit" => RiskLevel::Medium,
            _ => RiskLevel::Medium,
        }
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
