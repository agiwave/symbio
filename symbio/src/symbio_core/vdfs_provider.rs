//! 核心 VdfsProvider —— 统一资源访问的唯一契约（唯一接口：`dispatch`）。
//!
//! 本模块是 VDFS 的 **centerpiece**，只暴露**纯接口**：
//! - [`VdfsProvider`] 只有一个方法 [`VdfsProvider::dispatch`]：
//!   `dispatch(ctx, path, req)` —— **`path` 是第一个分发键**（先按地址找到资源域，
//!   再由域内实现决定操作怎么落地），`req`（[`VdfsRequest`] 枚举）只携带**操作的
//!   载荷**（写什么、删不递归、动作是什么……）；列 / 读 / 写 / 删 / 建 / 移 / 动作 /
//!   订阅全部落在枚举变体上；
//! - 每个资源域 = 一份 [`VdfsProvider`] 实现，`Arc<dyn VdfsProvider>` 是使用方与
//!   实现方之间**唯一**的交换物；
//! - 线上形状（`vdfs/*` 请求 / 响应信封、协议路径常量）定义在 vdfs 插件内部
//!   （`plugins/vdfs/protocol.rs`），**core 不暴露这些类型**。
//!
//! **分层、依赖方向、开放边界（本模块可原样抽出为独立 crate）见
//! `docs/design/vdfs.md` §2**——此处不复述。
//!
//! 数据模型速览：
//!
//! - 一切资源 = 目录树上的**节点**（[`VdfsNode`]），地址 = 树内相对路径
//!   `<目录>/<rel>`（全路径由使用方拼接、回填；provider 不知道自己被放在哪层目录下）；
//! - 节点的能力 = 四个**访问位**（[`VdfsAccess`]：`r` 读 / `w` 写 / `l` 列 / `t` 遍历）；
//! - 内容 = [`VdfsContent`]（文本 `text` 或二进制 `b64`，互斥）；
//! - 呈现 = 节点的 `ext`（扩展名）→ 使用方选渲染器；渲染器所需描述经 `schema` 透传；
//! - 变更 = [`VdfsChange`]（子树内**相对路径** + 可选**业务载荷** `data`，
//!   缺失 = 回读收敛），经 [`VdfsChangeSink`] 由使用方补成展示地址后投递。

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

/// 场景类型：会话的**转写列表**（`<根>/session/<id>/<段>`）。
///
/// `kind` 是场景可自定义的（会话叶子自己就声明 `kind = "session"`），
/// 这里给转写列表一个**稳定的 ASCII 语义类型**：它的 `name` / `title` 是
/// 面向用户的展示名（可能随语言或文案调整），不能被消费者当成标识来认；
/// 而 `kind` 是**协议词**——消费者按它发现「哪个子目录是转写」，不必硬编码段名。
pub const VDFS_KIND_MESSAGES: &str = "messages";

/// 场景类型：会话的**收件箱**（`<根>/session/<id>/inbox`）。
///
/// 与 [`VDFS_KIND_MESSAGES`] 同一手法：`name` / `title` 是展示名，`kind` 是协议词。
/// 收件箱里的一条是**还没被消费的用户消息**，因此条目的 `ext` 沿用
/// [`VDFS_EXT_MESSAGE`]（它就是一条消息），靠 `kind` 与会话转写里的消息区分开：
/// 一个是"待发"，一个是"已发生"。
pub const VDFS_KIND_INBOX: &str = "inbox";

// ==================== 会话节点属性：上一轮结局（`attributes.outcome`） ====================
//
// 会话叶子用 `status` 表达「现在在不在跑」，用 `outcome` 表达「上一轮怎么结束的」。
// 两者是同一份运行态投影出的两个属性（`SessionRuntime`），所以词表必须住在一起
// ——消费方（前端、CLI、子智能体转播）读的是同一批字面量，谁都不许自己拼。
//
// 与 [`VDFS_STATUS_*`] 同处一处的理由：`outcome` 只在 `status != working` 时有
// 意义（`working` 时结局作废），两者一起读才构成完整的运行态。

