//! 沙箱引擎：横切接口 [`SandboxFace`] 的实现方（ / DE-058）。
//!
//! ## 归属：策略与配置全在这里，架构层看不到
//!
//! SDK 只定义了接口形状与 ctx 键；**规则表、具名沙箱配置、虚拟环境参数**
//! 全部是本模块的内部数据。其它插件只拿到 `Arc<dyn SandboxFace>` 句柄，
//! 看不到任何配置项——这正是用户「沙箱配置是内部开关，不暴露给其它插件」的落点。
//!
//! ## 沙箱不认识工具
//!
//! - [`SandboxFace::permits`] 只匹配工具提交的**意图事实**（开放 kind + 归一化 value），
//!   不认识工具名；新工具提交新 kind 时，沙箱按用户配的同 kind 规则匹配，零代码改动。
//! - [`SandboxFace::intercept`] 收到的 `tool_name` 是**不透明透传**：本实现不内置任何
//!   工具清单，只在用户显式配了该工具名的拦截规则时才拦；默认放行。
//!
//! ## 统一规则模型（能力约束配置的真相）
//!
//! 用户要的「命令黑白名单 / 目录黑白名单 / 各自风险档」统一成一张有序规则表：
//!
//! ```text
//! 规则 = { 匹配对象(kind), 匹配模式, 值, 判定动作, 风险档 }
//! ```
//!
//! - **首个命中生效**：用户可用顺序表达「先 deny 危险命令，再 allow 常用命令」；
//! - **deny ⇒ 拒绝**（闸门直接拦，工具侧再查一次作纵深防御）；
//! - **allow + 风险档 ⇒ 参与审批**：命中规则的风险档是工具风险的**下限**，
//!   多条事实取最高档，不得调低工具自报风险；风险档留空 = 继承工具自己的判定；
//! - **某 kind 有规则但全未命中 ⇒ 拒绝**（fail-closed，沿用旧白名单语义）；
//!   要表达「只禁这几条、其余放行」，显式加一条 `glob / * / allow` 兜底规则；
//! - **该 kind 完全没有规则 ⇒ 放行且不覆盖风险**（缺席即不设限，旧档案平滑升级）。
//!
//! 旧档案的 `allowed_facts`（kind → 白名单值）在载入时**迁移**成 exact/allow 规则，
//! 不需要用户手工改文件。
//!
//! ## 配置真相：本插件的 VDFS 存储
//!
//! 每个具名沙箱是一个 `sandbox.json` 档案（见 [`SandboxProfileConfig`]），由
//! [`crate::plugin`] 的 VDFS provider 管理；本模块从档案列表重建 [`SandboxRegistry`]。
//! `approval_mode` / `risk_threshold` 是沙箱内部开关；其它插件只经 `ApprovalPolicy`
//! 拿到本次判断，不读取档案本身。
//!
//! ## 缺席与默认：未选择用 default + 诚实边界
//!
//! 本仓当前没有真容器、进程或网络隔离。新策略路径的 `selected=None` 解析为内置
//! `default`；显式 `none` 才不限制。显式档案缺失、损坏或注册表无快照时拒绝。
//! `SandboxFace::decide(None, ..)` 的放行仅保留给旧兼容调用，不是新执行闸门的缺席语义。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use symbio_plugin_api::collectors::{ApprovalScopes, PolicyBinding};
use symbio_plugin_api::{vdfs_normalize_path, SandboxDecision, SandboxFace, SandboxFact};

/// 匹配模式：开放给所有事实 kind 复用的**通用字符串谓词**。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuleMatch {
    /// 全等
    #[default]
    Exact,
    /// 前缀（适合目录子树 `/home/me/docs`）
    Prefix,
    /// 包含（适合命令片段 `rm -rf`）
    Contains,
    /// 通配符（`*` / `?`，适合路径模式 `/home/me/docs/**`）
    Glob,
}

impl RuleMatch {
    fn hits(self, pattern: &str, value: &str) -> bool {
        match self {
            Self::Exact => pattern == value,
            Self::Prefix => value.starts_with(pattern),
            Self::Contains => value.contains(pattern),
            Self::Glob => glob_match(pattern, value),
        }
    }
}

