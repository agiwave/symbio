//! 系统提示词族的**读写协议**——表（`Registry` 的 `put` / `get` / `list`）
//! 之上的提示词专用词法（DE-025 §3）。
//!
//! ## 写：`put(CapabilityRow::prompt(段名, 正文))`
//!
//! 语义只有一条：**注册在这里的东西一定会送达模型**——全部条目按注册顺序拼接，
//! 不做「取一个」的竞争。
//!
//! ### 为什么没有「基座 / 片段」两条通道
//!
//! `register_system_prompt`（人格基座，按优先级链取一个）与
//! `register_system_prompt_segment`（机制级片段，全部追加）做的是**同一件事**——
//! 往一个命名有序集合里塞一段文字——差异只在消费侧「取一个」还是「取全部」。
//!
//! 而「取一个」这个需求本身站不住：人格的**选择**发生在**注册期**（model 插件已按
//! `PROVIDER_ID` 解析出唯一生效 provider，注册的就是当次该用的那一份），消费侧再按
//! key 选一次是空动作。实测那两处「双键注册」（`provider_id` 与 `"default"`）写的
//! 还是**同一份内容**。
//!
//! 于是只有一个入口（`put` 提示词行）、一个有序集合（[`capability_prompts`]）。
//! 「谁在前谁在后」= 注册顺序 = 遍历顺序；要控制顺序就控制注册时机。
//!
//! ### 段名是覆盖键，不是分类
//!
//! 同名覆盖（`IndexMap` 语义）用于「同一段文字注册两次时取后者」，不承担任何分类
//! 职责——不要用命名前缀去表达「这是人格」「这是记忆」。作用域（子智能体）是**唯一**
//! 加前缀的地方，且那份前缀改变的是**键**（`agent/<id>/<段名>`），不是分类。
//!
//! ## 读：`capability_prompts`
//!
//! 消费侧两处（会话装配系统提示词、`compose` 拼自己的提示词）都要「按注册序的全部
//! 段」；口径在这一处定义，两处各自过滤（空段跳过）是消费侧的判断。

use super::{family, Registry};

/// 列出已注册的系统提示词：`(段名, 正文)`，保注册顺序。
///
/// 空段**不在这里**过滤：注册了空串也照常列出来——「这一段是空的」是注册方给的事实，
/// 消费侧（拼接）才有资格说「跳过它」（会话侧的 `resolve_system_prompt` 与 `compose`
/// 各自都要跳，两处判断一致是因为它们各自都写了那三行，不是因为这里替它们决定）。
pub async fn capability_prompts(visitor: &dyn Registry) -> Vec<(String, String)> {
    visitor
        .list(family::PROMPT)
        .await
        .into_iter()
        .filter_map(|row| {
            row.prompt_text()
                .map(|text| (row.name().to_string(), text.to_string()))
        })
        .collect()
}
