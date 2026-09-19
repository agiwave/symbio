//! VDFS 线上形状（`vdfs/*` 协议信封）—— 插件侧唯一消费
//!
//! ## 与 core 的分工（重要）
//!
//! - **纯接口**（[`VdfsProvider`] trait 与其域类型）在 core：
//!   `symbio_core::vdfs_provider`——那是 VDFS 的 centerpiece；
//! - **线路格式**（请求 / 响应信封 + 协议路径常量）在本文件——只有 vdfs 插件
//!   自己消费，**core 不暴露本文件的任何类型**。
//!
//! 这与 model 插件把 `ModelProtocol` 钩子与注册常量收在
//! `plugins/model/protocols/` 的做法一致（见 `symbio_core::model_provider` 的模块文档）。
//!
//! [`VdfsProvider`]: crate::symbio_core::vdfs_provider::VdfsProvider

use crate::symbio_core::vdfs_provider::{VdfsContent, VdfsNode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ==================== 协议操作路径 ====================

/// **进入地址空间**：列出虚拟根——**不给地址**。
///
/// 根叫什么归本插件（`fs::VDFS_ADDR_ROOT`），消费方不该知道它。于是需要一个
/// 「无地址入参」的入口：回包（[`VdfsListResponse`]）里的 `path` 即**根地址**，
/// 消费方拿到后把它当**运行期数据**持有，之后一律从父地址往下拼（像目录一样），
/// 不再问根。前端启动期调一次即可。
pub const VDFS_ROOT: &str = "vdfs/root";
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
/// 移动 / 重命名
pub const VDFS_MOVE: &str = "vdfs/move";
/// 内容编辑（精确字符串替换）
pub const VDFS_EDIT: &str = "vdfs/edit";
/// 文件名模式搜索（glob）
pub const VDFS_SEARCH: &str = "vdfs/search";
/// 订阅指定路径的数据变更
pub const VDFS_WATCH: &str = "vdfs/watch";
/// 取消订阅（与 watch 配对）
pub const VDFS_UNWATCH: &str = "vdfs/unwatch";
/// 执行**节点动作**（provider 自持的动词，如「测试连接」）
pub const VDFS_ACTION: &str = "vdfs/action";

/// 全部 VDFS 操作（宿主据此判定是否为本协议请求）
pub const VDFS_OPS: &[&str] = &[
    VDFS_ROOT,
    VDFS_LIST,
    VDFS_TREE,
    VDFS_STAT,
    VDFS_READ,
    VDFS_EDIT,
    VDFS_SEARCH,
    VDFS_WRITE,
    VDFS_DELETE,
    VDFS_MKDIR,
    VDFS_MOVE,
    VDFS_WATCH,
    VDFS_UNWATCH,
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
    /// 乐观并发：期望的当前版本
    #[serde(default)]
    pub etag: Option<String>,
}

impl VdfsWriteRequest {
    /// 转为内容体（线路信封 → 域类型的唯一入口）
    pub fn to_content(&self) -> VdfsContent {
        match (&self.text, &self.b64) {
            (Some(t), _) => VdfsContent {
                path: self.path.clone(),
                size: t.len() as u64,
                text: Some(t.clone()),
                etag: self.etag.clone(),
                create: self.create,
                ..Default::default()
            },
            (None, Some(b)) => VdfsContent {
                path: self.path.clone(),
                b64: Some(b.clone()),
                binary: true,
                etag: self.etag.clone(),
                create: self.create,
                ..Default::default()
            },
            (None, None) => VdfsContent {
                path: self.path.clone(),
                etag: self.etag.clone(),
                create: self.create,
                ..Default::default()
            },
        }
    }
}

/// 移动 / 重命名请求
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsMoveRequest {
    pub from: String,
    pub to: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsListResponse {
    /// 被列出目录的全路径（`vdfs/root` 时为虚拟根地址）
    pub path: String,
    /// 目录自身节点
    pub node: VdfsNode,
    pub items: Vec<VdfsNode>,
}

/// `vdfs/tree` 响应（扁平节点列表，`path` 字段表达层级）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsTreeResponse {
    pub path: String,
    pub nodes: Vec<VdfsNode>,
    /// 是否因深度 / 数量上限被截断
    #[serde(default)]
    pub truncated: bool,
}

/// `vdfs/delete` 响应
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsDeleteResponse {
    pub path: String,
}

/// `vdfs/move` 响应
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsMoveResponse {
    pub from: String,
    pub to: String,
}

/// `vdfs/edit` 响应 —— 编辑是**访问层的组合操作**（`read` → 精确替换 → `write`），
/// **不属于 `VdfsProvider` trait**（provider 只出原子操作，组合逻辑只写一次）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsEditResponse {
    pub path: String,
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

/// 总线上的数据变更事件。
///
/// provider 侧的 [`VdfsChange`]只有子树内相对路径；门面（[`super::fs::UnifiedFs`]）
/// 在投递前把路径补成**对外展示地址**（`.vdfsv2/<类别>/…` 或工作目录相对地址），
/// 形成本形状后经事件总线下发前端。消费者按 `path` 前缀自行分流、防抖重拉
/// （`subscribe({ kind: 'vdfs' })`）。
///
/// [`VdfsChange`]: crate::symbio_core::vdfs_provider::VdfsChange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsChangeEvent {
    /// 发生变更的节点全路径（对外展示口径）
    pub path: String,
    /// 变更类型（`created` / `updated` / `deleted` / `renamed` / `appended`）
    pub change: String,
    /// 重命名时的目标全路径
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// **追加型变更**（`appended`）携带的增量文本；其余变更为 `None`。
    ///
    /// 「追加」是列表项内容尾部新增一段——消费者据此增量应用，无需重读整个节点。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    /// **节点视图**（`created` / `updated` 可携带）：变更后该节点的元数据。
    ///
    /// 消费者据此免掉一次 `stat`；`None` = provider 未附带，消费者自行回读。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<VdfsNode>,
    /// **内容快照**（`created` / `updated` 可携带）：变更后该节点的正文。
    ///
    /// 与 `delta` 的区别是**全量 vs 增量**，两者不会同时出现；
    /// `None` = provider 未附带，消费者自行 `read`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_are_unique_and_prefixed() {
        assert_eq!(VDFS_OPS.len(), 14, "新增协议操作请同步本计数与文档");
        for op in VDFS_OPS {
            assert!(op.starts_with("vdfs/"), "协议路径必须以 vdfs/ 开头：{op}");
        }
        let mut sorted = VDFS_OPS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), VDFS_OPS.len(), "协议路径不得重复");
    }

    #[test]
    fn write_request_to_content_text_and_binary() {
        let c = VdfsWriteRequest {
            path: "/setting/local".into(),
            text: Some("{\"a\":1}".into()),
            ..Default::default()
        }
        .to_content();
        assert_eq!(c.text.as_deref(), Some("{\"a\":1}"));
        assert!(!c.binary);
        assert_eq!(c.size, 7);

        let b = VdfsWriteRequest {
            path: "/x/y".into(),
            b64: Some("AAEC".into()),
            ..Default::default()
        }
        .to_content();
        assert!(b.binary);
        assert_eq!(b.b64.as_deref(), Some("AAEC"));
    }

    #[test]
    fn write_request_carries_create_intent() {
        // create 位随线路信封进入域内容体（provider 据此区分新建 / 覆盖）
        let c = VdfsWriteRequest {
            path: "/session/abc.session".into(),
            text: Some(String::new()),
            create: true,
            ..Default::default()
        }
        .to_content();
        assert!(c.create, "create 位必须透传到 VdfsContent");
        assert_eq!(c.text.as_deref(), Some(""));

        // 缺省为覆盖（create = false），且 false 不污染线上形状
        let plain = VdfsWriteRequest {
            path: "/a".into(),
            text: Some("x".into()),
            ..Default::default()
        }
        .to_content();
        assert!(!plain.create);
        let v = serde_json::to_value(&plain).unwrap();
        assert!(v.get("create").is_none(), "false 不序列化");
    }

    #[test]
    fn list_response_roundtrips_node_paths() {
        let resp = VdfsListResponse {
            path: "/mem".into(),
            node: VdfsNode::dir(
                "mem",
                "内存",
                crate::symbio_core::vdfs_provider::VdfsAccess::LIST,
            ),
            items: vec![VdfsNode::file(
                "a.txt",
                "A",
                crate::symbio_core::vdfs_provider::VdfsAccess::READ,
            )],
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["items"][0]["name"], serde_json::json!("a.txt"));
        let back: VdfsListResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.items.len(), 1);
    }
}
