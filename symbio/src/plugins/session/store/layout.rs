//! 会话的**磁盘布局**：目录寻址 + 两种落盘形状 + 同步读件（实现段之三）。
//!
//! 分在这里的判据是「这一段错了谁会红」：本文件出错，红的是**目录与文件本身**——
//! 会话读不出来、整份写坏（并发交错）、清单少一项、子会话寻不到址。
//! 全是自由函数、不需要 `&self`：「哪一段依赖实例状态」因此在名字上一眼可辨
//! （`SessionStore::session_dir` 需要，`dir_for` 不需要）。
//!
//! 布局词汇的**唯一定义在 [`super::paths`]**（子目录名 / 安全段名 / WAL 文件名），
//! 本文件只做「根 + id → 路径」这一段、两种文件的落盘形状，以及围绕它们的
//! 原子写与自愈解析。事实源（WAL）那一头见 [`super::facts`]。
//!
//! ## 形状
//!
//! ```text
//! 顶层会话：`<根>/<safe_id>/session.json`     ← 元数据 + 清单投影（小）
//!           `<根>/<safe_id>/messages.json`    ← 消息（大）
//! 消息压缩存档：`<根>/<safe_id>/messages/msg_*.txt`
//! ```
//!
//! ## 为什么元数据与消息要分两个文件
//!
//! 清单只读 `session.json`，其大小与会话聊了多久**无关**；`messages.json` 只有真的
//! 要取转写时才读。代价是清单字段（标题 / 条数 / 摘要 / 标签）不能现算，必须在 `save`
//! 时算好落盘——它们是**投影**（[`super::types::SessionSummary`]），可重算，不是第二份
//! 真相。性能取舍见 `../docs/106-perf.md`。
//!
//! 旧布局（消息内联在 `session.json`）仍**可读**：`messages.json` 缺失时回落到内联
//! `messages` 字段；投影缺失时用内联消息**就地补算**（[`SessionMetaFile::into_summary`]）
//! ——缺了这条兜底，清单会静默退化成显示 id。
//!
//! ## 子会话嵌套存储（机制约定，见 ../../../../../docs/003-design/009-VDFS规范.md）
//!
//! 会话可派生子会话：子会话不是顶层平级条目，而是存放在父会话目录内的
//! `sessions/` 子目录——`<根>/<safe(父)>/sessions/<safe(子)>/`。
//!
//! 归属声明与路由规则（调用方无感知）：
//! - **归属声明**：`metadata.parent_session_id`（父会话 id）。save 时据此
//!   路由到嵌套目录（无该元数据 = 顶层会话）；
//! - **load / delete / session_dir**：先查顶层，未命中则扫描各会话目录的
//!   `sessions/<id>/`（子会话可凭自身 id 直接寻址）；
//! - **list_sessions_window**：只列顶层（子会话不出现在顶层清单）；
//!   子会话清单走 [`SessionStore::list_sub_sessions`]；
//! - **级联删除**：删除父会话 = `remove_dir_all(父目录)`，子会话随之删除。
//!
//! 子会话 id 由系统生成，与顶层 id 同一格式约束。临时会话无目录概念，
//! 因此不参与嵌套（`list_sub_sessions` 恒空、`session_dir` 恒 `None`）。

use super::*;

/// `<base>/<safe_id>/`
///
/// id→段名的唯一实现在宿主层，经 [`paths::safe_id`] 进入（见模块头）。
pub(super) fn dir_for(base: &Path, session_id: &str) -> PathBuf {
    base.join(paths::safe_id(session_id))
}

/// `<base>/<safe_id>/session.json`
pub(super) fn file_for(base: &Path, session_id: &str) -> PathBuf {
    dir_for(base, session_id).join(SESSION_FILE)
}

/// 子会话目录：`<base>/<safe(父)>/sessions/<safe(子)>/`
pub(super) fn sub_dir_for(base: &Path, parent_id: &str, session_id: &str) -> PathBuf {
    base.join(paths::safe_id(parent_id))
        .join(SUB_SESSIONS_DIR)
        .join(paths::safe_id(session_id))
}

