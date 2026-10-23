//! VDFS 线上形状（`vdfs/*` 协议信封）—— 插件侧唯一消费
//!
//! ## 与 core 的分工（重要）
//!
//! - **纯接口**（[`VdfsProvider`] trait 与其域类型）在 core：
//!   `symbio_core::vdfs`——那是 VDFS 的 centerpiece；
//! - **线路格式**（请求 / 响应信封 + 协议路径常量）在本文件——只有 vdfs 插件
//!   自己消费，**core 不暴露本文件的任何类型**。
//!
//! ⚠️ **三个例外**：`vdfs/root` / `vdfs/watch` / `vdfs/unwatch` **跨插件可见**
//! （`agent` 拼 Run 根地址、`cli` 订阅 vdfs 频道、前端按它们发请求），因此它们的
//! 常量**归 core**（`symbio_core::ROUTE_VDFS_*`），本文件只引用、不重复定义——
//! 同一路由两份常量就是「一个事实两个 owner」，必然漂移。
//!
//! 这与 model 插件把 `ModelProtocol` 钩子与注册常量收在
//! `plugins/model/protocols/` 的做法一致（见 `symbio_core::model_provider` 的模块文档）。
//!
//! `VdfsProvider`

use crate::symbio_core::{
    VdfsContent, VdfsItem, VdfsNode, ROUTE_VDFS_ROOT, ROUTE_VDFS_UNWATCH, ROUTE_VDFS_WATCH,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ==================== 协议操作路径 ====================
//
// 三个跨插件可见的路径（`vdfs/root` / `vdfs/watch` / `vdfs/unwatch`）不在此定义
// —— 它们归 `symbio_core::ROUTE_VDFS_*`（见模块文档的「三个例外」）。

/// 列目录（一级）
pub const VDFS_LIST: &str = "vdfs/list";
/// 树状遍历（递归；节点的 `t` 位控制可遍历性）
pub const VDFS_TREE: &str = "vdfs/tree";
/// 读元数据
pub const VDFS_STAT: &str = "vdfs/stat";
/// 读内容（`r` 位）
pub const VDFS_READ: &str = "vdfs/read";
/// 写内容（`w` 位）
pub const VDFS_WRITE: &str = "vdfs/write";
/// 删除节点
pub const VDFS_DELETE: &str = "vdfs/delete";
/// 新建目录
pub const VDFS_MKDIR: &str = "vdfs/mkdir";
/// 内容编辑（精确字符串替换）
pub const VDFS_EDIT: &str = "vdfs/edit";
/// 文件名模式搜索（glob）
pub const VDFS_SEARCH: &str = "vdfs/search";
/// 执行**节点动作**（provider 自持的动词，如「测试连接」）
pub const VDFS_ACTION: &str = "vdfs/action";

/// 全部 VDFS 操作（宿主据此判定是否为本协议请求）
pub const VDFS_OPS: &[&str] = &[
    ROUTE_VDFS_ROOT,
    VDFS_LIST,
    VDFS_TREE,
    VDFS_STAT,
    VDFS_READ,
    VDFS_EDIT,
    VDFS_SEARCH,
    VDFS_WRITE,
    VDFS_DELETE,
    VDFS_MKDIR,
    ROUTE_VDFS_WATCH,
    ROUTE_VDFS_UNWATCH,
    VDFS_ACTION,
];

// ==================== 请求 ====================

/// 只带一个地址的请求——`list` / `stat` / `read` / `delete` / `mkdir` /
/// `watch` / `unwatch` 共用。
///
/// 七种操作的入参形状**本就相同**（一个地址），因此只有一个信封类型；
/// `recursive` 是 `delete` 的附加位，对其余操作无意义（缺省 `false`，
/// 线上不出现）。不为形状相同的操作各造一个类型——那只会让「同一件事」
/// 在类型层面看起来像七件事。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsPathRequest {
    #[serde(default)]
    pub path: String,
    /// 删除目录时是否递归
    #[serde(default)]
    pub recursive: bool,
    /// 有界列表：至多返回多少项（`None` = 全量，与加窗口之前完全一致）。
    ///
    /// 单位是**provider 自己定的**：会话清单按「会话条目」计，转写按**根节点**计
    /// （一个 Turn 及其全部子孙算一个，因此实际返回条数可能多于 `limit`）。
    #[serde(default)]
    pub limit: Option<u32>,
    /// 有界列表：游标（**地址**），返回它之前的一页（`None` = 最新的一页）。
    #[serde(default)]
    pub before: Option<String>,
}

