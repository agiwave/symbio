//! 沙箱引擎：横切接口 [`SandboxFace`] 的实现方（D-531 / DE-058）。
//!
//! ## 归属：策略与配置全在这里，架构层看不到
//!
//! SDK 只定义了接口形状与 ctx 键；**黑白名单规则、具名沙箱配置、虚拟环境参数**
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

/// 一份沙箱档案的可序列化形状（本插件 VDFS 存储的一个 `sandbox.json`）。
///
/// 这是**沙箱插件的内部数据**：具名沙箱、审批开关、事实白名单、被拦工具名。
/// 全部带 `#[serde(default)]`——缺字段的旧档案也能读，退化成安静放行器。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct SandboxProfileConfig {
    /// 展示名（VDFS 节点标题取它）。
    pub(crate) name: String,
    /// 描述。
    pub(crate) description: Option<String>,
    /// 内置档只读（用户不能删除/改名，只能复制后改）。
    #[serde(default)]
    pub(crate) builtin: bool,
    /// 审批模式：`auto`（无人值守，需审批时返回友好错误）/ `interactive`（渲染审批卡）。
    /// 承接原 `MODE` 会话选项；空 = 沿用默认 `interactive`。
    pub(crate) approval_mode: Option<String>,
    /// 风险阈值：`low` / `medium` / `high`。
    /// 承接原 `RISK_LEVEL` 会话选项；空 = 沿用默认 `medium`。
    pub(crate) risk_threshold: Option<String>,
    /// 事实 kind → 允许值清单；清单为空 = 该 kind 不设限。
    pub(crate) allowed_facts: HashMap<String, Vec<String>>,
    /// 工具名 → 拦截载荷（不透明 JSON）；只对显式配置的工具名生效。
    pub(crate) blocked_tools: HashMap<String, Value>,
}

/// 兼容旧 PLUGIN.yml 的集中配置形状（`{sandboxes, active}`）。
/// 新真相是 VDFS 档案，本结构只作过渡读取与测试夹具。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SandboxConfig {
    pub(crate) sandboxes: HashMap<String, SandboxProfileConfig>,
    pub(crate) active: Option<String>,
}

/// 一条内部规则：对某个事实 kind 的取值集合做允许/拒绝判定。
///
/// 规则是**沙箱插件的私有数据**；`allowed_values` 为空表示不设限，非空则是白名单——
/// 值不在清单里即拒（fail-closed，与 Agent 权限的空清单语义一致）。
#[derive(Debug, Clone, Default)]
struct FactRule {
    allowed_values: Vec<String>,
}

impl FactRule {
    fn permits(&self, value: &str) -> bool {
        self.allowed_values.is_empty() || self.allowed_values.iter().any(|v| v == value)
    }
}

/// 一个具名沙箱的内部配置（由档案投影而来）。
#[derive(Debug, Clone, Default)]
struct SandboxProfile {
    /// 按事实 kind 索引的规则；kind 是开放字符串，新 kind 无需改本模块。
    fact_rules: HashMap<String, FactRule>,
    /// 工具名层的拦截载荷（不透明 JSON），只在用户显式配置时才有条目。
    blocked_tools: HashMap<String, Value>,
    /// 审批开关（承接 mode/risk_level），只在装配期被读取一次写进 ctx。
    approval_mode: Option<String>,
    risk_threshold: Option<String>,
}

impl SandboxProfile {
    fn from_config(cfg: SandboxProfileConfig) -> Self {
        Self {
            fact_rules: cfg
                .allowed_facts
                .into_iter()
                .map(|(kind, allowed_values)| (kind, FactRule { allowed_values }))
                .collect(),
            blocked_tools: cfg.blocked_tools,
            approval_mode: cfg.approval_mode,
            risk_threshold: cfg.risk_threshold,
        }
    }

    /// 能力约束判定：工具提交的每条事实都要被对应规则放行（全部放行才放行）。
    fn permits(&self, facts: &[SandboxFact]) -> bool {
        facts.iter().all(|f| {
            self.fact_rules
                .get(&f.kind)
                .map(|rule| rule.permits(&f.value))
                .unwrap_or(true) // 没有配该 kind 的规则 = 不设限 = 放行
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
    pub fn from_config(config: SandboxConfig) -> Self {
        Self::from_profiles(config.sandboxes.into_iter().collect())
    }

    /// 按 id 取档案（句柄绑定 id 后用它裁决）；缺席 = 没有该档案 = 全放行。
    fn profile(&self, id: Option<&str>) -> Option<&SandboxProfile> {
        id.and_then(|name| self.profiles.get(name))
    }

    /// 安装一个具名档案（测试夹具用；生产走 `from_profiles`）。
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