fn normalized_path_value(value: &str) -> Option<String> {
    let normalized = vdfs_normalize_path(value);
    let drive_relative = normalized.as_bytes().get(1) == Some(&b':')
        && normalized
            .as_bytes()
            .get(2)
            .is_none_or(|byte| *byte != b'/');
    if drive_relative {
        return None;
    }
    let drive = normalized
        .as_bytes()
        .get(1)
        .is_some_and(|byte| *byte == b':');
    let root = if normalized.starts_with('/') || drive {
        "/"
    } else {
        ""
    };
    let body = if drive {
        normalized
            .get(2..)
            .unwrap_or_default()
            .trim_start_matches('/')
    } else {
        normalized.trim_start_matches('/')
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in body.split('/').filter(|segment| !segment.is_empty()) {
        if segment == "." {
            continue;
        }
        if segment == ".." {
            let parent = segments.pop()?;
            if parent == ".." {
                return None;
            }
        } else {
            segments.push(segment);
        }
    }
    let body = segments.join("/");
    let anchor = if drive {
        normalized.get(..2).unwrap_or_default()
    } else {
        root
    };
    let value = format!("{anchor}/{body}").trim_end_matches('/').to_string();
    #[cfg(windows)]
    let value = value.to_ascii_lowercase();
    Some(if value.is_empty() {
        "/".to_string()
    } else {
        value
    })
}

/// 路径前缀按目录段比较：`/foo` 命中 `/foo` / `/foo/bar`，但不命中 `/foobar`。
/// 命令前缀仍走原始 `starts_with`，避免把 shell 文本误当路径。
fn path_prefix_hits(pattern: &str, value: &str) -> bool {
    if !value.starts_with(pattern) {
        return false;
    }
    if value.len() == pattern.len() || pattern.ends_with(['/', '\\']) {
        return true;
    }
    matches!(value.as_bytes().get(pattern.len()), Some(b'/' | b'\\'))
}

/// 通配符匹配（`*` 任意长、`?` 单字符）。手写 DP，不引入依赖。
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut dp = vec![false; t.len() + 1];
    dp[0] = true;
    for pc in &p {
        let mut next = vec![false; t.len() + 1];
        if *pc == '*' {
            next[0] = dp[0];
            for j in 1..=t.len() {
                next[j] = dp[j] || next[j - 1];
            }
        } else {
            for j in 1..=t.len() {
                if *pc == '?' || *pc == t[j - 1] {
                    next[j] = dp[j - 1];
                }
            }
        }
        dp = next;
    }
    dp[t.len()]
}

/// 判定动作。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuleAction {
    #[default]
    Allow,
    Deny,
}

/// 命中规则的风险档。`Inherit` = 不调低工具自报风险（表单留空即此值）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuleRisk {
    #[default]
    Inherit,
    Low,
    Medium,
    High,
}

impl RuleRisk {
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Inherit => None,
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
        }
    }
}

/// 一条能力约束规则（沙箱插件私有数据，前端按通用表格声明列来编辑）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct SandboxRule {
    /// 匹配对象 = 事实 kind（开放字符串；`shell_command` / `vdfs_path` / `url`…）。
    pub(crate) kind: String,
    /// 匹配模式。
    #[serde(rename = "match", alias = "mode")]
    pub(crate) mode: RuleMatch,
    /// 匹配值（命令片段 / 路径模式）。
    pub(crate) value: String,
    /// 允许 / 拒绝。
    pub(crate) action: RuleAction,
    /// 风险档（留空 = 继承工具自身判定）。
    pub(crate) risk: RuleRisk,
}

