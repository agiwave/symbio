//! 核心 VdfsProvider —— 统一资源访问的唯一契约（纯 object-safe trait）。
//!
//! 本模块是 VDFS 的 **centerpiece**，只暴露**纯接口**：
//! - [`VdfsProvider`] 一个 trait 收拢全部资源操作（列 / 读 / 写 / 删 / 建 / 移 / 订阅）；
//! - 每个资源域 = 一份 [`VdfsProvider`] 实现，`Arc<dyn VdfsProvider>` 是使用方与
//!   实现方之间**唯一**的交换物；
//! - 线上形状（`vdfs/*` 请求 / 响应信封、协议路径常量）定义在 vdfs 插件内部
//!   （`plugins/vdfs/protocol.rs`），**core 不暴露这些类型**——与
//!   [`crate::symbio_core::model_provider`] 的组织方式一致。
//!
//! ## 组合根不属于 provider
//!
//! **provider 完全不知道自己被放在哪层目录下**。目录树的组织是**使用方**的概念：
//! 谁用它、谁决定它在目录树上的名字。因此：
//!
//! - trait 上没有 `name()` 之类的方法——provider 不管理也不提供自己的目录名；
//! - provider 的每个方法只接收**本子树内的相对路径**（`""` = 自身根），
//!   已由使用方完成规范化与穿越校验；
//! - 全路径（`<目录>/<rel…>`）由使用方拼接、回填。
//!
//! ## 依赖方向
//!
//! - **实现方**（如 `setting` 插件）实现本 trait，在 `traverse` 广播中经
//!   `CapabilityVisitor::register_vdfs_provider(目录名, provider)` 注册自身。
//!   **目录名由使用方选定**，约定用插件名（`PLUGIN_*` 常量）——插件名在宿主内
//!   唯一，天然就是合格的目录名；
//! - **`composite` 容器**只负责**目录合成**：它把各子插件注册的 provider 按
//!   注册名铺成一级子目录（`plugins/composite/vdfs.rs`）。它自身不是「根级
//!   provider」——机制里没有根级 provider 这个概念，`.vdfs` 只是使用方在合成
//!   视图之上取的展示地址；
//! - **`vdfs` 插件**取容器注册的根，按 `vdfs/*` 协议分发（前端与 LLM 走同一条
//!   分发链路，不存在第二套实现）；
//! - 机制只认 [`VdfsAccess`] 的四个访问位（`r` / `w` / `l` / `t`），不做任何
//!   按类型的特判——这是 VDFS 保持通用的根基。
//!
//! ## 开放边界
//!
//! 本模块**只依赖** `std` / `serde` / `serde_json` / `async_trait`，不引用任何
//! 宿主专有类型（宿主运行时状态经 [`VdfsContext`] 不透明注入），因此可原样抽出
//! 为独立 crate 供任何宿主复用。symbio 侧的接线（上下文注入 + 错误翻译）在
//! [`crate::symbio_core::vdfs::host`]。
//!
//! 数据模型速览：
//!
//! - 一切资源 = 目录树上的**节点**（[`VdfsNode`]），地址 = 树内相对路径
//!   `<目录>/<rel>`（宿主门面负责把它映射成对外展示地址，见 plugins/vdfs/fs.rs）；
//! - 节点的能力 = 四个**访问位**（[`VdfsAccess`]：`r` 读 / `w` 写 / `l` 列 / `t` 遍历）；
//! - 内容 = [`VdfsContent`]（文本 `text` 或二进制 `b64`，互斥）；
//! - 呈现 = 节点的 `ext`（扩展名）→ 使用方选渲染器；渲染器所需描述经 `schema` 透传；
//! - 变更 = [`VdfsChange`]（子树内**相对路径** + **按类型可选的载荷**：
//!   `appended` 带增量 `delta`，`created` / `updated` 可带节点视图与内容快照；
//!   经 [`VdfsChangeSink`] 由使用方补成展示地址后投递）。

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::any::Any;
use std::sync::Arc;

// ==================== 状态取值 ====================

pub const VDFS_STATUS_ACTIVE: &str = "active";
pub const VDFS_STATUS_WORKING: &str = "working";
pub const VDFS_STATUS_DISABLED: &str = "disabled";
/// **以错误结束** —— 节点存在，但上一次运行失败了。
///
/// 与 [`VDFS_STATUS_ACTIVE`]（就绪）并列的一个**真实状态**，不是标志位：
/// 「会话上次失败了」= `status == failed`，而不是「状态 + `last_failed` 布尔」——
/// 后者要求读状态的人同时读两个字段，漏读一处就静默错。
///
/// 词面与消息层的 `MessageStatus::Failed` 一致：同一个概念在两类节点上不换词。
pub const VDFS_STATUS_FAILED: &str = "failed";
pub const VDFS_STATUS_UNKNOWN: &str = "unknown";
/// **无运行状态** —— 显式声明「本节点没有会变化的状态」。
///
/// 与 [`VDFS_STATUS_ACTIVE`]（就绪，一个**真实**状态）不同，本值表示**不适用**：
/// 静态资源（如设置分区）本来就没有「运行中 / 就绪」可言，给它画一个状态点
/// 只是噪音。列表据此**不渲染状态点**（见 `docs/design/vdfs-frontend.md` §4.2）。
///
/// 缺省仍是 `active`（见 [`default_status`]）：只有**显式**声明本值的节点才会
/// 失去状态点，因此这是「声明出来的无状态」，不是「忘了填」。
pub const VDFS_STATUS_NONE: &str = "";

/// 节点基础类型：目录
pub const VDFS_KIND_DIR: &str = "dir";
/// 节点基础类型：文件
pub const VDFS_KIND_FILE: &str = "file";

/// 场景类型：会话的**转写列表**（`.vdfs/session/<id>/<段>`）。
///
/// `kind` 是场景可自定义的（会话叶子自己就声明 `kind = "session"`），
/// 这里给转写列表一个**稳定的 ASCII 语义类型**：它的 `name` / `title` 是
/// 面向用户的展示名（可能随语言或文案调整），不能被消费者当成标识来认；
/// 而 `kind` 是**协议词**——消费者按它发现「哪个子目录是转写」，不必硬编码段名。
pub const VDFS_KIND_MESSAGES: &str = "messages";

// ==================== 呈现扩展名（约定，宿主可自行扩展） ====================
//
// 节点 `ext` 是宿主选择详情呈现方式的键。VDFS 只透传、不解释；
// 以下是**约定俗成**的几个取值，宿主可自由增添自己的扩展名。

/// 定义驱动表单（呈现描述放 `node.schema`）
pub const VDFS_EXT_FORM: &str = "form";
/// 会话工作区（实时对话流）
pub const VDFS_EXT_SESSION: &str = "session";
/// 单条对话消息（**列表项**：正文在内容里，结构在 `attributes` 里）
pub const VDFS_EXT_MESSAGE: &str = "message";
/// 纯文本编辑器
pub const VDFS_EXT_TEXT: &str = "text";
/// JSON 编辑器
pub const VDFS_EXT_JSON: &str = "json";
/// Markdown 编辑器
pub const VDFS_EXT_MARKDOWN: &str = "md";
/// 文件树（目录节点的默认呈现）
pub const VDFS_EXT_DIR: &str = "dir";
/// 整包（zip）——**导入**用扩展名：内容是一整个资源目录的压缩包
pub const VDFS_EXT_ZIP: &str = "zip";

// ==================== 节点动作（约定） ====================

