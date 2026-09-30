//! 委派判定 —— **纯函数**，输入用户消息与配置，输出「这轮该不该开 worker」+ 理由。
//!
//! ## 为什么需要它（而不是"总是启动"）
//!
//! "总是开一个 worker 读主会话再决定"有两个代价：① 每条寒暄都产生一个后台会话
//! （状态面噪声、存储噪声）；② 判定权落在模型手里，**不可审计**——用户看不到
//! "为什么这轮没动手"。判定放在这里 = 规则可读、可配、可单测，且**必须回理由**。
//!
//! ## 判据（按序，先命中先赢）
//!
//! 1. 插件关闭 ⇒ [`Dispatch::Chat`]（平凡值，`reason = "已关闭"`）
//! 2. **显式前缀**：`/work …` ⇒ `Work`——用户直接指挥，零歧义；
//! 3. **关键词命中**（不区分大小写）⇒ `Work`；
//! 4. **主题长度** ≥ 阈值（阈值 > 0 时）⇒ `Work`；
//! 5. 否则 `Chat`。
//!
//! ## 漏判怎么办（不许静默）
//!
//! 判 `Chat` 只是"这一轮由主会话直接回答"，不是"永久拒绝"。主会话的回复里应带
//! 一句可申诉的话（"需要我动手做吗"），用户一句话即可补齐——这条人工兜底是
//! 规则判定的**必要配套**，理由与 v2 的 J3 同源：判错的方向必须是**可见**的。

use super::config::DelegateConfig;

/// 委派去向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// 主会话直接回答（不动手）
    Chat,
    /// 开一个 worker 会话去动手
    Work,
}

impl Dispatch {
    /// 线格式词（`decide` 路由的返回值；前端 / 日志共用一份）
    pub const fn wire(self) -> &'static str {
        match self {
            Dispatch::Chat => "chat",
            Dispatch::Work => "work",
        }
    }
}

/// 判定结果（`dispatch` + **理由**——理由不是装饰，是"为什么没动手"的唯一出口）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub dispatch: Dispatch,
    pub reason: String,
}

impl Decision {
    fn chat(reason: impl Into<String>) -> Self {
        Self {
            dispatch: Dispatch::Chat,
            reason: reason.into(),
        }
    }
    fn work(reason: impl Into<String>) -> Self {
        Self {
            dispatch: Dispatch::Work,
            reason: reason.into(),
        }
    }

    pub fn should_delegate(&self) -> bool {
        self.dispatch == Dispatch::Work
    }
}

/// 判定这一轮要不要委派给 worker。**纯函数**——同样的输入永远同样的结论（可双跑比对）。
pub fn decide(user_text: &str, cfg: &DelegateConfig) -> Decision {
    if !cfg.enabled {
        return Decision::chat("委派已关闭（delegate.enabled = false）");
    }

    let text = user_text.trim();
    if text.is_empty() {
        return Decision::chat("空消息");
    }

    // ② 显式前缀
    if let Some(prefix) = cfg.effective_force_prefix() {
        if text.starts_with(prefix) {
            return Decision::work(format!("显式前缀 {prefix:?}"));
        }
    }

    // ③ 关键词（子串、忽略大小写）
    let lower = text.to_lowercase();
    for kw in &cfg.keywords {
        let k = kw.trim();
        if k.is_empty() {
            continue;
        }
        if lower.contains(&k.to_lowercase()) {
            return Decision::work(format!("关键词命中 {k:?}"));
        }
    }

    // ④ 主题长度
    if cfg.min_chars > 0 {
        let chars = text.chars().count();
        if chars >= cfg.min_chars {
            return Decision::work(format!("主题长度 {chars} ≥ 阈值 {}", cfg.min_chars));
        }
    }

    Decision::chat("未命中任何委派判据（默认对话）")
}

#[cfg(test)]
#[path = "decide.test.rs"]
mod tests;
