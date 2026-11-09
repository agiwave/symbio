//! 行为名的**收集面**：名字由实现自报，本模块不持有任何能力名（D-363）。
//!
//! 一个能力包要新开一种行为，只在**自己的 crate** 里做两件事：定义一个常量，
//! 连同 [`ToolBehavior::behavior`] / [`ToolBehavior::kind`] 经
//! [`submit_behavior!`](crate::submit_behavior) 链接期登记。宿主经本模块收集，
//! **不需要改 `symbio-plugin-api`**——手写名单一旦存在，新增能力就又要回到
//! 「装一个能力改架构模块」的老路。
//!
//! ## 为什么收集面在这里
//!
//! 宿主有两处要问「这个名字按契约造不造格」：写口的违约判据
//! （[`crate::plugin::effects::expects_facts`]）与声明面的校验（[`is_known`]）。问题问的是
//! **名字**，不是某个实现，所以由本模块从已登记的实现里折出「名字 → 造格声明」；
//! 换一个实现，答案不变。
//!
//! 本模块随 `effects` 整块住 `symbio-sdk`（`D-408`）：收集面读的是注册表，
//! 而注册表与扩展点是同一个类型闭包；门面 `symbio_plugin_api::behaviors` 只转出。
//!
//! 名字仍是**跨进程的**（工具元数据随插件通道走），改名等于改协议；只在语义真的要
//! 变的时候改，并在 DE-024 留一条决策。**跨插件共享**的行为名（声明方与实现方分属两个
//! crate）住 `symbio_plugin_api::vocab`：那是词汇层，不是本模块的收集面（D-364）。
//!
//! ## 同名多实现
//!
//! 一个行为名可以有多个实现（按 owner 序逐个跑），但**造格声明必须一致**——
//! 名字是一份契约。声明不一致视为未登记：判据按「应当造格」处理（fail-closed）。

use crate::plugin::effects::handlers;

/// 一个行为**造不造格**。
///
/// 逐位声明在实现上、不在中心的表里：判据问的是「这个行为名按契约该不该产出事实」，
/// 而不是「某个实现的 `after` 会不会返回空」——后者是运行时近似，换个实现就漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 造格型：`after` 交出事实草稿；声明了它却 0 草稿 = 违约（fail-closed）。
    LandsFacts,
    /// 富化型 / 纯前置判断型：只补既有事实的字段（`annotate`）或只判前置条件
    /// （`before`），不造新格。0 草稿是它的正常形态。
    NoFacts,
}

/// 已登记的行为名（按字典序，去重）。
///
/// 这就是「已知行为」的**全部**：没有第二份清单可漂移，新增能力不改本 crate。
pub fn known() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = handlers().iter().map(|b| b.behavior()).collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// 这个行为名登记的造格声明。
///
/// `None` = 未登记（没有任何实现自报这个名字），或同名实现的声明互相矛盾。
pub fn kind_of(name: &str) -> Option<Kind> {
    let mut found: Option<Kind> = None;
    for behavior in handlers() {
        if behavior.behavior() != name {
            continue;
        }
        match found {
            None => found = Some(behavior.kind()),
            Some(kind) if kind == behavior.kind() => {}
            // 同名不同声明：契约自相矛盾，按未登记处理（fail-closed）。
            Some(_) => return None,
        }
    }
    found
}

/// 这个行为名是否**应当**产出事实草稿。
///
/// `None` = 未登记（未知行为）：判据按「应当造格」处理，宁可假红也不让一个没声明
/// 种类的行为静默空转。
pub fn lands_facts(name: &str) -> Option<bool> {
    kind_of(name).map(|kind| kind == Kind::LandsFacts)
}

/// 这个名字是不是宿主已知的行为。
///
/// **未知行为静默不生效**是 J3 要消灭的形态，所以声明面的读取方应当拿它校验；
/// 本函数只回答词汇问题，不替任何闸门决定语义。
pub fn is_known(name: &str) -> bool {
    kind_of(name).is_some()
}

#[cfg(test)]
#[path = "behaviors.test.rs"]
mod tests;