/// 节点动作标识：**连接测试**（`vdfs/action` 的 `action` 取值之一）。
///
/// 动作标识由 provider 自持，VDFS 只透传、不解释（与 `ext` 同构）。此处登记的
/// 是当前的内置约定：
///
/// - [`VDFS_ACTION_TEST`]「测试连接」——模型 / MCP 这类外部资源的连通性自检；
/// - [`VDFS_ACTION_EXPORT`]「导出」——把整目录资源打包成一个 zip（结果随
///   [`VdfsActionResult::data`] 返回，与导入的二进制写入互为逆向）；
/// - [`VDFS_ACTION_TRUNCATE`] / [`VDFS_ACTION_CLEAR`]——列表类资源的**区间删除**：
///   前者删「该条及其之后」，后者清空整个列表。
///
/// ## 为什么「截断 / 清空」是动作而不是 `delete`
///
/// [`VdfsProvider::delete`] 的全局语义是「**这一个**节点没了」，对应
/// [`VDFS_CHANGE_DELETED`]。
///
/// - 「从这里到末尾全没了」需要**自己的**变更值 [`VDFS_CHANGE_TRUNCATED`]：
///   若按 `deleted` 逐条下发，消费者既分不清「删这一个」与「从这里删到末尾」，
///   代价也随被删条数线性增长（见该常量的文档）。
/// - 「整个列表空了」**不需要**新值：它落在**列表目录**这个地址上，`deleted`
///   在此处只有一种读法（目录没了 ⇒ 里面的条目都没了），地址本身已把语义定死。
///
/// 所以判据不是「每种删除各配一个值」，而是**「`deleted` 在该地址上仍有歧义的，
/// 才配自己的值」**——`truncated` 是这种情况，清空不是。会话 provider 的清空
/// 因此发的是落在 `<id>/消息` 上的 `deleted`（见 `SessionPlugin::
/// emit_transcript_cleared`）。
///
/// 那为什么后两者仍不做成 `delete`？因为 `delete` 的语义是**逐节点**的：拿它
/// 表达「删一个节点却删掉了它后面所有」会成为一条**没人能预期的默认行为**；
/// 而拿 `cascade: bool` 之类的附加位区分，则让「是哪种删除」变成两个字段必须
/// 一起读。动作是 provider 自持的动词，正好承载这类**集合操作**：VDFS 只透传，
/// 不解释。
pub const VDFS_ACTION_TEST: &str = "test";
/// 节点动作标识：**导出**（打包下载；与「新建类型 `zip`」的导入互为逆向）
pub const VDFS_ACTION_EXPORT: &str = "export";
/// 节点动作标识：**截断**（列表资源：删除该条目**及其之后**的全部条目）。
///
/// 结果里带被删条目的 id 列表（随 [`VdfsActionResult::data`]）——消费方用它做
/// 幂等对齐：本地若因锚点缺失而删窄了，据权威列表补齐。
pub const VDFS_ACTION_TRUNCATE: &str = "truncate";
/// 节点动作标识：**清空**（列表资源：保留容器本身，清掉全部条目）
pub const VDFS_ACTION_CLEAR: &str = "clear";

// ==================== 可接受的新建类型 ====================

/// 目录**可接受的新建元素类型**——「新建」入口的类型清单元素。
///
/// 一个目录（含 provider 根）声明自己能新建哪些类型；使用方据此决定
/// 是否显示「添加」入口、以及是否先弹类型选择：
///
/// - 清单非空 → 显示添加入口；
/// - 多于一项 → 先选类型再命名；
/// - 为空 → 不显示添加入口。
///
/// ## 两条独立的键：`ext` 与 `node_ext`
///
/// [`ext`](Self::ext) 是**呈现扩展名**——地址末段可能带的后缀，provider 用
/// [`entry::id_of`] 一族按它剥出条目 id（`.vdfs/model/openai-1.model` → `openai-1`）。
/// 它**不**决定详情怎么渲染：配置型资源（`model` / `mcp` / `skill`）落成后统一是
/// `ext = form`，呈现扩展名只留在地址里。
///
/// [`node_ext`](Self::node_ext) 才是**新元素落成后的 [`VdfsNode::ext`]**（详情渲染器键）。
/// 缺省 = 用 `ext`——会话这类「呈现扩展名就是渲染器键」的资源不必声明。
///
/// 两个键分开声明，是为了让使用方在**还没创建**时就能渲染出该类型的详情页
/// （草稿节点：无 id、无名字，但渲染器与 `schema` 与落成后完全一致）。
///
/// ## 内容来源 [`VdfsNewType::source`]
///
/// 「新建」在机制上就是一次 [`VdfsProvider::write`]（`create: true`），因此要说清
/// **写进去的内容从哪来**——这是创建语义的一部分，由 provider 声明：
///
/// - `None`（默认）：在详情页里边看边填（先进入草稿详情，保存时一次写入）；
/// - [`VDFS_NEW_SOURCE_FILE`]：内容取自**本地文件**，使用方给文件选择器，
///   字节走 [`VdfsContent::b64`] 二进制通道（如 zip 整包导入）。
///
/// 本结构是**纯呈现元数据**：VDFS 只透传、不解释；具体创建语义由 provider 在
/// [`VdfsProvider::write`] 中自持。
///
/// [`VdfsProvider::write`]: VdfsProvider::write
/// [`VdfsContent::b64`]: VdfsContent::b64
/// [`entry::id_of`]: crate::providers::vdfs_service::entry::id_of
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsNewType {
    /// 新元素**呈现扩展名**（地址末段后缀，`id_of` 按它剥 id；**不是**渲染器键）
    pub ext: String,
    /// 展示标题（如「会话」「模型」）
    pub title: String,
    /// 语义说明（缺省不显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 图标名（使用方纯 UI 映射）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 内容来源（见结构文档）：`None` = 在详情页里填；`"file"` = 选择本地文件
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 新元素落成后的 [`VdfsNode::ext`]（**详情渲染器键**）；缺省 = 与 [`ext`](Self::ext) 相同
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_ext: Option<String>,
    /// 新元素的呈现描述（与 [`VdfsNode::schema`] 同义）。
    ///
    /// 由 provider 下发——草稿详情页据此渲染出与落成后**同一张**表单。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

impl VdfsNewType {
    /// 仅 ext + title 的最小构造
    pub fn new(ext: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            ext: ext.into(),
            title: title.into(),
            description: None,
            icon: None,
            source: None,
            node_ext: None,
            schema: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// 新元素落成后的呈现扩展名（详情渲染器键）——与 [`ext`](Self::ext) 不同的资源必填
    pub fn with_node_ext(mut self, node_ext: impl Into<String>) -> Self {
        self.node_ext = Some(node_ext.into());
        self
    }

    /// 新元素的呈现描述（草稿详情页据此渲染出与落成后同一张详情）
    pub fn with_schema(mut self, schema: Value) -> Self {
        self.schema = Some(schema);
        self
    }
}

/// 新建内容来源：**本地文件**（[`VdfsNewType::source`] 的取值之一）。
///
/// 声明它的类型意味着「新建 = 选一个本地文件，把它的字节写进目标地址」。
pub const VDFS_NEW_SOURCE_FILE: &str = "file";

// ==================== 访问位 ====================

/// 节点访问位：`r` 读 / `w` 写 / `l` 列表 / `t` 树状遍历。
///
/// 线上表示为紧凑字符串（按 `r` `w` `l` `t` 顺序拼接，缺位即无该能力）：
/// `"rl"` = 可读 + 可列；`"rw"` = 可读可写；`"wlt"` = 可写可列可遍历；`""` = 不可访问。
///
/// 机制与消费者**只认访问位**，不做任何按类型的特判——这是 VDFS 保持通用的根基。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct VdfsAccess {
    /// `r`：可读内容
    pub read: bool,
    /// `w`：可写内容
    pub write: bool,
    /// `l`：可列出直接子节点
    pub list: bool,
    /// `t`：可树状递归遍历
    pub traverse: bool,
}

impl VdfsAccess {
    pub const NONE: Self = Self {
        read: false,
        write: false,
        list: false,
        traverse: false,
    };
    /// 只读文件（`r`）
    pub const READ: Self = Self {
        read: true,
        ..Self::NONE
    };
    /// 可读写文件（`rw`）
    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        ..Self::NONE
    };
    /// 只读目录（`l`）
    pub const LIST: Self = Self {
        list: true,
        ..Self::NONE
    };
    /// 只读目录 + 树状遍历（`lt`）
    pub const LIST_TRAVERSE: Self = Self {
        list: true,
        traverse: true,
        ..Self::NONE
    };
    /// 可读写目录 + 树状遍历（`lwt`）
    pub const LIST_WRITE_TRAVERSE: Self = Self {
        list: true,
        write: true,
        traverse: true,
        ..Self::NONE
    };

    /// 目录（可选择可写 / 可遍历）
    pub fn dir(write: bool, traverse: bool) -> Self {
        Self {
            list: true,
            write,
            traverse,
            read: false,
        }
    }

    /// 文件（可选择可写）
    pub fn file(write: bool) -> Self {
        Self {
            read: true,
            write,
            list: false,
            traverse: false,
        }
    }

    /// 紧凑表示（`r` `w` `l` `t` 顺序）
    pub fn flags(&self) -> String {
        let mut s = String::with_capacity(4);
        if self.read {
            s.push('r');
        }
        if self.write {
            s.push('w');
        }
        if self.list {
            s.push('l');
        }
        if self.traverse {
            s.push('t');
        }
        s
    }

    /// 从紧凑表示解析（忽略顺序与未知字符）
    pub fn parse(s: &str) -> Self {
        let mut a = Self::NONE;
        for c in s.chars() {
            match c {
                'r' | 'R' => a.read = true,
                'w' | 'W' => a.write = true,
                'l' | 'L' => a.list = true,
                't' | 'T' => a.traverse = true,
                _ => {}
            }
        }
        a
    }

    /// 是否含全部给定位
    pub fn contains(&self, other: Self) -> bool {
        (!other.read || self.read)
            && (!other.write || self.write)
            && (!other.list || self.list)
            && (!other.traverse || self.traverse)
    }

    /// 是否无任何能力
    pub fn is_none(&self) -> bool {
        !self.read && !self.write && !self.list && !self.traverse
    }
}

