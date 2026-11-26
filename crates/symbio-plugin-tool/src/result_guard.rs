//! L0：单次工具结果守卫（执行体侧）
//!
//! 工具结果往往很长（shell 输出、文件全文、网页抓取…），单条即可撑爆上下文窗口。
//! 本模块在结果交回环**之前**按 token 预算裁剪：
//!
//! 1. 估算 token 数 ≤ 预算 → 原样返回；
//! 2. 否则：把全文存档到**会话作用域**（`SESSION_VDFS` 下的 [`ARCHIVE_REL_DIR`]，
//!    取回走会话地址面 `<根>/session/<sid>/tool_archives/…`——根名由 vdfs 插件静态
//!    声明、经运行期数据取得，不在此写字面量），并对正文做 **head/tail 摘要**
//!    （保留前 60% + 后 40% 预算，尾部通常是结论/错误，更关键），中间插入占位说明。
//!
//! 存档不再由插件拿物理路径直写——调用方递来作用域 provider，本模块只
//! 认相对地址。这样后端换成内存 / 网络存储时插件无感，也避免每个插件各拼一份
//! 物理路径。 把存档根从工作区收窄到会话：会话私有数据不污染工作区根目录，
//! 滚动淘汰也不再跨会话互相挤掉。provider 缺席（无会话也无工作区）= 不存档，
//! 降级为「未存档」提示。
//!
//! 文件名 = 毫秒时间戳 + token 数 + 内容 FNV 指纹，杜绝「同秒同 token 数互相覆盖」
//! 的碰撞；目录按修改时间保留最新 [`ARCHIVE_KEEP`] 个文件，防止无限累积。
//!
//! 计量与头尾切分机制住在 `symbio_plugin_api::text`（环与世界共用同一套预算算术）；
//! 本模块只管**工具结果这一种策略**：触发条件、60/40 配比、占位符文案与存档布局。

use symbio_plugin_api::text::{default_tokenizer, split_head_tail, HeadTailSplit, Tokenizer};
use symbio_plugin_api::{
    clock_now_ms, descend_addr, DynVdfsProvider, VdfsContent, VdfsContext, VdfsRequest,
    PLUGIN_ID_SESSION,
};

/// 单条工具结果 token 预算（默认 ≈ 32KB 文本），可由调用方按需覆盖。
pub const DEFAULT_TOKEN_CAP: usize = 8192;

/// 每个会话的 L0 存档目录保留的最新文件数（防无限累积；超出按修改时间淘汰）。
pub const ARCHIVE_KEEP: usize = 20;

/// 存档目录在 `SESSION_VDFS` 下的相对地址。
///
/// 相对地址是 provider 的寻址键；印给模型的取回地址由 [`retrieval_address`] 在此
/// 基础上补会话地址面前缀，两者指向同一物理文件。
pub const ARCHIVE_REL_DIR: &str = "tool_archives";

/// 由存档相对地址算出印给模型的取回地址。
///
/// 有会话 id **且**地址空间已装配（vdfs 插件声明了根名）⇒ 给会话地址面的虚拟
/// 地址（相对物理路径不泄露、不因机器 / 目录迁移失效）；无会话（宿主态，存档降级
/// 落在工作区）或根名未声明（诊断性降级）⇒ 退回相对地址，`vdfs_read` 按 workdir
/// 解析即可取回。两种形态都落在 [`archive_full_text`] 实际写入的那份文件上。
///
/// 根名不在此写字面量：经 [`descend_addr`] 从 vdfs 插件的运行期静态声明取得
/// （S-010）。`descend_addr("", "")` 为空 ⇔ 根名未声明（构建里没有 vdfs 插件）。
fn retrieval_address(rel: &str, session_id: Option<&str>) -> String {
    let Some(sid) = session_id.filter(|s| !s.trim().is_empty()) else {
        return rel.to_string();
    };
    let root = descend_addr("", "");
    if root.is_empty() {
        return rel.to_string();
    }
    // 会话挂载段名 = 插件 id（跨插件同一拼写，经门面取，不另立常量）
    let session_mount = descend_addr(&root, PLUGIN_ID_SESSION);
    format!("{session_mount}/{sid}/{rel}")
}

/// 取回指引文案（三层压缩占位符共用同一格式；随驱动器走，因为写占位符的是这里）。
///
/// 与 `session/paths.rs` 的取回协议同义——它描述的是**用户可见的取回入口**，
/// 不是会话存储布局；跟着写文案的一侧住，避免环为了一个字符串去认识会话。
pub const RETRIEVAL_HINT: &str = "（取回：vdfs_read 该路径，按 offset/limit 分段读取）";

/// 守卫结果
#[derive(Default)]
pub struct GuardedResult {
    /// 裁剪后的文本（已可安全放进上下文）
    pub text: String,
    /// 是否被截断/存档
    pub truncated: bool,
    /// 原始 token 估算
    pub original_tokens: usize,
    /// 存档相对地址（若写入成功；`vdfs_read` 可直接取回）
    pub archive_path: Option<String>,
}