/// 一份沙箱档案的可序列化形状（本插件 VDFS 存储的一个 `sandbox.json`）。
///
/// 这是**沙箱插件的内部数据**：具名沙箱、审批开关、规则表、被拦工具名。
/// 缺字段的旧档案仍可读（审批参数留空即由消费方回落默认），但**任何档案在发布前
/// 都必须通过严格语义校验**；损坏档案不能被投影成无限制放行档。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct SandboxProfileConfig {
    /// 展示名（VDFS 节点标题取它）。
    pub(crate) name: String,
    /// 描述。
    pub(crate) description: Option<String>,
    /// 内置档只读（用户不能删除/改名，只能复制后改）。
    pub(crate) builtin: bool,
    /// 审批模式：`auto`（无人值守，需审批时返回友好错误）/ `interactive`（渲染审批卡）。
    pub(crate) approval_mode: Option<String>,
    /// 风险阈值：`low` / `medium` / `high`。
    pub(crate) risk_threshold: Option<String>,
    /// 是否允许把本会话阈值作为自动放行偏好；缺省 `true`。
    pub(crate) allow_session_approval: Option<bool>,
    /// 是否允许保存跨会话永久授权；缺省 `true`。
    pub(crate) allow_permanent_approval: Option<bool>,
    /// 统一规则表（首个命中生效）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) rules: Vec<SandboxRule>,
    /// 旧档兼容：`kind → 允许值清单`，载入时迁移成 exact/allow 规则。
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub(crate) allowed_facts: HashMap<String, Vec<String>>,
    /// 工具名 → 拦截载荷（不透明 JSON）；只对显式配置的工具名生效。
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub(crate) blocked_tools: HashMap<String, Value>,
}

/// 档案语义校验错误；`field` 直接作为 VDFS 字段路径返回给管理面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SandboxValidationError {
    pub(crate) field: String,
    pub(crate) message: String,
}

impl SandboxProfileConfig {
    pub(crate) fn approval_scopes(&self) -> ApprovalScopes {
        let mut scopes = ApprovalScopes::ONCE;
        if self.allow_session_approval.unwrap_or(true) {
            scopes = scopes.with(ApprovalScopes::SESSION);
        }
        if self.allow_permanent_approval.unwrap_or(true) {
            scopes = scopes.with(ApprovalScopes::PERMANENT);
        }
        scopes
    }

    pub(crate) fn risk_ceiling(&self) -> &str {
        self.risk_threshold.as_deref().unwrap_or("medium")
    }

    /// 旧档规则迁移 + 新规则，保持稳定顺序并附带可定位的字段路径。
    fn rules_with_fields(&self) -> Vec<(SandboxRule, String)> {
        let mut rules: Vec<_> = self
            .rules
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, rule)| (rule, format!("rules.{i}")))
            .collect();
        let mut legacy: Vec<_> = self.allowed_facts.iter().collect();
        legacy.sort_by(|a, b| a.0.cmp(b.0));
        for (kind, values) in legacy {
            for (i, value) in values.iter().enumerate() {
                rules.push((
                    SandboxRule {
                        kind: kind.clone(),
                        mode: RuleMatch::Exact,
                        value: value.clone(),
                        action: RuleAction::Allow,
                        risk: RuleRisk::Inherit,
                    },
                    format!("allowed_facts.{kind}.{i}"),
                ));
            }
        }
        rules
    }

    fn migrated_rules(&self) -> Vec<SandboxRule> {
        self.rules_with_fields()
            .into_iter()
            .map(|(rule, _)| rule)
            .collect()
    }

    /// 发布前的严格语义校验。空规则数组合法，表示不限制任何事实种类。
    pub(crate) fn validate(&self) -> Vec<SandboxValidationError> {
        let mut errors = Vec::new();
        if let Some(mode) = &self.approval_mode {
            if !matches!(mode.as_str(), "auto" | "interactive") {
                errors.push(SandboxValidationError {
                    field: "approval_mode".into(),
                    message: "必须是 auto 或 interactive".into(),
                });
            }
        }
        if let Some(threshold) = &self.risk_threshold {
            if !matches!(threshold.as_str(), "low" | "medium" | "high") {
                errors.push(SandboxValidationError {
                    field: "risk_threshold".into(),
                    message: "必须是 low、medium 或 high".into(),
                });
            }
        }
        for (i, rule) in self.rules.iter().enumerate() {
            if rule.kind.trim().is_empty() {
                errors.push(SandboxValidationError {
                    field: format!("rules.{i}.kind"),
                    message: "规则对象不能为空".into(),
                });
            } else if rule.kind != rule.kind.trim() {
                errors.push(SandboxValidationError {
                    field: format!("rules.{i}.kind"),
                    message: "规则对象不能包含首尾空白".into(),
                });
            }
            if rule.value.trim().is_empty() {
                errors.push(SandboxValidationError {
                    field: format!("rules.{i}.value"),
                    message: "规则值不能为空".into(),
                });
            }
        }
        let mut legacy: Vec<_> = self.allowed_facts.iter().collect();
        legacy.sort_by(|a, b| a.0.cmp(b.0));
        for (kind, values) in legacy {
            if kind.trim().is_empty() {
                errors.push(SandboxValidationError {
                    field: "allowed_facts".into(),
                    message: "旧版规则对象不能为空".into(),
                });
            }
            for (i, value) in values.iter().enumerate() {
                if value.trim().is_empty() {
                    errors.push(SandboxValidationError {
                        field: format!("allowed_facts.{kind}.{i}"),
                        message: "旧版规则值不能为空".into(),
                    });
                }
            }
        }

        // 首个命中生效，因此同一匹配身份（kind/match/value）出现两次时，
        // 后一条永远不可达；动作和风险不同也不能救回它。
        let mut seen = HashSet::new();
        for (rule, field) in self.rules_with_fields() {
            let value = if rule.kind == "vdfs_path" {
                let Some(value) = normalized_path_value(&rule.value) else {
                    errors.push(SandboxValidationError {
                        field: if field.starts_with("rules.") {
                            format!("{field}.value")
                        } else {
                            field
                        },
                        message: "路径不能越出根目录".into(),
                    });
                    continue;
                };
                value
            } else {
                rule.value.clone()
            };
            let identity = (rule.kind, rule.mode, value);
            if !seen.insert(identity) {
                errors.push(SandboxValidationError {
                    field: if field.starts_with("rules.") {
                        format!("{field}.value")
                    } else {
                        field
                    },
                    message: "重复规则身份会使后一条永远不可达".into(),
                });
            }
        }
        errors
    }
}

