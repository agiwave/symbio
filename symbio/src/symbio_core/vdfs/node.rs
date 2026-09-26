//! 节点与条目：VdfsNode 自述、VdfsItem（地址 + 节点）与 VdfsNewType（新建类型）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::access::VdfsAccess;
use super::words::{VDFS_KIND_DIR, VDFS_KIND_FILE, VDFS_STATUS_ACTIVE};

// ==================== 可接受的新建类型 ====================

/// 目录**可接受的新建元素类型**——「新建」入口的类型。
///
/// 一个目录（含 provider 根）**至多**声明一种自己能新建的元素类型
/// （[`VdfsNode::new_type`] 是 `Option`）：
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
/// 现在只留**类型**这一层：本结构描述「这类东西落成后长什么样」，由 provider
/// 自持。**导入不是类型的一种**，它是详情页上的一条动作
/// （[`VDFS_ACTION_IMPORT`](super::VDFS_ACTION_IMPORT)），与 [`VDFS_ACTION_EXPORT`](super::VDFS_ACTION_EXPORT) / `delete` 同级——
/// 见 `docs/DECISIONS.md` ADR-029。
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
/// ## 创建语义仍归 provider
///
/// 「新建」在机制上就是一次 [`VdfsRequest::Write`](super::VdfsRequest::Write)（`create: true`）。本结构
/// 只声明**草稿长什么样**；具体写什么、怎么校验，由 provider 在 `write` 中自持。
/// 需要「先选个本地包再落盘」这类**额外操作**时，那是详情页的动作
/// （[`VDFS_ACTION_IMPORT`](super::VDFS_ACTION_IMPORT)），不是本结构的字段。
///
/// [`entry::id_of`]: crate::providers::vdfs_id_of
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
}

// ==================== 节点 ====================

/// 虚拟文件系统节点（文件或目录）——**一份自述，不含地址**。
///
/// 目录与文件共用同一结构：由 [`VdfsAccess`] 的 `l`（可列）与 `r`（可读）区分形态；
/// `kind` 只承载**场景语义**（如 `session` / `model`），不参与机制判定。
///
/// ## 为什么这里没有 `path`
///
/// 地址是**某一份列表**给这个节点的定位，不是节点自己的属性——同一个节点可以在
/// 不同列表里以不同地址出现。实证：设置页的一项指向插件自己那份配置文档
/// （[`capability_entry_of`] 给它的地址是 `<目录名>/PLUGIN.yml`，落在**另一个挂载点**里），
/// 而同一份文档在自己的目录里就叫 `PLUGIN.yml`。若把 `path` 放进节点，这两个
/// 列表就必须各造一个节点副本，且「谁填的」无从判定。
///
/// 于是地址落在**条目**上（[`VdfsItem`]：地址 + 节点），由分发层按
/// `<父地址>/<name>` 回填，provider 只在「地址不是这个形状」时才自己填。
///
/// [`capability_entry_of`]: crate::symbio_core::capability_entry_of
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsNode {
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
    /// 同一个类型的多种落盘路径（表单填 / 整包导入）不是两种类型，后者是详情页
    /// 上的一条动作（[`VDFS_ACTION_IMPORT`](super::VDFS_ACTION_IMPORT)），不在这里堆成一张清单。
    ///
    /// `Box` 不是随手加的：`VdfsNewType` 带四个 `Option<String>` + `schema`
    /// （≈150 字节），而 `VdfsNode` 是**全系统数量最多**的类型
    /// （每条消息 / 每个文件 / 每个目录都是它）。这个字段只有**挂载点目录**才有，
    /// 内联进结构体等于给每个消息节点白背那 150 字节——`Option<Box<_>>` 是 8 字节。
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
            .or_else(|| vdfs_derive_ext(&self.name))
    }
}

/// 由名字推导扩展名（`prompts/a.md` → `md`；无扩展名 → `None`）
pub fn vdfs_derive_ext(name: &str) -> Option<String> {
    let base = name.rsplit('/').next().unwrap_or(name);
    let (stem, ext) = base.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

// ==================== 列表条目 ====================

/// 列表条目 = **地址 + 节点**。
///
/// 这是列表响应里元素的形状（[`VdfsResponse::List`](super::VdfsResponse::List) / `vdfs/list` / `vdfs/tree`），
/// 也是「地址为什么不在 [`VdfsNode`] 里」的答案：地址属于**这一次列举**，不属于
/// 节点——同一个节点可以在不同列表里以不同地址出现（见 [`VdfsNode`] 的文档）。
///
/// ## 线格式：与「带 path 的节点」逐字节相同
///
/// `node` 是 `#[serde(flatten)]` 的，因此线上形状仍是 `{path, name, title, …}`——
/// 这一层拆分**不改变任何既有消费者读到的 JSON**。
///
/// ## 谁填 `path`
///
/// - **默认**：provider 不填，分发层按 `<父地址>/<name>` 回填（[`VdfsNode::name`]
///   就是父节点内的路径段，所以这条推导总是成立）；
/// - **例外**：地址不是那个形状时由 provider 自己填——设置页的条目指向插件自己
///   那份配置文档（`<目录名>/PLUGIN.yml`，落在另一个挂载点里），这是当前唯一的
///   实例，也是本字段必须存在（而不能由消费者自己拼）的理由。
///
/// 「留空」不等于「没有地址」：它表示「按推导填」，填这件事由分发层负责，provider
/// 不需要知道自己在树里的位置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsItem {
    /// 条目地址（展示口径）；空 = 由分发层按 `<父地址>/<name>` 回填
    #[serde(default)]
    pub path: String,
    /// 节点自述（扁平展开到本条目上）
    #[serde(flatten)]
    pub node: VdfsNode,
}

impl VdfsItem {
    /// 一个还没有地址的条目（绝大多数 provider 用这个——地址由分发层推导）
    pub fn new(node: VdfsNode) -> Self {
        Self {
            path: String::new(),
            node,
        }
    }

    /// 显式给出地址（地址不是 `<父地址>/<name>` 时用）
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }
}

impl From<VdfsNode> for VdfsItem {
    fn from(node: VdfsNode) -> Self {
        Self::new(node)
    }
}

/// serde 辅助：`false` 不序列化（保持线上形状与「缺省即 false」的字段兼容）
///
/// 用于 [`VdfsNode::hidden`] 与 [`VdfsContent::create`] 这类**绝大多数情况为
/// false** 的布尔位——省掉它们能让既有消费者的 JSON 形状一字不变。
pub(super) fn is_false(b: &bool) -> bool {
    !*b
}