impl Serialize for VdfsAccess {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.flags())
    }
}

impl<'de> Deserialize<'de> for VdfsAccess {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(d)?))
    }
}

// ==================== 节点 ====================

/// 虚拟文件系统节点（文件或目录）。
///
/// 目录与文件共用同一结构：由 [`VdfsAccess`] 的 `l`（可列）与 `r`（可读）区分形态；
/// `kind` 只承载**场景语义**（如 `session` / `model`），不参与机制判定。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsNode {
    /// 全路径（**展示口径**，如 `.vdfs/session/abc`）；由分发层回填，provider 可留空
    #[serde(default)]
    pub path: String,
    /// 唯一标识：父节点内的路径段
    pub name: String,
    /// 展示标题（给人 / 给 LLM 看）
    #[serde(default)]
    pub title: String,
    /// 语义描述
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 场景类型（`dir` / `file` / `mount` / 场景自定义）
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 运行状态（`active` / `working` / `disabled` / `error` / `unknown`）
    #[serde(default = "default_status")]
    pub status: String,
    /// 访问位（`r` / `w` / `l` / `t`）
    pub access: VdfsAccess,
    /// 扩展名 —— **宿主选择详情呈现方式的键**。
    /// 缺省由 `name` 的字面扩展名推导（`config.json` → `json`）；
    /// provider 可显式覆盖（如设置分区的 `form`、会话工作区的 `session`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    /// 字节数（文件）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// 最后更新时间（**Unix 时间戳，秒或毫秒都可**）
    ///
    /// 机制不规定精度：物理层用秒（文件系统 `mtime`），会话层用毫秒
    /// （`session/update` 的时间戳口径）。消费者按量级判别
    /// （`ts < 1e12` 视为秒），前端 `relativeTime()` 即此规则。
    /// 这里不做归一——归一需要精度信息，而字段只有一个整数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 直接子节点数量（目录）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<u32>,
    /// 内容是否为二进制（文件）
    #[serde(default)]
    pub binary: bool,
    /// **隐藏属性**：是否在父目录的列表里隐藏（文件 / 目录通用）
    ///
    /// 与文件系统的隐藏属性同义，是**机制级的节点属性**——任何节点都可以带，
    /// 与它是不是目录、属于哪个场景无关（provider 的根也不过是一个目录节点）。
    ///
    /// 语义边界只有两条：
    /// - **列表里不出现**：父目录 `list` 的结果不含它（目录的 `children` 计数同理）；
    /// - **可达性不受影响**：按路径 `stat` / `read` / `write` / 子树操作一概照常，
    ///   因此「隐藏」既不是权限，也不是卸载。
    ///
    /// 典型用法：没有用户资源、只有一份配置文档的目录不必出现在导航列表里。
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// 自描述呈现描述（**宿主方言，VDFS 只透传**）。
    ///
    /// 约定：当 `ext` 命中某个需要结构描述（如表单）的渲染器时，渲染器所需数据
    /// 由 provider 放在这里；不关心呈现的消费者忽略即可。VDFS 不解释其内容，
    /// 因此接入方可以自由定义（JSON Schema、宿主自有表单定义……皆可）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    /// 本目录可接受的新建元素类型（「新建」入口的类型清单）。
    ///
    /// 空（缺省）= 不可新建；非空 → 使用方显示添加入口（多于一项时先选类型）。
    /// 仅目录节点有意义；文件节点恒为空。纯呈现元数据，VDFS 不解释其创建语义。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_types: Vec<VdfsNewType>,
    /// 场景扩展字段（flatten）
    #[serde(flatten)]
    pub attributes: serde_json::Map<String, Value>,
}

fn default_kind() -> String {
    VDFS_KIND_FILE.to_string()
}

fn default_status() -> String {
    VDFS_STATUS_ACTIVE.to_string()
}

impl Default for VdfsNode {
    fn default() -> Self {
        Self {
            path: String::new(),
            name: String::new(),
            title: String::new(),
            description: None,
            kind: default_kind(),
            status: default_status(),
            access: VdfsAccess::NONE,
            ext: None,
            size: None,
            updated_at: None,
            children: None,
            binary: false,
            hidden: false,
            schema: None,
            new_types: Vec::new(),
            attributes: serde_json::Map::new(),
        }
    }
}

impl VdfsNode {
    /// 目录节点（`l`，可按需 `w` / `t`）
    pub fn dir(name: impl Into<String>, title: impl Into<String>, access: VdfsAccess) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            kind: VDFS_KIND_DIR.to_string(),
            access,
            ..Default::default()
        }
    }

    /// 文件节点（`r`，可按需 `w`）
    pub fn file(name: impl Into<String>, title: impl Into<String>, access: VdfsAccess) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            kind: VDFS_KIND_FILE.to_string(),
            access,
            ..Default::default()
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// 显式指定呈现扩展名（覆盖 `name` 推导）
    pub fn with_ext(mut self, ext: impl Into<String>) -> Self {
        self.ext = Some(ext.into());
        self
    }

    /// 附带呈现描述（宿主方言，VDFS 透传）
    pub fn with_schema(mut self, schema: Value) -> Self {
        self.schema = Some(schema);
        self
    }

    /// 声明本目录可接受的新建类型（整表替换）
    pub fn with_new_types(mut self, new_types: Vec<VdfsNewType>) -> Self {
        self.new_types = new_types;
        self
    }

    /// 追加一个可接受的新建类型
    pub fn with_new_type(mut self, new_type: VdfsNewType) -> Self {
        self.new_types.push(new_type);
        self
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = kind.into();
        self
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    pub fn with_children(mut self, n: u32) -> Self {
        self.children = Some(n);
        self
    }

    pub fn with_binary(mut self, binary: bool) -> Self {
        self.binary = binary;
        self
    }

    /// 写入一个场景扩展字段
    pub fn with_attribute(mut self, key: impl Into<String>, value: Value) -> Self {
        self.attributes.insert(key.into(), value);
        self
    }

    /// 是否为目录形态（机制判定只看访问位）
    pub fn is_dir(&self) -> bool {
        self.access.list
    }

    /// 生效的呈现扩展名：显式 `ext` 优先，否则由 `name` 的字面扩展名推导
    pub fn effective_ext(&self) -> Option<String> {
        self.ext
            .clone()
            .filter(|e| !e.trim().is_empty())
            .or_else(|| derive_ext(&self.name))
    }
}

