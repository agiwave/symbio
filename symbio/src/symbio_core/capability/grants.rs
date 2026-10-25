//! 跨会话的工具授权清单（审批卡第三档「永久放行」的存储形状，D-170）。
//!
//! ## 为什么是「工具 → 风险上限」而不是布尔白名单
//!
//! 同一个工具的风险随参数变化（`shell` 是唯一已知例子）：「上次批的是 `git status`」
//! 不能被读成「以后 `rm -rf` 也免问」。上限因此记的是**这一档及以下都不再问**，
//! 高于上限时仍走会话阈值与审批。
//!
//! ## 为什么住能力域
//!
//! 它的键是能力名、值的能力风险档，读侧是审批闸门（`providers/collectors`）与授权
//! 级联的记录方（`plugins/loop`）——两处隔着插件墙，只有 core 共同可见。

use super::CapabilityRiskLevel;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

/// 工具授权清单：能力名 → 允许的最高风险档。
///
/// ## 读侧容错是**逐条**的，不是整份
///
/// 手写 `PLUGIN.yml` 里的一个笔误（未知档位 / 空工具名）只丢掉那一行；整份配置
/// 照常加载。整份失败会让一次笔误重置用户的所有会话配置（`build` 的回退是
/// `SessionConfig::default()`），代价与「那一行授权没生效」不成比例。
///
/// 丢掉的永远是**授权**方向：认不出的值不猜成某一档，因此不可能因为写错一个词
/// 就静默打开某个工具。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalGrants(BTreeMap<String, CapabilityRiskLevel>);

impl ApprovalGrants {
    /// 从 JSON 文本解析（`ctx[APPROVAL_GRANTS]` 的线格式）。
    ///
    /// 非对象 / 非法 JSON ⇒ 空清单（fail-closed：没有授权就没有免问）。
    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    /// 某个能力名当前允许的最高风险档；不在清单里 ⇒ `None`。
    pub fn ceiling(&self, tool: &str) -> Option<CapabilityRiskLevel> {
        self.0.get(tool).copied()
    }

    /// 把某个工具的上限**抬**到 `level`（只抬不降），返回生效后的上限。
    ///
    /// 只抬不降与「本会话放行」同纪律：一次批准不该顺手收紧已有的授权
    /// （撤销走配置文档本身，D-170 决策 5）。
    pub fn raise(
        &mut self,
        tool: impl Into<String>,
        level: CapabilityRiskLevel,
    ) -> CapabilityRiskLevel {
        let entry = self.0.entry(tool.into()).or_insert(level);
        if *entry < level {
            *entry = level;
        }
        *entry
    }

    /// 线格式（`ctx[APPROVAL_GRANTS]`）：JSON 对象。
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.0).unwrap_or_else(|_| "{}".to_string())
    }

    fn from_wire(raw: BTreeMap<String, Value>) -> Self {
        Self(
            raw.into_iter()
                .filter_map(|(tool, value)| {
                    let tool = tool.trim();
                    if tool.is_empty() {
                        return None;
                    }
                    let level = value.as_str().and_then(CapabilityRiskLevel::parse)?;
                    Some((tool.to_string(), level))
                })
                .collect(),
        )
    }
}

impl Serialize for ApprovalGrants {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // 线上就是一张普通映射：配置文档（手工可编辑）与 ctx 键共用同一个形状。
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ApprovalGrants {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // `null` 与缺席同解（表单可能交回空槽位）。
        let raw = Option::<BTreeMap<String, Value>>::deserialize(deserializer)?;
        Ok(raw.map(Self::from_wire).unwrap_or_default())
    }
}

#[cfg(test)]
#[path = "grants.test.rs"]
mod tests;
