//! 线格式节点的**类型面**已迁入 `symbio-kernel`；本文件保留宿主侧的转写地址与投影。
//!
//! 判据（D-188 C2b）：`ChatNode` 及同文件的值类型是 `ModelProvider` / 能力行的签名类型，
//! 住内核；`SEG_MESSAGES`、`message_of_node` 与呈现派生依赖宿主地址 / 业务知识，留在这里。

use crate::chat::principal::Principal;
use serde_json::Value;

pub use symbio_kernel::chat::chat_node::*;

// ==================== 转写地址与节点投影 ====================
//
// 下面两项从 `crates/symbio-plugin-session` 上移到 core：它们描述的是**跨插件的契约**，
// 不是会话插件的内部实现。上移之前，`crates/symbio-plugin-agent` 为了构造子会话转播桥，
// 不得不从会话插件的内部模块取类型 —— 那是全仓**唯一**一处
// 真跨插件引用，违反插件独立原则（判据 `plugin-entry-audit` E-009）。
//
// 判据：一个符号该不该在 core，看**消费方是谁**——只要有一个消费方不是它所在
// 的那个插件，它就不是那个插件的私事（与 D-023「准入判据 = 依赖方数量」同款）。

/// 会话内部：转写列表的**路径段**（ASCII，进地址）。
///
/// 转写是**列表**：`<根>/session/<id>/message` 的每一项是一条消息，顺序由
/// `seq`（唯一权威顺序锚点）决定。这个地址只服务**读面**（一次 `read` 拿整份
/// 历史）与**写面**（`vdfs/action` 的截断 / 清空）。
///
/// **实时面不在这个目录上**：一条消息的流式 = 它**自己那个地址**
/// （`<id>/message/<mid>`）上的 `delta` 增量，目录只承载列表。把实时面挂在目录上
/// 会让 `path` 的含义随帧类型漂移，且无法推广到第二类集合——理由见
/// D-025 追记。
pub const SEG_MESSAGES: &str = "message";

/// [`ChatNode`] 的**逆投影**：VDFS 节点 + 正文 ⇒ 一条消息。
///
/// ## 为什么需要它
///
/// VDFS 变更**只在热路径上带载荷**：消息帧的 `data` 就是那条 `ChatNode`
/// （`delta` / `content` / `status`），而资源信号是**无载荷**的。因此「拿到一条
/// 无载荷变更、要还原成消息」的消费端（如 agent 转播桥）只能 `stat` + `read`。
///
/// ## 为什么在 core
///
/// 它必须与「拼」（`crates/symbio-plugin-session/src/plugin/nodes.rs::message_node`）**成对演进**：
/// `attributes` 增字段时不可能只改一边。而消费方（agent 的转播桥）不是 session
/// 插件——契约住 core，两侧都依赖它，就不会有一侧从对方内部「借」实现。
///
/// `None` = 节点状态词不在 [`NodeStatus`] 的词表里（正常不该发生；
/// 发生即两侧已分叉，宁可丢这一条也不要造出一个状态错误的消息）。
pub fn message_of_node(node: &crate::VdfsNode, text: String) -> Option<ChatNode> {
    /// attributes 里的值都是 `json!(..)` 塞进去的，原样反序列化即可回读类型。
    fn attr<T: serde::de::DeserializeOwned>(
        m: &serde_json::Map<String, Value>,
        k: &str,
    ) -> Option<T> {
        serde_json::from_value(m.get(k).cloned()?).ok()
    }
    // `attributes.type` 按**词**读再走 [`legacy_shape_to_style_tag`] 折叠，而不是
    // serde 直进 `NodeKind`：存量数据里存的是当时的 wire 词（旧六值已不在枚举里，
    // serde 会静默失败成 `None`）。折叠产出的 style_tag 只在 meta **还没有**该键时
    // 注入——新写方带着完整形态进来时不覆盖。
    let (msg_type, legacy_tag) = attr::<String>(&node.attributes, "type")
        .as_deref()
        .and_then(legacy_shape_to_style_tag)
        .map(|(kind, tag)| (Some(kind), tag))
        .unwrap_or((None, None));
    let mut meta = node
        .attributes
        .get("meta")
        .cloned()
        .filter(|v| !v.is_null());
    // 存量属性里的旧结构键**非覆盖**折进 meta（与 style_tag 同一条纪律：新写方
    // 带着完整 meta 进来时不覆盖）。：顶层 `name` 退役后，属性
    // `tool_name` 的唯一归宿是 `meta.tool_name`。
    for (key, value) in [
        ("style_tag", legacy_tag.map(Value::from)),
        (
            "tool_name",
            attr::<String>(&node.attributes, "tool_name").map(Value::from),
        ),
    ] {
        let Some(v) = value else { continue };
        if meta
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .is_some()
        {
            continue;
        }
        let slot = meta.get_or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(obj) = slot.as_object_mut() {
            obj.insert(key.to_string(), v);
        }
    }
    let role: Option<NodeRole> = attr(&node.attributes, "role");
    // 存量 `role` 属性在**逆投影**时折成主体（[D-147]）：新写方的属性表只有
    // `principal`，这一折只服务尚未重写的存量节点；折完仍回填 `role` 读法，
    // 让 `node_role()` 的「主体在先」与旧属性读法两边一致。
    let principal = attr::<Principal>(&node.attributes, "principal").or_else(|| {
        role.clone()
            .map(|r| legacy_principal(r, attr::<String>(&node.attributes, "tool_name").as_deref()))
    });
    Some(ChatNode {
        id: node.name.clone(),
        role,
        msg_type,
        parent_id: attr(&node.attributes, "parent_id"),
        tool_call_id: attr(&node.attributes, "tool_call_id"),
        seq: attr(&node.attributes, "seq"),
        error: attr(&node.attributes, "error"),
        principal,
        // `message_node` 把「没有 meta」写成 `null`；这里还原成"没有"
        meta,
        status: Some(NodeStatus::of(&node.status)?),
        content: Some(MessageContent::Text(text)),
        ..Default::default()
    })
}

