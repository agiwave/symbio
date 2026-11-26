//! 「单文件长期记忆」的实现 —— 三层记忆**同一份口径**，归 `memory` 插件私有。
//!
//! > ⚠️ **与 `vdfs_service` 的「内存后端 VDFS」无关**：那个 `memory` 是「**内存**后端」
//! > 的 VDFS（运行期驻留、不落盘）；本模块的 `memory` 是「**长期记忆文件**」（落盘、
//! > 要给模型读写，文件名各层自定）。读代码时先看路径（本文件 vs `vdfs_service/memory.rs`）。
//!
//! ## 几层是同一件事的几个作用域
//!
//! | 层 | 读写面（VDFS 地址） | 注入者 | 物理落位 |
//! |---|---|---|---|
//! | 工作区 | memory（`<根>/memory/WORKSPACE.md`） | memory | `{work_dir}/MEMORY.md` |
//! | 会话 | memory（`<根>/memory/SESSION.md`） | memory | `{session_dir}/MEMORY.md` |
//! | 系统智能体 | memory（`<根>/memory/MEMORY.md`） | memory | `{agent_dir}/MEMORY.md` |
//! | 子智能体 | memory（`<根>/subagent/<id>/memory/MEMORY.md`） | memory | `<agentdir>/MEMORY.md` |
//!
//! 三层物理文件名**统一 `MEMORY.md`**：机制一致、可预测。挂载名与物理名
//! **刻意不同**——工作区腿叫 `WORKSPACE.md`、会话腿叫 `SESSION.md`、智能体腿叫
//! `MEMORY.md`，地址里已写作用域，列表里不会撞名（见 `crates/symbio-plugin-memory/src/workspace.rs`）。
//!
//! ⚠️ **指令文件 `AGENTS.md` 不在这里**：那是稳定指令 / 人格，由其它能力读取并注入，
//! 与本模块管的动态记忆语义隔离、物理隔离。
//!
//! 各层形态完全一样——**一个 UTF-8 文本文件、两道容量闸门、一行头信息的提示词片段、
//! 一个 VDFS 读写节点**。几份实现就是几份会各自漂移的口径：「写侧是拒绝还是截断」
//! 「读不到算不算错误」这类判断一旦分叉，用户看到的行为就会随「这条记忆属于哪一层」
//! 而变化，而用户根本无从知道差异从哪来。
//!
//! ## 为什么归在 memory 插件
//!
//! 判据是**「谁拥有它」**：三层记忆只有 `memory` 一个生产消费方，实现此前
//! 住在共享 provider 层只是让「唯一所有者」在编译图上不成立。现在实现与三层的落位
//! 规则同处一个 crate，文件名仍由各层自己定义（`memory::MEMORY_FILE` /
//! `workspace::WORK_MEMORY_FILE` / `session::SESSION_MEMORY_FILE`），本模块不认识它们。
//!
//! 它是**实现**而不是契约：没有任何一处需要 `dyn MemoryProvider`，硬抽 trait 只会把
//! 一次构造换成一次字符串查表。
//!
//! ## 边界：本模块收「**有地址、要限容**」的东西
//!
//! 判据是一条可检验的线：
//!
//! > 收「模型能自己改的东西」（要有地址、要限容）；
//! > 没有地址的只读片段走普通注册即可，硬塞进来只会多出一堆「可选字段」。
//!
//! ⚠️ 这条线划的是**形态**：`AGENTS.md` 曾以「可读写的智能体记忆」身份进过本模块
//! （地址 `<根>/memory/AGENTS.md`、设置页可编辑）。 之后职责归位——它是**稳定指令 /
//! 人格**，只读注入（两道闸门与地址对它不再适用），本模块只管三层的 `MEMORY.md` 动态记忆。
//! 反过来，没有地址的人格片段也永远不进来。
//!
//! ## 归属：一个作用域只有一个所有者
//!
//! **谁能读写它，谁负责注入它。** 这条原则消灭了两类问题：
//!
//! - **重叠**：同一份文件被两个插件同时读、同时注入（同一内容进两次上下文）；
//! - **模糊**：同一个文件被一处叫「指令」（只读）、另一处叫「记忆」（可写），
//!   用户看不出该往哪写。
//!
//! 因此三层的 `{work_dir}/MEMORY.md`、`{agent_dir}/MEMORY.md` 与
//! `{session_dir}/MEMORY.md` **都归 memory**——它同时给出读写面
//! （`<根>/memory/…`）与注入面，三个作用域同属一个所有者，印在片段里的地址与闸门
//! 都是**它自己会执行**的；`AGENTS.md` 两级指令另属只读注入能力，作用域信息由上下文
//! 按值递来，本模块不读任何指令文件。指令与记忆物理隔离、所有者不同。
//!
//! ## 个性不进来
//!
//! 本模块**不认识**「全局」「工作区」「会话」「智能体」这些词——它们只以 `path` /
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
//! ## 本模块**不认识**文件名
//!
//! 名字由**各层自己**给：本模块只拿到相对地址（`rel`），节点名从地址末段推导
//! （[`MemoryFile::file_name`]），**没有**任何硬编码的兜底文件名。 之后三层
//! 恰好都选了 `MEMORY.md`（统一是可预测性收益，不是本模块强制的）——但那三个字面量
//! 仍各定义在 memory 插件的三层模块里，本模块不共享、不假设它们同名。名字也不是
//! 「配置」：宿主不解析内容，只做三件事——**注入**、**限容**、**给地址**。

