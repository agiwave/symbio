//! 唯一接口的操作载荷与出参：VdfsRequest / VdfsResponse 及写入、动作结果。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::change::VdfsChangeSink;
use super::content::VdfsContent;
use super::node::{VdfsItem, VdfsNode};

// ==================== 写入结果 ====================

/// 写入结果 —— [`VdfsRequest::Write`](super::VdfsRequest::Write) 的返回值。
///
/// ## 为什么没有「写到哪了」的地址
///
/// 写入的目标地址是**调用方给的**（[`VdfsProvider::dispatch`](super::VdfsProvider::dispatch) 的 `path` 参数），
/// 回传它等于把调用方已经知道的东西还回去。唯一调用方不知道的是**匿名写**
/// （打在目录自身上的那一次，见 [`VdfsRequest::Write`] 的两种目标形态）里
/// provider 生成的**名字**——那正是 [`Self::name`] 承载的唯一信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsWriteResponse {
    /// provider 生成的条目名（**仅匿名写有**；具名写为 `None`）
    ///
    /// 值是 [`VdfsNode::name`] 口径的**路径段**，不是地址：地址由调用方拿它和
    /// 自己请求的那个目录拼（它本来就知道请求的是哪个目录）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

// ==================== 动作结果 ====================

/// 动作结果 —— [`VdfsRequest::Action`](super::VdfsRequest::Action) 的返回值。
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
//   provider 生成并在 [`VdfsWriteResponse::name`] 里交回（那是使用方拿到新名字的
//   **唯一**途径；漏填等于新建之后找不到新节点）。
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
    /// 有界窗口，provider 不认就当没传（全量）——见 [`VDFS_PARAM_LIMIT`](super::VDFS_PARAM_LIMIT) /
    /// [`VDFS_PARAM_BEFORE`](super::VDFS_PARAM_BEFORE)
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

/// [`VdfsProvider::dispatch`](super::VdfsProvider::dispatch) 的响应：与请求变体一一对应。
///
/// [`VdfsResponse::Unit`] 承载「成功但没有产物」的操作（delete / mkdir /
/// watch / unwatch——失败走 `Err`，成功无值可带）。
///
/// **响应里不出现请求地址**：地址是调用方给的，回传没有信息量。唯一的例外是
/// [`VdfsResponse::Write`] 的 [`VdfsWriteResponse::name`]——匿名写（打在目录
/// 自身上的那一次）里 provider 生成的名字，调用方不可能知道。
#[derive(Debug, Clone)]
pub enum VdfsResponse {
    /// [`VdfsRequest::List`]：直接子节点清单（地址 + 节点，见 [`VdfsItem`]）
    List(Vec<VdfsItem>),
    /// [`VdfsRequest::Stat`]：节点元数据
    Stat(VdfsNode),
    /// [`VdfsRequest::Read`]：节点内容
    Read(VdfsContent),
    /// [`VdfsRequest::Write`]：写入结果（匿名写时带 provider 生成的名字）
    Write(VdfsWriteResponse),
    /// [`VdfsRequest::Action`]：动作结果
    Action(VdfsActionResult),
    /// 成功无产物（delete / mkdir / watch / unwatch）
    Unit,
}

impl VdfsResponse {
    /// 便捷构造：绝大多数 provider 的清单**没有地址知识**——条目地址由分发层按
    /// `<父地址>/<name>` 回填，因此这里只收节点，包成无地址的 [`VdfsItem`]。
    ///
    /// 也接受已经造好的 [`VdfsItem`]（`T: Into<VdfsItem>`）——清单里混有
    /// 「地址推得出来的条目」与「地址得自己填的条目」时不必手工拼两次。
    ///
    /// 地址不是那个形状时（设置页条目指向别的挂载点）才需要手工造 [`VdfsItem`]。
    pub fn list<I, T>(nodes: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<VdfsItem>,
    {
        Self::List(nodes.into_iter().map(Into::into).collect())
    }

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
    pub fn into_list(self) -> Option<Vec<VdfsItem>> {
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