/// 由名字推导扩展名（`prompts/a.md` → `md`；无扩展名 → `None`）
pub fn derive_ext(name: &str) -> Option<String> {
    let base = name.rsplit('/').next().unwrap_or(name);
    let (stem, ext) = base.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

// ==================== 内容 ====================

/// 节点内容（文本或二进制，二者互斥）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsContent {
    /// 全路径
    #[serde(default)]
    pub path: String,
    /// 文本内容（`binary == false`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// base64 内容（`binary == true`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b64: Option<String>,
    /// 是否二进制
    #[serde(default)]
    pub binary: bool,
    /// 字节数
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// 内容版本（乐观并发令牌；provider 可选实现）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// **写意图**：允许创建缺失的节点（`vdfs/write` 的 `create` 位）。
    ///
    /// provider 据此区分「新建」与「覆盖」——规范 §5.3 的「新建」正是
    /// 「对目标地址的一次 `vdfs/write`（`create: true`）」，创建语义（生成
    /// 标识、校验归属……）由 provider 自持。读取结果恒为 `false`。
    ///
    /// ⚠️ 它只回答「目标不存在时怎么办」，**不改变内容的处理方式**：内容一律取自
    /// 本次写入（见 [`VdfsProvider::write`] 的 `create` 位一节）。唯一例外是内容为空
    /// ——那是「先建一个，随后再填」，由 provider 落最小合法内容。
    ///
    /// 具名目标 + 不存在：**写入型资源应就地创建**（「给了名字就写得进去」），
    /// 只有「更新既有对象的字段」型语义才报 [`VdfsError::NotFound`]。
    ///
    /// 写**目录自身**（无名字，见 [`VdfsProvider::write`]）时本字段是唯一判据：
    /// 那种写没有任何「已存在的目标」可覆盖，`false` 只能报错。
    #[serde(default, skip_serializing_if = "is_false")]
    pub create: bool,
}

/// serde 辅助：`false` 不序列化（保持线上形状与「缺省即 false」的字段兼容）
///
/// 用于 [`VdfsNode::hidden`] 与 [`VdfsContent::create`] 这类**绝大多数情况为
/// false** 的布尔位——省掉它们能让既有消费者的 JSON 形状一字不变。
fn is_false(b: &bool) -> bool {
    !*b
}

impl VdfsContent {
    /// 文本内容
    pub fn text(path: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            path: path.into(),
            size: text.len() as u64,
            text: Some(text),
            ..Default::default()
        }
    }

    /// 二进制内容（base64）
    pub fn binary(path: impl Into<String>, b64: impl Into<String>, size: u64) -> Self {
        Self {
            path: path.into(),
            b64: Some(b64.into()),
            binary: true,
            size,
            ..Default::default()
        }
    }

    pub fn with_mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    pub fn with_etag(mut self, etag: impl Into<String>) -> Self {
        self.etag = Some(etag.into());
        self
    }

    /// 标记为「新建」写意图（允许创建缺失节点）
    pub fn with_create(mut self) -> Self {
        self.create = true;
        self
    }

    /// 取文本视图（二进制返回 `None`）
    pub fn as_text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

// ==================== 写入结果 ====================

/// 写入结果 —— [`VdfsProvider::write`] 的返回值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsWriteResponse {
    pub path: String,
    pub created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

// ==================== 动作结果 ====================

/// 动作结果 —— [`VdfsProvider::action`] 的返回值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsActionResult {
    /// 被执行的动作标识（回显，便于调用方配对请求）
    pub action: String,
    /// 是否成功
    pub ok: bool,
    /// 结果说明（成功摘要 / 失败原因，可直接展示）
    pub message: String,
    /// 动作产出的附加数据（可选；宿主方言，VDFS 只透传）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ==================== 错误 ====================

/// 字段级校验错误（provider 在 `write` 中校验后返回，消费者据此逐字段高亮）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VdfsFieldError {
    /// 字段键
    pub field: String,
    /// 人类可读原因
    pub message: String,
}

/// 校验失败载荷
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VdfsValidationError {
    pub message: String,
    #[serde(default)]
    pub fields: Vec<VdfsFieldError>,
}

impl VdfsValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fields: Vec::new(),
        }
    }

    pub fn with_field(mut self, field: impl Into<String>, message: impl Into<String>) -> Self {
        self.fields.push(VdfsFieldError {
            field: field.into(),
            message: message.into(),
        });
        self
    }

    /// 是否已记录字段级错误
    pub fn has_fields(&self) -> bool {
        !self.fields.is_empty()
    }
}

/// VDFS 操作结果（错误类型由协议自身定义，不依赖任何宿主错误体系）
pub type VdfsResult<T> = Result<T, VdfsError>;

/// VDFS 错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VdfsError {
    /// 节点 / 挂载点不存在
    NotFound(String),
    /// 该 provider 未实现此操作
    NotImplemented,
    /// 校验失败（可携带字段级错误）
    Invalid(VdfsValidationError),
    /// 访问位不允许（无 `r` / `w` 等）
    Forbidden(String),
    /// 冲突（并发 / 已存在 / 非空目录）
    Conflict(String),
    /// 内部错误
    Internal(String),
}

impl std::fmt::Display for VdfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "节点不存在：{m}"),
            Self::NotImplemented => write!(f, "该操作未实现"),
            Self::Invalid(e) => write!(f, "校验失败：{}", e.message),
            Self::Forbidden(m) => write!(f, "操作被拒绝：{m}"),
            Self::Conflict(m) => write!(f, "冲突：{m}"),
            Self::Internal(m) => write!(f, "内部错误：{m}"),
        }
    }
}

impl std::error::Error for VdfsError {}

impl VdfsError {
    /// 机器可读码（跨边界传输时使用，避免字符串比较）
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "NOT_FOUND",
            Self::NotImplemented => "NOT_IMPLEMENTED",
            Self::Invalid(_) => "VALIDATION_ERROR",
            Self::Forbidden(_) => "FORBIDDEN",
            Self::Conflict(_) => "CONFLICT",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }

    /// 便捷构造校验错误
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(VdfsValidationError::new(message))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// 是否为「未实现」——宿主据此在能力判定中隐藏对应入口
    pub fn is_not_implemented(&self) -> bool {
        matches!(self, Self::NotImplemented)
    }
}

// ==================== 变更通知 ====================

pub const VDFS_CHANGE_CREATED: &str = "created";
pub const VDFS_CHANGE_UPDATED: &str = "updated";
pub const VDFS_CHANGE_DELETED: &str = "deleted";
pub const VDFS_CHANGE_RENAMED: &str = "renamed";
/// **追加型**变更：节点内容尾部新增了一段（携带 [`VdfsChange::delta`]）。
///
/// 与 `updated` 的区别是**增量的**：`updated` 是「这个节点变了，请重读」，
/// `appended` 是「这个节点尾部多了这些字，直接用」。列表型 provider 用它承载
/// 流式输出——一条消息的正文不断追加，消费者无需为每个片段重读整条消息。
pub const VDFS_CHANGE_APPENDED: &str = "appended";

/// **尾部截断**变更：`path` 所指节点**及其之后的全部兄弟**都已被移除。
///
/// ## 为什么不能复用 `deleted`
///
/// `deleted` 的语义是「**这一个**节点没了」，消费者据此移除一项即可，与顺序无关
/// ——这正是它作为通用变更类型该有的样子（重试一轮时删掉它的子树、恢复工具调用时
/// 删掉旧的结果子节点，都是这种**逐节点**删除）。
///
/// 而「删除某条消息，其后的消息一并删除」是另一种语义：它描述的是**列表尾部的一段
/// 区间**，而不是若干个独立节点。若仍用 `deleted` 逐条下发，会同时坏掉两件事：
///
/// 1. **消费者无从分辨**——收到的每一条都长得一样，「删这一个」与「从这里删到末尾」
///    在载荷上完全不可区分，只能靠外部知识去猜；
/// 2. **代价随被删数量线性增长**——删一条早期消息要发 N 条变更，N 可达上百。
///
/// 因此把它拆成一个**独立的变更类型**，而不是在 `deleted` 上挂一个 `cascade` 布尔：
/// 「是哪种删除」只有一个取值处（`change` 本身），不会出现两个字段必须一起读才正确
/// 的写法。消费者侧也不需要预知任何上下文——按自己的列表顺序取「该节点及其后」即可，
/// 与它是怎么被删的无关。
pub const VDFS_CHANGE_TRUNCATED: &str = "truncated";

