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
//! ## 挂载不属于 provider
//!
//! **provider 完全不知道自己被挂在哪里**。挂载是**使用方**的概念：谁用它、
//! 谁决定它在虚拟树上的名字。因此：
//!
//! - trait 上**没有** `mount()` 之类的方法——provider 不管理也不提供挂载名；
//! - provider 的每个方法只接收**本子树内的相对路径**（`""` = 自身根），
//!   已由使用方完成规范化与穿越校验（见 [`normalize_path`]）；
//! - 全路径（`/<挂载名>/<rel…>`）由使用方拼接、回填。
//!
//! ## 依赖方向
//!
//! - **实现方**（如 `setting` 插件）实现本 trait，在 `traverse` 广播中经
//!   `CapabilityVisitor::register_vdfs_provider(挂载名, provider)` 注册自身。
//!   **注册名由使用方选定的**，约定用插件名（`PLUGIN_*` 常量）——插件名在宿主内
//!   唯一，天然就是合格的挂载名；
//! - **`vdfs` 插件**收集全部注册项，按 `vdfs/*` 协议分发（前端与 LLM 走同一条
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
//! - 一切资源 = 虚拟树上的**节点**（[`VdfsNode`]），地址 = `/<mount>/<rel>`；
//! - 节点的能力 = 四个**访问位**（[`VdfsAccess`]：`r` 读 / `w` 写 / `l` 列 / `t` 遍历）；
//! - 内容 = [`VdfsContent`]（文本 `text` 或二进制 `b64`，互斥）；
//! - 呈现 = 节点的 `ext`（扩展名）→ 使用方选渲染器；渲染器所需描述经 `schema` 透传；
//! - 变更 = [`VdfsChange`]（子树内**相对路径**、**不含挂载名**；经
//!   [`VdfsChangeSink`] 由使用方补挂载名后投递）。

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::any::Any;
use std::sync::Arc;

// ==================== 状态取值 ====================

pub const VFDS_STATUS_ACTIVE: &str = "active";
pub const VFDS_STATUS_WORKING: &str = "working";
pub const VFDS_STATUS_DISABLED: &str = "disabled";
pub const VFDS_STATUS_ERROR: &str = "error";
pub const VFDS_STATUS_UNKNOWN: &str = "unknown";

/// 节点基础类型：目录
pub const VFDS_KIND_DIR: &str = "dir";
/// 节点基础类型：文件
pub const VFDS_KIND_FILE: &str = "file";
/// 节点基础类型：挂载点
pub const VFDS_KIND_MOUNT: &str = "mount";

/// 虚拟根路径
pub const VFDS_ROOT: &str = "/";

/// 挂载点节点属性键：**是否作为导航项出现**（`false` = 不占导航位）。
///
/// 由 [`VdfsProvider::nav_visible`] 决定，仅在该方法返回 `false` 时写入
/// （缺省 `true` 不序列化）；消费者按「缺省可见」处理。
pub const VFDS_ATTR_NAV_VISIBLE: &str = "nav_visible";

// ==================== 呈现扩展名（约定，宿主可自行扩展） ====================
//
// 节点 `ext` 是宿主选择详情呈现方式的键。VDFS 只透传、不解释；
// 以下是**约定俗成**的几个取值，宿主可自由增添自己的扩展名。

/// 定义驱动表单（呈现描述放 `node.schema`）
pub const VFDS_EXT_FORM: &str = "form";
/// 会话工作区（实时对话流）
pub const VFDS_EXT_SESSION: &str = "session";
/// 纯文本编辑器
pub const VFDS_EXT_TEXT: &str = "text";
/// JSON 编辑器
pub const VFDS_EXT_JSON: &str = "json";
/// Markdown 编辑器
pub const VFDS_EXT_MARKDOWN: &str = "md";
/// 文件树（目录节点的默认呈现）
pub const VFDS_EXT_DIR: &str = "dir";
/// 整包（zip）——**导入**用扩展名：内容是一整个资源目录的压缩包
pub const VFDS_EXT_ZIP: &str = "zip";

// ==================== 节点动作（约定） ====================