/// 上一轮**正常结束**
pub const VDFS_OUTCOME_COMPLETED: &str = "completed";
/// 上一轮**被用户中止**（与 `completed` 区分：提示音音色、收尾文案不同）
pub const VDFS_OUTCOME_ABORTED: &str = "aborted";
/// 上一轮**以错误结束**——此时 `attributes.error` 带错误文案
pub const VDFS_OUTCOME_FAILED: &str = "failed";

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
/// [`VdfsProvider::delete`] 的全局语义是「**这一个**节点没了」——它是**逐节点**
/// 的。拿它表达「删一个节点却删掉了它后面所有」会成为一条**没人能预期的默认
/// 行为**；而拿 `cascade: bool` 之类的
/// 附加位区分，则让「是哪种删除」变成两个字段必须一起读。动作是 provider 自持的
/// 动词，正好承载这类**集合操作**：VDFS 只透传，不解释。
///
/// ## 区间删除怎么通知消费端
///
/// **逐条**发移除帧——每条是「那个节点没了」的元数据（路径 + 状态，几十字节）。
/// 另一种形态是「列表目录整份重读」，但它会把**保留的**条目也重传一遍：对「删几条」
/// 这个动作，逐条通知是更便宜的。
///
/// 移除帧走 provider 自己的变更通道（与它的增 / 改变更同一条）：从机制看，被删的
/// 就是那一个条目——「删这一段」与「删这一个」在**单条**变更上完全同形，机制因此
/// 不需要认识「区间」这个概念。会话消息的落地形态见
/// `session/plugin/vdfs_provider.rs::truncate_messages` / `clear_messages`。
///
/// 回执里的被删 id 列表（随 [`VdfsActionResult::data`]）是**权威**列表：
/// 调用方据此幂等对齐本地视图，不依赖推送。
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

/// 目录**可接受的新建元素类型**——「新建」入口的类型。
///
/// 一个目录（含 provider 根）**至多**声明一种自己能新建的元素类型
/// （[`VdfsNode::new_type`] / [`VdfsProvider::root_new_type`] 都是 `Option`）：
///
/// - `Some` → 显示添加入口；
/// - `None` → 不显示添加入口（该目录由系统管理）。
///
/// ## 为什么是「一种」而不是一张清单
///
/// 一个目录接受的是**一类**东西：`session` 目录只收会话、`model` 目录只收模型。
/// 曾经这里是一张清单，于是 `skill` / `mcp` 各自登记了两项——但那是**同一个
/// 资源类型的两种入口形态**（表单新建 / 整包导入），不是两种类型：落成后它们
/// 是同一形状的节点、走同一个渲染器。把「入口形态」混进「类型清单」的代价是
/// 消费端必须先去重再判定，而任何「按类型」的判定（如 `vdfsScheme` 靠
/// `ext = session` 认挂载点）都要遍历清单才能表达「是不是这种类型」。
///
/// 现在两类入口各归其位：**类型**（本结构）描述落成后的节点，**导入入口**
/// （[`VdfsNewImport`]）描述同一个类型的另一种内容来源。
///
/// ## 两条独立的键：`ext` 与 `node_ext`
///
/// [`ext`](Self::ext) 是**呈现扩展名**——地址末段可能带的后缀，provider 用
/// [`entry::id_of`] 一族按它剥出条目 id（`<根>/model/openai-1.model` → `openai-1`）。
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
    /// **备选的整包导入入口**（可选）：声明后，使用方在「新建」上额外给出
    /// 「导入」形态——内容取自本地文件（见 [`VdfsNewImport`]）。
    ///
    /// 它与 [`source`](Self::source) 的分工：`source = file` 是**主入口本身就是
    /// 选文件**（没有「边看边填」的过程，如 agent 整包）；本字段是**主入口之外
    /// 再给一个导入入口**（主入口仍是表单，如 skill / mcp）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<VdfsNewImport>,
}

