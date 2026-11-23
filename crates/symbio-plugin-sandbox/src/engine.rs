//! 沙箱引擎：横切接口 [`SandboxFace`] 的实现方（D-531 / DE-058）。
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
//! - **allow + 风险档 ⇒ 参与审批**：命中规则的风险档覆盖工具自报风险，
//!   与会话阈值比较；风险档留空 = 继承工具自己的判定；
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
//! `approval_mode` / `risk_threshold` 承接原会话级 mode/risk_level 的语义（裁决 B），
//! 但它们是**沙箱内部开关**——其它插件只经通用 ctx 键拿到引擎算出的值，看不到档案本身。
//!
//! ## 缺席与默认：全放行 + 诚实边界
//!
//! 本仓当前没有真容器隔离。默认引擎（无具名配置）一律放行，与
//! 「没有沙箱句柄」的降级语义一致。把默认实现写成一个安静的放行器，比假装做了隔离更安全。

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use symbio_plugin_api::{SandboxFace, SandboxFact};

/// 匹配模式：开放给所有事实 kind 复用的**通用字符串谓词**。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
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

/// 命中规则的风险档。`Inherit` = 不覆盖工具自报风险（表单留空即此值）。
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
/// 全部带 `#[serde(default)]`——缺字段的旧档案也能读，退化成安静放行器。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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

/// 兼容旧 PLUGIN.yml 的集中配置形状（`{sandboxes, active}`）。
/// 新真相是 VDFS 档案，本结构只作过渡读取与测试夹具。
/// `#[cfg(test)]`：生产路径一律走 VDFS 档案（`from_profiles`），本形状只服务测试。
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
    /// 放行；`Some(档位)` 表示命中规则指定了风险档（覆盖工具自报）。
    Allow(Option<String>),
}

/// 一个具名沙箱的内部配置（由档案投影而来）。
#[derive(Debug, Clone, Default)]
struct SandboxProfile {
    /// 有序规则表（首个命中生效）。
    rules: Vec<SandboxRule>,
    /// 工具名层的拦截载荷（不透明 JSON），只在用户显式配置时才有条目。
    blocked_tools: HashMap<String, Value>,
    /// 审批开关（承接 mode/risk_level），只在装配期被读取一次写进 ctx。
    approval_mode: Option<String>,
    risk_threshold: Option<String>,
}

impl SandboxProfile {
    fn from_config(cfg: SandboxProfileConfig) -> Self {
        let mut rules = cfg.rules;
        // 旧档迁移：allowed_facts 的每一项变成 exact/allow 规则，风险档继承工具。
        // 按 kind 排序保证顺序稳定（HashMap 遍历序不保证），新规则始终排在迁移规则之前。
        if !cfg.allowed_facts.is_empty() {
            let mut legacy: Vec<_> = cfg.allowed_facts.into_iter().collect();
            legacy.sort_by(|a, b| a.0.cmp(&b.0));
            for (kind, values) in legacy {
                for value in values {
                    rules.push(SandboxRule {
                        kind: kind.clone(),
                        mode: RuleMatch::Exact,
                        value,
                        action: RuleAction::Allow,
                        risk: RuleRisk::Inherit,
                    });
                }
            }
        }
        Self {
            rules,
            blocked_tools: cfg.blocked_tools,
            approval_mode: cfg.approval_mode,
            risk_threshold: cfg.risk_threshold,
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
            if rule.mode.hits(&rule.value, &fact.value) {
                return match rule.action {
                    RuleAction::Deny => Verdict::Deny,
                    RuleAction::Allow => Verdict::Allow(rule.risk.as_str().map(str::to_string)),
                };
            }
        }
        if has_kind_rules {
            Verdict::Deny // 白名单语义：配了就是限制，未列出即不放行
        } else {
            Verdict::Allow(None) // 该 kind 没配规则 = 不设限
        }
    }

    /// 能力约束判定：工具提交的每条事实都要被放行（全部放行才放行）。
    fn permits(&self, facts: &[SandboxFact]) -> bool {
        facts
            .iter()
            .all(|f| !matches!(self.verdict(f), Verdict::Deny))
    }

    /// 命中允许规则的风险档（若有）。闸门用它覆盖工具自报风险。
    fn risk_of(&self, facts: &[SandboxFact]) -> Option<String> {
        facts.iter().find_map(|f| match self.verdict(f) {
            Verdict::Allow(Some(risk)) => Some(risk),
            _ => None,
        })
    }

    /// 执行隔离判定：只对显式配置的工具名返回拦截载荷；其它一律放行。
    fn intercept(&self, tool_name: &str, _args: &Value) -> Option<Value> {
        self.blocked_tools.get(tool_name).cloned()
    }
}

/// 沙箱注册表：档案全集（内部数据）。
///
/// 默认态为空——没有具名沙箱时，引擎是安静的放行器。
/// **不持有"当前选中项"**：那是会话级选择，随 `ctx[SANDBOX_ID]` 传来并绑定进
/// [`Engine`]，插件级不能留跨会话共享的可变选中态（否则多会话互相污染）。
#[derive(Debug, Default)]
pub struct SandboxRegistry {
    profiles: HashMap<String, SandboxProfile>,
    /// 原始档案（VDFS 列表/详情读的就是它，避免二次投影丢字段）。
    archives: HashMap<String, SandboxProfileConfig>,
}

impl SandboxRegistry {
    /// 从档案列表重建注册表（VDFS 加载后的唯一入口）。
    pub fn from_profiles(profiles: Vec<(String, SandboxProfileConfig)>) -> Self {
        let archives: HashMap<String, SandboxProfileConfig> = profiles.into_iter().collect();
        let compiled = archives
            .iter()
            .map(|(id, cfg)| (id.clone(), SandboxProfile::from_config(cfg.clone())))
            .collect();
        Self {
            profiles: compiled,
            archives,
        }
    }

    /// 兼容旧集中配置（过渡读取 / 测试夹具）。
    #[cfg(test)]
    pub fn from_config(config: SandboxConfig) -> Self {
        Self::from_profiles(config.sandboxes.into_iter().collect())
    }

    /// 按 id 取档案（句柄绑定 id 后用它裁决）；缺席 = 没有该档案 = 全放行。
    fn profile(&self, id: Option<&str>) -> Option<&SandboxProfile> {
        id.and_then(|name| self.profiles.get(name))
    }

    /// 安装一个具名档案（测试夹具用；生产走 `from_profiles`）。
    #[cfg(test)]
    fn install(&mut self, name: impl Into<String>, profile: SandboxProfile) {
        self.profiles.insert(name.into(), profile);
    }

    /// VDFS 层读档案全集（含 name/builtin 等展示元数据）。
    pub fn archives(&self) -> &HashMap<String, SandboxProfileConfig> {
        &self.archives
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
        guard
            .profile(self.selected.as_deref())
            .map(|p| p.permits(facts))
            .unwrap_or(true) // 没选中档案 = 全放行
    }

    fn risk_of(&self, facts: &[SandboxFact]) -> Option<String> {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        guard
            .profile(self.selected.as_deref())
            .and_then(|p| p.risk_of(facts))
    }

    fn intercept(&self, tool_name: &str, args: &Value) -> Option<Value> {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        guard
            .profile(self.selected.as_deref())
            .and_then(|p| p.intercept(tool_name, args))
    }

    fn approval_params(&self) -> (Option<String>, Option<String>) {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        guard
            .profile(self.selected.as_deref())
            .map(|p| (p.approval_mode.clone(), p.risk_threshold.clone()))
            .unwrap_or((None, None))
    }
}

#[cfg(test)]
#[path = "engine.test.rs"]
mod tests;