/// `session.json` 的落盘形状 —— **元数据 + 清单投影**（小，与消息量无关）
///
/// `title` / `message_count` / `summary` / `meta_tags` 是**投影**：由
/// [`SessionSummary::of`] 在保存时算好，清单只读它们、绝不读消息。
/// 三者都带 `#[serde(default)]`，因为它们是后加的——存量文件里没有，
/// 此时由 [`SessionMetaFile::into_summary`] 用内联消息就地补算。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SessionMetaFile {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    metadata: serde_json::Value,
    #[serde(default)]
    message_count: usize,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    meta_tags: Vec<String>,
    /// 旧布局把消息内联在这里。新布局**不写**该字段；读的时候仍要认，
    /// 否则存量会话会读成空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    messages: Option<Vec<ChatNode>>,
}

impl SessionMetaFile {
    pub(super) fn of(s: &SessionSummary) -> Self {
        Self {
            id: s.id.clone(),
            title: s.title.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            metadata: s.metadata.clone(),
            message_count: s.message_count,
            summary: s.summary.clone(),
            meta_tags: s.meta_tags.clone(),
            messages: None,
        }
    }

    /// 补上消息，得到完整 `Session`
    pub(super) fn into_session(self, messages: Vec<ChatNode>) -> Session {
        Session {
            id: self.id,
            messages,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata,
        }
    }

    /// 清单投影。
    ///
    /// ⚠️ 存量文件没有投影字段（`title` 空 / `message_count` 0 / `summary` None），
    /// 此时用**内联消息就地补算**——少了这段，清单会静默退化成显示会话 id
    /// （一串短 guid），且不报任何错。
    fn into_summary(self) -> SessionSummary {
        let projection_missing = self.title.is_empty();
        if projection_missing {
            if let Some(messages) = self.messages.as_ref() {
                let s = Session {
                    id: self.id.clone(),
                    messages: messages.clone(),
                    created_at: self.created_at,
                    updated_at: self.updated_at,
                    metadata: self.metadata.clone(),
                };
                return SessionSummary::of(&s);
            }
        }
        SessionSummary {
            id: self.id,
            title: self.title,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata,
            message_count: self.message_count,
            summary: self.summary,
            meta_tags: self.meta_tags,
        }
    }
}

/// `messages.json` 的落盘形状 —— 只有消息（大，只有真要取转写时才读）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MessagesFile {
    #[serde(default)]
    messages: Vec<ChatNode>,
}

/// 原子写：先落 tmp 再 rename。
///
/// 直接 `fs::write`（O_TRUNC + write）在并发保存交错时会留下「短 JSON + 长旧内容
/// 残留」，产生 trailing characters 损坏；rename 保证磁盘上永远是某一刻的完整版本。
pub(super) async fn atomic_write<T: Serialize>(
    target: &Path,
    value: &T,
) -> Result<(), PluginError> {
    let json = serde_json::to_string(value)
        .map_err(|e| PluginError::InternalError(format!("序列化会话文件失败: {e}")))?;
    let tmp = tmp_path_for(target);
    tokio::fs::write(&tmp, json)
        .await
        .map_err(|e| PluginError::InternalError(format!("写入会话临时文件失败: {e}")))?;
    tokio::fs::rename(&tmp, target)
        .await
        .map_err(|e| PluginError::InternalError(format!("替换会话文件失败: {e}")))
}

/// 解析 session.json 内容；存在尾部残留时截取首个完整 JSON 自愈
/// （流式反序列化取首个完整对象），完全无法解析时返回 None。
pub(super) fn parse_meta_content(content: &str) -> Option<SessionMetaFile> {
    parse_or_self_heal::<SessionMetaFile>(content)
}

pub(super) fn parse_messages_content(content: &str) -> Option<Vec<ChatNode>> {
    parse_or_self_heal::<MessagesFile>(content).map(|f| f.messages)
}

/// 先整体解析，失败则退回「取首个完整对象」自愈
pub(super) fn parse_or_self_heal<T: serde::de::DeserializeOwned>(content: &str) -> Option<T> {
    match serde_json::from_str(content) {
        Ok(v) => Some(v),
        Err(_) => serde_json::Deserializer::from_str(content)
            .into_iter::<T>()
            .next()
            .and_then(Result::ok),
    }
}

