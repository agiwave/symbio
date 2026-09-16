//! 「单文件长期记忆」的共用内核 —— 三层记忆**同一份实现**。
//!
//! ## 三层是同一件事的三个作用域
//!
//! | 层 | 所有者插件 | 物理落位 | VDFS 地址 |
//! |---|---|---|---|
//! | 工作区 | work | `{workdir}/AGENTS.md` | `.vdfs/work/AGENTS.md` |
//! | 会话 | session | `{会话目录}/AGENTS.md` | `.vdfs/session/<id>/AGENTS.md` |
//! | 智能体 | agent | `{bundle 目录}/AGENTS.md` | `.vdfs/agent/<id>/AGENTS.md` |
//!
//! 三层形态完全一样——**一个 UTF-8 文本文件、两道容量闸门、一行头信息的提示词片段、
//! 一个 VDFS 读写节点**。三份实现就是三份会各自漂移的口径：「写侧是拒绝还是截断」
//! 「读不到算不算错误」这类判断一旦分叉，用户看到的行为就会随「这条记忆属于哪一层」
//! 而变化，而用户根本无从知道差异从哪来。
//!
//! ## 边界：**可编辑的记忆**进来，**只读的指令**不进来
//!
//! 还有一份 `{homedir}/AGENTS.md`（全局），它**不在**本内核里，因为它不是记忆：
//! 宿主只读它、不提供地址、不设容量、模型改不动它——它是「用户写给所有会话的规则」。
//! 判据因此是一条可检验的线：
//!
//! > 内核收「模型能自己改的东西」（要有地址、要限容）；
//! > 只读指令走普通片段即可，硬塞进来只会让内核多出一堆「可选字段」。
//!
//! ## 归属：一个作用域只有一个所有者
//!
//! **谁能读写它，谁负责注入它。** 这条原则消灭了两类问题：
//!
//! - **重叠**：同一份文件被两个插件同时读、同时注入（同一内容进两次上下文）；
//! - **模糊**：同一个文件被一处叫「指令」（只读）、另一处叫「记忆」（可写），
//!   用户看不出该往哪写。
//!
//! 因此 `{workdir}/AGENTS.md` **只归 work**——session 不再读它，哪怕它叫「指令」。
//!
//! ## 为什么是内核而不是 provider（trait）
//!
//! 「provider」的前提是**调用方需要多态**（运行时替换实现）：`VdfsProvider` 有
//! file / sqlite / memory 三后端，`ModelProvider` 有各家模型，它们必须是 trait。
//! 记忆不是——work / session / agent 在**编译期**就知道自己要用哪种记忆，
//! 全项目没有一处需要 `dyn MemoryProvider`。硬抽 trait 只会多一层间接。
//!
//! 所以这里给的是**值对象 + 纯函数**：内核负责「一个记忆文件怎么读写、怎么限容、
//! 怎么渲染成片段、长成什么节点」，插件负责「它是哪个作用域、落在哪、叫什么、
//! 地址是什么、闸门开多大」。
//!
//! ## 个性不进来
//!
//! 内核**不认识**「全局」「工作区」「会话」「智能体」这些词——它们只以 `path` /
//! `address` / `title` 的形式出现在参数里。凡是需要判断「这是哪一层」的逻辑，
//! 一律留在插件里。人格（`agent` 的多文件装配）也不进来：它不是单文件记忆。
//!
//! ## 两道闸门（三层统一）
//!
//! | 闸门 | 取值 | 位置 | 行为 |
//! |---|---|---|---|
//! | 写入 | `write_max_bytes` | [`MemoryFile::write`] | **拒绝**（不截断、不部分写入） |
//! | 注入 | `inject_max_bytes` | [`MemoryFile::inject`] | **截断** + 明确告知地址 |
//!
//! 写侧拒绝的理由：截断会让模型「以为写进去了、其实丢了一半」——静默丢内容是最坏的
//! 一类失败；拒绝把判断权交回模型（精简后重写，或改用追加式表达）。
//!
//! 读侧截断的理由：记忆文件可以比注入预算大（写入上限通常远大于注入预算），
//! 超出部分靠模型按地址 `vdfs_read` 读取。两者取值不同才有意义。
//!
//! ## 文件名的行业约定
//!
//! 三层都叫 `AGENTS.md`，**放在哪个作用域就管哪个作用域**。对齐行业惯例
//! （给编码智能体的指令 / 记忆文件，与 `CLAUDE.md`、`.cursorrules` 同一族）的收益是
//! 记忆**不属于 symbio**：能被 `git` 版本化、能被 review、换个工具照样生效。
//!
//! 它不是「配置」：宿主不解析内容，只做三件事——**注入**、**限容**、**给地址**。

use super::{VdfsAccess, VdfsNode};
use std::path::{Path, PathBuf};

/// 记忆文件名 —— 三层共用同一个名字（跨插件约定，见模块文档）
pub const AGENTS_FILE: &str = "AGENTS.md";

