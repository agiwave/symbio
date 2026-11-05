//! 债表契约：端口插件自报债行；宿主收集，环只读。
//!
//! 这里的类型是**数据**：端口名与债身份都由申报方给出，契约本身不认识任何端口。
//! 住在契约层（与 [`crate::drive`] 同一族）：扩展层插件只依赖 `symbio_plugin_api`，
//! 不自报债的插件不需要看见这些类型。

use std::sync::Arc;

use serde_json::Value;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow};

pub use crate::drive::Unless;

/// 债行所在的 `multi` 族。加一个端口 = 由该插件往本族报到，不改族。
pub const DEBT_FAMILY: CapabilityFamily = CapabilityFamily::multi("debt");

/// 一次驱动的应用侧输入：会话地址、选定的 provider 与会话主体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveRequest {
    /// 会话地址里的 id（应用数据）。
    pub session_id: String,
    /// 选定的 provider 标识（可为空）。
    pub provider: Option<String>,
    /// 本会话主体（开面时从 `agent_id` 经 `principal_of` 得到；D-339）。
    ///
    /// 债表用它给请求载荷定归属，避免写死 `"user"` 让两套主体并存。
    pub principal: String,
}

/// 一笔债在五事实里的结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    /// 应答落账。
    Ok,
    /// 判死落账。
    Failed,
}

/// 五事实的只读视图（按债 id 归并；同一会话内 id 唯一，跨端口也不得重名）。
///
/// 另带**全种类事实的身份**（种类 + id，不含载荷）：欠债方要从「哪条事实」派生
/// 自己的债 id，而不是自己计数（禁计数）。这一格是中性形状——种类是申报方给的
/// 字符串，契约不解释它属于哪个世界。
#[derive(Debug, Clone, Default)]
pub struct DebtFacts {
    closings: Vec<(String, Closing, Value)>,
    attempts: Vec<(String, u32)>,
    identities: Vec<(String, String)>,
    /// 事实身份 → 载荷（只供**构造请求**用；判定仍只看种类与身份，D-338）。
    payloads: Vec<(String, String, Value)>,
}

impl DebtFacts {
    /// 记一条事实的身份（种类 + id；载荷不进来）。
    pub fn note_fact(&mut self, kind: &str, id: &str) {
        if self
            .identities
            .iter()
            .any(|(known, key)| known == kind && key == id)
        {
            return;
        }
        self.identities.push((kind.to_string(), id.to_string()));
    }

    /// 记一条事实的载荷（只读副本；同一身份只留第一份）。
    ///
    /// 申报方要拿「在等」事实里的提示正文构造请求载荷（D-338）；它不是判定位。
    pub fn note_fact_payload(&mut self, kind: &str, id: &str, payload: Value) {
        if self
            .payloads
            .iter()
            .any(|(known, key, _)| known == kind && key == id)
        {
            return;
        }
        self.payloads
            .push((kind.to_string(), id.to_string(), payload));
    }

    /// 某条事实的载荷；没有或未记 ⇒ `None`。
    pub fn fact_payload(&self, kind: &str, id: &str) -> Option<&Value> {
        self.payloads
            .iter()
            .find(|(known, key, _)| known == kind && key == id)
            .map(|(_, _, payload)| payload)
    }

    /// 某**种类**的全部事实身份与载荷（按记入顺序）。
    ///
    /// 机制事实只有一个种类名、行名在载荷里（D-338），申报方要按载荷挑出自己那行，
    /// 只能按种类取全集再筛；判定仍是种类 + 身份，不因载荷分叉出新的债。
    pub fn facts_of(&self, kind: &str) -> Vec<(&str, &Value)> {
        self.payloads
            .iter()
            .filter(|(known, _, _)| known == kind)
            .map(|(_, id, payload)| (id.as_str(), payload))
            .collect()
    }

    /// 该种类**最后记入**的事实 id；没有 ⇒ `None`。
    ///
    /// 记入顺序由宿主按事实源顺序给出，同 id 去重——「最新」是位置，不是计数。
    pub fn latest_fact_id(&self, kind: &str) -> Option<&str> {
        self.identities
            .iter()
            .rev()
            .find(|(known, _)| known == kind)
            .map(|(_, id)| id.as_str())
    }

    /// 记一笔尝试号（取最大值）。
    pub fn note_attempt(&mut self, id: &str, attempt: u32) {
        match self.attempts.iter_mut().find(|(key, _)| key == id) {
            Some((_, current)) if *current >= attempt => {}
            Some((_, current)) => *current = attempt,
            None => self.attempts.push((id.to_string(), attempt)),
        }
    }

    /// 记一笔结局。
    pub fn note_closing(&mut self, id: &str, closing: Closing, payload: Value) {
        if let Some(slot) = self.closings.iter_mut().find(|(key, _, _)| key == id) {
            *slot = (id.to_string(), closing, payload);
        } else {
            self.closings.push((id.to_string(), closing, payload));
        }
    }

    /// 某笔债的结局与载荷。
    pub fn closing(&self, id: &str) -> Option<(Closing, &Value)> {
        self.closings
            .iter()
            .find(|(key, _, _)| key == id)
            .map(|(_, closing, payload)| (*closing, payload))
    }