/// 内容 FNV-1a 64 位指纹（取高 32 位十六进制）。
///
/// 同秒同 token 数的两个不同结果，指纹几乎必然不同 → 文件名不碰撞；
/// 完全相同的内容共享同一指纹 → 覆盖无害（内容一致）。
fn content_fingerprint(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (h >> 32) as u32)
}

/// 把全文写入作用域根（会话；宿主态降级工作区）。provider 缺席或写入失败 → None，不阻断主流程。
///
/// 文件名 = `tool_{毫秒ts}_{token数}_{内容指纹}.txt`：毫秒 + 指纹双保险。
async fn archive_full_text(
    text: &str,
    token_count: usize,
    provider: Option<&DynVdfsProvider>,
) -> Option<String> {
    let provider = provider?;
    let now = clock_now_ms();
    let file_name = format!(
        "tool_{}_{}_{}.txt",
        now,
        token_count,
        content_fingerprint(text)
    );
    let rel = format!("{ARCHIVE_REL_DIR}/{file_name}");
    let vctx = VdfsContext::new(());
    provider
        .dispatch(
            &vctx,
            &rel,
            VdfsRequest::Write {
                content: VdfsContent::text(text),
            },
        )
        .await
        .ok()?;
    prune_archive_dir(provider, ARCHIVE_KEEP).await;
    Some(rel)
}

/// 按修改时间保留最新 `keep` 个存档文件，淘汰更旧的（best-effort，失败静默）。
///
/// `List` 返回的节点带 `updated_at`（ScopeDirVdfs 从 mtime 填），按它排序；同秒
/// 的并列再按名字排——文件名头是毫秒时间戳，字典序即时间序。
async fn prune_archive_dir(provider: &DynVdfsProvider, keep: usize) {
    let vctx = VdfsContext::new(());
    let Ok(resp) = provider
        .dispatch(&vctx, ARCHIVE_REL_DIR, VdfsRequest::List {})
        .await
    else {
        return;
    };
    let Some(items) = resp.into_list() else {
        return;
    };
    let mut files: Vec<(i64, String)> = items
        .into_iter()
        .map(|item| (item.node.updated_at.unwrap_or(0), item.node.name))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1))); // 新 → 旧
    for (_, name) in files.into_iter().skip(keep) {
        let rel = format!("{ARCHIVE_REL_DIR}/{name}");
        let _ = provider
            .dispatch(&vctx, &rel, VdfsRequest::Delete { recursive: false })
            .await;
    }
}

/// 占位符前缀：omit 统计口径保持一致。
fn omit_placeholder_prefix(omit: usize) -> String {
    format!("\n[... 已省略约 {omit} tokens 的中间内容。")
}

/// 守卫工具结果：超过预算则存档 + head/tail 摘要，否则原样返回。
///
/// `provider`：会话作用域 provider（调用方从 `ctx[SESSION_VDFS]` 取，缺席降级
/// `ctx[WORKSPACE_VDFS]`）。`session_id`：仅用于生成占位符里的会话地址面取回地址。
/// provider 缺席（没有可写作用域）→ 不存档，占位符如实说明。
pub async fn guard_result(
    text: &str,
    budget_tokens: usize,
    provider: Option<&DynVdfsProvider>,
    session_id: Option<&str>,
) -> GuardedResult {
    let tok = default_tokenizer();
    let n = tok.count(text);
    if n <= budget_tokens {
        return GuardedResult {
            text: text.to_string(),
            truncated: false,
            original_tokens: n,
            archive_path: None,
        };
    }

    let archive_path = archive_full_text(text, n, provider).await;

    let omit = n.saturating_sub(budget_tokens);
    // 统一取回协议：占位符印会话地址面虚拟地址（与写入根同源）。
    let archive_hint = match &archive_path {
        Some(p) => format!(
            "完整输出已存档至: {}{RETRIEVAL_HINT}",
            retrieval_address(p, session_id)
        ),
        None => "完整输出未存档（无可写作用域）".to_string(),
    };
    let placeholder = format!("{} {archive_hint} ...]", omit_placeholder_prefix(omit));

    let HeadTailSplit { head, tail } = split_head_tail(
        text,
        ((budget_tokens as f64) * 0.6) as usize,
        budget_tokens - ((budget_tokens as f64) * 0.6) as usize,
        tok,
    );

    let mut out = String::with_capacity(head.len() + tail.len() + placeholder.len());
    out.push_str(&head);
    out.push_str(&placeholder);
    out.push_str(&tail);

    GuardedResult {
        text: out,
        truncated: true,
        original_tokens: n,
        archive_path,
    }
}

#[cfg(test)]
#[path = "result_guard.test.rs"]
mod tests;
