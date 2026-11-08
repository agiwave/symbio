//! 对话面的三张词汇表（[DE-024](../../../../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-203 裁决三 / D-206 同一形状）。
//!
//! ## 它替掉了什么
//!
//! core 的折叠（`rebuild` / `transcript` / `prompt_fidelity` / `branch` / `mutations` /
//! `turns` / `turnstate` /
//! `actors::commitment` / `actors::control`）此前**按名字**认事件：`e.kind ==
//! EVENT_ASSISTANT_FINAL`。名字住内核 ⇒「加一条对话面事件」= 改内核两张表 + 一条
//! `core-shape-allow`，那正是 DE-024 要拆掉的那条边。
//!
//! 折叠里真正需要的不是「这个名字叫什么」，而是「这条事实在我的算法里是哪一格」。
//! 把名字改成**构造期形参**之后，core 只留折叠的形状，拼写由调用方交进来：
//! 两个消费方（`crates/symbio-plugin-loop` 与 `crates/symbio-plugin-session`）各自从 owner 的清单生成物取常量，
//! core 一行都不读（D-205 的单名字版本扩成整面）。
//!
//! ## 为什么是三张表，不是一张
//!
//! 一张表交整面最省事，但它会让 `actors::commitment`（只认承诺四条）也收到「开场白叫什么」。
//! 耦合面比算法实际需要的宽，读代码的人会以为承诺折叠与发言格有关——那不是真的。
//! 按**面**切开之后，每张表的大小就是那个算法真正的依赖：
//!
//! | 表 | 字段 | 消费方 |
//! |---|---|---|
//! | [`ChatVocab`] | 10：开轮 / 分支两支 / 汇报 / 开场 / 定稿 / 兜底 / 进展 / 被拒 / 模型端口 | `turns` `turnstate` `branch` `rebuild` `transcript` `prompt_fidelity` |
//! | [`MessageVocab`] | 6：派发 / 产物 / 增 / 改 / 删 / 巩固 | `mutations` `rebuild` `transcript` `prompt_fidelity` |
//! | [`CommitmentVocab`] | 4：立约 / 履约 / 违约 / 断言 | `actors::commitment` |
//!
//! 三个折叠（`rebuild` / `transcript` / `prompt_fidelity`）确实要两张表——它们折的
//! 就是「对话 + 消息」这一整片，两个形参比一张 16 字段的合并表更诚实。
//!
//! ## 加一条新事件要动哪张表
//!
//! **只有当 core 的折叠语义本身变了**才加字段。加一个新能力（自带事件、自带投影）时
//! 三张表一个字段都不动——那正是这三张表存在的理由：它们不是事件登记处，是**已有折叠
//! 算法的参数表**，大小跟着折叠走，不跟着能力数量走。
//!
//! ## 字段为什么按算法角色命名
//!
//! `turn.branch` 与 `chat.assistant.reported` 同落 `Turn × Asserted`，`chat.assistant.final`
//! 与 `chat.assistant.fallback` 同落 `Turn × Closed`，`commitment.released` 与
//! `commitment.broken` 同落 `Commitment × Closed`——按格子命名字段会撞名，按事件名命名
//! 字段则这张表退化成名字表的别名。按「这一格在折叠里算什么」命名，才是真正的闭集。
//!
//! ## `&'static str` 而不是 `&'a str`
//!
//! 与 [`super::task_vocab::TaskVocab`] 的差别只在这一点，理由是可判的：对话面的拼写
//! **只**来自两处——owner 插件里手写的 `const` 与清单生成物里的 `const`，两处都是
//! `'static`。于是三张表都是 `Copy` 的纯值，`Projection::new` 的闭包直接捕获它们，
//! 不必在构造期拷一份所有权表（`TaskVocab` 借用 `'a` 是因为它允许夹具交运行期
//! 字符串，而这里没有那条需求）。用例交自己的字面量也照样是 `'static`。

