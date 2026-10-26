//! 落库回包：「谁落库、谁发帧」这条硬不变量的**唯一落地点**
//! （`../docs/109-vdfs-session-messages.md` §3.4）。
//!
//! ## 为什么这一段原来住在宿主外壳、现在住在转写域
//!
//! 它原先在 `plugin.rs` 里，于是被切的两片（驱动器、上下文）都反过来 `use super::plugin::`
//! 拿它——那是 [D-075](../../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P3 判据点名的
//! 「跨片调用走表或端口」里最机械的一类：这个函数的真实依赖是**会话引擎**
//! （`PersistentChatSession::apply`）与**两条发布通道**（执行期 sink / 活跃名册），
//! 插件那一格从未参与。owner 跟着真实依赖走，它就搬进转写域（消息级变更的唯一写入点与发射器）。
//!
//! ## 为什么单独立一个文件，而不是并进 `deliver.rs`
//!
//! `deliver.rs` 只放「一帧怎么发、要不要合、日志打几行」这类**纯机械**，不持有也不改动状态；
//! 本文件的 [`append_and_publish`] 要**落库**（在存储临界区内补权威 `seq`），
//! 两者混在一个文件里就会让「纯」那条边界失效。
//!
//! ## 测试在哪
//!
//! 追加与回包的行为经 `chat_loop` / `orchestrator` / `context` 三侧的用例覆盖——
//! 它是三条路径共用的出口，脱离调用方单测等于把同一段语义钉两遍
//! （见 `../docs/104-module-layout.md` §2）。

use super::super::active::ActiveSessionManager;
use super::super::chat_session::{Carrier, MessageChange, PersistentChatSession};
use symbio_core::chat_node as cm;
use symbio_core::PluginError;

/// 落库 + **落库回包**的唯一实现（`../docs/109-vdfs-session-messages.md` §3.4）。
///
/// 两步顺序写死在这里：`apply(Born)` 落库（存储在临界区内给消息补权威 `seq` /
/// `timestamp`，交回**权威副本**），随即把权威副本逐条下发——权威号由此换入实时面。
///
/// ## 为什么发的是交回的那份，而不是入参那份
///
/// 号是**临界区内补在存储自己的副本上**的，调用方手里那条没有号——发它等于把「没有
/// 号」写进前端（这也是 `append_messages` 返回消息而非条数的唯一原因）。因此这里
/// 下发的是它交回的权威副本，不必再从存储读回来找。
///
/// 正文以 `content`（整条替换）而非 `delta` 下发，正是为了这里：持有乐观副本 / 在途
/// 副本的消费端要**替换**成权威正文，而不是往自己那份后面再拼一遍。这也是 `delta` /
/// `content` 两个字段必须分开的原因——同一个消费端既可能需要追加（流式），也可能
/// 需要替换（权威副本对齐），而帧必须自证是哪一种。
///
/// ## 落库补身份在这里，不在别处（plan/11 批 1 ②）
///
/// 本函数是**唯一的**消息追加写点（生产代码里没有第二处往列表里追加节点），
/// 所以 [`attributed`] 补在这里就等于「每条持久化消息都带身份」——补在各处建消息
/// 的地方则必然漏：建消息的写点有七八处（用户发言 / 助手正文 / 工具结果 / 压缩
/// 记录 / 系统说明），漏一处就造出「有身份、没有身份」两种同义消息。
///
/// `session_principal` 是**本会话主体**，谁说话按 role 分：人（`user`）不随会话变，
/// 其余都是本会话主体说的。
///
/// ## 用户发言那条为什么也在这个出口里
///
/// 它由 `orchestrator/entry.rs` 直连存储追加——前端手里只有自己的**乐观副本**，
/// 其 `seq` 是本地游标发的号，永远拿不到存储分配的那个；不回包就是两套序号空间并存。
///
/// 只在**成功**时下发；落库失败原样返回 `Err`，错误怎么呈现交给调用方——三条路径的
/// 降级策略各不相同（chat_loop 发 `Warn` 继续、编排广播失败结局并中断、压缩只记日志），
/// 那是各自的领域判定，不该被这个出口统一掉。
pub(crate) async fn append_and_publish(
    session: &PersistentChatSession,
    messages: Vec<cm::ChatNode>,
    carrier: Carrier,
    mgr: &ActiveSessionManager,
    session_id: &str,
    session_principal: &str,
) -> Result<Vec<cm::ChatNode>, PluginError> {
    // 事实落格收在 `apply` 里（[D-054] 决策 2 的顺序反转）：`carrier` 声明
    // 「这条节点的诞生事实由谁落」——运行器承载的（用户发言 / 助手正文 / 工具结果）
    // 不另落，没有承载格的（压缩记录）落 `message.added`。
    let persisted = session
        .apply(MessageChange::Born {
            nodes: messages
                .into_iter()
                .map(|m| attributed(m, session_principal))
                .collect(),
            carrier,
        })
        .await?;
    for message in &persisted {
        mgr.transcript_apply(session_id, symbio_core::llm_message_frame(message))
            .await;
    }
    // 交回权威副本：loop 经地址面调用本条出口时，本地镜像要靠它换入存储分配的
    // `seq` / `timestamp`（[D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §4）。
    Ok(persisted)
}

/// 落库前补身份的**唯一执行点**：写方知道这一轮是谁在说话，只补**没标过**的——
/// 转播进来的别的会话 / 子智能体消息带着自己的主体，不得被本会话覆盖（那是串主体）。
///
/// 人说的话是 `user`（会话外的一方，不随会话变），其余（助手正文、工具结果、
/// 压缩记录、系统说明）都是本会话主体说的。见 [`append_and_publish`]：为什么补
/// 在落库这一个点上、而不是各处建消息时。
fn attributed(m: cm::ChatNode, session_principal: &str) -> cm::ChatNode {
    if m.principal.is_some() {
        return m;
    }
    use symbio_core::principal::Principal;
    let mut out = m;
    // 归属判据与 [] 的字符串主体一致，这里只把它折成 `Principal` 结构体：
    // `person/local` ⇒ Person 一族，`session/*` ⇒ Agent 一族（`from_legacy` 按 namespace 派生）。
    //
    // `role = tool` 的结果节点归**工具自己**（Tool 一族，地址由
    // `meta.tool_name` 合成）——authz 登记的主体行（`vdfs/write`…）就是这批值。
    // 没有工具名的（协议级失败调用）退回本会话主体，与字段删除前的今天同格。
    out.principal = Some(match out.node_role() {
        Some(cm::NodeRole::User) => Principal::from_legacy(symbio_core::PRINCIPAL_USER),
        Some(cm::NodeRole::Tool) => match out.tool_name() {
            Some(name) => Principal::tool_principal(name),
            None => Principal::from_legacy(session_principal),
        },
        _ => Principal::from_legacy(session_principal),
    });
    out
}

#[cfg(test)]
#[path = "publish.test.rs"]
mod tests;