/// 节点动作标识：**连接测试**（`vdfs/action` 的 `action` 取值之一）。
///
/// 动作标识由 provider 自持，VDFS 只透传、不解释（与 `ext` 同构）。此处登记的
/// 是当前唯一的内置约定：「测试连接」——模型 / MCP 这类外部资源的连通性自检。
pub const VFDS_ACTION_TEST: &str = "test";

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
/// 类型以 [`VdfsNewType::ext`] 标识（扩展名），与 [`VdfsNode::ext`] 同一命名空间，
/// 因此**新建后的详情渲染器与既有节点一致**。本结构是**纯呈现元数据**：
/// VDFS 只透传、不解释；具体创建语义由 provider 在 [`VdfsProvider::write`] 中自持。
///
/// ## 内容来源 [`VdfsNewType::source`]
///
/// 「新建」在机制上就是一次 [`VdfsProvider::write`]（`create: true`），因此要说清
/// **写进去的内容从哪来**——这是创建语义的一部分，由 provider 声明：
///
/// - `None`（默认）：先命名、后写入（内容为空或 provider 的最小合法内容）；
/// - [`VFDS_NEW_SOURCE_FILE`]：内容取自**本地文件**，使用方给文件选择器，
///   字节走 [`VdfsContent::b64`] 二进制通道（如 zip 整包导入）。
///
/// [`VdfsProvider::write`]: VdfsProvider::write
/// [`VdfsContent::b64`]: VdfsContent::b64
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsNewType {
    /// 新元素扩展名（决定创建后的详情渲染器）
    pub ext: String,
    /// 展示标题（如「会话」「模型」）
    pub title: String,
    /// 语义说明（缺省不显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 图标名（使用方纯 UI 映射）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 内容来源（见结构文档）：`None` = 命名后写入；`"file"` = 选择本地文件
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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
}

/// 新建内容来源：**本地文件**（[`VdfsNewType::source`] 的取值之一）。
///
/// 声明它的类型意味着「新建 = 选一个本地文件，把它的字节写进目标地址」。
pub const VFDS_NEW_SOURCE_FILE: &str = "file";

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
    /// 全路径（含挂载点，如 `/setting/session`）；由分发层回填，provider 可留空
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
    /// 最后更新时间（Unix 秒）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 直接子节点数量（目录）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<u32>,
    /// 内容是否为二进制（文件）
    #[serde(default)]
    pub binary: bool,
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
    VFDS_KIND_FILE.to_string()
}

fn default_status() -> String {
    VFDS_STATUS_ACTIVE.to_string()
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
            kind: VFDS_KIND_DIR.to_string(),
            access,
            ..Default::default()
        }
    }

    /// 文件节点（`r`，可按需 `w`）
    pub fn file(name: impl Into<String>, title: impl Into<String>, access: VdfsAccess) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            kind: VFDS_KIND_FILE.to_string(),
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub create: bool,
}

/// serde 辅助：`false` 不序列化（保持读取结果的线上形状不变）
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

pub const VFDS_CHANGE_CREATED: &str = "created";
pub const VFDS_CHANGE_UPDATED: &str = "updated";
pub const VFDS_CHANGE_DELETED: &str = "deleted";
pub const VFDS_CHANGE_RENAMED: &str = "renamed";

/// 数据变更事件（**provider 视角**）。
///
/// **不含挂载名**——provider 不知道自己被挂在哪里（见模块文档）。`path` 是该
/// provider 子树内的**相对路径**，与其 `list` / `stat` 等的路径坐标系一致；
/// 使用方（分发层）投递时补上挂载名、拼成全路径后转发给消费者。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsChange {
    /// 变更节点在本 provider 子树内的相对路径
    pub path: String,
    /// 变更类型（`created` / `updated` / `deleted` / `renamed`）
    pub change: String,
    /// 重命名时的目标路径（同样为本子树内相对路径）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

impl VdfsChange {
    pub fn new(path: impl Into<String>, change: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            change: change.into(),
            to: None,
        }
    }

    pub fn renamed(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            path: from.into(),
            change: VFDS_CHANGE_RENAMED.to_string(),
            to: Some(to.into()),
        }
    }
}

// ==================== 路径 ====================

/// 规范化全路径：统一前导 `/`、折叠空段、拒绝向上穿越。
///
/// - `""` / `"/"` / `"///"` → `"/"`
/// - `"a/b/"` → `"/a/b"`
/// - 含 `..` 段 → [`VdfsError::Invalid`]
///
/// 分发层在调用 provider 前调用一次，provider 因此**无需**再做穿越校验。
pub fn normalize_path(raw: &str) -> VdfsResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(VFDS_ROOT.to_string());
    }
    let mut segs: Vec<&str> = Vec::new();
    for seg in trimmed.split('/') {
        let seg = seg.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(VdfsError::invalid(format!(
                "VDFS 路径不允许向上穿越：{raw}"
            )));
        }
        segs.push(seg);
    }
    if segs.is_empty() {
        return Ok(VFDS_ROOT.to_string());
    }
    Ok(format!("/{}", segs.join("/")))
}