/// 兼容旧 PLUGIN.yml 的集中配置形状（`{sandboxes, active}`）。
/// 新真相是 VDFS 档案，本结构只作过渡读取与测试夹具。
/// `#[cfg(test)]`：生产路径一律走 VDFS 档案（`try_from_profiles`），本形状只服务测试。
#[cfg(test)]
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SandboxConfig {
    pub(crate) sandboxes: HashMap<String, SandboxProfileConfig>,
    pub(crate) active: Option<String>,
}

/// 一条事实的裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// 命中 deny 规则：拒绝，且不再看下一条规则。
    Deny,
    /// 放行；`Some(档位)` 表示命中规则指定了风险下限（不覆盖工具自报风险）。
    Allow(Option<&'static str>),
}

/// 一个具名沙箱的内部配置（由档案投影而来）。
#[derive(Debug, Clone, Default, PartialEq)]
struct SandboxProfile {
    /// 有序规则表（首个命中生效）。
    rules: Vec<SandboxRule>,
    /// 工具名层的拦截载荷（不透明 JSON），只在用户显式配置时才有条目。
    blocked_tools: HashMap<String, Value>,
    /// 审批开关（承接 mode/risk_level），与本次裁决的其它字段在同一快照内返回。
    approval_mode: Option<String>,
    risk_threshold: Option<String>,
    approval_scopes: ApprovalScopes,
}

impl SandboxProfile {
    fn from_config(cfg: SandboxProfileConfig) -> Self {
        let rules = cfg.migrated_rules();
        let approval_scopes = cfg.approval_scopes();
        Self {
            rules,
            blocked_tools: cfg.blocked_tools,
            approval_mode: cfg.approval_mode,
            risk_threshold: cfg.risk_threshold,
            approval_scopes,
        }
    }