/// 对话的**发言面**：一轮里谁说了一句话、这一轮怎么开的、为什么停。
///
/// 8 个事件名字段 + 1 个端口名字段 = 这一面的闭集。加一条发言类事件 ⇒ 加一个字段 ⇒
/// 编译期点出全部折叠与全部调用方；**内核与 core 的文件零 diff**，这正是本表存在的理由。
///
/// 「本轮为什么停」不在这里留名字字段：拒因事实的 owner 已按写方拆开（D-390），
/// 名字是各插件的私有拼写，读者认的是**格位 + 载荷的语音键**（见
/// [`ChatVocab::denies_speech`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatVocab {
    /// 开轮：`turn × opened`。折叠据它认「这一轮从哪开始」（幕锚）。
    pub user_message: &'static str,
    /// 重试留下的分支事实：`turn × asserted`，载荷 `{ of, into }`。
    pub branch: &'static str,
    /// 「此刻读哪一支」：`turn × asserted`，载荷 `{ root }`。
    pub branch_selected: &'static str,
    /// 中途汇报：`turn × asserted`。
    pub reported: &'static str,
    /// 开场白：`turn × asserted`。
    pub opening: &'static str,
    /// 定稿发言：`turn × closed`。**只有它算「这一轮答完了」**（`fallback` 不算）。
    pub final_reply: &'static str,
    /// 兜底：`turn × closed`。生成失败也必须有输出（I3）。
    pub fallback: &'static str,
    /// 用户看得见的进展：`turn × progressed`。
    pub progress: &'static str,
    /// 这一轮模型往返的**端口名**（`response` / `failed` 载荷里的 `port`）。
    ///
    /// 它和上面八个不是同类：上面是事件名，它是端口名。放在同一张表里的理由与
    /// 「这一面的闭集」相同——折叠需要判「这条往返是不是这一轮对读者说的」，
    /// 而那个判据的两个维度（端口、有无 `via`）只能由调用方交进来；写字面量就是
    /// core 又认识了一个世界名（D-203 / D-206 同一形状）。
    pub model_port: &'static str,
}

impl ChatVocab {
    /// 这一条事实是**定稿**还是兜底：收束格上两个名字的唯一区分处。
    ///
    /// 收束格的读侧闸比写侧宽（兜底也是一条输出），所以「算不算这一轮答完了」只在
    /// 定稿上成立——两处各写一遍这个判断就是两套口径。
    pub fn closes_as_final(&self, kind: &str) -> bool {
        kind == self.final_reply
    }

    /// 这一条事实落在**收束格**上吗（定稿 ∨ 兜底）。
    pub fn is_closure(&self, kind: &str) -> bool {
        kind == self.final_reply || kind == self.fallback
    }

    /// 这一条事实是**助手说过的一句话**吗（汇报 / 开场白 / 定稿 / 兜底 / 进展）。
    ///
    /// 静默时钟的起点候选之一：用户看得见的任何一次动静。五个名字同属「助手说话了」
    /// 这一档语义，按名字点收会漏掉新加的那一种，故收成一个判据。
    pub fn is_spoken(&self, kind: &str) -> bool {
        kind == self.reported
            || kind == self.opening
            || kind == self.final_reply
            || kind == self.fallback
            || kind == self.progress
    }

    /// 这一条事实是不是「本轮发言被写侧闸拒」的停格。
    ///
    /// 判据是**格位 + 载荷键**，不是事件名：拒因事实的名字已按写方私有化（D-390），
    /// 会话仍要认出「本轮那句答复被拒了」。`key` 取本表自己的定稿格名
    /// `chat.assistant.final`，格位取 `(Scope, Asserted)`——写侧闸的否定事实只有这一格，
    /// 而 `chat.assistant.final` 本身落在 `Scope × Closed`，不会误伤定稿。
    ///
    /// 工具侧的授权拒因落在同一格位、但 `key` 是调用身份（不是语音键），因此不命中。
    pub fn denies_speech(&self, e: &crate::Event) -> bool {
        e.entity == crate::event::Entity::Scope
            && e.verb == crate::event::Verb::Asserted
            && e.payload.get("key").and_then(|v| v.as_str()) == Some(self.final_reply)
    }
}