use std::path::Path;
use symbio_plugin_api::{
    text_floor_char_boundary, DynVdfsProvider, VdfsAccess, VdfsContent, VdfsContext, VdfsError,
    VdfsNode, VdfsRequest, VdfsResponse,
};

/// 注入用正文的产物 —— 「按预算截断」这一步的全部信息。
///
/// 四个字段都是片段需要的：正文（要贴的）、是否截断（要不要提示去读全文）、
/// 文件总字节数（头信息里的「当前」——**截断后的长度不是当前容量**）、
/// 本次预算（截断提示里报的那个数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInjection {
    /// 注入正文（已按 `budget_bytes` 截断）
    pub text: String,
    /// 是否因注入预算而截断
    pub truncated: bool,
    /// 记忆文件的**总**字节数
    pub total_bytes: u64,
    /// 本次使用的注入预算
    pub budget_bytes: usize,
}

impl MemoryInjection {
    /// 按字节预算截断一份文本，边界回退到最近的 UTF-8 字符位置。
    ///
    /// 按**字节**而非字符计预算：上下文窗口与磁盘都按字节算，按字符计会在多字节
    /// 文本上系统性低估占用。边界回退复用 `text_floor_char_boundary`
    /// （全项目唯一的「安全字节切片」实现，不在此另写一份）。
    ///
    /// `text_floor_char_boundary`
    pub fn cut(text: &str, budget_bytes: usize) -> Self {
        let end = text_floor_char_boundary(text, budget_bytes);
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
pub struct MemorySegmentSpec<'a> {
    /// 片段标题，渲染为 `【{title}】`（如「工作区记忆」）
    pub title: &'a str,
    /// 可编辑地址（如 `<根>/memory/WORKSPACE.md`）——**必须真实可达**
    pub address: &'a str,
    /// 附加说明（可选），如「与【工作区记忆】相互独立」
    pub note: Option<&'a str>,
    /// 内容为空时的一句话提示（渲染时自动加括号）
    pub empty_hint: &'a str,
}

/// VDFS 节点的规格
#[derive(Debug, Clone)]
pub struct MemoryNodeSpec<'a> {
    /// 展示标题
    pub title: &'a str,
    /// 节点名覆盖（`None` = 用真实文件名）。
    ///
    /// **什么时候需要覆盖**：同一挂载点里住两份记忆（智能体 / 工作区）而它们的
    /// 物理文件同名（都叫 `MEMORY.md`）时，挂载里的名字必须能区分「这份记忆是谁的」
    /// ——挂载名是模型与界面的寻址面，物理名是磁盘上的文件名，两者不必相同。
    pub name: Option<&'a str>,
    /// 场景类型（`kind` 只承载场景语义，不参与机制判定）
    pub kind: &'a str,
    /// 语义描述
    pub description: &'a str,
}