    /// 单条事实的裁决：首个命中生效；该 kind 有规则却全未命中 ⇒ fail-closed 拒绝。
    fn verdict(&self, fact: &SandboxFact) -> Verdict {
        let mut has_kind_rules = false;
        for rule in &self.rules {
            if rule.kind != fact.kind {
                continue;
            }
            has_kind_rules = true;
            let hits = if fact.kind == "vdfs_path" {
                let (Some(pattern), Some(value)) = (
                    normalized_path_value(&rule.value),
                    normalized_path_value(&fact.value),
                ) else {
                    return Verdict::Deny;
                };
                match rule.mode {
                    RuleMatch::Prefix => path_prefix_hits(&pattern, &value),
                    _ => rule.mode.hits(&pattern, &value),
                }
            } else {
                rule.mode.hits(&rule.value, &fact.value)
            };
            if hits {
                return match rule.action {
                    RuleAction::Deny => Verdict::Deny,
                    RuleAction::Allow => Verdict::Allow(rule.risk.as_str()),
                };
            }
        }
        if has_kind_rules {
            Verdict::Deny // 白名单语义：配了就是限制，未列出即不放行
        } else {
            Verdict::Allow(None) // 该 kind 没配规则 = 不设限
        }
    }

    fn has_kind_rules(&self, kind: &str) -> bool {
        self.rules.iter().any(|rule| rule.kind == kind)
    }

    /// 路径规则存在时，shell 无法判定的裸参数不能当作“不是路径”而放行。
    fn unresolved_path_arguments_denied(&self, facts: &[SandboxFact]) -> bool {
        self.has_kind_rules("vdfs_path")
            && !self.has_kind_rules("shell_unresolved_argument")
            && facts
                .iter()
                .any(|fact| fact.kind == "shell_unresolved_argument")
    }

    /// 能力约束判定：工具提交的每条事实都要被放行（全部放行才放行）。
    fn permits(&self, facts: &[SandboxFact]) -> bool {
        !self.unresolved_path_arguments_denied(facts)
            && facts
                .iter()
                .all(|f| !matches!(self.verdict(f), Verdict::Deny))
    }

    /// 所有命中允许规则的风险下限，取最高档；消费方再与工具自身风险取最大值。
    fn risk_of(&self, facts: &[SandboxFact]) -> Option<String> {
        let mut highest: Option<&'static str> = None;
        for fact in facts {
            let Verdict::Allow(Some(risk)) = self.verdict(fact) else {
                continue;
            };
            if highest.is_none_or(|current| risk_rank(risk) > risk_rank(current)) {
                highest = Some(risk);
            }
        }
        highest.map(str::to_string)
    }

    /// 执行隔离判定：只对显式配置的工具名返回拦截载荷；其它一律放行。
    fn intercept(&self, tool_name: &str, _args: &Value) -> Option<Value> {
        self.blocked_tools.get(tool_name).cloned()
    }
}

fn risk_rank(risk: &str) -> u8 {
    match risk {
        "low" => 1,
        "medium" => 2,
        "high" => 3,
        _ => 0,
    }
}

/// 沙箱注册表：档案全集（内部数据）。
///
/// 默认态为空——只有注册表没有快照或显式档案缺失时拒绝。新策略把未选择解析为
/// 内置 `default`；`SandboxFace::decide(None, ..)` 的无档案放行仅用于旧兼容查询。
/// **不持有"当前选中项"**：那是会话级选择，随 `ctx[SANDBOX_ID]` 传来并绑定进
/// [`Engine`]，插件级不能留跨会话共享的可变选中态（否则多会话互相污染）。
#[derive(Debug, Default)]
pub struct SandboxRegistry {
    profiles: HashMap<String, SandboxProfile>,
    /// 原始档案（VDFS 列表/详情读的就是它，避免二次投影丢字段）。
    archives: HashMap<String, SandboxProfileConfig>,
    /// 每次成功的事务性替换递增；旧 SandboxFace 兼容裁决仍返回全局快照号。
    version: u64,
    /// 每个档案独立 revision；只有该档案变化才使一次性审批绑定失效。
    revisions: HashMap<String, u64>,
    /// 最近一次重载失败；执行仍使用上一份良好快照，管理面必须显式提示。
    pub reload_error: Option<String>,
    initialized: bool,
}

