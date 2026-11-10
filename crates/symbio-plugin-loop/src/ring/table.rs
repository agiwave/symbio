//! 债表：应用交来的行数据。环只按行里的端口名与身份取债，不解释行名。

use std::sync::Arc;

use crate::ring::facts::{DebtKey, FactView};
use serde_json::Value;
use symbio_kernel::decl::Frozen;
use symbio_kernel::event::Event;
use symbio_plugin_api::debt::{FailureProjection, ReplyRole};
use symbio_plugin_api::drive::Unless;

/// 声明行这一圈的**选债输入**：冻结声明表 + 事实切片。
///
/// 数据源只交这两样，**求解归选债口**（`plan`）：在数据源里问 `owed` 再把结果交上来，
/// 等于在派发器之外先做一次「现在该还哪笔」的判定——那就是第二个选债口（`TC-1b`）。
pub(crate) struct DeclInput {
    pub(crate) frozen: Arc<Frozen>,
    pub(crate) events: Vec<Event>,
}

/// 一笔待还的债（应用侧的数据）。
#[derive(Debug, Clone)]
pub(crate) struct DebtRow {
    /// 行名（应用侧的标签；环不解释，只用于回读）。
    pub(crate) name: &'static str,
    /// 欠哪个端口（数据；运行时字符串，端口名由活插件装配期申报）。
    pub(crate) port: String,
    /// 债身份，由欠债方从产生这笔债的事实派生（禁计数）。
    pub(crate) id: String,
    /// 交给执行体的载荷草稿。
    pub(crate) payload: Value,
    /// 只有这笔先前的债已收束，本行才可还；`None` = 无前置。
    pub(crate) after: Option<DebtKey>,
    /// 触发条件：该**种类**的事实在本可见域存在才欠；`None` = 无条件。
    /// 环只把字符串原样交给宿主的 `exists`，不解释它的语义。
    pub(crate) when: Option<String>,
    /// 触发条件（种类 + 身份）：该条输入事实存在才欠。与 `when` 同一条读面，
    /// 只是多带一位身份——「有没有过这种事实」与「这一轮的输入到了没有」是两件事。
    pub(crate) when_id: Option<(String, String)>,
    /// 该事实已存在 ⇒ 本行不欠（排除条件，与 `when` 同一条读面）。环只透传，不解释。
    pub(crate) unless: Option<Unless>,
    /// 该端口的债全部还清才欠；`None` = 无此前提。环只读端口名，不解释语义。
    pub(crate) when_no_open: Option<&'static str>,
    /// 前提（读**债状态**）：该端口**这一圈**还有人认领（可派发 / 在飞 / 挂起）⇒
    /// 本行不进计划；该端口没人认领 ⇒ 前提已了（**缺席即收束**）。
    ///
    /// 与 `when_no_open` 同族、方向相反：两者都只读债状态；`when_no_open` 是「端口
    /// 干净才**欠**」，本谓词是「端口干净才**可还**」。环只认端口名，不认行名。
    /// 上一轮遗留的未收束债不算认领——债表按当轮事实现算，遗留债不会被重新申报。
    pub(crate) after_port: Option<&'static str>,
    /// 一次派发的租约时长（毫秒）。
    pub(crate) lease_ms: u64,
    /// 最多尝试次数（含首次）。
    pub(crate) max_attempts: u32,
    /// 应答在读者面的角色（环据此盖章）。
    pub(crate) role: ReplyRole,
    /// 应答进用量台账。
    pub(crate) accounts_usage: bool,
    /// 应答进金额台账。
    pub(crate) accounts_spend: bool,
    /// 失败的落格形状。
    pub(crate) failure: FailureProjection,
}