/// 树状遍历请求
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsTreeRequest {
    #[serde(default)]
    pub path: String,
    /// 最大深度（缺省 3；`0` = 不限）
    #[serde(default)]
    pub depth: Option<u32>,
    /// 最多返回节点数（缺省 500）
    #[serde(default)]
    pub limit: Option<u32>,
}

/// 执行节点动作请求
///
/// `action` 是 provider 自持的动词标识（VDFS 不解释），`payload` 原样透传；
/// 当前宿主约定的取值见 `VDFS_ACTION_TEST`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsActionRequest {
    pub path: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// 写入请求
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsWriteRequest {
    pub path: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub b64: Option<String>,
    /// 允许创建缺失的中间目录 / 节点
    #[serde(default)]
    pub create: bool,
}

impl VdfsWriteRequest {
    /// 转为内容体（线路信封 → 域类型的唯一入口）
    pub fn to_content(&self) -> VdfsContent {
        match (&self.text, &self.b64) {
            (Some(t), _) => VdfsContent {
                size: t.len() as u64,
                text: Some(t.clone()),
                create: self.create,
                ..Default::default()
            },
            (None, Some(b)) => VdfsContent {
                b64: Some(b.clone()),
                binary: true,
                create: self.create,
                ..Default::default()
            },
            (None, None) => VdfsContent {
                create: self.create,
                ..Default::default()
            },
        }
    }
}

/// 编辑请求（`vdfs/edit`）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsEditRequest {
    #[serde(default)]
    pub path: String,
    /// 待查找并替换的文本（必须精确匹配一次）
    pub old_string: String,
    /// 替换为的文本（默认空字符串）
    #[serde(default)]
    pub new_string: String,
}

/// 搜索请求（`vdfs/search`）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsSearchRequest {
    /// 搜索基目录（子树内相对路径，缺省 = 子树根）
    #[serde(default)]
    pub path: String,
    /// 文件名的 Glob 模式（相对、不含 `..` 遍历）
    pub pattern: String,
}

// ==================== 响应 ====================

/// `vdfs/list` 响应；`vdfs/root` 复用同一形状——后者的 `path` 就是**根地址**
/// （调用方没给地址，由宿主填上它自己挂的那个名字）。
///
/// `path` 是本响应**唯一**的地址字段，它存在的理由只有上面那一条：`vdfs/root`
/// 的调用方无从知道根叫什么。`vdfs/list` 走同一形状，于是那里它等于回显请求
/// 地址（同一份响应形状服务两个操作，不为回显单独造一个类型）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsListResponse {
    /// 被列出目录的全路径（`vdfs/root` 时为虚拟根地址）
    pub path: String,
    /// 目录自身节点
    pub node: VdfsNode,
    /// 子条目（地址 + 节点，见 [`VdfsItem`]）
    pub items: Vec<VdfsItem>,
}