    /// 某笔债的最大尝试号；从未出现 ⇒ 0。
    pub fn attempt(&self, id: &str) -> u32 {
        self.attempts
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, attempt)| *attempt)
            .unwrap_or(0)
    }
}

/// `session.config` 条目的**幂等键**：由本轮 `user.message` 的事实 id 派生。
///
/// 写方（会话开轮单元）用它做本轮的 config 键，读方（模型债行的 `when_id`）用同一个
/// 键声明「我依赖的是**这一轮**的输入」。两边必须由这里派生，否则对不上——只按种类
/// 判会把上一轮的 config 当成这一轮的输入。
pub fn session_config_id(user_fact_id: &str) -> String {
    format!("session-config:{user_fact_id}")
}

/// `session.recall` 条目的**幂等键**：同样由本轮 `user.message` 的事实 id 派生。
///
/// 一回合一条读数：同一条用户消息重放（续跑 / 心跳）不会落出第二格。
pub fn session_recall_id(user_fact_id: &str) -> String {
    format!("session-recall:{user_fact_id}")
}

/// 一行债：申报方给出的数据，宿主原样转给环。
#[derive(Debug, Clone)]
pub struct DebtSpec {
    /// 行名（申报方的标签）。
    pub name: &'static str,
    /// 欠哪个端口——申报方自己的端口名，契约不解释。
    pub port: &'static str,
    /// 债身份，由申报方从事实派生（禁计数）。
    pub id: String,
    /// 交给执行体的载荷。
    pub payload: Value,
    /// 前置债 id：它收束之前本行不可还（跨端口时只写 id，不写端口名）。
    pub after: Option<String>,
    /// 触发条件：该**种类**的事实在本可见域存在才欠；宿主判定，契约不解释。
    pub when: Option<String>,
    /// 触发条件（种类 + 身份）：该种类、该身份的事实存在才欠；与 `when` 同一条读面。
    ///
    /// 用途是**依赖**而不是时序：派出这一行的执行体要读某条输入事实（如模型读
    /// `session.config` 的工具表），行就必须把那条输入的身份写进 `when` —— 只按种类
    /// 判「有没有过」会把**上一轮**的旧事实当成这一轮的输入。
    pub when_id: Option<(String, String)>,
    /// 排除条件：该事实已存在 ⇒ 本行不欠（与 `when` 同一条读面，按种类读）。
    pub unless: Option<Unless>,
    /// 该端口的债全部还清才欠（读债状态）；环只读端口名，不解释语义。
    pub when_no_open: Option<&'static str>,
    /// 单次派发租约（毫秒）。
    pub lease_ms: u64,
    /// 最多尝试次数（含首次）。
    pub max_attempts: u32,
}

impl DebtSpec {
    /// 无前置的一行债；租约 60s、最多 3 次。
    pub fn new(
        name: &'static str,
        port: &'static str,
        id: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            name,
            port,
            id: id.into(),
            payload,
            after: None,
            when: None,
            when_id: None,
            unless: None,
            when_no_open: None,
            lease_ms: 60_000,
            max_attempts: 3,
        }
    }

    /// 声明前置债 id。
    pub fn after(mut self, id: impl Into<String>) -> Self {
        self.after = Some(id.into());
        self
    }

    /// 声明触发条件：该种类的事实存在才欠（字符串由宿主判定）。
    pub fn when(mut self, kind: impl Into<String>) -> Self {
        self.when = Some(kind.into());
        self
    }

    /// 声明触发条件（种类 + 身份）：该条输入事实存在才欠。
    pub fn when_id(mut self, kind: impl Into<String>, id: impl Into<String>) -> Self {
        self.when_id = Some((kind.into(), id.into()));
        self
    }

    /// 声明种类级排除条件：该种类的事实在本可见域存在 ⇒ 本行不欠。
    pub fn unless(mut self, kind: impl Into<String>) -> Self {
        self.unless = Some(Unless::Kind(kind.into()));
        self
    }

    /// 声明身份级排除条件：该种类、该身份的事实存在 ⇒ 本行不欠。
    pub fn unless_id(mut self, kind: impl Into<String>, id: impl Into<String>) -> Self {
        self.unless = Some(Unless::Id {
            kind: kind.into(),
            id: id.into(),
        });
        self
    }

    /// 声明「该端口无未还债才欠」。
    pub fn when_no_open(mut self, port: &'static str) -> Self {
        self.when_no_open = Some(port);
        self
    }

    /// 调整租约与尝试上限。
    pub fn lease(mut self, lease_ms: u64, max_attempts: u32) -> Self {
        self.lease_ms = lease_ms;
        self.max_attempts = max_attempts.max(1);
        self
    }
}

/// 一个插件申报的债表（数据源）。
pub trait DebtSource: Send + Sync {
    /// 当前会话该还的债。
    fn rows(&self, request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec>;
}

/// 注册载荷：把债表挂进 [`DEBT_FAMILY`]。
#[derive(Clone)]
pub struct DebtTable(pub Arc<dyn DebtSource>);

impl std::fmt::Debug for DebtTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DebtTable")
    }
}

/// 造一行 `debt` 族登记（`active = false`，与 port 族同一手法）。
pub fn debt_row(name: &str, source: Arc<dyn DebtSource>) -> CapabilityRow {
    CapabilityRow::new(
        DEBT_FAMILY,
        format!("<debt>{name}"),
        DebtTable(source),
        false,
    )
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
