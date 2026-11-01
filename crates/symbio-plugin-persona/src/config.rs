//! 人格配置 —— 本插件唯一的旋钮（`<根>/persona/PLUGIN.yml`）。
//!
//! ## 为什么人格是一份**配置**而不是一段常量
//!
//! 「这个人是谁」没有出厂答案：它是用户对自己助手的一句话描述，改它不该需要
//! 重新编译。判据与本仓其它插件同一条——**插件目录 = 配置目录**，各管各的旋钮。
//!
//! ## 为什么只有一个键
//!
//! 语气、口头禅、称呼习惯、禁忌话题，本质是同一件事的四个侧面，拆成四个字段只会
//! 逼用户把一段连贯的话切碎，再让消费侧猜该怎么拼回来。**一段文字、一个段名**
//! 就是这一类事实的最小完整形状（与 `capability/prompt.rs` 的「一个入口一个有序
//! 集合」同一取舍）。
//!
//! 也**没有** `enabled` 键：关掉它的方式是把正文清空（平凡值）或停用插件
//! （装配位）。再多一个开关，就多一种「开关开着但正文是空的」的中间态需要解释。

use serde::{Deserialize, Serialize};

/// 人格配置（`<根>/persona/PLUGIN.yml` 的业务键）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonaConfig {
    /// 人格正文：这个助手是谁、用什么口吻说话。
    ///
    /// **缺席 / 空串 / 纯空白同义 ⇒ 不注入任何段**（见 [`Self::text`]）。三种写法
    /// 同义是刻意的：设置页把输入框清空得到的是 `""`，手写文件删掉这一行得到的是
    /// 缺席，二者在语义上没有区别，让它们分叉只会让「为什么清空了还生效」变成
    /// 一个需要解释的问题。
    #[serde(default)]
    pub persona: Option<String>,
}

impl PersonaConfig {
    /// 生效的人格正文（空 ⇒ `None`）。
    pub fn text(&self) -> Option<&str> {
        self.persona
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