/// 注入用正文的产物 —— 「按预算截断」这一步的全部信息。
///
/// 四个字段都是片段需要的：正文（要贴的）、是否截断（要不要提示去读全文）、
/// 文件总字节数（头信息里的「当前」——**截断后的长度不是当前容量**）、
/// 本次预算（截断提示里报的那个数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectedMemory {
    /// 注入正文（已按 `budget_bytes` 截断）
    pub text: String,
    /// 是否因注入预算而截断
    pub truncated: bool,
    /// 记忆文件的**总**字节数
    pub total_bytes: u64,
    /// 本次使用的注入预算
    pub budget_bytes: usize,
}

impl InjectedMemory {
    /// 按字节预算截断一份文本，边界回退到最近的 UTF-8 字符位置。
    ///
    /// 按**字节**而非字符计预算：上下文窗口与磁盘都按字节算，按字符计会在多字节
    /// 文本上系统性低估占用。边界回退复用 [`floor_char_boundary`]
    /// （全项目唯一的「安全字节切片」实现，不在此另写一份）。
    ///
    /// [`floor_char_boundary`]: crate::symbio_core::floor_char_boundary
    pub fn cut(text: &str, budget_bytes: usize) -> Self {
        let end = crate::symbio_core::floor_char_boundary(text, budget_bytes);
        Self {
            text: text[..end].to_string(),
            truncated: end < text.len(),
            total_bytes: text.len() as u64,
            budget_bytes,
        }
    }
}

/// 片段排版的规格 —— **只有插件知道的东西**都在这里。
#[derive(Debug, Clone)]
pub struct SegmentSpec<'a> {
    /// 片段标题，渲染为 `【{title}】`（如「工作区记忆」）
    pub title: &'a str,
    /// 可编辑地址（如 `.vdfs/work/工作区/AGENTS.md`）——**必须真实可达**
    pub address: &'a str,
    /// 附加说明（可选），如「与【工作区记忆】相互独立」
    pub note: Option<&'a str>,
    /// 内容为空时的一句话提示（渲染时自动加括号）
    pub empty_hint: &'a str,
}

/// VDFS 节点的规格
#[derive(Debug, Clone)]
pub struct NodeSpec<'a> {
    /// 展示标题
    pub title: &'a str,
    /// 场景类型（`kind` 只承载场景语义，不参与机制判定）
    pub kind: &'a str,
    /// 语义描述
    pub description: &'a str,
}

/// 一个作用域里的一个记忆文件。
///
/// `path = None` 表示**该作用域不存在**（没选工作区 / 没选智能体）——这不是错误，
/// 是「这层记忆此刻不适用」。所有方法据此降级：读 → 空串，注入 → `None`，
/// 写 → 明确报错。作用域判断因此不需要在每个调用点重复一遍。
#[derive(Debug, Clone)]
pub struct MemoryFile {
    path: Option<PathBuf>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
}

impl MemoryFile {
    /// 由落位与两道闸门构造（`path = None` = 作用域不存在）
    pub fn new(path: Option<PathBuf>, write_max_bytes: usize, inject_max_bytes: usize) -> Self {
        Self {
            path,
            write_max_bytes,
            inject_max_bytes,
        }
    }

    /// 作用域不存在时的构造（无工作区 / 无智能体）
    pub fn absent(write_max_bytes: usize, inject_max_bytes: usize) -> Self {
        Self::new(None, write_max_bytes, inject_max_bytes)
    }

    /// 作用域是否存在（无作用域 = 这层记忆的一切读写都无从谈起）
    pub fn has_scope(&self) -> bool {
        self.path.is_some()
    }

    /// 记忆文件路径（无作用域 → `None`）
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 文件名（节点名从这里来——**地址用真实文件名**，不另传一份字面量）
    pub fn file_name(&self) -> &str {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .unwrap_or(AGENTS_FILE)
    }

    /// 写入闸门取值
    pub fn write_max_bytes(&self) -> usize {
        self.write_max_bytes
    }

    /// 注入闸门取值
    pub fn inject_max_bytes(&self) -> usize {
        self.inject_max_bytes
    }

    /// 记忆文件是否已存在
    pub fn exists(&self) -> bool {
        self.path.as_ref().is_some_and(|p| p.is_file())
    }

