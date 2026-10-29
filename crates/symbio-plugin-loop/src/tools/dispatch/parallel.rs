//! 批内执行策略：**风险档 → 并行/串行**的唯一映射。
//!
//! ## 判据从哪来
//!
//! 分档用的是工具**已有的自声明风险档** [`CapabilityRiskLevel`]（`CapabilityMeta.risk`，
//! 未声明兜底 `Medium`），不新造「是否只读」布尔：审批闸门读的就是同一档
//! （`local/policy`），两处若各判一次，新增一个工具时必然有一处漏掉且没有东西会红。
//!
//! 只把 `Low`（只读、不触碰资源）放进并行；`Medium` / `High` 一律串行——写文件、
//! 改配置、发消息这类调用并行会引入顺序不确定，而「宁可多问一次」是本仓风险档
//! 的既定方向。
//!
//! ## 为什么用**静态**档而不是 `risk_for(args)`
//!
//! `Capability::risk_for(args)` 可以随参数降档（`shell` 的 `git status` 是 `Low`），
//! 但它需要**参数**与能力实例；批内调度发生在执行之前，若为调度先解一遍参数风险，
//! 调度就成了执行的一部分（且两处解析可能漂移）。调度按声明的静态档取**保守值**：
//! `shell` 因此恒串行——少并行一次，不会错一次。
//!
//! ## 交互模式
//!
//! `interactive` 下审批 / 提问是「一次只处理一张卡」的语义，并行会把两张卡同时推
//! 给用户，故该模式**整批串行**（判据在 [`policy_for_mode`]）。
//!
//! ## 并发上限
//!
//! 一次响应可能带很多个只读调用（megabatch 的上限在流层）。无上限扇出会让每个
//! 调用各起一条子进程 / 网络请求，机器被自己的工具打满。上限**由端口声明**
//! （`PortDecl.concurrency`）给出，执行层不持有一个常量：常量会让第二个端口的上界
//! 只能跟着改引擎，而「加端口 = 加声明」是终态成本（DE-015 §6 判据 1）。
//! `0` / `1` = 串行；更大的值 = 至多几条同时在飞。

use std::collections::HashMap;
use symbio_core::{CapabilityMeta, CapabilityRiskLevel};

/// 一个调用在批内的执行策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecPolicy {
    /// 可与其它 `Parallel` 调用同时在途（结果仍按调用顺序收口）。
    Parallel,
    /// 必须独占执行窗口：前序在途调用先全部收口，本调用跑完再放下一条。
    Serial,
}

/// 风险档 → 执行策略的**唯一映射**（新增一档只改这里）。
pub(crate) const fn policy_for_risk(risk: CapabilityRiskLevel) -> ExecPolicy {
    match risk {
        CapabilityRiskLevel::Low => ExecPolicy::Parallel,
        CapabilityRiskLevel::Medium | CapabilityRiskLevel::High => ExecPolicy::Serial,
    }
}

/// 会话运行模式 → 整批策略。交互模式整批串行（理由见模块文档）。
pub(crate) fn policy_for_mode(mode: &str, risk_policy: ExecPolicy) -> ExecPolicy {
    if mode == "interactive" {
        ExecPolicy::Serial
    } else {
        risk_policy
    }
}

/// 端口声明 → 整批策略。`concurrency <= 1` 是**硬闸**：即使风险档是 `Low`、
/// 模式是 `auto`，也不得并行。更大的值仍走 [`policy_for_mode`] 的既有规则；
/// 「至多几条在飞」由调用方按同一个数截断。
///
/// 声明与策略分开的理由见 `PortDecl.concurrency`：端口只说「同批最多几笔在飞」，
/// 具体哪几条能同时在途由风险档决定；两者不是一个问题，不能互相替代。
pub(crate) fn policy_for_port(concurrency: u16, mode: &str, risk_policy: ExecPolicy) -> ExecPolicy {
    if concurrency <= 1 {
        ExecPolicy::Serial
    } else {
        policy_for_mode(mode, risk_policy)
    }
}

/// 批首读一次的工具风险表（键 = **线上名**与能力名，两者都收）。
///
/// 键要收两种形态，因为两条链路各用各的：注册表里是能力名（`local/read`），
/// 模型发来的是线上名（`local__read`），而 `capability_resolve` 的消解规则是
/// 「字面名优先、投影名次之」——本表按同一规则收键（先收投影名，再用字面名覆盖），
/// 于是查表结果与执行期的解析结果同源。
#[derive(Debug, Default)]
pub(crate) struct ToolRisks(HashMap<String, CapabilityRiskLevel>);

impl ToolRisks {
    pub(crate) fn of_metas(metas: &[CapabilityMeta]) -> Self {
        let mut map = HashMap::new();
        // 投影名先收：两个字面名投到同一个线上名时，后收的会覆盖前一个——
        // 与 `capability_resolve` 的「字面相等优先」不冲突（那里也先认字面名）。
        for meta in metas {
            map.insert(
                symbio_core::capability_to_wire(&meta.name),
                meta.effective_risk(),
            );
        }
        // 字面名后收，覆盖同键的投影值。
        for meta in metas {
            map.insert(meta.name.clone(), meta.effective_risk());
        }
        Self(map)
    }

    /// 线上名 → 生效风险档；查不到 = 未声明 = `Medium`（与 `effective_risk` 同一兜底）。
    pub(crate) fn effective_risk(&self, wire_name: &str) -> CapabilityRiskLevel {
        self.0
            .get(wire_name.trim())
            .copied()
            .unwrap_or(CapabilityRiskLevel::Medium)
    }

    /// 本条调用的风险档策略（交互模式另由 [`policy_for_mode`] 压成串行）。
    pub(crate) fn policy_for_call(&self, wire_name: &str) -> ExecPolicy {
        policy_for_risk(self.effective_risk(wire_name))
    }
}

#[cfg(test)]
#[path = "parallel.test.rs"]
mod tests;