/// **整包导入入口**——同一个新建类型的另一种内容来源（[`VdfsNewType::import`]）。
///
/// 「整包导入」= 选一个本地文件，把它的字节写进目标地址；provider 把它解释为
/// **导入一个完整目录包**（语义自持，VDFS 不解释）。它不额外占一个操作，
/// 也不另立一种节点：导入落成的就是该类型声明的那个节点（`node_ext` / `schema`
/// 与表单新建完全一致）。
///
/// 三处与 [`VdfsNewType`] 不同的地方只有「包」本身：
/// - [`ext`](Self::ext) 是**包地址的后缀**（如 `zip`）——目标名由文件名推导
///   （`demo.zip` → `demo`），provider 用 `entry::pack_name_of` 按它剥建议名；
/// - [`title`](Self::title) 是导入入口自己的展示名（如「技能包」）；
/// - 没有 `node_ext` / `schema`——落成后的呈现由所属类型决定，不由包决定。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsNewImport {
    /// 包地址末段的后缀（如 `zip`；`pack_name_of` 按它剥建议名）
    pub ext: String,
    /// 导入入口的展示标题（如「技能包」）
    pub title: String,
    /// 语义说明（缺省不显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl VdfsNewImport {
    /// 仅 ext + title 的最小构造
    pub fn new(ext: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            ext: ext.into(),
            title: title.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
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
            import: None,
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

    /// 同 [`Self::with_schema`]，但接受可缺省值（`None` = 不挂定义）——
    /// 供「定义需运行期汇流、缓存值可能尚未就绪」的构造点使用
    pub fn with_schema_opt(mut self, schema: Option<Value>) -> Self {
        self.schema = schema;
        self
    }

    /// 追加**备选的整包导入入口**（主入口之外再给一条「导入」路径）
    pub fn with_import(mut self, import: VdfsNewImport) -> Self {
        self.import = Some(import);
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
    /// 全路径（**展示口径**，如 `<根>/session/abc`）；由分发层回填，provider 可留空
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
    /// 本目录可接受的**新建元素类型**（「新建」入口的唯一依据）。
    ///
    /// `None`（缺省）= 不可新建；`Some` → 使用方显示添加入口。
    /// 仅目录节点有意义；文件节点恒为 `None`。纯呈现元数据，VDFS 不解释其创建语义。
    ///
    /// ⚠️ 它是**至多一种**（见 [`VdfsNewType`]）：一个目录接受的是**一类**东西。
    /// 同一类型的多种入口形态由类型自己的 [`VdfsNewType::import`] 表达，不在这里
    /// 堆成一张清单。
    ///
    /// `Box` 不是随手加的：`VdfsNewType` 带六个 `Option<String>` + `schema` +
    /// 可选导入入口（≈250 字节），而 `VdfsNode` 是**全系统数量最多**的类型
    /// （每条消息 / 每个文件 / 每个目录都是它）。这个字段只有**挂载点目录**才有，
    /// 内联进结构体等于给每个消息节点白背那 250 字节——`Option<Box<_>>` 是 8 字节。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_type: Option<Box<VdfsNewType>>,
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
            new_type: None,
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

    /// 声明本目录可接受的**新建元素类型**（至多一种；`None` = 不可新建）
    ///
    /// 装箱在**这里**完成，调用方不必知道字段是 `Box`（理由见字段文档）。
    pub fn with_new_type(mut self, new_type: Option<VdfsNewType>) -> Self {
        self.new_type = new_type.map(Box::new);
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
//
// ## 信封**没有操作枚举**：`{path, data?}`，语义全在 `data` 的字段上
//
// 信封只回答「**哪条路径、带来了什么**」：`data` 是该路径的**业务载荷**——
// 路径是消息项（`<id>/message/<mid>`）时它是 `ChatMessage`（`delta` 有 ⇒ 尾部追加、
// `content` 有 ⇒ 整条替换、`status = removed` ⇒ 就地移除，语义由字段本身给出，
// 不从类型反推）；路径是会话叶子且带视图时它是 `VdfsNode`（全量节点视图，幂等）。
// `data` 缺失 = 「变了，但本变更不携带载荷」——消费端按需回读；对**资源域**的
// 删除而言这是**唯一**表达（删掉的节点没有视图可带），回读 `NotFound` 即删除。
//
// 判据不变：**一个载荷字段必须有生产性生产者**，否则它不是词汇的一部分。
//
// | `data` 形状 | 生产者 |
// |---|---|
// | 缺失 | 全部 provider 的资源信号（`grep -rn "VdfsChange::bare("`） |
// | `ChatMessage`（含 `delta`） | 消息域——`session/transcript.rs` 的 `Transcript::apply` |
// | `VdfsNode` | 会话运行态——`Transcript::emit_session_state` |
//
// ## 为什么没有操作枚举
//
// 形状收敛到 `{path, data?}` 的取舍见 ADR-025：「资源层面发生了什么」与 `data`
// 描述的「业务数据变成了什么」是同一件事的两种说法，而消费端真正消费的只有后者——
// 保留前者只会让每个消费端都背上一次「枚举 → 分派」的翻译。删除的表达力由此让位给
// 「载荷缺失 + 回读 `NotFound`」，消息的删除由 `ChatMessage.status = removed` 承载
// （消息词汇本就有它）。

/// 数据变更事件（**provider 视角**）。
///
/// **不含挂载名**——provider 不知道自己被挂在哪里（见模块文档）。`path` 是该
/// provider 子树内的**相对路径**，与其 `list` / `stat` 等的路径坐标系一致；
/// 使用方（分发层）投递时补上挂载名、拼成全路径后转发给消费者。
///
/// ## 形状：`path` + 可选 `data`
///
/// `data` 是**业务载荷**：消息项上是 `ChatMessage`（字段语义见模块文档的词汇表），
/// 会话叶子的运行态上是 `VdfsNode`。**缺失 = 无载荷**——不是一种「类型」，而是
/// 「本次变更不带业务数据」：消费端按需回读（幂等），回读 `NotFound` 即删除。
///
/// ## 为什么信封是**不透明**的 `Value` 而不是枚举
///
/// 信封跨插件边界转发（`map_paths` 补挂载名后原样投递），它**不解释**载荷——
/// 「这是消息还是会话节点」由**路径**回答，由最终消费端按自己的词汇解释。
/// 在信封上建 `enum { Node(..), Message(..) }` 等于让机制层认识所有业务形状，
/// 每新增一种可推送载荷都要改它——那正是「会话特化机制」换了个方向复活。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsChange {
    /// 变更节点在本 provider 子树内的相对路径
    pub path: String,
    /// 业务载荷（`ChatMessage` / `VdfsNode` 的 JSON）；缺失 = 无载荷（回读收敛）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl VdfsChange {
    /// 一条**无载荷**变更：「这条路径变了」，内容一概回读。
    ///
    /// 资源域的绝大多数变更长这样——provider 只知道「变了」（配置写入了、
    /// 目录建了、节点删了），手头没有（也不该现造）一份业务视图。
    pub fn bare(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            data: None,
        }
    }

    /// 一条**带业务载荷**的变更：`data` 是该路径当前的业务数据
    /// （消息帧 / 节点视图，由**生产者**按自己的词汇序列化）。
    ///
    /// 与 [`Self::bare`] 分开，是为了让「绝大多数变更不携带载荷」这件事在
    /// 调用点上一眼可见：带载荷是一个**显式动作**，不是默认行为。
    pub fn with_data(path: impl Into<String>, data: impl serde::Serialize) -> Self {
        Self {
            path: path.into(),
            data: Some(serde_json::to_value(data).unwrap_or(Value::Null)),
        }
    }

    /// 用 `f` 重写事件里的路径。
    ///
    /// 使用方（分发层）补挂载前缀时调用它——**而不是逐字段重建** `VdfsChange`：
    /// 逐字段重建会在新增字段时被漏掉（新字段静默丢在转发层，且没有任何编译
    /// 错误提示）。把「路径都要翻译」收进一个函数，漏翻译在结构上不可能发生。
    ///
    /// 目前只有 `path` 一个**路径型**字段（`data` 是载荷，不需要翻译）——保留
    /// 这个函数正是为了让那句话继续成立。
    pub fn map_paths(mut self, f: impl Fn(&str) -> String) -> Self {
        self.path = f(&self.path);
        self
    }
}

// ==================== 总线帧解包（信封契约的唯一实现） ====================

/// **VDFS 变更唯一的解包入口**——CLI 与 agent 转播桥都走这里。
///
/// 输入是 `event_bus` 投递的一帧，信封形状由
/// `event_bus::build_envelope` 定义：
///
/// ```text
/// { type: "bus_event", data: { kind, session_id, data: <VdfsChange> } }
/// ```
///
/// 因此判定分两步：外层 `type == "bus_event"`（`event_bus` 的约定），内层
/// `kind == KIND_VDFS`（本域关心的事件类型）。任一不符 → `None`
/// （帧不是给本域消费的，属正常情况，不是错误）。
///
/// ## 为什么它必须住在这里
///
/// 「拆信封 → 取 `data` → 反序列化」分散在消费端各手写一份。信封形状是跨模块
/// 契约，副本数 ≥2 时其中一份漂移只是时间问题。
/// 本函数与 [`VdfsChange`] 同模块：**形状改了，这里先响**。
///
/// 借用 `&Value` 反序列化，**不克隆载荷**——帧是热路径，每帧一份整树深拷贝很贵。
pub fn vdfs_change_of(frame: &crate::symbio_core::PluginFrame) -> Option<VdfsChange> {
    use crate::symbio_core::PluginFrame;
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    let bus = v.get("data")?;
    if bus.get("kind").and_then(Value::as_str) != Some(crate::symbio_core::event_bus::KIND_VDFS) {
        return None;
    }
    VdfsChange::deserialize(bus.get("data")?).ok()
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
    /// **当前父地址**：本 provider 挂载点的绝对地址（由转发方写入）。
    ///
    /// provider 收到的地址一律是自身子树内的相对地址；绝大多数操作只需要相对
    /// 地址。只有少数协议级场合需要全局地址，此时从本字段 + 相对地址拼出。
    /// 顶层为空串（可诊断的降级：绝对地址退化为根相对地址）。
    parent_addr: String,
}

impl VdfsContext {
    /// 由宿主状态构造
    pub fn new<H: Send + Sync + 'static>(host: H) -> Self {
        Self {
            host: Arc::new(host),
            params: Arc::new(VdfsParams::new()),
            parent_addr: String::new(),
        }
    }

    /// 转发方写入当前父地址（同名覆盖；vdfs 核心协议把操作派发给 provider 的
    /// 那一跳调用——见 `plugins/composite/vdfs.rs` 的 `dispatch`）
    pub fn with_parent_addr(mut self, addr: impl Into<String>) -> Self {
        self.parent_addr = addr.into();
        self
    }

    /// 当前父地址（空串 = 未设置，绝对地址随之退化为根相对地址）
    pub fn parent_addr(&self) -> &str {
        &self.parent_addr
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

// ==================== 请求（唯一接口的操作载荷） ====================
//
// 操作的**地址不在这里**——它是 [`VdfsProvider::dispatch`] 的独立参数
// `path`（本子树内相对路径，`""` = 自身根，已由分发方规范化、无穿越风险）。
// 分发因此是**先 path 后操作**：转发方按 path 首段找到下一层（或域内实现按
// path 段找到资源），再由 `req` 决定操作怎么落地；转发方**不需要**为了知道
// 「把请求转给谁」而去 match 操作。
//
// `VdfsRequest` 只收拢**操作载荷**——**地址一律走 `path` 参数**，载荷里没有
// 任何地址字段。这不是巧合，是刻意的：地址有两个（`from` / `to`）的操作无法
// 在「一个 provider = 一棵子树」的模型下定义清楚——provider 只认自己子树内的
// 相对路径，「移动」跨出子树就不再是本层能表达的动作。见下方「没有 `Move`」。
//
// ## 为什么是枚举而不是一排 trait 方法
//
// 曾经是 12 个方法（list / stat / read / write / …）。问题出在**纯转发型
// provider**（容器、门面、作用域代理）身上：每新增一种操作，每个转发方都要补一个
// 「拆路径 → 转发 → 回填路径」的方法，漏一个就是静默能力缺口。收敛成一个
// `dispatch` 之后，「新增一种操作」落在枚举的一个变体上——所有实现体的 `match`
// 编译期穷尽，漏译在结构上不可能；转发方剥掉首段、把剩余路径与请求整体递下去
// 即可，与操作种类完全无关。
//
// ## 各操作的语义（原 trait 方法文档的归所）
//
// - [`VdfsRequest::Write`]：**实现方在此完成全部校验**（必填 / 范围 / 格式，失败
//   返回 [`VdfsError::Invalid`]）。`path` 可指向**具名节点**，也可指向**目录自身**
//   （`""` = 根）——后者是「新建」的机制形态：使用方只说建在哪个目录，名字由
//   provider 生成并在 [`VdfsResponse::Write`] 的 `path` 里交回（那是使用方拿到
//   新地址的**唯一**途径；漏填等于新建之后找不到新节点）。
// - [`VdfsRequest::Write`] 的 `content.create` 是**使用方的写意图**（不存在时
//   怎么办）：具名节点缺省**就地创建**——配置型资源的地址就是它的身份
//   （`model` / `mcp` / `skill` 皆如此），只有「更新既有对象的字段」型语义才报
//   [`VdfsError::NotFound`]；目录自身 + `create = false` 必报错（没有可覆盖的
//   目标）。⚠️ `create` 只回答「目标不存在时怎么办」，**不改变内容的处理方式**——
//   内容一律取自本次写入，不要实现成「忽略内容、落默认值」；唯一例外是内容为空
//   （「先建一个，随后再填」），由 provider 落最小合法内容。
// - [`VdfsRequest::Delete`]：`recursive` 仅对目录有意义。
// - [`VdfsRequest::Action`]：动作是 **provider 自持的动词**（如 [`VDFS_ACTION_TEST`]
//   「测试连接」），VDFS 只透传 `(路径, 动作标识, 载荷)`，**不解释语义**；未实现
//   的动作返回 [`VdfsError::NotImplemented`]，消费方据此不给出入口。
// - [`VdfsRequest::Watch`] / [`VdfsRequest::Unwatch`]：订阅指定子树的数据变更，
//   检测到变化时调用 `sink`（[`VdfsChangeSink`]，同步非阻塞）。无实时能力的
//   provider 应返回成功——与「无实时能力」并不冲突，语义是「订阅成功、无事件」。
//
// ## 没有 `Move`
//
// 曾经有 `Move { to }`，现已删除，且**不应加回来**。理由是模型层面的：
//
// - 一个 provider = **一棵子树**，它只认自己子树内的相对路径。于是「同一个
//   provider 内移动」= 同一棵树内的重命名，provider 用原生能力（`rename`）能做得
//   很省事——这是它唯一的价值；
// - 但**跨虚拟挂载树**时，`from` 与 `to` 分属两棵子树，「移动」就不再是原语，
//   而是 `copy + delete`。此时 trait 上的 `Move` 表达不出这件事，只能由某个
//   provider 假装自己同时拥有两端（`composite` 就不得不先解析 `to` 属于哪个子
//   目录、再拒绝跨目录的情形——即「用错误表达能力的缺失」）；
// - 于是 `Move` 变成只有**恰好一个**实现者能真做（物理盘），其余实现者一律
//   `NotImplemented`；而「复制 + 删除」这个真正的通用形态反而无处安放。
//
// 因此本层不收 `Move`：**要用移动，由外层组合**（读 + 写 + 删，或专门的重命名
// 访问层操作）。当前外层**也没有提供**它——重命名入口与 `vdfs/move` 协议操作、
// `vdfs_move` 工具一并下线，需要时再加，加在外层而不是这里。
#[derive(Clone)]
pub enum VdfsRequest {
    /// 列出 `path` 目录的直接子节点（`l` 位）；`limit` / `before` 是**可选**的
    /// 有界窗口，provider 不认就当没传（全量）——见 [`VDFS_PARAM_LIMIT`] /
    /// [`VDFS_PARAM_BEFORE`]
    List {
        limit: Option<u32>,
        before: Option<String>,
    },
    /// 读取 `path` 节点的元数据
    Stat,
    /// 读取 `path` 节点的内容（`r` 位）
    Read,
    /// 写入 `path` 节点——语义见本模块「各操作的语义」一节
    Write { content: VdfsContent },
    /// 删除 `path` 节点（`recursive` 仅对目录有意义）
    Delete { recursive: bool },
    /// 在 `path` 处新建目录
    Mkdir,
    /// 对 `path` 节点执行**节点动作**（provider 自持的动词，VDFS 只透传）
    Action {
        action: String,
        payload: Option<Value>,
    },
    /// 订阅 `path` 子树的数据变更；检测到变化时调用 `sink`
    Watch { sink: VdfsChangeSink },
    /// 取消订阅（与 [`VdfsRequest::Watch`] 严格配对）
    Unwatch,
}

impl std::fmt::Debug for VdfsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // sink 不可 Debug、载荷可能巨大：只印变体名
        let name = match self {
            Self::List { .. } => "List",
            Self::Stat => "Stat",
            Self::Read => "Read",
            Self::Write { .. } => "Write",
            Self::Delete { .. } => "Delete",
            Self::Mkdir => "Mkdir",
            Self::Action { .. } => "Action",
            Self::Watch { .. } => "Watch",
            Self::Unwatch => "Unwatch",
        };
        f.write_str(name)
    }
}