/// 消息**列表的变更面**：追加 / 改写 / 删除 / 巩固，以及落在同一片事实里的产物。
///
/// 六个名字都改变「对话列表长什么样」，`mutations` / `changes` / `transcript` /
/// `rebuild` / `prompt_fidelity` 折的是同一片。`artifact.added` 在这里不是凑数：
/// 它是工具结果的**事实来源**（`rebuild` 据它折工具节点、`changes` 据它折改动账），
/// 与消息变更同属「这一轮留下了什么」。`tool.requested` 也不是凑数：工具卡片的
/// **出生来源**是它的 `calls[]`，助手往返只供增量内容（`D-382`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageVocab {
    /// 工具派发：`tool × asserted`，载荷 `{ root_id, calls[] }`。卡片按这里的
    /// `calls[]` 逐一出生（`D-382`），不由助手往返的 `tool_calls[]` 倒推。
    pub tool_requested: &'static str,
    /// 产物落地：`artifact × asserted`。工具结果也是一次结算。
    pub artifact_added: &'static str,
    /// 消息列表新增：`message × opened`。
    pub message_added: &'static str,
    /// 消息被改：`message × asserted`。
    pub message_edited: &'static str,
    /// 消息被删（投影层，Log 永不删）：`message × closed`。
    pub message_removed: &'static str,
    /// 会话巩固：`message × progressed`。载荷带整份快照与 `turns` 水位。
    pub consolidated: &'static str,
}

/// 承诺面：立约 / 履约 / 违约 / 对等断言四格。
///
/// 与 [`ChatVocab`] 分开的理由是域不同而不是消费方少：承诺的配对规则（载荷 `id`）
/// 是承诺域的形状知识，与发言折叠没有共同语义。声誉投影与成格函数是它仅有的两个
/// 消费方，把 11 条发言名塞给它们只会让耦合面说谎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitmentVocab {
    /// 立约：`commitment × opened`。
    pub opened: &'static str,
    /// 履约：`commitment × closed`。
    pub released: &'static str,
    /// 违约：`commitment × closed`。与履约同格，靠这一格区分。
    pub broken: &'static str,
    /// 断言履约（外部证据 / 违约告知）：`commitment × asserted`。
    pub asserted: &'static str,
}

/// 一条事件在对话折叠里占**哪一格**——闭集，加一档 = 编译期点出全部 `match`。
///
/// 它是三张词汇表与三个折叠（`rebuild` / `transcript` / `prompt_fidelity`）之间的
/// 唯一翻译：折叠不再各自把「实体 × 动词 × 名字」三目键写一遍（那会写三份，且每一份
/// 都要在加名字时改），而是 `match slot_of(..)`。协议五格与控制面事件不在这张表上
/// （`None`）——它们不属于对话折叠。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatSlot {
    /// 开轮（`turn × opened`）。
    UserMessage,
    /// 定稿发言（`turn × closed`）。**只有它算「这一轮答完了」**。
    FinalReply,
    /// 用户看得见的进展（`turn × progressed`）。
    Progress,
    /// 兜底（`turn × closed`）。
    Fallback,
    /// 中途汇报（`turn × asserted`）。
    Reported,
    /// 开场白（`turn × asserted`）。
    Opening,
    /// 产物落地（`artifact × asserted`）。
    ArtifactAdded,
    /// 消息列表新增（`message × opened`）。
    MessageAdded,
    /// 消息被改（`message × asserted`）。
    MessageEdited,
    /// 消息被删（`message × closed`）。
    MessageRemoved,
    /// 会话巩固快照（`message × progressed`）。
    Consolidated,
}

