//! 模型族的**读写协议**——表（`Registry` 的 `put` / `get` / `list`）之上的
//! 模型专用词法（DE-025 §3）。
//!
//! ## 写侧：只剩 `put`，不变量改由**结构**承载
//!
//! 机制化之前模型族的写入口是一次批量（`register_model_providers(active_id, available)`），
//! 理由是两条事实必须一致：**生效者必然是目录里的一员**——拆成两次注册就允许了
//! 「生效者不在目录里」这个没有错误信号的非法状态。表只有三方法，这条不变量于是换了
//! 承载方式：生效位**长在目录行上**（[`CapabilityRow::model`] 的 `active` 参数），
//! 生效者不在目录里从此**不可表达**；写出整批的 model 插件知道哪个是当次生效者，逐行
//! `put` 即可（`crates/symbio-plugin-model/src/plugin.rs` 的 traverse 分支）。
//!
//! 唯一生效由表的写入实现维持：带生效位的行入表时先把同族旧生效行降位
//! （见 [`Registry::put`] 文档与 `CapabilityRow::demote`）。
//!
//! ## 读侧：两个问题，两条口径
//!
//! | 问题 | 函数 | 口径 |
//! |---|---|---|
//! | 会话引擎该用哪个模型 | [`capability_model_active`] | 取**生效位**那一行；没有生效行 ⇒ `None` |
//! | 插件该用哪个模型 | [`capability_model_resolve`] | 先**严格**查自己配的 id，查不到落回生效者 |
//!
//! **严格**的含义：插件问的是「有没有这一个」，替它挑一个别的就等于悄悄换了它的模型
//! （目录里没有的 id ⇒ 落回生效者，是 `resolve` 显式给的降级，不是查找的语义）。
//! 反过来，**降级而不是失效**：一个写错的 provider id 不该让判决或措辞整条产线消失——
//! 判决退回 `Escalate`、措辞退回模板虽然都「还能工作」，但那是功能没了。故降级方向
//! **只有这一处定义**：判决与措辞各写一遍，就会有一天两处不一致。
//!
//! **已知缺口：没有候选链**。这里的降级只有一跳（首选 id → 当次生效者），生效者自己
//! 不可用（限流 / 故障）时 `resolve` 同样返回它，调用方随即失败——对单会话单 provider
//! 不构成痛点，小时-天的自主链路构成。届时要加的是目录行上的**优先级位**，与生效位同一条 `put` 规则，
//! 不在这里长第二条链路。

use super::{family, Registry};
use crate::ModelProvider;
use std::sync::Arc;

/// 取**当次生效**的模型服务（会话引擎用的那个）。
///
/// 生效位由 model 插件在 traverse 里按 `ctx[PROVIDER_ID] > 默认 > 首个启用` 解析后标在
/// 目录行上；那一行同时就是目录里的一员，故这里不需要再去目录里做第二次查找。
/// 一个生效行都没有（没有可用 provider / 生效者未进目录）⇒ `None`：调用方各走自己的
/// 降级路径（会话报「未找到可用的 Model Provider」、两个插件退回模板），**不**替它挑
/// 一个别的——静默换模型比没有模型更难查。
pub async fn capability_model_active(visitor: &dyn Registry) -> Option<Arc<dyn ModelProvider>> {
    visitor
        .list(family::MODEL)
        .await
        .into_iter()
        .find(|row| row.active())
        .and_then(|row| row.provider().cloned())
}

/// 取一个模型服务：优先 `preferred`（**插件自己配的**那个），取不到落回当次生效者。
///
/// 这是「插件选自己的模型」的唯一形态（`classify` 的判决模型 / `compose` 的措辞模型）。
/// 严格查找 + 单点降级，两条理由见模块文档。
pub async fn capability_model_resolve(
    visitor: &dyn Registry,
    preferred: Option<&str>,
) -> Option<Arc<dyn ModelProvider>> {
    if let Some(id) = preferred {
        // 严格查找：名字撞上工具行时 `get` 会取回那一行，而它没有 provider ⇒ 同样落空
        // （表键是全表唯一的名字空间，跨族同名 = 覆盖，见 `Registry::put`）
        if let Some(provider) = visitor
            .get(id)
            .await
            .and_then(|row| row.provider().cloned())
        {
            return Some(provider);
        }
    }
    capability_model_active(visitor).await
}