/// 一个作用域里的一个记忆文件。
///
/// `provider = None` 表示**该作用域不存在**（没选工作区 / 没选智能体）——这不是错误，
/// 是「这层记忆此刻不适用」。所有方法据此降级：读 → 空串，注入 → `None`，
/// 写 → 明确报错。作用域判断因此不需要在每个调用点重复一遍。
/// 记忆文件的存储后端：经作用域 provider 读写，**插件不拼物理路径**。
#[derive(Clone)]
struct MemoryBackend {
    provider: DynVdfsProvider,
    /// 相对地址（如 `MEMORY.md`），不含物理目录。
    rel: String,
}

#[derive(Clone)]
pub struct MemoryFile {
    backend: Option<MemoryBackend>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
}

impl MemoryFile {
    /// 由作用域 provider + 相对地址构造。
    /// `provider = None` = 作用域不存在（没选工作区 / 无会话）。
    pub fn via(
        provider: Option<DynVdfsProvider>,
        rel: impl Into<String>,
        write_max_bytes: usize,
        inject_max_bytes: usize,
    ) -> Self {
        Self {
            backend: provider.map(|provider| MemoryBackend {
                provider,
                rel: rel.into(),
            }),
            write_max_bytes,
            inject_max_bytes,
        }
    }

    /// 作用域是否存在（无作用域 = 这层记忆的一切读写都无从谈起）
    pub fn has_scope(&self) -> bool {
        self.backend.is_some()
    }

    /// 文件名 —— 节点名从这里来（**地址用真实文件名**，不另传一份字面量）。
    ///
    /// 无作用域 → `None`：没有文件，就没有名字。本模块**没有**兜底文件名
    /// （叫什么由各层自己定，见模块文档），所以这里如实返回 `None`。
    pub fn file_name(&self) -> Option<&str> {
        self.backend
            .as_ref()
            .and_then(|b| Path::new(&b.rel).file_name().and_then(|s| s.to_str()))
    }

    /// 记忆文件是否已存在
    pub async fn exists(&self) -> bool {
        match self.backend.as_ref() {
            Some(MemoryBackend { provider, rel }) => stat_vdfs(provider, rel).await.is_ok(),
            None => false,
        }
    }

    /// 记忆文件字节数（不存在 / 无作用域 → 0）
    pub async fn size(&self) -> u64 {
        match self.backend.as_ref() {
            Some(MemoryBackend { provider, rel }) => stat_vdfs(provider, rel)
                .await
                .ok()
                .and_then(|n| n.size)
                .unwrap_or(0),
            None => 0,
        }
    }

    /// 最后修改时间（Unix 秒；取不到 → `None`）
    pub async fn updated_at(&self) -> Option<i64> {
        match self.backend.as_ref() {
            Some(MemoryBackend { provider, rel }) => stat_vdfs(provider, rel)
                .await
                .ok()
                .and_then(|n| n.updated_at),
            None => None,
        }
    }

    /// 读记忆全文。
    ///
    /// 文件不存在 → **空串**（不是错误）：记忆是「可以还没有」的东西，把它当错误
    /// 会让每一次「首次读取」都变成异常路径。无作用域 → 明确报错。
    pub async fn read(&self) -> Result<String, String> {
        let Some(MemoryBackend { provider, rel }) = self.backend.as_ref() else {
            return Err("当前作用域没有记忆文件".to_string());
        };
        let vctx = VdfsContext::new(());
        match provider.dispatch(&vctx, rel, VdfsRequest::Read).await {
            Ok(resp) => Ok(resp.into_read().and_then(|c| c.text).unwrap_or_default()),
            // 「还没写过」不是故障：provider 报节点不存在 ⇒ 空串。否则每一次
            // 首次读取都会进收集期错误桶，工作区腿的空内容提示也永远出不来。
            Err(VdfsError::NotFound(_)) => Ok(String::new()),
            Err(e) => Err(format!("读取记忆失败：{e}")),
        }
    }