// ==================== 响应（唯一接口的出参） ====================

/// [`VdfsProvider::dispatch`] 的响应：与请求变体一一对应。
///
/// [`VdfsResponse::Unit`] 承载「成功但没有产物」的操作（delete / mkdir /
/// watch / unwatch——失败走 `Err`，成功无值可带）。响应里的 `path`（节点 /
/// 内容 / 写入结果上的）一律是**本子树内**的相对路径（与请求的 `path` 参数
/// 同坐标系），由分发方负责补成树内 / 展示口径。
#[derive(Debug, Clone)]
pub enum VdfsResponse {
    /// [`VdfsRequest::List`]：直接子节点清单
    List(Vec<VdfsNode>),
    /// [`VdfsRequest::Stat`]：节点元数据
    Stat(VdfsNode),
    /// [`VdfsRequest::Read`]：节点内容
    Read(VdfsContent),
    /// [`VdfsRequest::Write`]：写入结果（`path` 必填——provider 生成的名字全靠它交回）
    Write(VdfsWriteResponse),
    /// [`VdfsRequest::Action`]：动作结果
    Action(VdfsActionResult),
    /// 成功无产物（delete / mkdir / watch / unwatch）
    Unit,
}

impl VdfsResponse {
    pub fn is_list(&self) -> bool {
        matches!(self, Self::List(_))
    }