impl SandboxRegistry {
    /// 候选档案全集；只有**全部**通过语义校验后才可成为发布候选。
    pub fn try_from_profiles(
        profiles: Vec<(String, SandboxProfileConfig)>,
    ) -> Result<Self, Vec<SandboxValidationError>> {
        let mut errors = Vec::new();
        let mut ids = HashSet::new();
        for (id, cfg) in &profiles {
            if id.trim().is_empty() {
                errors.push(SandboxValidationError {
                    field: "id".into(),
                    message: "沙箱档案标识不能为空".into(),
                });
            }
            if !ids.insert(id.clone()) {
                errors.push(SandboxValidationError {
                    field: "id".into(),
                    message: format!("沙箱档案标识重复：{id}"),
                });
            }
            errors.extend(cfg.validate().into_iter().map(|mut error| {
                error.message = format!("档案 {id}：{}", error.message);
                error
            }));
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        let archives: HashMap<String, SandboxProfileConfig> = profiles.into_iter().collect();
        let compiled = archives
            .iter()
            .map(|(id, cfg)| (id.clone(), SandboxProfile::from_config(cfg.clone())))
            .collect();
        Ok(Self {
            profiles: compiled,
            archives,
            version: 0,
            revisions: HashMap::new(),
            reload_error: None,
            initialized: true,
        })
    }

    /// 兼容旧集中配置（过渡读取 / 测试夹具），同样执行严格校验。
    #[cfg(test)]
    pub fn from_config(config: SandboxConfig) -> Result<Self, Vec<SandboxValidationError>> {
        Self::try_from_profiles(config.sandboxes.into_iter().collect())
    }

    /// 按 id 取档案；缺席由调用方按「显式选中即拒绝」处理。
    fn profile(&self, id: Option<&str>) -> Option<&SandboxProfile> {
        id.and_then(|name| self.profiles.get(name))
    }

    /// 安装一个具名档案（测试夹具用；生产走 `try_from_profiles`）。
    #[cfg(test)]
    fn install(&mut self, name: impl Into<String>, profile: SandboxProfile) {
        self.profiles.insert(name.into(), profile);
        self.initialized = true;
    }

    /// VDFS 层读档案全集（含 name/builtin 等展示元数据）。
    pub fn archives(&self) -> &HashMap<String, SandboxProfileConfig> {
        &self.archives
    }

    pub fn has_snapshot(&self) -> bool {
        self.initialized
    }

    #[cfg(test)]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// 事务性替换：调用方必须先构造并校验 `next`，此方法只做一次锁内提交。
    pub fn replace(&mut self, next: Self) {
        self.reload_error = None;
        self.initialized = true;
        if self.profiles == next.profiles && self.archives == next.archives {
            return;
        }
        for id in next.profiles.keys() {
            let changed = self.profiles.get(id) != next.profiles.get(id);
            let previous = self.revisions.get(id).copied().unwrap_or(0);
            self.revisions.insert(
                id.clone(),
                if changed {
                    previous.saturating_add(1)
                } else {
                    previous
                },
            );
        }
        // 删除档案后保留 revision，防止同名档案重建时复用旧审批绑定。
        self.profiles = next.profiles;
        self.archives = next.archives;
        self.version = self.version.saturating_add(1);
    }

    pub(crate) fn resolved_profile_id(&self, selected: Option<&str>) -> Result<String, String> {
        if !self.initialized {
            return Err("sandbox registry has no successfully loaded snapshot".into());
        }
        let id = selected
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        if !self.profiles.contains_key(id) {
            return Err(format!(
                "selected sandbox profile '{id}' is missing or invalid"
            ));
        }
        Ok(id.to_string())
    }

    pub(crate) fn profile_config(&self, id: &str) -> Option<&SandboxProfileConfig> {
        self.archives.get(id)
    }

    pub(crate) fn policy_binding(&self, id: &str) -> PolicyBinding {
        PolicyBinding::new(id, self.revisions.get(id).copied().unwrap_or(0))
    }

    pub(crate) fn decide_policy(
        &self,
        selected: Option<&str>,
        facts: &[SandboxFact],
        tool_name: &str,
        args: &Value,
    ) -> Result<SandboxDecision, String> {
        let id = self.resolved_profile_id(selected)?;
        Ok(self.decide(Some(&id), facts, tool_name, args))
    }

    /// 在同一注册表快照内完成全部裁决，避免许可、风险、拦截与审批参数跨版本组合。
    fn decide(
        &self,
        selected: Option<&str>,
        facts: &[SandboxFact],
        tool_name: &str,
        args: &Value,
    ) -> SandboxDecision {
        let version = self.version;
        if !self.initialized {
            return SandboxDecision::denied("sandbox registry has no successfully loaded snapshot");
        }
        let Some(profile) = self.profile(selected) else {
            return match selected {
                None => SandboxDecision {
                    permitted: true,
                    profile_version: version,
                    ..SandboxDecision::default()
                },
                Some(id) => SandboxDecision {
                    permitted: false,
                    profile_version: version,
                    reason: Some(format!(
                        "selected sandbox profile '{id}' is missing or invalid"
                    )),
                    ..SandboxDecision::default()
                },
            };
        };

        let facts_permitted = profile.permits(facts);
        let risk = profile.risk_of(facts);
        let intercept = profile.intercept(tool_name, args);
        let permitted = facts_permitted && intercept.is_none();
        let reason = if !facts_permitted {
            facts
                .iter()
                .find(|fact| matches!(profile.verdict(fact), Verdict::Deny))
                .or_else(|| {
                    profile
                        .unresolved_path_arguments_denied(facts)
                        .then(|| {
                            facts
                                .iter()
                                .find(|fact| fact.kind == "shell_unresolved_argument")
                        })
                        .flatten()
                })
                .map(|fact| format!("sandbox denied fact kind '{}'", fact.kind))
        } else if intercept.is_some() {
            Some("sandbox intercepted the tool call".to_string())
        } else {
            None
        };
        SandboxDecision {
            permitted,
            risk,
            intercept,
            approval_mode: profile.approval_mode.clone(),
            risk_threshold: profile.risk_threshold.clone(),
            profile_version: version,
            reason,
        }
    }
}

/// 对外发布的沙箱句柄：**绑定到本会话选中的 `sandbox_id`**（裁决 B / 按 id 解析）。
///
/// 每个会话在收集期按自己的 `ctx[SANDBOX_ID]` 造一份 Engine；句柄里的 `selected`
/// 就是那一份选择。没有插件级共享可变选中态，多会话并行不互相污染。
pub struct Engine {
    registry: Arc<std::sync::RwLock<SandboxRegistry>>,
    selected: Option<String>,
}

impl Engine {
    pub fn new(
        registry: Arc<std::sync::RwLock<SandboxRegistry>>,
        selected: Option<String>,
    ) -> Self {
        Self { registry, selected }
    }
}

impl SandboxFace for Engine {
    fn permits(&self, facts: &[SandboxFact]) -> bool {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        if !guard.initialized {
            return false;
        }
        match self.selected.as_deref() {
            None => true,
            Some(id) => guard.profile(Some(id)).is_some_and(|p| p.permits(facts)),
        }
    }

