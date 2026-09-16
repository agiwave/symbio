//! 智能体**人格与记忆**的系统提示词构造。
//!
//! ## 从「工具锚定」到「提示词注入」
//!
//! 改造前人格只经 `agent_identity` **工具说明**送达模型（160 字摘要 + 调用取回全文）。
//! 那条路能工作，但人格对模型来说是「一个可以调用的工具」，而不是「我是谁」——
//! 工具说明里的摘要只在工具列表里出现，模型要先决定去调用它才看得到全文。
//!
//! 现在人格**每轮随系统提示词一起注入**，模型一开始就知道自己是谁。
//! `agent_identity` 工具保留——它现在的职责是**取回全文**（注入只带预算内的部分）。
//!
//! ## 一份产物：人格
//!
//! | 产物 | 内容 | 地址 | 谁在用 |
//! |---|---|---|---|
//! | [`identity_segment`] | 装配出的人格（`prompts/` + `skills/`） | `.vdfs/agent/<id>/…` | 模型、用户 |
//!
//! 它在**会话选择了该智能体**时注册（见 `plugin::AgentPlugin::attach_bundle`
//! 的 `ctx[AGENT_ID]` 分支）：没选智能体就没有人格可注入。
//!
//! **智能体记忆不在这里**——它是**单文件**（bundle 根 `AGENTS.md`），机制走
//! `symbio_core::memory`、个性在 [`super::memory`]。人格不是单文件（由 `prompts/` +
//! `skills/` 装配），内核收不了，所以留在本模块自己排版。
//!
//! ## 形态：一行头信息 + 正文
//!
//! 人格**每轮**都会进上下文，因此按「token 预算」而不是「文档完备性」设计：头信息
//! 只占一行，四要素按重要性排序（**地址**能改 → **上限**写超会被拒 → **当前**还剩
//! 多少 → **写法**先读后写），不写「这是什么」「该写什么」这类解释。
//!
//! 排版与内核 [`render_segment`](crate::symbio_core::render_segment) **形态对齐**
//! （一行头信息 + 正文 + 空 / 截断各一行短提示），差别只在地址形态：人格是
//! 「一个目录 + 三类条目模板」，记忆是一个文件。

use super::config::AgentConfig;
use super::vdfs::item_address_templates;
use crate::symbio_core::{InjectedMemory, PLUGIN_AGENT};

/// 人格条目的注册名（同名覆盖的键）
pub const SEGMENT_NAME: &str = "agent-identity";

/// 人格条目的标题（渲染为 `【智能体人格】`）
const IDENTITY_TITLE: &str = "智能体人格";

/// bundle 的 VDFS 展示地址（`.vdfs/agent/<id>`）
pub fn bundle_address(bundle_id: &str) -> String {
    format!(".vdfs/{PLUGIN_AGENT}/{bundle_id}")
}

/// 条目地址模板的**紧凑**写法：公共前缀（bundle 目录）只出现一次，
/// 三类条目用花括号并列。
///
/// 模板来自 [`item_address_templates`]（与 VDFS 寻址**同一份声明**），只是换了排版
/// ——逐条写全路径会让头信息从一行涨到三行，而它**每轮**都要付 token。
fn item_templates_inline() -> String {
    let hints: Vec<&str> = item_address_templates()
        .into_iter()
        .map(|(_, hint)| hint)
        .collect();
    hints.join(" · ")
}

/// 构造人格条目。
///
/// `identity_text` 是 [`assemble_bundle`](super::super::core::spec::assembly) 装配出的
/// 人格全文（`prompts/` + `skills/` 按 priority 拼接），此处按配置预算截断。
pub fn identity_segment(bundle_id: &str, identity_text: &str, cfg: &AgentConfig) -> String {
    let body = InjectedMemory::cut(identity_text, cfg.effective_inject_bytes());
    let dir = bundle_address(bundle_id);

    let mut out = String::with_capacity(body.text.len() + 320);
    out.push_str(&format!(
        "【{IDENTITY_TITLE}】（bundle：{bundle_id}；条目：{dir}/{{{}}}；\
         条目上限：{}字节，当前：{}字节；改前先 vdfs_read，合并后整篇 vdfs_write）\n",
        item_templates_inline(),
        cfg.effective_item_max_bytes(),
        body.total_bytes,
    ));

    if body.text.trim().is_empty() {
        out.push_str("（该智能体暂无提示词 / 技能，可写入 `prompts/` 建立人格）\n");
    } else {
        out.push_str(body.text.trim_end());
        out.push('\n');
    }

    if body.truncated {
        out.push_str(&format!(
            "（已截断至 {} 字节，完整人格用 agent_identity 工具或 vdfs_read 读取）\n",
            body.budget_bytes
        ));
    }

    out
}

#[cfg(test)]
#[path = "prompt.test.rs"]
mod tests;