    /// 写记忆全文（Vdfs 后端的父目录由 provider 自动创建）。
    ///
    /// **容量闸门**：`write_max_bytes` 是硬限制，超出**直接拒绝**（见模块文档：
    /// 不截断、不部分写入——被拒绝的写入不得留下半截内容）。
    pub async fn write(&self, text: &str) -> Result<(), String> {
        let Some(MemoryBackend { provider, rel }) = self.backend.as_ref() else {
            return Err("当前作用域没有记忆文件，不可写".to_string());
        };
        if text.len() > self.write_max_bytes {
            return Err(format!(
                "记忆超出容量上限（{}）：当前 {} 字节，上限 {} 字节。\
                 请精简后再写入（可先读取现有内容，合并改写而不是整篇重写）。",
                rel,
                text.len(),
                self.write_max_bytes
            ));
        }
        let vctx = VdfsContext::new(());
        provider
            .dispatch(
                &vctx,
                rel,
                VdfsRequest::Write {
                    content: VdfsContent::text(text),
                },
            )
            .await
            .map_err(|e| format!("写入记忆失败：{e}"))?;
        Ok(())
    }

    /// 注入产物：按 `inject_max_bytes` 截断。无作用域 → `None`。
    ///
    /// `total_bytes` 取**读到的全文长度**而不是文件 metadata：与写入闸门同一把尺子
    /// （`write` 也是比 `text.len()`）。头信息里的「当前」因此就是「离被拒还有多远」，
    /// 不会出现「显示 8000 字节、再写一点就被拒」的错位。
    pub async fn inject(&self) -> Result<Option<MemoryInjection>, String> {
        if !self.has_scope() {
            return Ok(None);
        }
        let text = self.read().await?;
        Ok(Some(MemoryInjection::cut(&text, self.inject_max_bytes)))
    }

    /// 注入产物 → 提示词片段（无作用域 → `None`，即「这层不注入」）
    pub async fn segment(&self, spec: &MemorySegmentSpec<'_>) -> Result<Option<String>, String> {
        let Some(body) = self.inject().await? else {
            return Ok(None);
        };
        Ok(Some(memory_render_segment(
            spec,
            &body,
            self.write_max_bytes,
        )))
    }

    /// VDFS 节点 —— `list` 与 `stat` **共用同一份形状**，两条链路不会分叉。
    ///
    /// 节点名取**真实文件名**（`file_name`），因此只应在**有作用域**时调用：
    /// 无作用域没有文件，也就没有名字。三个插件的调用点都在 `has_scope()` 之后——
    /// 这里对无作用域退回空名，与 `instruction::host_dir` 对「取不到父目录」的处理
    /// 同一口径：理论上不该发生，但降级比 panic 好。
    pub async fn node(&self, spec: &MemoryNodeSpec<'_>) -> VdfsNode {
        let name = spec
            .name
            .map(String::from)
            .or_else(|| self.file_name().map(String::from))
            .unwrap_or_default();
        let mut n = VdfsNode::file(name, spec.title, VdfsAccess::READ_WRITE);
        n.kind = spec.kind.to_string();
        n.size = Some(self.size().await);
        n.updated_at = self.updated_at().await;
        n.description = Some(spec.description.to_string());
        n
    }
}

/// 经 provider 取节点元数据（`exists` / `size` / `updated_at` / `node` 共用）。
async fn stat_vdfs(provider: &DynVdfsProvider, rel: &str) -> Result<VdfsNode, String> {
    let vctx = VdfsContext::new(());
    let resp = provider
        .dispatch(&vctx, rel, VdfsRequest::Stat)
        .await
        .map_err(|e| format!("读取记忆元数据失败：{e}"))?;
    match resp {
        VdfsResponse::Stat(node) => Ok(node),
        _ => Err("provider 未返回 Stat 节点".to_string()),
    }
}

/// 渲染「一行头信息 + 正文」的记忆片段。
///
/// ```text
/// 【工作区记忆】（地址：<根>/memory/WORKSPACE.md，上限：16384字节，当前：111字节；改写前先 vdfs_read，合并后整篇 vdfs_write）
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
pub fn memory_render_segment(
    spec: &MemorySegmentSpec,
    body: &MemoryInjection,
    write_max_bytes: usize,
) -> String {
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
#[path = "memory_file_fixture.rs"]
pub(crate) mod fixture;

#[cfg(test)]
#[path = "memory_file.test.rs"]
mod tests;