/// 读消息——**真源是 `v2-events.wal`**（D-049）：
/// 有 WAL 就从它重建；WAL 不在才回落 `messages.json` / 元数据内联（v1 时代
/// 的遗留布局——那批会话的事实从未入格，文件是它们的唯一记录）。
pub(super) async fn load_messages(dir: &Path, meta: &SessionMetaFile) -> Vec<ChatNode> {
    if let Some(msgs) = rebuild_from_wal_dir(dir) {
        return msgs;
    }
    let path = dir.join(MESSAGES_FILE_NAME);
    if let Ok(content) = tokio::fs::read_to_string(&path).await {
        if let Some(msgs) = parse_messages_content(&content) {
            return msgs;
        }
    }
    meta.messages.clone().unwrap_or_default()
}

/// 从 `<会话目录>/v2-events.wal` 重建消息列表（真源读取）。
///
/// WAL 不在 / 打不开 ⇒ `None`（调用方回落遗留读法——**没有 WAL 不等于没有历史**，
/// v1 时代落的 `messages.json` 仍是那批会话的唯一记录）。只读打开：读路径无写副作用。
pub(super) fn rebuild_from_wal_dir(dir: &Path) -> Option<Vec<ChatNode>> {
    let path = dir.join(crate::plugins::session::paths::V2_WAL_FILE);
    if !path.exists() {
        return None;
    }
    let store = crate::plugins::session::store::EventWalStore::open_readonly(&path).ok()?;
    // `range` 是 `Store` trait 的方法：trait 必须在作用域里（局部引入，
    // 不为一次读取把这个 trait 抬进本文件的顶层导入）。
    use crate::symbio_core::Store as _;
    let snapshot = store.range(crate::symbio_core::Seq::new(0));
    Some(crate::symbio_core::rebuild(&snapshot))
}

/// 同步版读 `messages.json`
pub(super) fn read_messages_sync(dir: &Path) -> Option<Vec<ChatNode>> {
    let content = std::fs::read_to_string(dir.join(MESSAGES_FILE_NAME)).ok()?;
    parse_messages_content(&content)
}

/// 读 `session.json`（同步；用于 spawn_blocking 里的批量读取）
pub(super) fn read_meta(dir: &Path) -> Option<SessionMetaFile> {
    let content = std::fs::read_to_string(dir.join(SESSION_FILE)).ok()?;
    parse_meta_content(&content)
}

/// 读整个会话目录 = 元数据 + 消息（同步；嵌套查找的 miss 路径用）。
///
/// 消息同样以 WAL 为准（与 [`load_messages`] 同一条真源纪律），遗留读法是回落。
pub(super) fn read_session_dir(dir: &Path) -> Option<Session> {
    let meta = read_meta(dir)?;
    // 「两个来源都没有消息」= 空会话（不是「不存在」），别把「没有消息」当成「不存在」
    let messages = rebuild_from_wal_dir(dir)
        .or_else(|| read_messages_sync(dir))
        .or_else(|| meta.messages.clone())
        .unwrap_or_default();
    Some(meta.into_session(messages))
}

/// 嵌套查找：在所有顶层会话目录的 `sessions/<safe_id>/` 中定位子会话，
/// 返回其目录。仅在顶层未命中时调用（miss 路径，开销可接受）。
pub(super) fn find_nested_dir(base: &Path, session_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let dir = entry
            .path()
            .join(SUB_SESSIONS_DIR)
            .join(paths::safe_id(session_id));
        if dir.join(SESSION_FILE).is_file() {
            return Some(dir);
        }
    }
    None
}

/// 列出某目录下所有 `<dir>/<子>/session.json` 的**摘要**（跳过不可解析项）
///
/// 只读 `session.json`——这里正是「清单不碰消息」的落点。
pub(super) fn read_summaries_in(dir: &Path) -> Vec<SessionSummary> {
    let mut out = Vec::new();
    for child in session_dirs_under(dir) {
        if let Some(meta) = read_meta(&child) {
            out.push(meta.into_summary());
        }
    }
    out
}