/// 数据变更事件（**provider 视角**）。
///
/// **不含挂载名**——provider 不知道自己被挂在哪里（见模块文档）。`path` 是该
/// provider 子树内的**相对路径**，与其 `list` / `stat` 等的路径坐标系一致；
/// 使用方（分发层）投递时补上挂载名、拼成全路径后转发给消费者。
///
/// ## 载荷是**按变更类型可选**的，不是可有可无的装饰
///
/// 消费者要知道「变成了什么」，而 `path` + `change` 只说「哪里、怎么变」。
/// 三种获取方式对应三种成本：
///
/// | 变更 | 载荷 | 消费者动作 | 额外往返 |
/// |---|---|---|---|
/// | `appended` | [`Self::delta`] | 尾部拼接 | **0**（热路径，逐帧） |
/// | `created` / `updated` | [`Self::node`]（+ [`Self::content`]） | 就地插入 / 替换 | **0** |
/// | 未带载荷 | — | 回退：`stat` + `read` | 1–2 |
///
/// 设计要点：**热路径窄、冷路径全**。流式追加每帧只多一小段（`delta`），
/// 若把整节点挂在每帧上，流量会退化成 O(n²)；而 `created` / `updated` 每轮
/// 只有寥寥数次，把节点视图一并带上就能免掉消费者的回读——这正是既有
/// `kind = "session"` 消息通道在做的事，VDFS 承载它即可，不必让消费者退步。
///
/// 载荷**可选**：provider 可以选择不填（消费者回退到 `stat` / `read`），
/// 因此这是纯增益扩展，不构成对实现的强制。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsChange {
    /// 变更节点在本 provider 子树内的相对路径
    pub path: String,
    /// 变更类型（`created` / `updated` / `deleted` / `renamed` / `appended` / `truncated`）
    pub change: String,
    /// 重命名时的目标路径（同样为本子树内相对路径）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// 追加型变更（`appended`）的**增量文本**；其余变更为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    /// **节点视图**（`created` / `updated` 的载荷）：变更后该节点的元数据。
    ///
    /// 消费者据此免掉一次 `stat`——结构（`name` / `attributes` / `status` / `ext`）
    /// 直接可用。`None` = 未附带，消费者需自行 `stat`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<VdfsNode>,
    /// **内容快照**（`created` / `updated` 的载荷）：变更后该节点的正文。
    ///
    /// 与 [`Self::delta`] 的区别是**全量 vs 增量**：本字段是「现在是什么」，
    /// `delta` 是「多了什么」。两者不会同时出现。
    /// `None` = 未附带，消费者需自行 `read`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl VdfsChange {
    pub fn new(path: impl Into<String>, change: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            change: change.into(),
            to: None,
            delta: None,
            node: None,
            content: None,
        }
    }

    pub fn renamed(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            path: from.into(),
            change: VDFS_CHANGE_RENAMED.to_string(),
            to: Some(to.into()),
            delta: None,
            node: None,
            content: None,
        }
    }

    /// 追加型变更：`path` 节点尾部新增了 `delta` 这段文本。
    pub fn appended(path: impl Into<String>, delta: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            change: VDFS_CHANGE_APPENDED.to_string(),
            to: None,
            delta: Some(delta.into()),
            node: None,
            content: None,
        }
    }

    /// 附带**节点视图**（免掉消费者的 `stat`）
    pub fn with_node(mut self, node: VdfsNode) -> Self {
        self.node = Some(node);
        self
    }

    /// 附带**内容快照**（免掉消费者的 `read`）
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// 用 `f` 重写事件里**全部**路径（`path` / `to` / `node.path`）。
    ///
    /// 使用方（分发层）补挂载前缀时调用它——**而不是逐字段重建** `VdfsChange`：
    /// 逐字段重建会在新增字段时被漏掉（新字段静默丢在转发层，且没有任何编译
    /// 错误提示）。把「路径都要翻译」收进一个函数，漏翻译在结构上不可能发生。
    ///
    /// 空路径不动（`node.path` 常为空 = provider 未填，此时以 `path` 为准）。
    pub fn map_paths(mut self, f: impl Fn(&str) -> String) -> Self {
        self.path = f(&self.path);
        self.to = self.to.take().map(|t| f(&t));
        if let Some(node) = self.node.as_mut() {
            if !node.path.is_empty() {
                node.path = f(&node.path);
            }
        }
        self
    }
}

// ==================== 路径判定（地址契约的唯一实现） ====================
//
// 下面三个函数是**地址规则**的实现，因此归本模块所有：任何按路径段比较、
// 或需要收敛坐标系的场合都调用它们，不得各写一份。
//
// 历史教训：`..` 判定曾按「是否以 `../` 开头」实现，Windows 下
// `src\..\..\..\Windows` 既不以 `../` 也不以 `..\` 开头，直接绕过守卫；
// 黑名单前缀曾用裸 `starts_with("/etc")`，把 `/etcfoo` 一并误伤。
// 两处 bug 同源——**按字符串前缀代替按路径段比较**。

/// 路径中是否含 `..` 段——**两种分隔符都算**。
///
/// 只查 `/` 会让 Windows 的 `src\..\..\..\Windows` 绕过守卫；只查带分隔符的
/// `../` / `..\` 前缀会放过 `a/..`（`..` 收尾）。因此按**段**判定，与分隔符无关。
pub fn has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg == "..")
}

/// `path` 是否落在 `prefix` 之内——相等，或紧随一个分隔符。
///
/// 前缀必须按**路径段**比较：裸 `starts_with("/etc")` 会把 `/etcfoo` 误伤。
pub fn path_within(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches(['/', '\\']);
    if prefix.is_empty() {
        return false;
    }
    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'),
        None => false,
    }
}

// ==================== 宿主上下文（不透明） ====================

/// 调用级自定义参数的键值表（**使用方注入 → provider 取用**）。
///
/// 键名是**约定**而非类型：使用方与 provider 通过共享常量对齐（如
/// [`VDFS_PARAM_WORKDIR`]）。之所以用 JSON 值而非类型化槽位，是为了让机制不依赖
/// 任何具体资源语义——**新增一个约定参数不需要改动接口**。
pub type VdfsParams = serde_json::Map<String, Value>;

/// 参数键：工作目录（本地文件子树解析相对路径的基准）。
///
/// 与宿主 ctx 的 `WORKDIR` 键同名同义：vdfs 访问层把请求 ctx 里的 workdir
/// 透传给 provider，使「相对路径从工作目录开始」这条既有本地地址规则
/// 在虚拟地址空间里保持不变。
pub const VDFS_PARAM_WORKDIR: &str = "workdir";

/// 参数键：有界列表的**条数上限**（`list`）。
///
/// 可选约定：provider 不认就当没传（全量），不认它的 provider 无需任何改动。
pub const VDFS_PARAM_LIMIT: &str = "limit";

/// 参数键：有界列表的**游标**——取该地址之前的一页（`list`）。
///
/// 游标是**地址**而不是页码：清单在两次请求之间会变（新增 / 删除 / 被顶到前面），
/// 偏移量会重复或漏项，地址不会。
pub const VDFS_PARAM_BEFORE: &str = "before";

/// 不透明宿主上下文：VDFS 不假设宿主形态，宿主把运行时状态放进袋子里，
/// provider 按需 `downcast` 取用。
///
/// 除宿主句柄外还携带一袋**调用级参数**（[`VdfsParams`]）：不透明、无类型约束、
/// 由使用方按约定键名注入，provider 按同一约定取出。资源语义因此不必进入接口。
///
/// ```ignore
/// // 使用方（访问层）
/// let vctx = vdfs_context(&ctx).with_param(VDFS_PARAM_WORKDIR, workdir);
/// // provider
/// let workdir = ctx.param_str(VDFS_PARAM_WORKDIR)?;
/// ```
#[derive(Clone)]
pub struct VdfsContext {
    host: Arc<dyn Any + Send + Sync>,
    params: Arc<VdfsParams>,
}

