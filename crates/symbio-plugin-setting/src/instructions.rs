//! 两级指令文件（AGENTS.md）的**读取与覆盖注入** —— setting 的第二条注入腿。
//!
//! ## 与记忆（`memory` 的 `MEMORY.md`）语义隔离
//!
//! AGENTS.md 是**稳定指令 / 人格**（"我是谁、我怎么做事"），一般不修改；三层记忆
//! 是**动态事实**（"我记得什么"），模型可读写、可叠加。两者物理文件不同、所有者
//! 不同、组合语义也不同：
//!
//! | | 指令（本模块） | 记忆（`symbio-plugin-memory`） |
//! |---|---|---|
//! | 文件 | `{workdir}/AGENTS.md`、`{agentdir}/AGENTS.md` | 三层统一 `MEMORY.md` |
//! | 层级 | 2 级：工作区 + 智能体 | 3 层：会话 + 工作区 + 智能体 |
//! | 组合 | **覆盖**：有工作区则不读智能体级 | **叠加**：三层各自注入、各可开关 |
//! | 归属 | setting | memory |
//!
//! ## 覆盖语义（用户拍板：仅指令覆盖，记忆仍叠加）
//!
//! 工作区级存在且非空 → 只注入【工作区指令】，**完全跳过**智能体级；
//! 工作区级不存在 / 空 → 回退智能体级，注入【本智能体指令】；
//! 两级都没有 → 不注入（缺省不改任何一轮的提示词）。
//!
//! 这符合行业惯例（AGENTS.md / CLAUDE.md 的"就近优先"）：工作区级是更贴近当前
//! 上下文的指令，智能体级是通用兜底，二者不该同时进上下文重复占用预算。
//!
//! ## 只读：内容不在配置里
//!
//! 本模块经作用域 VDFS provider（`ctx[WORKSPACE_VDFS]` / `ctx[AGENT_VDFS]`）**现读**
//! 文件正文，不提供写接口、不落地投影字段。文件名由 [`super::config`] 配，默认
//! `AGENTS.md`。稳定指令归用户在其文件里维护，setting 只负责"读出来、给模型看"。

use symbio_plugin_api::{
    DynVdfsProvider, PluginInvokeRequest, PluginInvokeRequestExt, VdfsContext, VdfsRequest,
    VdfsResponse, AGENT_VDFS, WORKSPACE_VDFS,
};

/// 【工作区指令】段名（收集器覆盖键）与标题
pub const WORKSPACE_SEGMENT_NAME: &str = "workspace-instructions";
pub const WORKSPACE_SEGMENT_TITLE: &str = "工作区指令";

/// 【本智能体指令】段名（收集器覆盖键）与标题
pub const AGENT_SEGMENT_NAME: &str = "agent-instructions";
pub const AGENT_SEGMENT_TITLE: &str = "本智能体指令";

/// 读取一条指令文件的正文（文件不存在 / 无作用域 / 空 → `None`）。
///
/// 经作用域 provider 的相对地址读取：provider 的根已由宿主绑到工作区 / 智能体目录，
/// 这里只给纯文件名（由配置给出，已校验无路径分隔符）。"还没写过"是正常状态——
/// 读不到就当作这一层不存在，不往收集期错误桶里塞东西。
async fn read_instructions(provider: Option<DynVdfsProvider>, file_name: &str) -> Option<String> {
    let provider = provider?;
    let vctx = VdfsContext::new(());
    let text = match provider.dispatch(&vctx, file_name, VdfsRequest::Read).await {
        Ok(VdfsResponse::Read(content)) => content.text.unwrap_or_default(),
        // 文件不存在 / 无作用域（provider 已 None）/ 是目录 → 这一层没有指令
        _ => String::new(),
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 一条要注入的指令段（已按覆盖语义选定唯一一条）。
pub struct InstructionSegment {
    /// 收集器覆盖键
    pub name: &'static str,
    /// 片段标题（渲染为 `【{title}】`）
    pub title: &'static str,
    /// 正文
    pub body: String,
}

/// 按覆盖语义读取两级指令，返回**至多一条**要注入的段。
///
/// 工作区级优先且覆盖智能体级：工作区级有内容就直接返回它，连智能体级都不读。
/// 两级都没有 → `None`。
pub async fn resolve(ctx: &dyn PluginInvokeRequest, file_name: &str) -> Option<InstructionSegment> {
    // 工作区级：就近优先。有内容即覆盖，不再看智能体级。
    let ws = read_instructions(ctx.get(WORKSPACE_VDFS).flatten(), file_name).await;
    if let Some(body) = ws {
        return Some(InstructionSegment {
            name: WORKSPACE_SEGMENT_NAME,
            title: WORKSPACE_SEGMENT_TITLE,
            body,
        });
    }

    // 智能体级：工作区级缺席时的兜底。
    let agent = read_instructions(ctx.get(AGENT_VDFS).flatten(), file_name).await;
    agent.map(|body| InstructionSegment {
        name: AGENT_SEGMENT_NAME,
        title: AGENT_SEGMENT_TITLE,
        body,
    })
}

/// 指令段 → 提示词文本。
///
/// 极简：只写标题 + 正文，不加地址 / 上限 / 写法提示——稳定指令不归模型改写，
/// 每轮烧这些头文件是纯浪费。正文原样保留（不改一字），模型据此理解自己是谁。
pub fn render(seg: &InstructionSegment) -> String {
    format!("【{}】\n{}", seg.title, seg.body)
}

#[cfg(test)]
#[path = "instructions.test.rs"]
mod tests;