/// 分类：`None` = 这条事件不参与对话折叠（协议格、控制面、别的 owner 的格子）。
///
/// 先按 `(entity, verb)` 分派、再在**同一格内**比名字：`turn × asserted` 上挤着
/// 补充 / 汇报 / 开场白 / 派发 / 被拒，格子分不开它们，名字才是那一维的判据。
/// 最多三次字符串比较，与手写 `match` 的代价同阶。
pub fn slot_of(e: &crate::Event, c: &ChatVocab, m: &MessageVocab) -> Option<ChatSlot> {
    use crate::event::{Entity, Verb};
    match (e.entity, e.verb) {
        (Entity::Scope, Verb::Opened) if e.kind == c.user_message => Some(ChatSlot::UserMessage),
        (Entity::Scope, Verb::Closed) if e.kind == c.final_reply => Some(ChatSlot::FinalReply),
        (Entity::Scope, Verb::Closed) if e.kind == c.fallback => Some(ChatSlot::Fallback),
        (Entity::Scope, Verb::Progressed) if e.kind == c.progress => Some(ChatSlot::Progress),
        (Entity::Scope, Verb::Asserted) if e.kind == c.reported => Some(ChatSlot::Reported),
        (Entity::Scope, Verb::Asserted) if e.kind == c.opening => Some(ChatSlot::Opening),
        (Entity::Artifact, Verb::Asserted) if e.kind == m.artifact_added => {
            Some(ChatSlot::ArtifactAdded)
        }
        (Entity::Message, Verb::Opened) if e.kind == m.message_added => {
            Some(ChatSlot::MessageAdded)
        }
        (Entity::Message, Verb::Asserted) if e.kind == m.message_edited => {
            Some(ChatSlot::MessageEdited)
        }
        (Entity::Message, Verb::Closed) if e.kind == m.message_removed => {
            Some(ChatSlot::MessageRemoved)
        }
        (Entity::Message, Verb::Progressed) if e.kind == m.consolidated => {
            Some(ChatSlot::Consolidated)
        }
        _ => None,
    }
}

/// 一条事件在**对话读侧**承载的节点身份（`None` = 该格不产节点）。
///
/// ## 它是「哪一格对应哪个节点」的单点答案
///
/// 三个消费方都按它认节点：`rebuild`（节点树）、`transcript`（对话行）、
/// `prompt_fidelity`（变更索引的配对）。各写一遍的后果是**静默**的——
/// `message.edited` 定位不上要改的那条节点，表现为「编辑没生效」。
///
/// ## 它为什么从内核搬到这里
///
/// 旧实现在 `symbio_kernel::event::event_node_of`，判据是 `(entity, verb, kind)` 三目
/// 键——也就是把 21 条业务名字写进内核。名字迁走后那张键没法表达「哪些格子产节点」，
/// 而这件事**本来就是对话折叠的知识**：`ChatSlot` 已经逐格回答了「这一条在折叠里算
/// 什么」，节点身份只是它的一个附加属性。于是判据落在这里、按槽位枚举（加一档槽位
/// 会在此处编译期报不穷尽），内核只留纯载荷取法。
///
/// ## 只给「内容节点」，不给全部节点
///
/// 一次模型往返的正文与思考是两个节点（`response` / `failed` 各自承载），收束格只
/// **引用**其中正文那个的身份（`answer_id`）。对话读侧（转写行 / prompt 复核）只需要
/// 它；节点树（`rebuild`）要的是全部，所以它自己按节点逐个取。
///
/// 消息四格（`message.added/edited/removed` 与 `session.consolidated`）不在这里出现：
/// 它们承载的是**整条消息**，节点身份由载荷里的消息 id 与快照基线各自给出，不经过
/// 这条通路（旧实现同样让它们落 `_ => None`）。
pub fn slot_node_of(e: &crate::Event, c: &ChatVocab, m: &MessageVocab) -> Option<String> {
    match slot_of(e, c, m)? {
        // 开轮 / 汇报 / 开场：一格一句话，各带自己的节点 `id`。
        ChatSlot::UserMessage | ChatSlot::Reported | ChatSlot::Opening => {
            Some(crate::event::event_payload_node_id(e, "id"))
        }
        // 收束 / 进展 / 兜底：整格都是「本轮产出的正文」形状。
        ChatSlot::FinalReply | ChatSlot::Fallback | ChatSlot::Progress => {
            Some(crate::event::event_answer_node_id(e))
        }
        // 产物格：一格一张工具结果卡。
        ChatSlot::ArtifactAdded => Some(crate::event::event_result_node_id(e)),
        ChatSlot::MessageAdded
        | ChatSlot::MessageEdited
        | ChatSlot::MessageRemoved
        | ChatSlot::Consolidated => None,
    }
}

#[cfg(test)]
#[path = "vocab.test.rs"]
mod tests;