impl VdfsContext {
    /// 由宿主状态构造
    pub fn new<H: Send + Sync + 'static>(host: H) -> Self {
        Self {
            host: Arc::new(host),
            params: Arc::new(VdfsParams::new()),
        }
    }

    /// 无宿主状态（纯计算 / 测试场景）
    pub fn empty() -> Self {
        Self::new(())
    }

    /// 注入一个调用级参数（同名覆盖）
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        Arc::make_mut(&mut self.params).insert(key.into(), value.into());
        self
    }

    /// 批量注入调用级参数（同名覆盖）
    pub fn with_params(mut self, params: VdfsParams) -> Self {
        let slot = Arc::make_mut(&mut self.params);
        for (k, v) in params {
            slot.insert(k, v);
        }
        self
    }

    /// 取一个参数（原始 JSON）
    pub fn param(&self, key: &str) -> Option<&Value> {
        self.params.get(key)
    }

    /// 取一个字符串参数
    pub fn param_str(&self, key: &str) -> Option<&str> {
        self.param(key)?.as_str()
    }

    /// 取一个参数并反序列化为 `T`（失败 / 缺失均为 `None`）
    pub fn param_as<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_value(self.param(key)?.clone()).ok()
    }

    /// 全部参数（只读）
    pub fn params(&self) -> &VdfsParams {
        &self.params
    }

    /// 取用宿主状态（类型不匹配返回 `None`）
    pub fn host<H: Send + Sync + 'static>(&self) -> Option<&H> {
        self.host.downcast_ref::<H>()
    }

    /// 取用宿主状态，缺失时报 [`VdfsError::Internal`]
    pub fn require<H: Send + Sync + 'static>(&self) -> VdfsResult<&H> {
        self.host::<H>()
            .ok_or_else(|| VdfsError::internal("宿主上下文类型不匹配（provider 需要的类型未注入）"))
    }
}

impl Default for VdfsContext {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Debug for VdfsContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VdfsContext(<opaque>)")
    }
}

// ==================== 变更回调 ====================

/// 变更投递器：宿主在 `watch` 期间注入，provider 检测到变化时调用。
///
/// 回调必须是**同步且非阻塞**的（宿主内部做派发）；provider 不得在其中做 IO。
pub type VdfsChangeSink = Arc<dyn Fn(VdfsChange) + Send + Sync>;

// ==================== provider trait ====================