/// 拆分全路径为 `(挂载名, 相对路径)`；虚拟根返回 `None`。
pub fn split_mount(full: &str) -> Option<(&str, String)> {
    let p = full.trim_start_matches('/');
    if p.is_empty() {
        return None;
    }
    match p.find('/') {
        Some(i) => Some((&p[..i], p[i + 1..].to_string())),
        None => Some((p, String::new())),
    }
}

/// 拼接全路径（`join_path("session", "")` → `"/session"`）
pub fn join_path(mount: &str, rel: &str) -> String {
    let mount = mount.trim_matches('/');
    let rel = rel.trim_matches('/');
    if rel.is_empty() {
        format!("/{mount}")
    } else {
        format!("/{mount}/{rel}")
    }
}

// ==================== 宿主上下文（不透明） ====================

/// 调用级自定义参数的键值表（**使用方注入 → provider 取用**）。
///
/// 键名是**约定**而非类型：使用方与 provider 通过共享常量对齐（如
/// [`VFDS_PARAM_WORKDIR`]）。之所以用 JSON 值而非类型化槽位，是为了让机制不依赖
/// 任何具体资源语义——**新增一个约定参数不需要改动接口**。
pub type VdfsParams = serde_json::Map<String, Value>;

/// 参数键：工作目录（本地文件子树解析相对路径的基准）。
///
/// 与宿主 ctx 的 `WORKDIR` 键同名同义：vdfs 访问层把请求 ctx 里的 workdir
/// 透传给 provider，使「相对路径从工作目录开始」这条既有本地地址规则
/// 在虚拟地址空间里保持不变。
pub const VFDS_PARAM_WORKDIR: &str = "workdir";

