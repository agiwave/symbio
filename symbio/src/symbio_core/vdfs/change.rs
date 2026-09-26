//! 变更通知：VdfsChange 信封与 event_bus 总线帧的唯一解包入口。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

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
/// `kind == EVENT_BUS_KIND_VDFS`（本域关心的事件类型）。任一不符 → `None`
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
    if bus.get("kind").and_then(Value::as_str) != Some(crate::symbio_core::EVENT_BUS_KIND_VDFS) {
        return None;
    }
    VdfsChange::deserialize(bus.get("data")?).ok()
}

// ==================== 变更回调 ====================

/// 变更投递器：宿主在 `watch` 期间注入，provider 检测到变化时调用。
///
/// 回调必须是**同步且非阻塞**的（宿主内部做派发）；provider 不得在其中做 IO。
pub type VdfsChangeSink = Arc<dyn Fn(VdfsChange) + Send + Sync>;