    /// 记忆文件字节数（不存在 / 无作用域 → 0）
    pub fn size(&self) -> u64 {
        self.path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// 最后修改时间（Unix 秒；取不到 → `None`）
    pub fn updated_at(&self) -> Option<i64> {
        self.path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
    }

    /// 读记忆全文。
    ///
    /// 文件不存在 → **空串**（不是错误）：记忆是「可以还没有」的东西，把它当错误
    /// 会让每一次「首次读取」都变成异常路径。无作用域 → 明确报错。
    pub fn read(&self) -> Result<String, String> {
        let Some(path) = self.path.as_ref() else {
            return Err("当前作用域没有记忆文件".to_string());
        };
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("读取记忆失败（{}）：{e}", path.display())),
        }
    }

    /// 写记忆全文（父目录自动创建）。
    ///
    /// **容量闸门**：`write_max_bytes` 是硬限制，超出**直接拒绝**（见模块文档：
    /// 不截断、不部分写入——被拒绝的写入不得留下半截内容）。
    pub fn write(&self, text: &str) -> Result<(), String> {
        let Some(path) = self.path.as_ref() else {
            return Err("当前作用域没有记忆文件，不可写".to_string());
        };
        if text.len() > self.write_max_bytes {
            return Err(format!(
                "记忆超出容量上限（{}）：当前 {} 字节，上限 {} 字节。\
                 请精简后再写入（可先读取现有内容，合并改写而不是整篇重写）。",
                path.display(),
                text.len(),
                self.write_max_bytes
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建记忆目录失败（{}）：{e}", parent.display()))?;
        }
        std::fs::write(path, text).map_err(|e| format!("写入记忆失败（{}）：{e}", path.display()))
    }

    /// 注入产物：按 `inject_max_bytes` 截断。无作用域 → `None`。
    ///
    /// `total_bytes` 取**读到的全文长度**而不是文件 metadata：与写入闸门同一把尺子
    /// （`write` 也是比 `text.len()`）。头信息里的「当前」因此就是「离被拒还有多远」，
    /// 不会出现「显示 8000 字节、再写一点就被拒」的错位。
    pub fn inject(&self) -> Result<Option<InjectedMemory>, String> {
        if !self.has_scope() {
            return Ok(None);
        }
        let text = self.read()?;
        Ok(Some(InjectedMemory::cut(&text, self.inject_max_bytes)))
    }

    /// 注入产物 → 提示词片段（无作用域 → `None`，即「这层不注入」）
    pub fn segment(&self, spec: &SegmentSpec) -> Result<Option<String>, String> {
        let Some(body) = self.inject()? else {
            return Ok(None);
        };
        Ok(Some(render_segment(spec, &body, self.write_max_bytes)))
    }

    /// VDFS 节点 —— `list` 与 `stat` **共用同一份形状**，两条链路不会分叉。
    pub fn node(&self, spec: &NodeSpec) -> VdfsNode {
        let mut n = VdfsNode::file(self.file_name(), spec.title, VdfsAccess::READ_WRITE);
        n.kind = spec.kind.to_string();
        n.size = Some(self.size());
        n.updated_at = self.updated_at();
        n.description = Some(spec.description.to_string());
        n
    }
}

/// 渲染「一行头信息 + 正文」的记忆片段。
///
/// ```text
/// 【工作区记忆】（地址：.vdfs/work/工作区/AGENTS.md，上限：16384字节，当前：111字节；改写前先 vdfs_read，合并后整篇 vdfs_write）
/// <记忆正文>
/// ```
///
/// ## 为什么这么短
///
/// 这段文字**每一轮**都会进上下文，因此按「token 预算」而不是「文档完备性」设计：
///
/// - 头信息只有一行，四要素按重要性排序：**地址**（能改）、**上限**（写超会被拒）、
///   **当前**（还剩多少余量）、**写法**（先读后写，否则整篇覆盖）；
/// - 不写「这是什么」「该写什么」这类解释——`【工作区记忆】` 五个字已经说清了，
///   再展开就是每轮都在烧 token；
/// - 正文放在头信息**之后**：模型先拿到「去哪改、能写多大」，再读内容。
///
/// ## 两个附加提示各占一行
///
/// 内容为空 → `empty_hint`（此时最需要「你可以往里写」）；被截断 → 明确告知截到多少
/// 字节并指路读全文。**空内容不得谎报截断**，截断也不得沉默。
pub fn render_segment(spec: &SegmentSpec, body: &InjectedMemory, write_max_bytes: usize) -> String {
    let mut out = String::with_capacity(body.text.len() + 256);

    out.push_str(&format!(
        "【{}】（地址：{}，上限：{}字节，当前：{}字节",
        spec.title, spec.address, write_max_bytes, body.total_bytes
    ));
    if let Some(note) = spec.note.filter(|n| !n.is_empty()) {
        out.push('；');
        out.push_str(note);
    }
    out.push_str("；改写前先 vdfs_read，合并后整篇 vdfs_write）\n");

    if body.text.trim().is_empty() {
        out.push_str(&format!("（{}）\n", spec.empty_hint));
    } else {
        out.push_str(body.text.trim_end());
        out.push('\n');
    }

    if body.truncated {
        out.push_str(&format!(
            "（已截断至 {} 字节，完整内容用 vdfs_read 读取）\n",
            body.budget_bytes
        ));
    }

    out
}

#[cfg(test)]
#[path = "memory.test.rs"]
mod tests;