/// 不透明宿主上下文：VDFS 不假设宿主形态，宿主把运行时状态放进袋子里，
/// provider 按需 `downcast` 取用。
///
/// 除宿主句柄外还携带一袋**调用级参数**（[`VdfsParams`]）：不透明、无类型约束、
/// 由使用方按约定键名注入，provider 按同一约定取出。资源语义因此不必进入接口。
///
/// ```ignore
/// // 使用方（访问层）
/// let vctx = vdfs_context(&ctx).with_param(VFDS_PARAM_WORKDIR, workdir);
/// // provider
/// let workdir = ctx.param_str(VFDS_PARAM_WORKDIR)?;
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

    /// 自身根的访问位（缺省「可列目录」）
    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST
    }

    /// 自身根的状态
    fn root_status(&self) -> &str {
        VFDS_STATUS_ACTIVE
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

    /// 是否作为**导航项**出现在使用方的资源导航（左栏）中（缺省 `true`）。
    ///
    /// 与 [`Self::order`] / [`Self::icon`] 同属**呈现层声明**：隐藏的子树仍然
    /// 可被寻址、可读写、可被 LLM 使用，只是不占导航位。用于「能力存在但
    /// 不作为主资源类别」的挂载点（如本地文件树：是 VDFS 挂载点，却不是
    /// 与 session / model 并列的资源类别）。
    ///
    /// 约定：返回 `false` 时由使用方在合成挂载点节点时写入
    /// `nav_visible = false` 属性（场景数据，VDFS 只透传）；缺省即 `true`，
    /// 不额外序列化。消费者「缺省可见」，故新增 provider 无需关心本方法。
    fn nav_visible(&self) -> bool {
        true
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

    /// 写入内容（`w` 位）——**实现方在此完成全部校验**
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

    /// 执行**节点动作**（如 [`VFDS_ACTION_TEST`]「测试连接」）。
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

// ==================== 节点回填（机制级公共逻辑） ====================

/// 回填机制级字段：`path`（全路径）、`ext`（缺省由 `name` 推导）、`title`（缺省同 name）。
///
/// 分发层统一调用，provider 无需重复。
pub fn fill_node_paths(mount: &str, base_rel: &str, nodes: &mut [VdfsNode]) {
    for n in nodes.iter_mut() {
        let rel = if base_rel.is_empty() {
            n.name.clone()
        } else {
            format!("{base_rel}/{}", n.name)
        };
        if n.path.is_empty() {
            n.path = join_path(mount, &rel);
        }
        if n.ext.is_none() {
            n.ext = derive_ext(&n.name);
        }
        if n.title.is_empty() {
            n.title = n.name.clone();
        }
    }
}

/// 便捷：从节点里取回宿主方言的呈现描述
pub fn node_schema(node: &VdfsNode) -> Option<&Value> {
    node.schema.as_ref()
}

// ==================== 容器组合视图（机制级公共逻辑） ====================

/// 一串挂载：`(挂载名, 实现)`
pub type VdfsMounts = Vec<(String, DynVdfsProvider)>;

/// 把「一串挂载」组合成以虚拟根 `/` 为顶的一棵子树。
///
/// **通用机制**：任何容器（`composite` 等）直接复用，不必各自实现拓扑解析。
/// 它本身就是一棵可用的 provider 子树，因此也实现 [`VdfsProvider`]——容器把
/// 它注册为 VDFS 根，访问层（vdfs 插件）只管转发。
///
/// ## 挂载名从哪来
///
/// **容器的使用方**定名，通常就是子插件名；provider 自身不带名字
/// （见模块文档），所以这里有名、provider 里没有名。
///
/// ## 本结构承担的语义无关职责
///
/// | 职责 | 说明 |
/// |---|---|
/// | 路径解析 | 首段 = 挂载名，其余 = 该 provider 的**相对路径** |
/// | 全路径回填 | 子节点 / 内容 / 写入响应的 `path` 补成 `/挂载名/…` |
/// | 挂载根守卫 | 挂载根不可读 / 写 / 删，也不可 mkdir / move |
/// | 跨挂载点拒绝 | `move` 只允许在同一挂载点内 |
/// | 事件补全 | provider 报出的相对路径补成全路径再交给上层 sink |
///
/// `list("/")` 返回挂载点清单、`stat("/")` 返回虚拟根节点——两者都是
/// **组合出来的**，不需要任何 provider 参与。
///
/// 路径参数一律是**规范化后的全路径**（`/`、`/<挂载名>`、`/<挂载名>/…`）。
pub struct VdfsMountTable {
    mounts: VdfsMounts,
}

impl VdfsMountTable {
    /// 按 `order` 升序稳定排序（同序保持传入顺序）
    pub fn new(mut mounts: VdfsMounts) -> Self {
        mounts.sort_by_key(|(_, p)| p.order());
        Self { mounts }
    }

    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.mounts.len()
    }

    /// 已挂载的 `(挂载名, 实现)`，顺序 = 对外展示顺序
    pub fn mounts(&self) -> &[(String, DynVdfsProvider)] {
        &self.mounts
    }

    /// 挂载名清单（报错提示用）
    pub fn names_hint(&self) -> String {
        if self.mounts.is_empty() {
            return "（无）".to_string();
        }
        self.mounts
            .iter()
            .map(|(m, _)| format!("/{m}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// 虚拟根节点（`/`）
    pub fn root_node(&self) -> VdfsNode {
        let mut n = VdfsNode::dir(
            VFDS_ROOT,
            "虚拟文件系统",
            VdfsAccess {
                list: true,
                traverse: self.mounts.iter().any(|(_, p)| p.root_access().traverse),
                ..VdfsAccess::NONE
            },
        );
        n.path = VFDS_ROOT.to_string();
        n.kind = VFDS_KIND_MOUNT.to_string();
        n.description = Some("统一资源与数据根；子节点为各挂载点".to_string());
        n
    }

    /// 单个挂载点节点（`label` 缺省用挂载名代替）
    pub fn mount_node(&self, mount: &str, p: &DynVdfsProvider) -> VdfsNode {
        let mut n = VdfsNode::dir(
            mount.to_string(),
            p.label().unwrap_or(mount).to_string(),
            p.root_access(),
        );
        n.path = join_path(mount, "");
        n.kind = VFDS_KIND_MOUNT.to_string();
        n.status = p.root_status().to_string();
        n.description = p.description().map(str::to_string);
        n.new_types = p.root_new_types();
        if !p.nav_visible() {
            let _ = n
                .attributes
                .insert(VFDS_ATTR_NAV_VISIBLE.to_string(), Value::Bool(false));
        }
        n
    }

    /// 全部挂载点节点（顺序 = 对外展示顺序）
    pub fn mount_nodes(&self) -> Vec<VdfsNode> {
        self.mounts
            .iter()
            .map(|(m, p)| self.mount_node(m, p))
            .collect()
    }

    /// 全路径 → `(挂载名, provider, 相对路径)`；虚拟根或无匹配时按错误返回
    pub fn resolve<'a>(&'a self, full: &str) -> VdfsResult<(&'a str, &'a DynVdfsProvider, String)> {
        let Some((mount, rel)) = split_mount(full) else {
            return Err(VdfsError::invalid(
                "虚拟根不是可操作节点，请给出 /<挂载点>/... 路径",
            ));
        };
        let (m, p) = self
            .mounts
            .iter()
            .find(|(name, _)| name == mount)
            .ok_or_else(|| {
                VdfsError::not_found(format!(
                    "挂载点不存在：/{mount}（现有：{}）",
                    self.names_hint()
                ))
            })?;
        Ok((m.as_str(), p, rel))
    }
}

#[async_trait]
impl VdfsProvider for VdfsMountTable {
    fn label(&self) -> Option<&str> {
        Some("系统")
    }

    fn description(&self) -> Option<&str> {
        Some("组合视图：虚拟根及其下的各挂载点")
    }

    fn order(&self) -> i32 {
        0
    }

    fn root_access(&self) -> VdfsAccess {
        VdfsAccess {
            list: true,
            traverse: true,
            ..VdfsAccess::NONE
        }
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if path == VFDS_ROOT {
            return Ok(self.mount_nodes());
        }
        let (mount, p, rel) = self.resolve(path)?;
        let mut items = p.list(ctx, &rel).await?;
        fill_node_paths(mount, &rel, &mut items);
        Ok(items)
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        if path == VFDS_ROOT {
            return Ok(self.root_node());
        }
        let (mount, p, rel) = self.resolve(path)?;
        if rel.is_empty() {
            return Ok(self.mount_node(mount, p));
        }
        let mut n = p.stat(ctx, &rel).await?;
        fill_node_paths(mount, "", std::slice::from_mut(&mut n));
        n.path = path.to_string();
        Ok(n)
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let (_, p, rel) = self.resolve(path)?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden(
                "挂载根不是可读文件；请读取其子节点".to_string(),
            ));
        }
        let mut c = p.read(ctx, &rel).await?;
        if c.path.is_empty() {
            c.path = path.to_string();
        }
        Ok(c)
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let (_, p, rel) = self.resolve(path)?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden("挂载根不可写".to_string()));
        }
        let mut r = p.write(ctx, &rel, content).await?;
        if r.path.is_empty() {
            r.path = path.to_string();
        }
        Ok(r)
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        let (_, p, rel) = self.resolve(path)?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden("挂载根不可删除".to_string()));
        }
        p.delete(ctx, &rel, recursive).await
    }

    async fn mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let (_, p, rel) = self.resolve(path)?;
        if rel.is_empty() {
            return Err(VdfsError::invalid("挂载根已存在，无需创建"));
        }
        p.mkdir(ctx, &rel).await
    }

    async fn move_item(&self, ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
        let (from_mount, pf, rf) = self.resolve(from)?;
        let (to_mount, _, rt) = self.resolve(to)?;
        if from_mount != to_mount {
            return Err(VdfsError::invalid(format!(
                "不支持跨挂载点移动：{from_mount} → {to_mount}"
            )));
        }
        if rf.is_empty() || rt.is_empty() {
            return Err(VdfsError::Forbidden("挂载根不可移动".to_string()));
        }
        pf.move_item(ctx, &rf, &rt).await
    }

    async fn action(
        &self,
        ctx: &VdfsContext,
        path: &str,
        action: &str,
        payload: Option<&Value>,
    ) -> VdfsResult<VdfsActionResult> {
        let (_, p, rel) = self.resolve(path)?;
        p.action(ctx, &rel, action, payload).await
    }

    /// provider 报出的相对路径在此补成全路径，再交给上层 sink
    async fn watch(&self, ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        let (mount, p, rel) = self.resolve(path)?;
        let mount = mount.to_string();
        let wrapped: VdfsChangeSink = Arc::new(move |c: VdfsChange| {
            sink(VdfsChange {
                path: join_path(&mount, &c.path),
                change: c.change,
                to: c.to.as_deref().map(|t| join_path(&mount, t)),
            });
        });
        p.watch(ctx, &rel, wrapped).await
    }

    async fn unwatch(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let (_, p, rel) = self.resolve(path)?;
        p.unwatch(ctx, &rel).await
    }
}

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
        assert_eq!(c.change, VFDS_CHANGE_RENAMED);
        assert_eq!(c.path, "a");
        assert_eq!(c.to.as_deref(), Some("b"));

        let c = VdfsChange::new("sub/x.md", VFDS_CHANGE_UPDATED);
        assert_eq!(c.path, "sub/x.md");
        assert!(c.to.is_none());
    }

    #[test]
    fn normalize_path_rules() {
        assert_eq!(normalize_path("").unwrap(), "/");
        assert_eq!(normalize_path("/").unwrap(), "/");
        assert_eq!(normalize_path("///").unwrap(), "/");
        assert_eq!(normalize_path("/a/b/").unwrap(), "/a/b");
        assert_eq!(normalize_path("a//b").unwrap(), "/a/b");
        assert_eq!(normalize_path("/a/./b").unwrap(), "/a/b");
        assert!(normalize_path("/a/../b").is_err());
        assert!(normalize_path("../x").is_err());
    }

    #[test]
    fn split_and_join() {
        assert_eq!(split_mount("/"), None);
        assert_eq!(split_mount("/session"), Some(("session", String::new())));
        assert_eq!(
            split_mount("/session/a/b"),
            Some(("session", "a/b".to_string()))
        );
        assert_eq!(join_path("session", ""), "/session");
        assert_eq!(join_path("session", "a/b"), "/session/a/b");
        assert_eq!(join_path("/session/", "/a/"), "/session/a");
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

    #[test]
    fn fill_node_paths_derives_path_ext_title() {
        let mut nodes = vec![
            VdfsNode::file("a.md", "", VdfsAccess::READ),
            VdfsNode::dir("sub", "子目录", VdfsAccess::LIST),
        ];
        fill_node_paths("mem", "", &mut nodes);
        assert_eq!(nodes[0].path, "/mem/a.md");
        assert_eq!(nodes[0].ext.as_deref(), Some("md"));
        assert_eq!(nodes[0].title, "a.md", "空标题回填为 name");
        assert_eq!(nodes[1].path, "/mem/sub");
        assert!(nodes[1].ext.is_none());

        // 已有 path 不被覆盖（显式路径优先）；空 path 按 base_rel 推导
        nodes[0].path = "/explicit/keep.md".into();
        fill_node_paths("mem", "deep", &mut nodes);
        assert_eq!(nodes[0].path, "/explicit/keep.md", "已有 path 不被覆盖");

        let mut nested = vec![VdfsNode::file("b.txt", "B", VdfsAccess::READ)];
        fill_node_paths("mem", "deep", &mut nested);
        assert_eq!(
            nested[0].path, "/mem/deep/b.txt",
            "空 path 按 base_rel 推导"
        );
        assert_eq!(nested[0].ext.as_deref(), Some("txt"));
    }

    /// 自描述全部有缺省：`impl VdfsProvider for P {}` 即可编译——
    /// provider **不需要**提供任何挂载名（挂载是使用方的事）
    #[test]
    fn self_description_defaults_need_no_mount() {
        struct P;
        #[async_trait]
        impl VdfsProvider for P {}

        assert_eq!(P.label(), None, "label 缺省为空，由使用方以挂载名代替");
        assert_eq!(P.description(), None);
        assert_eq!(P.icon(), None);
        assert_eq!(P.order(), 100);
        assert_eq!(P.root_access(), VdfsAccess::LIST);
        assert_eq!(P.root_status(), VFDS_STATUS_ACTIVE);
        assert!(P.root_new_types().is_empty(), "缺省根下不可新建");
    }

    /// 挂载点节点携带 provider 声明的新建类型（使用方合成时回填）
    #[test]
    fn mount_node_carries_root_new_types() {
        struct P;
        #[async_trait]
        impl VdfsProvider for P {
            fn root_new_types(&self) -> Vec<VdfsNewType> {
                vec![VdfsNewType::new("session", "会话")]
            }
        }

        let p: DynVdfsProvider = Arc::new(P);
        let table = VdfsMountTable::new(vec![("session".to_string(), p.clone())]);
        let node = table.mount_node("session", &p);
        assert_eq!(node.name, "session");
        assert_eq!(node.kind, VFDS_KIND_MOUNT);
        assert_eq!(node.new_types.len(), 1);
        assert_eq!(node.new_types[0].ext, "session");

        // 未声明新建类型的 provider：挂载点节点不带 new_types
        struct Q;
        #[async_trait]
        impl VdfsProvider for Q {}
        let q: DynVdfsProvider = Arc::new(Q);
        let node = table.mount_node("other", &q);
        assert!(node.new_types.is_empty());
    }

    /// 导航可见性：缺省可见（不写属性）；声明不可见时挂载节点带 `nav_visible=false`
    #[test]
    fn mount_node_marks_nav_visibility() {
        struct Hidden;
        #[async_trait]
        impl VdfsProvider for Hidden {
            fn nav_visible(&self) -> bool {
                false
            }
        }
        struct Plain;
        #[async_trait]
        impl VdfsProvider for Plain {}

        let hidden: DynVdfsProvider = Arc::new(Hidden);
        let plain: DynVdfsProvider = Arc::new(Plain);
        let table = VdfsMountTable::new(vec![
            ("local".to_string(), hidden.clone()),
            ("session".to_string(), plain.clone()),
        ]);

        let h = table.mount_node("local", &hidden);
        assert_eq!(
            h.attributes
                .get(VFDS_ATTR_NAV_VISIBLE)
                .and_then(|v| v.as_bool()),
            Some(false),
            "声明不可见的挂载点应带 nav_visible=false"
        );
        // 隐藏只影响导航呈现，不改变能力：挂载点照旧是目录、照旧可列
        assert_eq!(h.kind, VFDS_KIND_MOUNT);
        assert!(h.access.list);

        let p = table.mount_node("session", &plain);
        assert!(
            p.attributes.get(VFDS_ATTR_NAV_VISIBLE).is_none(),
            "缺省可见的挂载点不写该属性（消费者按缺省可见处理）"
        );
        assert!(
            table.mount_nodes().iter().any(|n| n.name == "local"),
            "挂载表仍列出隐藏挂载点：隐藏是呈现层决定，不是能力裁剪"
        );
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

    // ==================== 容器组合视图 ====================

    use std::sync::Mutex;

    /// 记录型 provider：记下收到的**相对路径**，并原样回报
    struct Recorder {
        seen: Mutex<Vec<String>>,
        order: i32,
        label: Option<&'static str>,
    }

    impl Recorder {
        fn new(label: &'static str, order: i32) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
                order,
                label: Some(label),
            })
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl VdfsProvider for Recorder {
        fn label(&self) -> Option<&str> {
            self.label
        }

        fn order(&self) -> i32 {
            self.order
        }

        fn root_access(&self) -> VdfsAccess {
            VdfsAccess::LIST_TRAVERSE
        }

        async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
            self.seen.lock().unwrap().push(path.to_string());
            Ok(vec![VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE)])
        }

        async fn stat(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<VdfsNode> {
            Ok(VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE))
        }

        async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
            // 只认相对路径：全路径由组合视图回填
            Ok(VdfsContent::text("", format!("read:{path}")))
        }

        async fn write(
            &self,
            _ctx: &VdfsContext,
            path: &str,
            _c: &VdfsContent,
        ) -> VdfsResult<VdfsWriteResponse> {
            Ok(VdfsWriteResponse {
                path: String::new(),
                created: false,
                etag: Some(path.to_string()),
            })
        }

        async fn delete(&self, _ctx: &VdfsContext, _p: &str, _r: bool) -> VdfsResult<()> {
            Ok(())
        }

        async fn mkdir(&self, _ctx: &VdfsContext, _p: &str) -> VdfsResult<()> {
            Ok(())
        }

        async fn move_item(&self, _ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
            Err(VdfsError::Forbidden(format!("moved:{from}->{to}")))
        }

        async fn watch(
            &self,
            _ctx: &VdfsContext,
            _p: &str,
            sink: VdfsChangeSink,
        ) -> VdfsResult<()> {
            // 只报子树内相对路径
            sink(VdfsChange::new("x.md", VFDS_CHANGE_UPDATED));
            Ok(())
        }
    }

    fn as_provider(p: Arc<Recorder>) -> DynVdfsProvider {
        p
    }

    /// alpha(order 20) / beta(order 10) —— 传入顺序与展示顺序相反
    fn sample_table() -> (VdfsMountTable, Arc<Recorder>, Arc<Recorder>) {
        let a = Recorder::new("甲", 20);
        let b = Recorder::new("乙", 10);
        let table = VdfsMountTable::new(vec![
            ("alpha".to_string(), as_provider(Arc::clone(&a))),
            ("beta".to_string(), as_provider(Arc::clone(&b))),
        ]);
        (table, a, b)
    }

    #[test]
    fn mount_table_root_lists_mounts_by_order() {
        let (table, _, _) = sample_table();
        let root = table.root_node();
        assert_eq!(root.path, VFDS_ROOT);
        assert_eq!(root.kind, VFDS_KIND_MOUNT);
        assert!(root.is_dir());
        assert_eq!(root.title, "虚拟文件系统");

        let nodes = table.mount_nodes();
        assert_eq!(
            nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["beta", "alpha"],
            "按 provider 的 order 升序"
        );
        assert_eq!(nodes[0].path, "/beta");
        assert_eq!(nodes[0].title, "乙", "label 作 title");
        assert_eq!(nodes[0].kind, VFDS_KIND_MOUNT);
    }

    #[tokio::test]
    async fn mount_table_delegates_relative_paths_and_fills_full_paths() {
        let (table, a, _) = sample_table();
        let ctx = VdfsContext::empty();

        // 虚拟根 → 挂载点清单（无需 provider 参与）
        let items = table.list(&ctx, VFDS_ROOT).await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(a.seen().is_empty(), "list(/) 不应触达 provider");

        // 挂载根 → provider 拿到 ""（相对路径），返回项被回填全路径
        let items = table.list(&ctx, "/alpha").await.unwrap();
        assert_eq!(a.seen(), vec![""]);
        assert_eq!(items[0].path, "/alpha/a.txt");

        // 子目录 → provider 拿到 "sub"
        let _ = table.list(&ctx, "/alpha/sub").await.unwrap();
        assert_eq!(a.seen(), vec!["", "sub"]);

        // 读：provider 只见相对路径，返回内容被补全全路径
        let c = table.read(&ctx, "/alpha/a.txt").await.unwrap();
        assert_eq!(c.text.as_deref(), Some("read:a.txt"));
        assert_eq!(c.path, "/alpha/a.txt", "provider 未填 path 时回填");

        // 写：同上
        let w = table
            .write(&ctx, "/alpha/a.txt", &VdfsContent::text("", "x"))
            .await
            .unwrap();
        assert_eq!(w.path, "/alpha/a.txt");
        assert_eq!(
            w.etag.as_deref(),
            Some("a.txt"),
            "provider 收到的是相对路径"
        );

        // stat 挂载根由组合视图合成，provider 不参与
        let n = table.stat(&ctx, "/beta").await.unwrap();
        assert_eq!(n.kind, VFDS_KIND_MOUNT);
        assert_eq!(n.name, "beta");
        assert_eq!(n.title, "乙");
    }

    #[tokio::test]
    async fn mount_table_guards_mount_roots() {
        let (table, _, _) = sample_table();
        let ctx = VdfsContext::empty();

        assert!(matches!(
            table.read(&ctx, "/alpha").await.unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(matches!(
            table
                .write(&ctx, "/alpha", &VdfsContent::text("", "x"))
                .await
                .unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(matches!(
            table.delete(&ctx, "/alpha", true).await.unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(matches!(
            table.mkdir(&ctx, "/alpha").await.unwrap_err(),
            VdfsError::Invalid(_)
        ));
        // 虚拟根本身也不可操作
        assert!(matches!(
            table.read(&ctx, VFDS_ROOT).await.unwrap_err(),
            VdfsError::Invalid(_)
        ));
    }

    #[tokio::test]
    async fn mount_table_rejects_unknown_and_cross_mount_moves() {
        let (table, _, _) = sample_table();
        let ctx = VdfsContext::empty();

        let err = table.list(&ctx, "/nope").await.unwrap_err();
        assert!(matches!(err, VdfsError::NotFound(_)));
        assert!(err.to_string().contains("/alpha"), "提示现有挂载点");

        // 跨挂载点移动被拒
        assert!(matches!(
            table
                .move_item(&ctx, "/alpha/a", "/beta/a")
                .await
                .unwrap_err(),
            VdfsError::Invalid(_)
        ));
        // 同挂载点内移动：转发相对路径
        let err = table
            .move_item(&ctx, "/alpha/a", "/alpha/b")
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Forbidden(_)));
        assert!(
            err.to_string().contains("moved:a->b"),
            "provider 只收到相对路径"
        );
        // 挂载根不可移动
        assert!(matches!(
            table
                .move_item(&ctx, "/alpha", "/alpha/b")
                .await
                .unwrap_err(),
            VdfsError::Forbidden(_)
        ));
    }

    #[tokio::test]
    async fn mount_table_watch_prefixes_paths() {
        let (table, _, _) = sample_table();
        let ctx = VdfsContext::empty();
        let got: Arc<Mutex<Vec<VdfsChange>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_box = Arc::clone(&got);
        let sink: VdfsChangeSink = Arc::new(move |c| sink_box.lock().unwrap().push(c));

        table.watch(&ctx, "/alpha", sink).await.unwrap();
        let seen = got.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].path, "/alpha/x.md", "相对路径被补成全路径");
        assert_eq!(seen[0].change, VFDS_CHANGE_UPDATED);
    }

    #[tokio::test]
    async fn empty_mount_table_is_an_empty_vfs() {
        let table = VdfsMountTable::new(Vec::new());
        let ctx = VdfsContext::empty();
        assert!(table.is_empty());
        assert_eq!(table.names_hint(), "（无）");
        assert!(table.list(&ctx, VFDS_ROOT).await.unwrap().is_empty());
        assert!(matches!(
            table.list(&ctx, "/x").await.unwrap_err(),
            VdfsError::NotFound(_)
        ));
        assert_eq!(table.stat(&ctx, VFDS_ROOT).await.unwrap().path, VFDS_ROOT);
    }
}