#[cfg(test)]
impl DebtRow {
    /// 一行无前置的债；租约 60s、最多 3 次尝试。
    pub(crate) fn new(
        name: &'static str,
        port: impl Into<String>,
        id: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            name,
            port: port.into(),
            id: id.into(),
            payload,
            after: None,
            when: None,
            when_id: None,
            unless: None,
            when_no_open: None,
            after_port: None,
            lease_ms: 60_000,
            max_attempts: 3,
            role: ReplyRole::Internal,
            accounts_usage: false,
            accounts_spend: false,
            failure: FailureProjection::None,
        }
    }

    /// 声明前置债：它收束之前本行不可还。
    pub(crate) fn after(mut self, key: DebtKey) -> Self {
        self.after = Some(key);
        self
    }

    /// 声明触发条件：该种类的事实存在才欠（字符串由宿主判定）。
    pub(crate) fn when(mut self, kind: impl Into<String>) -> Self {
        self.when = Some(kind.into());
        self
    }

    /// 声明身份级触发条件：该种类、该身份的事实存在才欠（字符串由宿主判定）。
    pub(crate) fn when_id(mut self, kind: impl Into<String>, id: impl Into<String>) -> Self {
        self.when_id = Some((kind.into(), id.into()));
        self
    }

    /// 声明种类级排除条件：该种类的事实存在 ⇒ 本行不欠。
    pub(crate) fn unless(mut self, kind: impl Into<String>) -> Self {
        self.unless = Some(Unless::Kind(kind.into()));
        self
    }

    /// 声明身份级排除条件：该种类、该身份的事实存在 ⇒ 本行不欠。
    pub(crate) fn unless_id(mut self, kind: impl Into<String>, id: impl Into<String>) -> Self {
        self.unless = Some(Unless::Id {
            kind: kind.into(),
            id: id.into(),
        });
        self
    }

    /// 声明「该端口的债全部还清才欠」（读债状态，不读事实状态）。
    pub(crate) fn when_no_open(mut self, port: &'static str) -> Self {
        self.when_no_open = Some(port);
        self
    }

    /// 声明前提「该端口这一圈没人认领才算已了」（读债状态，与 [`Self::when_no_open`]
    /// 对偶）。依赖方只写端口名——不写兄弟插件的行名，也不拼债 id。
    pub(crate) fn after_port(mut self, port: &'static str) -> Self {
        self.after_port = Some(port);
        self
    }

    /// 调整租约与尝试上限。
    pub(crate) fn lease(mut self, lease_ms: u64, max_attempts: u32) -> Self {
        self.lease_ms = lease_ms;
        self.max_attempts = max_attempts.max(1);
        self
    }
}

/// 债表数据源：环每圈重新问一次，应用可据既有事实增减行。
///
/// 触发条件与排除条件都是**行数据的表达力**，不是环的机制：应用读事实投影，
/// 决定这一圈欠哪些债；环只按行选债，不解释世界条件。
pub(crate) trait DebtSource {
    /// 当前可还的债（应用按自己的事实投影决定）。
    fn rows(&self, facts: &FactView<'_>) -> Vec<DebtRow>;

    /// 声明行的选债输入。`None` = 这个宿主没有声明行。
    ///
    /// **只交数据，不交结论**：`owed` 由选债口（`plan`）调用。数据源在这里先求一遍
    /// 再把「欠哪几笔」交上去，就是派发器之外的第二个选债口（`TC-1b`）。
    fn declarations(&self) -> Option<DeclInput> {
        None
    }

    /// 「这种事实存在吗」——由宿主实现（宿主看得见全部事实，
    /// 不止环的五事实）。环只把行里的种类字符串原样透传，不解释它的语义。
    fn exists(&self, kind: &str) -> bool {
        let _ = kind;
        false
    }

    /// 「这种事实在 `since_ms` 之后（严格大于）出现过吗」——与 [`Self::exists`]
    /// 同一条读面，只多带一个**位置**。挂起后的重新点燃只认挂起点之后出现的触发事实。
    fn exists_since(&self, kind: &str, since_ms: u64) -> bool {
        let _ = (kind, since_ms);
        false
    }

    /// 「这种事实、这个身份，存在吗」——与 [`Self::exists`] 同一条读面。
    fn exists_id(&self, kind: &str, id: &str) -> bool {
        let _ = (kind, id);
        false
    }
}

/// 静态债表（测试夹具）。
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub(crate) struct Table {
    rows: Vec<DebtRow>,
}

#[cfg(test)]
impl Table {
    /// 由行数据造表。
    pub(crate) fn new(rows: Vec<DebtRow>) -> Self {
        Self { rows }
    }
}

#[cfg(test)]
impl DebtSource for Table {
    fn rows(&self, _facts: &FactView<'_>) -> Vec<DebtRow> {
        self.rows.clone()
    }
}

#[cfg(test)]
#[path = "table.test.rs"]
mod tests;