/// 某目录下所有「含 session.json 的子目录」
pub(super) fn session_dirs_under(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(SESSION_FILE).is_file())
        .collect()
}

/// 递归收集所有会话目录（含嵌套子会话；迁移用——子会话也可能还是旧布局）
pub(super) fn session_dirs_deep(base: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for child in session_dirs_under(dir) {
            out.push(child.clone());
            walk(&child.join(SUB_SESSIONS_DIR), out);
        }
    }
    let mut out = Vec::new();
    walk(base, &mut out);
    out
}

/// 把一个旧布局目录（消息内联）拆成两个文件，并补写缺失的投影。幂等。
///
/// ## 迁移边界（[D-049]）
///
/// `messages.json` 已**不再是**布局的一部分：真源是 `v2-events.wal`，本函数只为
/// **v1 遗留目录**服务——那些把消息内联在 `session.json` 里、事实从未入格的会话。
/// 因此判据是「**有内联消息吗**」：没有内联消息的目录（新布局会话 / 已迁过的）
/// 一律跳过。少了这条，新会话会在首次迁移时被写出一份**空 `messages.json`**
/// ——把我们刚停写的缓存又凭空造出来（读侧虽已不读它，但凭空多一份空文件是布局污染）。
/// 存量补投影那半仍保留（`title` 空的旧文件要补）。
pub(super) async fn split_inline_messages(dir: &Path) -> Result<(), PluginError> {
    let Some(meta) = read_meta(dir) else {
        return Ok(());
    };
    let has_split = dir.join(MESSAGES_FILE_NAME).is_file();
    let inline = meta.messages.clone();
    // 已是新布局（消息文件在 / 内联已空）、且投影齐全 → 无需改动。
    //
    // 「有内联消息」是唯一的迁移触发条件：`messages.json` 停写后，一份**既无**
    // 消息文件、**又无**内联消息的目录就是新布局的正常态，绝不能因此补写空缓存。
    let needs_split = inline.as_ref().is_some_and(|m| !m.is_empty());
    let projection_missing = meta.title.is_empty();
    if !needs_split && !projection_missing {
        return Ok(());
    }

    let messages = if has_split {
        // 消息文件读不出来时**不猜**：宁可留着旧投影，也不要把空写进元数据
        match read_messages_sync(dir) {
            Some(m) => m,
            None => return Ok(()),
        }
    } else {
        inline.unwrap_or_default()
    };

    // 只在**确有内联消息要拆**时才落 `messages.json`（停写后不再凭空补一份空缓存）。
    if !has_split && needs_split {
        atomic_write(
            &dir.join(MESSAGES_FILE_NAME),
            &MessagesFile {
                messages: messages.clone(),
            },
        )
        .await?;
    }

    // 投影**必须**按现有消息重算：这才是「存量文件补投影」的落点
    let summary = SessionSummary::of(&Session {
        id: meta.id.clone(),
        messages,
        created_at: meta.created_at,
        updated_at: meta.updated_at,
        metadata: meta.metadata.clone(),
    });
    atomic_write(&dir.join(SESSION_FILE), &SessionMetaFile::of(&summary)).await
}

/// 清单排序：`updated_at` 降序（两种驻留方式共用同一份，清单顺序不分叉）
pub(super) fn sort_summaries_desc(summaries: &mut [SessionSummary]) {
    summaries.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
}

/// 原子写的临时文件路径：`<目标名>.<pid>.<单调序号>.tmp`
///
/// tmp 名**每次唯一**。早先它由调用方传固定名（`messages.json.tmp`），两次并发
/// 保存会交错写同一个 tmp：A 写完、B 覆盖同一 tmp，A 的 rename 便把 B 的内容放进
/// A 的目标文件——**rename 的原子性只保证「不写坏」，保证不了「写的是自己那份」**。
///
/// 序号取进程内原子计数（同进程两次写必然不同名），`pid` 兼顾多进程
/// （同一目录被两个实例打开时）。
pub(super) fn tmp_path_for(target: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = target
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{}.{n}.tmp", std::process::id()));
    target.with_file_name(name)
}