/// VDFS provider —— 把一个资源域暴露为一棵可被使用的资源子树。
///
/// **provider 不知道自己被挂在哪里**：挂载名由使用方在注册时选定（见模块文档），
/// 因此 trait 上没有任何与挂载相关的成员。
///
/// ## 实现约定
///
/// - **只实现自己支持的操作**：其余保持 trait 默认（[`VdfsError::NotImplemented`]），
///   使用方据此在能力判定中不暴露对应入口。
/// - **`path` 是本子树内的相对路径**（`""` = 自身根），已规范化、无穿越风险。
/// - **`access` 是能力声明**：使用方与消费者只看访问位，不做类型特判。
/// - **校验归实现方**：写入的必填 / 范围 / 格式校验在 [`Self::write`] 内完成，
///   失败返回 [`VdfsError::Invalid`]（可带字段级错误）。
/// - **线程安全**：`&self` 可能被并发调用。
#[async_trait]
pub trait VdfsProvider: Send + Sync + 'static {
    /// 展示标签（缺省由使用方用挂载名代替）
    fn label(&self) -> Option<&str> {
        None
    }

    /// 语义说明（下发给前端与 LLM，帮助理解该子树的资源含义）
    fn description(&self) -> Option<&str> {
        None
    }

    /// 展示顺序（导航排序；小者靠前）
    fn order(&self) -> i32 {
        100
    }

    /// 图标名（使用方纯 UI 映射）
    fn icon(&self) -> Option<&str> {
        None
    }

    /// 自身根节点的**隐藏属性**（缺省不隐藏）
    ///
    /// 与 [`root_access`](Self::root_access) / [`root_status`](Self::root_status) /
    /// [`root_new_types`](Self::root_new_types) 同构：描述 provider 的**根**
    /// 这一层的元数据，由使用方在合成该目录节点时回填到 [`VdfsNode::hidden`]。
    ///
    /// 它不是什么新概念——provider 的根**本来就是一个目录节点**，
    /// 所以「这个目录在父目录的列表里显示还是隐藏」由这条声明回答，
    /// 与文件 / 目录的隐藏属性是同一件事（语义见 [`VdfsNode::hidden`]）。
    fn root_hidden(&self) -> bool {
        false
    }

    /// 自身根的访问位（缺省「可列目录」）
    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST
    }

    /// 自身根的状态
    fn root_status(&self) -> &str {
        VDFS_STATUS_ACTIVE
    }

    /// 自身根**可接受的新建类型**（缺省空 = 根下不可新建）。
    ///
    /// 与 [`Self::root_access`] / [`Self::root_status`] 同构：描述 provider 的
    /// **根**（挂载点）这一层的元数据，由使用方在合成挂载点节点时回填。
    /// 子树内更深层的目录在 [`Self::list`] / [`Self::stat`] 返回的节点上各自声明
    /// [`VdfsNode::new_types`]。
    fn root_new_types(&self) -> Vec<VdfsNewType> {
        Vec::new()
    }

    /// 列出目录的直接子节点（`l` 位）
    async fn list(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<Vec<VdfsNode>> {
        Err(VdfsError::NotImplemented)
    }

    /// 读取节点元数据
    async fn stat(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<VdfsNode> {
        Err(VdfsError::NotImplemented)
    }

    /// 读取内容（`r` 位）
    async fn read(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<VdfsContent> {
        Err(VdfsError::NotImplemented)
    }

    /// 写入内容（`w` 位）——**实现方在此完成全部校验**。
    ///
    /// ## 两种目标形态：具名节点 / 目录自身
    ///
    /// `path` 是本子树内的相对路径，它可以指向**两种东西**，实现方都必须考虑：
    ///
    /// - **具名节点**（`<名字>` 或 `<父>/<名字>`）：常规的「写这个节点」。
    ///   它存在就覆盖，不存在则看 `create` 位（见下）。
    /// - **目录自身**（`""` = 自身根，或任何指向目录的地址）：此时使用方**没有给名字**
    ///   ——「新建一个，叫什么由你定」。这是「新建」在机制上的形态：使用方只说
    ///   **建在哪个目录**，不说叫什么（名字是 provider 的私有知识，见
    ///   [`VdfsNode::name`] 的「唯一标识」定位）。
    ///
    /// 因此**写目录自身不是错误**：provider 若支持在自己名下创建条目，就生成一个
    /// 名字（id 归 provider）、落盘、并**在返回值里给出新节点的相对路径**——那是
    /// 使用方唯一能拿到新地址的地方。不支持（如该目录没有可新建的类型）则照常报错。
    ///
    /// ## `create` 位 = 使用方的写意图
    ///
    /// | 目标 | `create = false` | `create = true` |
    /// |---|---|---|
    /// | 已存在 | 覆盖（`created = false`） | 覆盖（`created = false`） |
    /// | 不存在 · **具名节点** | 写入型资源**就地创建**（`created = true`）；「更新既有对象的字段」型语义可报 [`VdfsError::NotFound`] | **创建**（`created = true`） |
    /// | 不存在 · **目录自身** | 报错（没有可覆盖的目标，见上） | **创建**，名字由 provider 生成 |
    ///
    /// ⚠️ **具名 + 目标不存在时不要一律报 `NotFound`**：使用方要的是「给了名字就写得
    /// 进去」——配置型资源的地址**就是它的身份**（`model` / `mcp` / `skill` 皆如此），
    /// 不存在就建一个。只有「写的是某个**既有对象的一个字段**」（如会话 metadata）
    /// 才该拒绝：没有对象就没有可更新的字段。
    ///
    /// 于是「保存一份还没落盘的草稿」与「新建一项」是同一个动作的两种意图，
    /// 使用方无需先 `stat` 再决定写还是建——那会引入一次多余的往返与竞态。
    ///
    /// ⚠️ **`create` 只管「不存在时怎么办」，不改变内容的处理方式**：内容一律取自
    /// [`VdfsContent`]（写目录自身时使用方可能给空内容，见下）。**不要**把
    /// `create = true` 实现成「忽略使用方给的内容、一律落默认值」——那会让
    /// 「在草稿详情页填好再保存」丢掉用户填的每一个字段。
    ///
    /// 唯一的例外是**内容为空**：那是「先建一个，随后再填」的合法形态
    /// （新建会话、或使用方只想要一份可用的初始配置），此时由 provider 落一份
    /// 自己的**最小合法内容**。
    ///
    ///
    /// ## 返回值
    ///
    /// [`VdfsWriteResponse::path`] 必须是**本子树内**的路径（与 `list` 返回的节点
    /// 同口径），使用方据此把结果翻译成展示地址并选中新节点。provider 生成了名字
    /// 却不填 `path`，使用方就找不到刚建出来的东西。
    async fn write(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        _content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        Err(VdfsError::NotImplemented)
    }

    /// 删除节点（`recursive` 仅对目录有意义）
    async fn delete(&self, _ctx: &VdfsContext, _path: &str, _recursive: bool) -> VdfsResult<()> {
        Err(VdfsError::NotImplemented)
    }

    /// 新建目录
    async fn mkdir(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<()> {
        Err(VdfsError::NotImplemented)
    }

    /// 移动 / 重命名（分发层保证同挂载点内）
    async fn move_item(&self, _ctx: &VdfsContext, _from: &str, _to: &str) -> VdfsResult<()> {
        Err(VdfsError::NotImplemented)
    }

    /// 执行**节点动作**（如 [`VDFS_ACTION_TEST`]「测试连接」）。
    ///
    /// 与固定操作集（列 / 读 / 写 / 删 …）不同，动作是 **provider 自持的动词**：
    /// VDFS 只把 `(节点路径, 动作标识, 载荷)` 透传给 provider，**不解释语义**；
    /// 未实现的动作返回 [`VdfsError::NotImplemented`]，消费方据此不给出入口。
    ///
    /// 动作的**呈现**（按钮文案 / 忙态 / 图标）不属于本层：与 `ext` 一样由宿主
    /// 方言决定（本宿主编在 `node.schema` 的详情定义里），VDFS 只负责把它送到
    /// 该去的 provider。
    async fn action(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        _action: &str,
        _payload: Option<&Value>,
    ) -> VdfsResult<VdfsActionResult> {
        Err(VdfsError::NotImplemented)
    }

    /// 订阅指定子树的数据变更；检测到变化时调用 `sink`。
    /// 默认 no-op：无实时能力的 provider 直接成功。
    async fn watch(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        _sink: VdfsChangeSink,
    ) -> VdfsResult<()> {
        Ok(())
    }

    /// 取消订阅（与 [`Self::watch`] 严格配对）
    async fn unwatch(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<()> {
        Ok(())
    }
}

/// 类型别名：便于使用方在容器里存放 `dyn VdfsProvider`
pub type DynVdfsProvider = Arc<dyn VdfsProvider>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_flags_roundtrip() {
        let a = VdfsAccess {
            read: true,
            write: false,
            list: true,
            traverse: true,
        };
        assert_eq!(a.flags(), "rlt");
        assert_eq!(VdfsAccess::parse("rlt"), a);
        assert_eq!(VdfsAccess::parse("tlrx"), a);
        assert_eq!(VdfsAccess::NONE.flags(), "");
        assert!(VdfsAccess::NONE.is_none());
    }

    #[test]
    fn access_serde_is_compact_string() {
        let n = VdfsNode::file("a.md", "A", VdfsAccess::READ_WRITE);
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["access"], serde_json::json!("rw"));
        let back: VdfsNode = serde_json::from_value(v).unwrap();
        assert_eq!(back.access, VdfsAccess::READ_WRITE);
    }

    #[test]
    fn contains_and_constructors() {
        assert!(VdfsAccess::READ_WRITE.contains(VdfsAccess::READ));
        assert!(!VdfsAccess::READ.contains(VdfsAccess::READ_WRITE));
        assert_eq!(VdfsAccess::dir(false, true).flags(), "lt");
        assert_eq!(VdfsAccess::dir(true, true).flags(), "wlt");
        assert_eq!(
            VdfsAccess::dir(true, true),
            VdfsAccess::LIST_WRITE_TRAVERSE,
            "常量与构造器同源"
        );
        assert_eq!(VdfsAccess::file(false).flags(), "r");
    }

    #[test]
    fn derive_ext_from_name() {
        assert_eq!(derive_ext("config.json").as_deref(), Some("json"));
        assert_eq!(derive_ext("prompts/a.MD").as_deref(), Some("md"));
        assert_eq!(derive_ext("noext"), None);
        assert_eq!(derive_ext(".env"), None);
        assert_eq!(derive_ext("a."), None);
    }

    #[test]
    fn effective_ext_prefers_explicit() {
        let mut n = VdfsNode::file("session", "会话设置", VdfsAccess::READ_WRITE);
        assert_eq!(n.effective_ext(), None);
        n.ext = Some("form".into());
        assert_eq!(n.effective_ext().as_deref(), Some("form"));

        let n = VdfsNode::file("note.md", "笔记", VdfsAccess::READ);
        assert_eq!(n.effective_ext().as_deref(), Some("md"));
    }

    #[test]
    fn node_is_dir_by_access_not_kind() {
        assert!(VdfsNode::dir("prompts", "提示词", VdfsAccess::LIST_TRAVERSE).is_dir());
        assert!(!VdfsNode::file("a.md", "a", VdfsAccess::READ).is_dir());
    }

    /// 可接受的新建类型：目录节点携带、文件节点为空、空表不序列化
    #[test]
    fn new_types_are_dir_scoped_and_omitted_when_empty() {
        let file = VdfsNode::file("a.md", "a", VdfsAccess::READ);
        assert!(file.new_types.is_empty());
        assert!(
            serde_json::to_value(&file)
                .unwrap()
                .get("new_types")
                .is_none(),
            "空表不得序列化（不污染文件节点）"
        );

        let dir = VdfsNode::dir("session", "会话", VdfsAccess::LIST_WRITE_TRAVERSE)
            .with_new_type(VdfsNewType::new("session", "会话").with_description("新建会话"));
        let v = serde_json::to_value(&dir).unwrap();
        assert_eq!(v["new_types"][0]["ext"], serde_json::json!("session"));
        assert_eq!(v["new_types"][0]["title"], serde_json::json!("会话"));
        assert_eq!(
            v["new_types"][0]["description"],
            serde_json::json!("新建会话")
        );
        let back: VdfsNode = serde_json::from_value(v).unwrap();
        assert_eq!(back.new_types.len(), 1);
        assert_eq!(back.new_types[0].ext, "session");
    }

    /// 宿主方言的呈现描述经 `schema` 透传，VDFS 不解释其内容
    #[test]
    fn schema_is_opaque_passthrough() {
        let n = VdfsNode::file("session", "会话设置", VdfsAccess::READ_WRITE)
            .with_ext("form")
            .with_schema(serde_json::json!({ "binding": "config" }));
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["schema"]["binding"], serde_json::json!("config"));
        // 场景扩展字段 flatten 到顶层
        let n = n.with_attribute("config_type", serde_json::json!("session"));
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["config_type"], serde_json::json!("session"));
    }

    #[test]
    fn validation_error_shape() {
        let e = VdfsValidationError::new("保存被拒绝").with_field("port", "必须在 1-65535");
        assert!(e.has_fields());
        // 载荷可序列化：宿主据此把它编码进 PluginError 文本/数据
        let json = serde_json::to_value(&e).expect("校验载荷必须可序列化");
        assert_eq!(json["message"], serde_json::json!("保存被拒绝"));
        assert_eq!(json["fields"][0]["field"], serde_json::json!("port"));
        assert_eq!(
            json["fields"][0]["message"],
            serde_json::json!("必须在 1-65535")
        );

        // VdfsError 自身只负责携带载荷，错误码由宿主层映射
        let err = VdfsError::Invalid(e);
        assert_eq!(err.code(), "VALIDATION_ERROR");
        assert!(!err.is_not_implemented());
    }

    #[test]
    fn error_codes_and_helpers() {
        assert_eq!(VdfsError::NotImplemented.code(), "NOT_IMPLEMENTED");
        assert!(VdfsError::NotImplemented.is_not_implemented());
        assert_eq!(VdfsError::not_found("a").code(), "NOT_FOUND");
        assert_eq!(VdfsError::invalid("x").code(), "VALIDATION_ERROR");
        assert_eq!(VdfsError::Forbidden("x".into()).code(), "FORBIDDEN");
        assert_eq!(VdfsError::Conflict("x".into()).code(), "CONFLICT");
        assert_eq!(VdfsError::internal("x").code(), "INTERNAL_ERROR");
    }

    #[test]
    fn change_constructors_are_mount_free() {
        // provider 只报子树内相对路径，不含挂载名
        let c = VdfsChange::renamed("a", "b");
        assert_eq!(c.change, VDFS_CHANGE_RENAMED);
        assert_eq!(c.path, "a");
        assert_eq!(c.to.as_deref(), Some("b"));

        let c = VdfsChange::new("sub/x.md", VDFS_CHANGE_UPDATED);
        assert_eq!(c.path, "sub/x.md");
        assert!(c.to.is_none());

        // 追加型变更必须携带增量——消费者不得据此触发重读
        let c = VdfsChange::appended("sub/x.md", "尾部新增");
        assert_eq!(c.change, VDFS_CHANGE_APPENDED);
        assert_eq!(c.delta.as_deref(), Some("尾部新增"));
        assert!(VdfsChange::new("a", VDFS_CHANGE_UPDATED).delta.is_none());
    }

    /// 载荷按变更类型可选：`appended` 只带增量（热路径窄），
    /// `created` / `updated` 可带节点视图 + 内容快照（冷路径免回读）。
    #[test]
    fn change_payload_is_kind_scoped() {
        // 默认构造不带任何载荷——provider 不填也能工作（消费者回退 stat/read）
        let bare = VdfsChange::new("a", VDFS_CHANGE_CREATED);
        assert!(bare.node.is_none() && bare.content.is_none());

        let node = VdfsNode::file("m1", "助手", VdfsAccess::READ);
        let c = VdfsChange::new("消息/m1", VDFS_CHANGE_CREATED)
            .with_node(node)
            .with_content("你好");
        assert_eq!(c.node.as_ref().map(|n| n.name.as_str()), Some("m1"));
        assert_eq!(c.content.as_deref(), Some("你好"));
        // 全量快照与增量是两个字段，不同时出现
        assert!(c.delta.is_none());

        // 追加型**只**带增量：逐帧载荷不得随正文变宽（否则退化成 O(n²)）
        let a = VdfsChange::appended("消息/m1", "世界");
        assert!(a.node.is_none() && a.content.is_none() && a.delta.is_some());

        // 空载荷字段不序列化（保持既有线上形状不变）
        let v = serde_json::to_value(VdfsChange::new("a", VDFS_CHANGE_UPDATED)).unwrap();
        assert!(v.get("delta").is_none() && v.get("node").is_none() && v.get("content").is_none());
    }

    /// `map_paths` 一次覆盖事件里**全部**路径——使用方补前缀不必逐字段重建。
    #[test]
    fn map_paths_covers_every_path_in_the_event() {
        let node = VdfsNode::file("m1", "助手", VdfsAccess::READ).with_path("abc/消息/m1");
        let c = VdfsChange::renamed("abc/消息/m1", "abc/消息/m2")
            .with_node(node)
            .with_content("正文")
            .map_paths(|p| format!("session/{p}"));

        assert_eq!(c.path, "session/abc/消息/m1");
        assert_eq!(c.to.as_deref(), Some("session/abc/消息/m2"));
        assert_eq!(
            c.node.as_ref().map(|n| n.path.as_str()),
            Some("session/abc/消息/m1")
        );
        // 载荷本身不是路径，不参与翻译
        assert_eq!(c.content.as_deref(), Some("正文"));
        assert_eq!(c.change, VDFS_CHANGE_RENAMED);

        // 节点路径为空（provider 未填）时不动——以事件的 `path` 为准
        let c = VdfsChange::new("a", VDFS_CHANGE_UPDATED)
            .with_node(VdfsNode::file("m1", "m1", VdfsAccess::READ))
            .map_paths(|p| format!("session/{p}"));
        assert_eq!(c.node.as_ref().map(|n| n.path.as_str()), Some(""));
    }

    /// `..` 判定按**路径段**，与分隔符无关。
    #[test]
    fn parent_segment_is_separator_agnostic() {
        assert!(has_parent_segment("../etc/passwd"));
        assert!(has_parent_segment(".."));
        assert!(has_parent_segment("a/../b"));
        // `..` 收尾：按前缀实现的旧判定会放过
        assert!(has_parent_segment("a/.."));
        // Windows 分隔符：按 `../` 前缀实现的旧判定会放过
        assert!(has_parent_segment(r"src\..\..\..\Windows"));
        assert!(has_parent_segment(r"..\etc"));
        // 含 `..` 但不是独立段 → 合法
        assert!(!has_parent_segment("a/..b/c"));
        assert!(!has_parent_segment("src/main.rs"));
    }

    /// 前缀判定按**路径段**——`/etcfoo` 不在 `/etc` 之内。
    #[test]
    fn path_within_respects_segment_boundary() {
        assert!(path_within("/etc", "/etc"));
        assert!(path_within("/etc/passwd", "/etc"));
        assert!(path_within(r"C:\Users\a\.ssh\id", r"C:\Users\a\.ssh"));
        // 裸 starts_with 会误伤这两个
        assert!(!path_within("/etcfoo", "/etc"));
        assert!(!path_within("/etc2/x", "/etc"));
        assert!(!path_within("/usr/local", "/etc"));
        // 前缀尾部多余的斜杠不影响判定
        assert!(path_within("/etc/passwd", "/etc/"));
        assert!(!path_within("/anything", ""));
    }

    #[test]
    fn context_downcast_and_require() {
        let ctx = VdfsContext::new(7u32);
        assert_eq!(ctx.host::<u32>(), Some(&7));
        assert!(ctx.host::<u64>().is_none());
        assert!(ctx.require::<u32>().is_ok());
        let err = ctx.require::<String>().unwrap_err();
        assert_eq!(err.code(), "INTERNAL_ERROR");

        // 无宿主状态的默认上下文
        let empty = VdfsContext::empty();
        assert!(empty.host::<u32>().is_none());
    }

    /// 自描述全部有缺省：`impl VdfsProvider for P {}` 即可编译——
    /// provider **不需要**提供任何目录名（目录名是使用方的事）
    #[test]
    fn self_description_defaults_need_no_dir_name() {
        struct P;
        #[async_trait]
        impl VdfsProvider for P {}

        assert_eq!(P.label(), None, "label 缺省为空，由使用方以目录名代替");
        assert_eq!(P.description(), None);
        assert_eq!(P.icon(), None);
        assert_eq!(P.order(), 100);
        assert_eq!(P.root_access(), VdfsAccess::LIST);
        assert_eq!(P.root_status(), VDFS_STATUS_ACTIVE);
        assert!(P.root_new_types().is_empty(), "缺省根下不可新建");
    }

    /// 未实现的操作返回 `NotImplemented`（使用方据此隐藏入口）
    #[tokio::test]
    async fn unimplemented_ops_default_to_not_implemented() {
        struct P;
        #[async_trait]
        impl VdfsProvider for P {}

        let ctx = VdfsContext::empty();
        assert!(P.list(&ctx, "").await.unwrap_err().is_not_implemented());
        assert!(P.stat(&ctx, "a").await.unwrap_err().is_not_implemented());
        assert!(P.read(&ctx, "a").await.unwrap_err().is_not_implemented());
        assert!(P
            .write(&ctx, "a", &VdfsContent::text("a", "x"))
            .await
            .unwrap_err()
            .is_not_implemented());
        assert!(P
            .delete(&ctx, "a", true)
            .await
            .unwrap_err()
            .is_not_implemented());
        assert!(P.mkdir(&ctx, "d").await.unwrap_err().is_not_implemented());
        assert!(P
            .move_item(&ctx, "a", "b")
            .await
            .unwrap_err()
            .is_not_implemented());
        // watch / unwatch 默认 no-op（无实时能力的 provider 也必须成功）
        assert!(P.watch(&ctx, "", Arc::new(|_| {})).await.is_ok());
        assert!(P.unwatch(&ctx, "").await.is_ok());
    }
}