    fn risk_of(&self, facts: &[SandboxFact]) -> Option<String> {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        guard
            .profile(self.selected.as_deref())
            .and_then(|p| p.risk_of(facts))
    }

    fn intercept(&self, tool_name: &str, args: &Value) -> Option<Value> {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        if !guard.initialized {
            return Some(serde_json::json!({
                "permitted": false,
                "reason": "sandbox registry has no successfully loaded snapshot"
            }));
        }
        match self.selected.as_deref() {
            None => None,
            Some(id) => guard.profile(Some(id)).map_or_else(
                || {
                    Some(serde_json::json!({
                        "permitted": false,
                        "reason": format!("selected sandbox profile '{id}' is missing or invalid")
                    }))
                },
                |p| p.intercept(tool_name, args),
            ),
        }
    }

    fn approval_params(&self) -> (Option<String>, Option<String>) {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        guard
            .profile(self.selected.as_deref())
            .map(|p| (p.approval_mode.clone(), p.risk_threshold.clone()))
            .unwrap_or((None, None))
    }

    fn decide(&self, facts: &[SandboxFact], tool_name: &str, args: &Value) -> SandboxDecision {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        guard.decide(self.selected.as_deref(), facts, tool_name, args)
    }
}

#[cfg(test)]
#[path = "engine.test.rs"]
mod tests;