/// `vdfs/tree` 响应（**扁平**条目列表——层级由每个条目的 `path` 表达）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsTreeResponse {
    /// 子条目（扁平；`path` 是各自的全路径，据此还原层级）
    #[serde(default)]
    pub nodes: Vec<VdfsItem>,
    /// 是否因深度 / 数量上限被截断
    #[serde(default)]
    pub truncated: bool,
    /// 因**配额耗尽**而没能展开的目录（截断时才非空）。
    ///
    /// 为什么必须回这一份：`d781a6c7` 号会话里，一次 `vdfs_tree({"depth":3})` 的 500 个
    /// 配额被 `.workbuddy-ai`(180) + `.git`(73) + `.symbio`(60) 吃掉大半，`docs/adr`
    /// 只剩一个目录节点——那 63 个 ADR 一个都没有，而回执**只说 `truncated: true`，
    /// 没说缺了什么**。于是模型开始**猜** ADR 编号（`../../../../docs/003-design/014-单环一表.md` …一路猜到 doc-link-allow D-010: 同行反引号路径是历史叙述——记录 d781a6c7 会话里模型实际猜过的编号（仓库只到 D-062，该文件当时不存在），非入链。
    /// `D-213.md`），逐个用 shell 试，132 次全是「系统找不到指定的文件」，会话跑
    /// 188 秒 181 次工具调用后被用户中止，**最终零产出**。
    ///
    ///
    /// 教训是通用的：**只声明「被截断了」而不说「缺了哪部分」，等于把补全工作交给
    /// 猜**。截断必须自带续查地址，消费者不必发挥想象力。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
    /// 自解释说明（截断时给出规模 + 续查指引；不截断时为空）。
    ///
    /// 与 `vdfs_search` 的 `message` 同款——那个回执会写「[结果已截断：显示前 1000 个
    /// 匹配]」，tree 此前什么都不写，于是同样的截断在 tree 上无声、在 search 上有声。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

impl VdfsTreeResponse {
    /// 收尾构造——**文案口径只有这一处**。
    ///
    /// 本仓 tree 有两份实现（`agent tools` 的 vdfs_tree 与 `host` 的 tree handler），
    /// 遍历部分各自 parent 走不同 provider，但**截断该怎么说**必须一个答案：两份各自
    /// 写一遍文案的结果是口径漂移（`PROTOCOL.md` 里那种「两个地方解释同一件事」的老
    /// 毛病），终有一天只有一份会带上新加的续查指引。
    pub fn finish(
        nodes: Vec<VdfsItem>,
        truncated: bool,
        pending: Vec<String>,
        count_limit: usize,
        root: &str,
    ) -> Self {
        let message = if !truncated {
            String::new()
        } else {
            format!(
                "[树已截断：本次最多返回 {count_limit} 个节点，部分目录未展开或只列了一部分]\n\
                 未展开的目录列在 `pending` 字段里——请对它们逐个 `vdfs_list`（或对子树单独\n\
                 `vdfs_tree` 并调大 limit）来补全。**不要凭这份输出推断里面有哪些文件名**：\n\
                 本次 {root} 的结果不完整，照编号或名字猜子路径必然出错。"
            )
        };
        Self {
            nodes,
            truncated,
            pending,
            message,
        }
    }
}

/// `vdfs/edit` 响应 —— 编辑是**访问层的组合操作**（`read` → 精确替换 → `write`），
/// **不属于 `VdfsProvider` trait**（provider 只出原子操作，组合逻辑只写一次）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsEditResponse {
    /// 实际替换次数（0 = 内容已为最新，无需修改）
    #[serde(default)]
    pub replaced: usize,
    /// 人类可读说明（成功 / 跳过原因）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `vdfs/search` 响应 —— 搜索是**访问层的组合操作**（递归 `list` + glob 过滤），
/// **不属于 `VdfsProvider` trait**；provider 的安全规则（白名单等）经 `list` 自动生效。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsSearchResult {
    pub results: Vec<String>,
    /// 是否因数量上限被截断
    #[serde(default)]
    pub truncated: bool,
}

// ==================== 事件 ====================
//
// 总线上**没有**独立的 `VdfsChangeEvent` 类型：信封与 provider 侧的
// [`VdfsChange`] 形状完全重合（`path` + 可选 `data`，且 `path` 由门面补成展示
// 地址后原样透传），复制一份只会造出「两个名字、一个形状」的翻译层——门面
// （`super::fs::UnifiedFs`）把 [`VdfsChange`] **原样**投上总线
// （`subscribe({ kind: 'vdfs' })`）。前端形状见 `tauri/src/schemas/vdfs.ts::VdfsChange`。
//
// `VdfsChange`

#[cfg(test)]
#[path = "protocol.test.rs"]
mod tests;