    pub fn is_stat(&self) -> bool {
        matches!(self, Self::Stat(_))
    }

    pub fn is_read(&self) -> bool {
        matches!(self, Self::Read(_))
    }

    pub fn is_write(&self) -> bool {
        matches!(self, Self::Write(_))
    }

    pub fn is_action(&self) -> bool {
        matches!(self, Self::Action(_))
    }

    pub fn is_unit(&self) -> bool {
        matches!(self, Self::Unit)
    }

    // ---- 取值器：转发方 / 访问层从响应中取出与请求变体对应的载荷 ----
    //
    // 变体不匹配返回 `None`（实现方返回错型响应是 bug，由调用方决定报错方式），
    // [`Self::Unit`] 一律视为「成功无产物」，调用方无需再写一臂。

    /// [`Self::List`] → 子节点清单
    pub fn into_list(self) -> Option<Vec<VdfsNode>> {
        match self {
            Self::List(v) => Some(v),
            Self::Unit => Some(Vec::new()),
            _ => None,
        }
    }

    /// [`Self::Stat`] → 节点元数据
    pub fn into_stat(self) -> Option<VdfsNode> {
        match self {
            Self::Stat(n) => Some(n),
            _ => None,
        }
    }

    /// [`Self::Read`] → 节点内容
    pub fn into_read(self) -> Option<VdfsContent> {
        match self {
            Self::Read(c) => Some(c),
            _ => None,
        }
    }