/// 存量 `role` 属性 → 主体的**一次性别名**（[D-069 §决策 6][]）。
///
/// 判据与落库点 `attributed` 同一份：人归 `person/local`，工具归工具名合成的
/// 真实地址，其余（助手 / 系统说明）归本机主智能体。只在
/// [`message_of_node`] 的逆投影里用作「属性表没有 `principal`」时的回填，
/// 新写方不走这条。
fn legacy_principal(role: NodeRole, tool_name: Option<&str>) -> Principal {
    match role {
        NodeRole::User => Principal::from_legacy(crate::PRINCIPAL_USER),
        NodeRole::Tool => match tool_name {
            Some(name) => Principal::tool_principal(name),
            None => Principal::from_legacy(crate::PRINCIPAL_MAIN),
        },
        _ => Principal::from_legacy(crate::PRINCIPAL_MAIN),
    }
}
/// 呈现派生的**宿主扩展**：形状谓词住内核，`title` / `icon` / `actions` 的翻译是宿主业务知识，
/// 故以扩展 trait 留在这里。调用点需 `use` 本 trait（与内建方法同形）。
pub trait ChatNodePresentationExt {
    /// 从节点**形状 + 状态 + meta**现算面向前端的呈现数据（幂等，不覆盖已有值）。
    fn with_derived_presentation(self) -> Self;
}

impl ChatNodePresentationExt for ChatNode {
    fn with_derived_presentation(mut self) -> Self {
        if self.is_reasoning() {
            self.title.get_or_insert_with(|| "思考过程".to_string());
            self.icon.get_or_insert_with(|| "💭".to_string());
        } else if self.is_compression() {
            self.title.get_or_insert_with(|| "上下文压缩".to_string());
            self.icon.get_or_insert_with(|| "🗜".to_string());
        } else if self.is_tool_call() {
            self.icon.get_or_insert_with(|| "🔧".to_string());
            if let Some(name) = self.tool_name().map(str::to_string) {
                self.title.get_or_insert(name);
            }
        }
        // 恢复动作：只在缺失时派生（前端交互面是唯一填充点，事实里永不含 actions）。
        if self.actions.is_none() {
            self.actions = derive_actions(&self);
        }
        self
    }
}

/// 从节点自身（形状 + 状态 + meta）派生可用恢复动作；无可给动作 → `None`。
fn derive_actions(node: &ChatNode) -> Option<Vec<ResumeAction>> {
    use NodeStatus::*;
    let failed = node.status == Some(Failed);
    let aborted = node.status == Some(Aborted);
    let waiting = node.status == Some(WaitingUserAction);
    let acts: Vec<ResumeAction> = if node.is_interaction() {
        match (waiting, prompt_kind(node).as_deref()) {
            (true, Some("confirm")) => vec![ResumeAction::Approve, ResumeAction::Reject],
            (true, Some("question")) => vec![ResumeAction::Answer],
            _ => vec![],
        }
    } else if node.is_tool_call() {
        let mut v = Vec::new();
        if failed && meta_bool(node, "recoverable") {
            v.push(ResumeAction::Retry);
            if meta_str(node, "failure_kind").as_deref() == Some("error") {
                v.push(ResumeAction::Supply);
            }
        }
        v
    } else if node.is_compression() {
        if failed {
            vec![ResumeAction::RetryCompaction]
        } else {
            vec![]
        }
    } else if node.is_run() {
        if failed || aborted {
            vec![ResumeAction::RetrySubtree]
        } else {
            vec![]
        }
    } else {
        vec![]
    };
    (!acts.is_empty()).then_some(acts)
}

/// 读 `meta` 顶层字符串字段（缺失 / 非串 → `None`）。
fn meta_str(node: &ChatNode, key: &str) -> Option<String> {
    node.meta
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// 读 `meta` 顶层布尔字段（缺失 / 非 bool → `false`）。
fn meta_bool(node: &ChatNode, key: &str) -> bool {
    node.meta
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 读 `meta.prompt.kind`（交互子形态：`confirm` / `question`）。
fn prompt_kind(node: &ChatNode) -> Option<String> {
    node.meta
        .as_ref()
        .and_then(|m| m.get("prompt"))
        .and_then(|p| p.get("kind"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
#[path = "chat_node.test.rs"]
mod tests;