    /// [`Self::Write`] → 写入结果
    pub fn into_write(self) -> Option<VdfsWriteResponse> {
        match self {
            Self::Write(r) => Some(r),
            _ => None,
        }
    }

    /// [`Self::Action`] → 动作结果
    /// [`Self::Unit`] → `Some(())`（delete / mkdir / watch / unwatch 的成功回执）
    pub fn into_unit(self) -> Option<()> {
        match self {
            Self::Unit => Some(()),
            _ => None,
        }
    }

    pub fn into_action(self) -> Option<VdfsActionResult> {
        match self {
            Self::Action(r) => Some(r),
            _ => None,
        }
    }
}

// ==================== provider trait ====================

/// VDFS provider —— 把一个资源域暴露为一棵可被使用的资源子树。
///
/// **provider 不知道自己被挂在哪里**：挂载名由使用方在注册时选定，trait 上没有
/// 任何与挂载相关的成员；自述（标题 / 描述 / 顺序 / 图标 / 根访问位）由
/// [`crate::symbio_core::PluginMeta`] 承载（`Plugin::meta()`）。
///
/// 唯一例外是 [`Self::root_new_type`]——它同样属于「根的自述」，却**不能**进
/// `PluginMeta`：表单 schema 可能需要运行期汇流（async），而 `PluginMeta` 是同步
/// 纯数据。因此这一项留在本 trait 上，由容器合成根节点时现场取。
///
/// ## 唯一接口
///
/// [`Self::dispatch`] 收 `(ctx, path, req)`：**`path` 是第一个分发键**——实现方
/// 先按它定位资源（转发型实现剥首段找下一层；叶子实现按段找自己的资源），再由
/// `req` 决定操作怎么落地。实现方按变体 `match`，**只实现自己支持的操作**——
/// 其余臂返回 [`VdfsError::NotImplemented`]，使用方据此隐藏对应入口。
///
/// ## 实现约定
///
/// - **地址是本子树内的相对路径**（`""` = 自身根），已规范化、无穿越风险；
/// - **`access` 是能力声明**：使用方与消费者只看访问位，不做类型特判；
/// - **校验归实现方**：写入的必填 / 范围 / 格式校验在实现内完成（见
///   [`VdfsRequest`] 的语义一节），失败返回 [`VdfsError::Invalid`]（可带字段级
///   错误）；
/// - **线程安全**：`&self` 可能被并发调用。
#[async_trait]
pub trait VdfsProvider: Send + Sync + 'static {
    /// 唯一入口：先按 `path` 定位资源域，再按 `req` 变体执行操作
    /// （各操作语义见 [`VdfsRequest`] 的模块级文档）
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse>;

    /// **挂载根**可接受的新建元素类型（至多一种；`None` = 根下不可新建）。
    ///
    /// 名字带 `root_` 前缀，是因为它只描述**本 provider 的根目录**——而根节点
    /// **不由 provider 产出**（容器合成，静态部分取自 `PluginMeta`），所以它没有
    /// 别的渠道把这份自述交出去。这与 `PluginMeta::root_access` / `hidden` 同族：
    /// 都是「根的自述」，只是这一项**不能进 `PluginMeta`**——表单 schema 可能需要
    /// 运行期汇流（如 session 的选项定义来自 options 广播），而那是同步纯数据。
    ///
    /// provider 自己 `list` 出来的**子目录**若也可新建，由该目录节点自己的
    /// [`VdfsNode::new_type`] 声明（容器只合成根，不碰更深层）。
    ///
    /// 容器合成根/子目录节点时现场调用；默认 `None`。
    async fn root_new_type(&self) -> Option<VdfsNewType> {
        None
    }
}

/// 类型别名：便于使用方在容器里存放 `dyn VdfsProvider`
pub type DynVdfsProvider = Arc<dyn VdfsProvider>;

#[cfg(test)]
#[path = "vdfs_provider.test.rs"]
mod tests;
