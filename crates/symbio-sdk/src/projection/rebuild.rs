//! 重建引擎 —— 事件网格 → `ChatNode` 节点树（D-049 的读侧真源）。
//!
//! ## 为什么需要它
//!
//! 目标态是「`v2-events.wal` 是会话消息的唯一事实源，`messages.json` 降为
//! 可重建的缓存」。读侧要给出前端与轮次窗口看的那份**图语义**消息列表，就必须有人
//! 把**事实**（事件）折回**节点树**。本模块就是那个人。
//!
//! 它与 `symbio_sdk::projection::transcript` 是**两个出口、同一份事实**：
//!
//! | 出口 | 形态 | 消费者 |
//! |---|---|---|
//! | `transcript()` | 扁平三角色（`role` / `text` / `tool`） | 组 prompt（模型只要对话内容） |
//! | 本模块 | `ChatNode` 节点树（`id` / `parent_id` / `type` / `status` / `seq`） | 前端列表、轮次窗口、会话叶子 |
//!
//! 两个出口**都不加工内容**——它们只是同一批事实的两种投影。任何「为了好看」的
//! 增删都会让模型或前端看到网格里没有的东西（`prompt_fidelity` 报的就是这个）。
//!
//! ## 形状来源：不在这里定义
//!
//! 本模块在 [D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §7 上移到 core：
//! session 的主读与 loop 的轮次窗口共用同一份折叠，不能各写一遍。
//!
//! **写入侧怎么构造，重建就怎么构造**——同一个节点只有一份形状定义。生产侧已经在
//! 哪里定义了它，就复用那一个：
//!
//! | 节点 | 形状的单点定义 | 重建怎么拿到 |
//! |---|---|---|
//! | Turn / Reasoning / Text / ToolCall / 工具结果 | [`crate::turn::node_build`] 的 `llm_build_*` | 直接调用 |
//! | 对话面节点（首响 / 汇报 / 答话） | [`crate::turn::node_build::dialog_node`]、[`crate::turn::facts::ReportDraft::to_message`] | 直接调用 |
//! | 用户发言 / 补充 | **写入侧内联构造**（CLI 请求体 / `turn_exec::supplement_into_request`） | 逐字复刻（无共享构造器） |
//!
//! 「逐字复刻」那两格是**已知的脆弱点**：没有共享构造器可调，只能靠判据钉住
//! （`rebuild.test.rs` 的形状复刻用例）。不在这里自作主张给用户消息补 `status`
//! ——落库那份没有，于是同一条发言在实时面与重建面是两个身份，前端的节点合并键
//! （把 `status` 算在内）当场失效。**重建不得"改进"形状**：改进属写侧，重建只负责
//! 把发生过的事原样折回来（D-049 的零行为变更）。
//!
//! ## 变更格不产节点（D-053）
//!
//! `message.edited` / `message.removed` 不在上表里，因为它们**没有自己的节点**——
//! 它们说的是「已有的某个节点现在读作 Y / 已经没了」，因此不经过 `nodes_of`，
//! 由 [`rebuild`] 主循环**先一步**按 id 折叠（表见 [`rebuild`] 的文档）。
//! 把它们塞进 `nodes_of` 会得到「一次编辑多出一条节点」，而那正是折叠要防的事。
//!
//! `message.added`（D-054）恰恰相反：它是
//! **诞生格**，载荷里每条消息自带完整形状（写侧 `message_events::added` 直给），所以
//! **每条都产节点**、经 `nodes_of` 的 `added_nodes` 逐条收下。边界同样封闭：只准
//! 「无其他承载方的节点」（压缩记录等界面文本）搭这格；带对话内容的节点必须由
//! 自己的诞生事实承载。
//!
//! `session.consolidated`（压缩快照）同理不产节点，但折法不同：它不是「改一条」而是
//! 「**整段被摘要取代**」，所以由 [`rebuild`] 的**前置换基**处理——先把它载荷里那份
//! 列表收成基线，再从它的下标之后接着折（见 [`rebuild`] 的文档）。
//!
//! **唯一允许的覆写是 `timestamp`**：构造器取「此刻」（`clock_now_ms`），而重建必须
//! 是**纯函数**（同一事件序列 ⇒ 逐字相同的输出，N1）。故本模块一律把时间戳覆写成
//! 事件自身的 `ts`——它才是「这件事发生在什么时候」的权威答案。
//!
//! ## 载荷里的节点结构（铺的路）
//!
//! 收束 / 进展 / 兜底三种格子共用一份产出形状（`turn_runner::produced_payload`），
//! 载荷里带这一组的产出身份与中途产出序列；`artifact.added` 带 `tool_call_id` /
//! `args` / `success`（**不复制分组标识**：分组按 `tool_call_id` 从发起这批调用的
//! 那条事实的 `calls[]` 反查）；
//! `turn.supplemented` 带 `id` / `ids`。结果节点 id 由调用 id 派生
//! （[`crate::event::result_node_id`]），不作为载荷键。**节点身份因此是
//! 事实的一部分**，重建不必凭空造号——只有「载荷确实没记的」（旧事件的补充格、
//! 恢复路的结果节点）才由事件 id 稳定合成（合成必须**稳定**，见
//! [`crate::event::event_payload_node_id`]）。
//!
//! ## `seq`：重建的顺序锚点
//!
//! 发号器、以及它与存储号为什么必须是两套，见 [`RebuildSeq`]。本模块只额外钉住
//! **确定性**这一条：同一事件序列恒得同一组 `seq`（纯函数，不看时钟、不随机）。
//! 前端要的是一个稳定且单调的锚点，不需要它是存储号。

use std::collections::HashSet;

use super::mutations::added_messages;
use super::mutations::is_reader_reply;
use crate::event::event_payload_node_id;
use crate::event::{
    event_produced_text_node_id, event_result_node_id, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE,
};
use crate::port_stamp::{STAMP_FAILURE, STAMP_KEY};
use crate::projection::vocab::{slot_of, ChatSlot};
use crate::projection::vocab::{ChatVocab, MessageVocab};
use crate::{Entity, Event, Verb};
use symbio_kernel::chat::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_COMPRESSION,
    CHAT_STYLE_TAG_REASONING, CHAT_STYLE_TAG_RUN,
};
use symbio_kernel::event::payload_str;

// 节点形状与汇报格的单点定义都在 core 词汇表（[D-082](../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三）：
// 写侧（落库）与重建侧（回放）引的是同一个构造器，留守层因此不必反向够驱动器。
use crate::turn::facts::ReportDraft;
use crate::turn::node_build::{
    dialog_node, llm_build_text_node, llm_build_tool_call_node, llm_build_tool_message,
};
use symbio_kernel::chat::principal::Principal;

/// 事件主体 → 节点身份：重建与写侧**同一条**派生（[`Principal::from_legacy`]）。
///
/// 助手一侧的节点（骨架 / 正文 / 思考 / 对话面）由本会话主体说，写侧落库时补的
/// 也是 `from_legacy(session_principal)`——两处同派生才认得出是同一个人。
fn actor_principal(e: &Event) -> Principal {
    Principal::from_legacy(&e.actor)
}

/// 用户一侧（发言 / 补充）：写侧 `attributed` 判 `NodeRole::User` ⇒ `person/local`。
fn user_principal() -> Principal {
    Principal::from_legacy(crate::keys::PRINCIPAL_USER)
}

/// **重建号**：[`rebuild`] 自己那台发号器（四套序号的归一表见 session 域文档
/// `crates/symbio-plugin-session/src/docs/109-vdfs-session-messages.md` §3.3）。
///
/// 它与存储分配的权威号是**两套各自递增的计数器，同写 `ChatNode.seq` 一个字段**——
/// 所以混用是本模块最容易顺手写出来的错：快照载荷里带的那份号是**存储层**的槽位号
/// （`snapshot_slot_seq` 的口径，量级可以很大且不连续），而重建的号必须是**本模块
/// 发号器**给出的连续顺序锚。两套号混用会让「快照列表内部单调、与后继事件不单调」，
/// 而前端的排序键只有一个。D-049 已判定「切源后 `seq` 分配属呈现细节」。
///
/// 做成类型之后，「把外来的号直接写进节点」不再是这个模块里一个能顺手写出来的动作：
/// 节点上的重建号只有一个来路——[`RebuildSeq::assign`]。
struct RebuildSeq(i64);

impl RebuildSeq {
    /// 发下一个号并写进节点。
    fn assign(&mut self, node: &mut ChatNode) {
        self.0 += 1;
        node.seq = Some(self.0);
    }
}

/// 从事件序列重建会话消息节点树（**纯函数**：同输入 ⇒ 逐字同输出）。
///
/// ## 顺序与去重
///
/// 顺序 = 事件的发射顺序；`seq` 按发射顺序单调发号。同 id 的节点只保留**首次**
/// 出现的那一份——收束载荷的 `tool_calls[]` 与 `artifact.added` 记的是同一批调用，
/// 只认先到的那条（它们形状相同，但结果正文只归 `artifact.added`，见模块文档）。
///
/// ## 折叠：不只是追加（D-053）
///
/// 事实网格里除了「追加了什么」，还有「**改过什么**」——`message.edited` /
/// `message.removed`。它们是**变更格**，语义不是「加一条节点」而是：
///
/// | 变更格 | 折叠动作 |
/// |---|---|
/// | `message.edited` | 按 id **整条替换**该节点（位置与 `seq` 原样保留） |
/// | `message.removed` | 按 id **丢弃**该节点（列表整体前移） |
///
/// 少了这一段，一次截断在重建面上等于**没发生过**——被删的消息会复活。
///
/// ## 压缩快照：**取代**前缀
///
/// `session.consolidated`（`message × progressed`）不是「改一条」，而是「**整段被摘要
/// 取代**」：它的载荷就是压缩后的整份消息列表，因此重建从它**开始**——先把它那份列表
/// 按序收下，再从它的**下标之后**接着折事实。它之前的事件一律不再参与——「被取代」发生在
/// **折叠侧**，不在读口：切片里带着那段前缀（`range(0)` 给完整史，前缀来自归档段，见
/// 会话插件的 `EventWalStore`）也照样不参与。
///
/// 少了这一段，一次压缩在重建面上等于**没发生过**——被压掉的消息会全部复活，
/// 而压缩的全部意义就是让它们不再进入上下文。
///
/// **只认最近一次**：更早的快照连它自己的列表都已被后来的快照取代。与变更格同一纪律
/// （先收索引、再投影）：快照排在它取代的那些事件之后，一趟顺序回放没法边扫边定。
///
/// ## 为什么被删节点的 `seq` 留缺口
///
/// 号按**发射顺序**发，删节点不回收号：`seq` 是**顺序锚**（谁在谁前面），不是
/// 条数。补号会让「同一段历史在删一条前后的两次重建里拿到不同的号」，而号是
/// 前端的排序键——补号的收益（连续）小于它带来的不稳定。
pub fn rebuild(events: &[Event], c: &ChatVocab, m: &MessageVocab) -> Vec<ChatNode> {
    let mut out: Vec<ChatNode> = Vec::new();
    // 节点 id → 它在 `out` 里的位置。
    //
    // 与 `out` 的对应关系必须**始终成立**，因此删除**不在循环里做**（`Vec::remove`
    // 会让它之后的所有位置失效）：先把要删的 id 记进 `gone`，循环结束再一次性
    // `retain`。这样 `at` 在整个循环里都是有效的，编辑能直接按下标改写。
    let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // 产物格不再自带分组标识：配对改成「按调用身份反查发起这批调用的那条事实」。
    // 那条事实的载荷带分组标识与 `calls[]`（每次调用的 `id`）；产物只带
    // `tool_call_id`。这比在每条产物上复制一份分组标识更准确——产物属于
    // **那一次调用**，不属于整批。
    //
    // **分组只来自 `tool.requested`**（`D-382` ①）：卡的存在由派发清单决定，不由
    // 助手往返的 `tool_calls[]` 倒推。一条产物若找不到发它的派发事实，就没有卡可挂。
    let group_of_call: std::collections::HashMap<String, String> = events
        .iter()
        .filter(|e| e.kind == m.tool_requested)
        .filter_map(|e| {
            let group = e.payload.get("root_id")?.as_str()?;
            Some((group, e.payload.get("calls")?.as_array()?))
        })
        .flat_map(|(group, calls)| {
            calls.iter().filter_map(move |call| {
                let id = call.get("id")?.as_str()?;
                Some((id.to_owned(), group.to_owned()))
            })
        })
        .collect();
    // 调用身份（provider 的 wire id）→ 工具卡节点 id。
    //
    // 两个身份**不同**：`tool_calls[].id` 是流式面上那张卡的节点 id（跨轮复用同一
    // wire id 时必须换一个节点 id，否则第二轮会更新第一轮的老卡）；`wire_id` 才是
    // 与产物 / 下一轮请求配对的键。重建若拿 wire id 当节点 id，实时面那张卡永远
    // 停在 `streaming`（t10 抓到）。这里只解「节点身份」，产物配对仍走 wire id。
    //
    // **两个来源，一份身份表**（`D-382` ③）：派发清单 `calls[]` 的
    // `{ id, node_id }` 是主来源（恢复后再次发问没有助手往返，只有它）；助手往返
    // 的 `tool_calls[]` 只补旧 WAL / 增量内容，不覆盖已有身份。同一批 `calls[]`
    // 先按 `call_id` 排序再折（`D-382` ②：顺序由调用 id 决定，不由到达时刻决定）。
    let mut node_of_call: std::collections::HashMap<String, String> = events
        .iter()
        .filter(|e| e.kind == m.tool_requested)
        .filter_map(|e| e.payload.get("calls")?.as_array())
        .flat_map(|calls| {
            let mut rows: Vec<(String, String)> = calls
                .iter()
                .filter_map(|call| {
                    let wire = call.get("id")?.as_str()?;
                    let node = call
                        .get("node_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())?;
                    Some((wire.to_owned(), node.to_owned()))
                })
                .collect();
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            rows
        })
        .collect();
    for call in events
        .iter()
        .filter_map(|e| e.payload.get("tool_calls")?.as_array())
        .flatten()
    {
        let Some(node) = call.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let wire = call
            .get("wire_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(node);
        node_of_call
            .entry(wire.to_owned())
            .or_insert_with(|| node.to_owned());
    }

    // 被 `message.removed` 判死的节点 id（循环末尾统一剔除）。
    let mut gone: HashSet<String> = HashSet::new();
    let mut seq = RebuildSeq(0);
    // 发言被写侧闸拒的轮（拒因格载荷 `key: chat.assistant.final`，D-390）。
    //
    // 这些轮的那次**答复**（无工具调用的往返）是模型产出的**事实**，但它从未获准
    // 进入对话——fail-closed 的呈现面要求它不出现在消息列表里。判据是跨事件的
    // （拒绝格排在应答格之后），所以先收索引、再折（与变更格同一纪律）。
    // 认的是**格位 + 载荷的语音键**，不是事件名：拒因事实的 owner 已按写方拆开，
    // 名字是各插件的私有拼写（[`ChatVocab::denies_speech`]）。
    // 配对键是**触发这一轮的事实身份**（`root_id`，即那条 `user.message` 的 id）：
    // 拒绝格与应答格都带同一位，不靠任何「第几轮」的编号。
    // 工具轮的中途正文不受影响：它们属已经准许的循环，不是被拒的那句话。
    let denied_speech: HashSet<String> = events
        .iter()
        .filter(|e| c.denies_speech(e))
        .filter_map(|e| {
            e.payload
                .get("root_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .collect();

    // ── 压缩快照：先收下它那份列表，再从其下标之后接着折（见函数文档）────────
    //
    // `keep_from`（D-401）：切片只压了最旧前缀时，首条未压事实在快照事件**之前**，
    // 必须从那里起折；否则被保留的后缀会被整段跳过。旧快照没有这一格 ⇒ 回落
    // `快照下标 + 1`，与旧行为逐字一致。
    let (start, snapshot_messages) =
        match crate::projection::mutations::mutations_latest_snapshot(events, m) {
            Some((i, msgs, keep_from)) => {
                let start = keep_from
                    .as_deref()
                    .and_then(|id| events.iter().position(|e| e.event_id.as_str() == id))
                    .filter(|index| *index < i)
                    .unwrap_or(i + 1);
                (start, msgs)
            }
            None => (0, Vec::new()),
        };
    for mut m in snapshot_messages {
        // 号按**本函数自己的发号器**重发，不用载荷里那份（口径见 [`RebuildSeq`]）。
        seq.assign(&mut m);
        at.insert(m.id.clone(), out.len());
        out.push(m);
    }

    for e in &events[start..] {
        if e.entity == Entity::Message {
            if e.verb == Verb::Asserted && e.kind == m.message_edited {
                if let Some(m) = edited_node(e) {
                    if let Some(&i) = at.get(&m.id) {
                        // 位置与顺序锚**原样保留**：编辑改的是内容，不是这条在历史里
                        // 的位置。重新发号等于把这条挪了个地方——前端按 `seq` 排序，
                        // 轮次窗口按位置切片，两者都会因此错位。
                        let kept = out[i].seq;
                        out[i] = m;
                        out[i].seq = kept;
                    }
                    // id 不在 `at` 里 ⇒ 这条节点的事实不在这段切片里（v1 时代的历史
                    // 消息、或切片被裁过）。**不硬插到末尾**：那会把它放到一个错误的
                    // 位置，比不显示更糟。与「同 id 只认首次」同一条纪律。
                }
                continue;
            }
            if e.verb == Verb::Closed && e.kind == m.message_removed {
                gone.extend(removed_ids(e));
                continue;
            }
        }
        // 产物格是**同一逻辑节点的新状态**（待用户动作的卡 → 恢复后的答复）：
        // 后来的事实覆盖先前的，位置与顺序锚保留。其余格子仍是「同 id 只认首次」
        // ——那里的重复是**引用**（收束的 `tool_calls[]`），不是状态推进。
        let carries_state = slot_of(e, c, m) == Some(ChatSlot::ArtifactAdded);
        for mut node in nodes_of(e, &denied_speech, &group_of_call, &node_of_call, c, m) {
            if gone.contains(&node.id) {
                continue;
            }
            if let Some(&i) = at.get(&node.id) {
                let carried_context = node
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("context"))
                    .cloned();
                // 终态移交的两格先取下来：`carries_state` 分支会把 `node` 整个搬走，
                // 之后不能再借它。
                //
                // 移交的判据是**状态不是 `Completed`**，不是「有没有 `error`」：中止轮
                // 没有错误信息（它没失败），但它的终态仍必须写到轮根上，否则这一轮
                // 「被用户按停」在读面与「正常结束」同形（`D-435`：中止轮不再为这件事
                // 造一条空正文节点）。成功轮不移交——既有节点自己的状态不被覆盖。
                let terminal = (node.status != Some(NodeStatus::Completed))
                    .then(|| (node.status.clone(), node.error.clone()));
                if carries_state {
                    let kept = out[i].seq;
                    out[i] = node;
                    out[i].seq = kept;
                }
                // 水位是**请求级**元数据：Turn 骨架的 id 就是发起这一轮的用户消息 id，
                // 该 id 已被用户节点占住（「同 id 只认首次」），骨架随之被合并掉。
                // 不能因此丢数——只补 `meta.context` 这一格，正文与状态的裁决不变。
                // 骨架只补字段、不参与发布身份（D-335）。
                if let Some(watermark) = carried_context {
                    let meta = out[i].meta.get_or_insert_with(|| serde_json::json!({}));
                    if meta.get("context").is_none() {
                        meta["context"] = watermark;
                    }
                }
                // 收束格的骨架与用户节点同 id（`D-335` 之后 `root_id` 就是用户消息
                // 事实 id）：这一轮的终态必须补到既有节点上，否则「以失败 / 中止结束」
                // 在读面不可见（`D-381` 的失败那一半；`D-435` 把中止那一半补上）。
                // `error` 缺席时**不覆盖**既有那格（中止没有错误信息，见上）。
                if let Some((status, error)) = terminal {
                    out[i].status = status;
                    if error.is_some() {
                        out[i].error = error;
                    }
                }
                continue;
            }
            seq.assign(&mut node);
            at.insert(node.id.clone(), out.len());
            out.push(node);
        }
    }

    // 变更格判死的节点在这里一次性剔除（见 `at` 的文档：循环里删会让下标失效）。
    if !gone.is_empty() {
        out.retain(|m| !gone.contains(&m.id));
    }

    // ── 未执行调用的投影补位（P1c-4 前置：节点是投影，不是可写对象）──────────
    //
    // 一笔工具债「没跑到」时只剩 `failed(port=tool)` 事实，没有结果子节点——调用卡
    // 与「未执行」结果子项因此都由投影器按事实渲染。真正跑过的调用已有
    // `artifact.added` 产出的两张卡，这里一条都不补；父节点终态由子事实齐备自然给出。
    let existing: HashSet<String> = out.iter().map(|n| n.id.clone()).collect();
    for mut node in unfinished_nodes(events, &existing, &out) {
        if existing.contains(&node.id) {
            continue;
        }
        seq.assign(&mut node);
        at.insert(node.id.clone(), out.len());
        out.push(node);
    }

    // 分支标注（[D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）：重试留下的几支折成成员根节点上的 `meta.branch`。
    // 在这里做而不是在读口做，是因为**两个读出口必须是同一棵树**（转写列表与会话叶子
    // 是同一份数据的两个地址）——标注属节点形状，随重建出格一次就够；
    // 「哪一支当前读」因此对重建的两个消费者同口径，不需要谁记得再调一次。
    crate::projection::branch::stamp_branches(&mut out, events, c);
    out
}

/// `message.edited` → **补丁应用后**的整条消息（载荷 `message`）。
///
/// 载荷不是合法 `ChatNode` ⇒ `None`（静默跳过）。与 `nodes_of` 对缺失字段的
/// 处置同一条口径：**重建必须对任意历史事件容错**（旧版本写下的、手工改过的
/// WAL 都不该让整次重建失败）。形状的唯一生产者是写侧 `message_events::edited`，
/// 它给出的形状由 `rebuild.test.rs` 的往返用例钉住。
fn edited_node(e: &Event) -> Option<ChatNode> {
    let m: ChatNode = serde_json::from_value(e.payload.get("message")?.clone()).ok()?;
    (!m.id.is_empty()).then_some(m)
}

/// `message.removed` → 被丢弃的节点 id 列表（载荷 `ids`）。
fn removed_ids(e: &Event) -> Vec<String> {
    e.payload
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 一条事件 → 它承载的节点（顺序即树内顺序）。
///
/// `denied_speech` 是本切片里发言被拒的分组（见 [`rebuild`] 的索引）。
/// `group_of_call` 是调用 id → 发起这批调用的那条事实给的分组（见 [`rebuild`]）。
/// `node_of_call` 是 wire id → 工具卡节点 id（见 [`rebuild`]）。
///
/// 分类走 [`slot_of`]（DE-024 D-203）：折叠 `match` 的是**闭集枚举**而不是名字，
/// 加一条对话面事件时 `match` 不全在编译期就红。协议格（`response` / `failed`）
/// 不在这张表上——它们是内核语法，名字仍住内核，按常量判。
fn nodes_of(
    e: &Event,
    denied_speech: &HashSet<String>,
    group_of_call: &std::collections::HashMap<String, String>,
    node_of_call: &std::collections::HashMap<String, String>,
    c: &ChatVocab,
    m: &MessageVocab,
) -> Vec<ChatNode> {
    // 压缩失败（D-402 ①）：`compact` 端口的 `failed` 不是这一轮对读者说的话，
    // 但它有独立的读面形状（失败压缩叶，重试入口挂在上面），必须先于模型往返分支。
    if let Some(node) = compression_failure_node(e) {
        return vec![node];
    }
    // 一次模型往返的产出（`response` / `failed`，端口 `model`、无 `via`）：
    // 正文 / 思考节点落在**应答落格的位置**，顺序因此是实时的
    // 「思考→卡片→思考→卡片」。内部往返（措辞 / 裁判 / 摘要）不是这一轮对
    // 读者说的话，一个节点都不发。
    if e.entity == Entity::Scope
        && e.verb == Verb::Asserted
        && (e.kind == EVENT_RESPONSE || e.kind == EVENT_FAILED)
    {
        return output_nodes(
            e,
            e.payload
                .get("root_id")
                .and_then(|v| v.as_str())
                .is_some_and(|turn| denied_speech.contains(turn)),
        );
    }
    match slot_of(e, c, m) {
        Some(ChatSlot::UserMessage) => vec![user_node(e)],
        // 收束 / 进展 / 兜底：三种格子共用一份产出形状（`produced_payload`），
        // 差别只在 `status`。它们只记**轮级结局**，正文 / 思考归对应的一次模型往返
        // （上一臂）——所以这里只出骨架（与无模型轮自己的正文）。
        Some(ChatSlot::FinalReply | ChatSlot::Fallback | ChatSlot::Progress) => produced_nodes(e),
        // 轮边界汇报：界面文本，进列表（用户看得见），但不是模型的对话内容。
        Some(ChatSlot::Reported) => vec![reported_node(e)],
        // 对话面开场白（`Escalate` 首响）：与汇报同形（带 `meta.surface = "reply"` /
        // `exclude_from_context`），只有理由码不同；父是**轮根**（`D-426`）——它是
        // 这一轮说给用户的第一句话，和工具卡 / 正文同级，不是独立的根级节点。
        Some(ChatSlot::Opening) => vec![opening_node(e)],
        // 工具结果：调用与结果两张卡都归这一格（一个事实一个 owner）。
        Some(ChatSlot::ArtifactAdded) => artifact_nodes(e, group_of_call, node_of_call),
        // 诞生格：**没有别的格可搭车**的节点在这里出生（压缩记录节点是最典型的一个）。
        // 按**发射顺序**插到列表末尾——顺序锚由本函数统一发号。
        Some(ChatSlot::MessageAdded) => added_messages(e),
        // 变更格与取代格在 [`rebuild`] 的循环里已经处理（改 / 删 / 换基），不产新节点。
        Some(ChatSlot::MessageEdited | ChatSlot::MessageRemoved | ChatSlot::Consolidated) => {
            Vec::new()
        }
        None => Vec::new(),
    }
}

/// 用户发言（`turn × opened`）：载荷 `{ text, tier, id }`。
///
/// `id` 是**流式期已广播的节点 id**（`user_node_id` 入格）——前端按 id 合并，
/// 重建必须复用它，否则同一条发言在实时面与重建面是两个身份。
///
/// ## 为什么不设 `status`（与"用户发言是完成态"的直觉相反）
///
/// 形状必须**逐字复刻落库那一份**，而落库的用户消息由调用方给（收件箱写入面
/// 经 `parse_inbox_message` 补的缺省，`..Default::default()`）——它**没有** `status`。
/// 重建若自作主张补一个 `Completed`，同一条发言在实时面（无 status）与重建面
/// （`completed`）就是两个身份：前端的节点合并键把 `status` 算在内
/// （`useChatConnection.ts` 的 `own` 键），合并当场失效、历史里冒出重复节点。
/// 「用户发言算不算完成态」是一个可以单独讨论的形状改进，但它属于**写侧**，
/// 不归重建——重建只负责把发生过的事原样折回来（D-049 的零行为变更）。
fn user_node(e: &Event) -> ChatNode {
    ChatNode {
        id: event_payload_node_id(e, "id"),
        principal: Some(user_principal()),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(text_of(e, "text"))),
        // `meta` 随载荷复刻（折进的合并补充带 `meta.supplement*`）：必须随用户格
        // 事实入格，否则同一条发言在实时面与重建面是两个身份。载荷没有该键 ⇒
        // `None`（普通发言），与落库那份同形。
        meta: e.payload.get("meta").filter(|m| !m.is_null()).cloned(),
        timestamp: Some(e.ts),
        ..Default::default()
    }
}

/// 轮边界汇报（`turn × asserted`）：界面文本。
///
/// 形状**复用写入侧的构造** [`ReportDraft::to_message`]
/// ——它就是落库镜像上那一格的定义（`dialog_node(text, "progress", true)`）。
/// 复抄一遍的代价是已知的：上一版落库侧抄漏 `msg_type`，节点落了库却不成形状；
/// 重建这边再抄一遍，迟早各错各的。`meta.exclude_from_context` 因此随构造一并得到
/// ——它是**前端与轮次窗口**读的判别位，缺了轮次窗口会把「给人看的汇报」当成模型的
/// 对话内容发进请求包。
///
/// `id` 由载荷给（缺则按事件 id 合成）；`timestamp` 覆写成
/// 事件自身的 `ts`（构造器取「此刻」会破坏纯函数性，见模块头）。
fn reported_node(e: &Event) -> ChatNode {
    let draft = ReportDraft {
        text: text_of(e, "text"),
        tool_rounds: e
            .payload
            .get("tool_rounds")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        quiet_ms: e
            .payload
            .get("quiet_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    };
    let mut m = draft.to_message(&actor_principal(e));
    m.id = event_payload_node_id(e, "id");
    m.timestamp = Some(e.ts);
    m
}

/// 对话面开场白（`turn × asserted`）：`Escalate` 首响。
///
/// 形状经 [`crate::turn::node_build::dialog_node`] **同一构造器**复刻（与 [`dialog_node`]
/// 及 `reported_node` 同一条纪律：构造只有一份，漂移才不可能）。理由码与
/// `exclude_from_context` 从载荷读——它们是这条节点的形状（`meta.reason` /
/// `meta.exclude_from_context`），由写侧 `apply_verdict` 原样透传。
///
/// ## 父是轮根（`D-426`）
///
/// `parent_id` 取载荷 `root_id`：首响是这一轮说给用户的第一句话，和工具卡 / 正文
/// 一样挂在用户消息节点下。根级会让结构轴递归的渲染把它排到整轮子树之后（首响
/// 变成轮末的一句），也会让按根切窗口的读数多算一轮。旧事件没有 `root_id`：节点
/// id 形如 `opening:<用户消息 id>`，按前缀回填——与 [`event_payload_node_id`] 缺字段
/// 时合成的同一条容错纪律，不把存量会话留在错误位置。
///
/// `id` 由载荷给（缺则按事件 id 合成，同 [`reported_node`]）；`timestamp` 覆写成
/// 事件自身的 `ts`（构造器取「此刻」会破坏纯函数性，见模块头）。
fn opening_node(e: &Event) -> ChatNode {
    let reason = e
        .payload
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let exclude = e
        .payload
        .get("exclude_from_context")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut node = dialog_node(
        text_of(e, "text").as_str(),
        reason,
        exclude,
        &actor_principal(e),
    );
    node.id = event_payload_node_id(e, "id");
    node.parent_id = e
        .payload
        .get("root_id")
        .and_then(|v| v.as_str())
        .filter(|root| !root.is_empty())
        .map(str::to_string)
        .or_else(|| node.id.strip_prefix("opening:").map(str::to_string));
    node.timestamp = Some(e.ts);
    node
}

/// 收束 / 进展 / 兜底格 → `[Turn 根, Reasoning 子?, Text 子?, rounds 的中途产出…]`。
///
/// 形状与 `crates/symbio-plugin-loop/src/message_build::llm_build_assistant_messages` 同源（同一判据：
/// reasoning-only 时那段内容归 Text 子节点，不另立 Reasoning 子，否则同一段内容
/// 在存储层出现两份）。
///
/// ## 答话轮走**这一条**，不再有 `dialog` 分支（`D-435`）
///
/// `chat.assistant.final` 的三个写方（模型端口 / 答话行 / 中止收束）交出同一份载荷
/// 形状，所以这里只有一条折叠路径：轮骨架（id = `root_id`，与用户节点同 id ⇒「同 id
/// 只认首次」并进那一条）+ 思考 / 正文子节点。对话面的两样标记（`surface` /
/// `reason`）走载荷 `meta`，随正文子节点复刻——与 [`user_node`] 同一条纪律，不是
/// 第二种节点形状。
///
/// 旧事件里答话写成 `dialog` 子对象的，`text` / `answer` 都还在顶层（写方两个键
/// 同值），折叠按下面同一条路径得到同一个节点；只有「`dialog` 里那段正文与顶层
/// 不一致」的残破事实才会少一个节点——那种事实没有第二条读法能自洽，不猜。
fn produced_nodes(e: &Event) -> Vec<ChatNode> {
    let Some(group) = payload_str(e, "root_id") else {
        return Vec::new();
    };
    let principal = actor_principal(e);
    let status = status_of(e);
    let direct = text_of(e, "text");
    // 末轮「思考即正文」：写侧把正文折成 `answer`（`text` 为空），读面按同一个数走，
    // 否则 `reasoning_only` 的判据两端对不上，正文子节点会缺席（t55）。
    let text = if direct.trim().is_empty() {
        text_of(e, "answer")
    } else {
        direct
    };
    let reasoning = text_of(e, "reasoning");
    // 兜底格的 `detail` = 这一轮**为什么失败**（写侧 = 会话收束的失败分支，见
    // `convergence::project_final_turn`）。
    // 它必须折回节点：界面的组级交代条（`TurnGroupNode` → `MessageErrorBox`）读的是
    // 节点 `error`，取不到就只剩一个 failed 状态 + 一串产出——用户看到的是「模型想了
    // 很久然后什么也没说」，而事实是「模型没答，原因 X」。失败原因在网格里是事实，
    // 到界面上却不是，等于把已记的东西丢掉。
    //
    // 读 `detail` 而不是 `reason`：后者是闭集（`fallback`），对用户没有信息量；
    // 闭集服务判据与统计，人话服务界面——这正是把两者分开的原因。
    let why = e
        .payload
        .get("detail")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    let mut nodes = Vec::new();

    // 骨架（一整轮响应）节点仅分组、无正文：与 `llm_build_assistant_messages` 同形。
    //
    // `meta.context` 随载荷复刻（写方 = 模型执行体在终答事实里交出本轮实测水位）。
    // 会话读数（`SessionSummary::context`）扫的就是这一格；载荷没有该键 ⇒ `None`
    // （旧事实 / provider 没回报用量），与「水位是 0」是两件事。
    nodes.push(
        ChatNode {
            id: group.clone(),
            principal: Some(principal.clone()),
            msg_type: Some(NodeKind::Skeleton),
            status: Some(status.clone()),
            error: why.clone(),
            meta: e
                .payload
                .get("context")
                .filter(|context| context.is_object())
                .map(|context| serde_json::json!({ "context": context })),
            timestamp: Some(e.ts),
            ..Default::default()
        }
        .with_style_tag(CHAT_STYLE_TAG_RUN),
    );

    // reasoning-only 的判据与落库侧**逐字相同**（`message_build::llm_build_assistant_messages`）：
    // 思考与正文是同一段内容且无独立文本回复时，内容归正文子节点。
    let reasoning_only = !reasoning.trim().is_empty() && reasoning.trim() == text.trim();

    if !reasoning.trim().is_empty() && !reasoning_only {
        nodes.push(
            ChatNode {
                id: payload_str(e, "reasoning_id").unwrap_or_else(|| format!("{group}-reasoning")),
                parent_id: Some(group.clone()),
                principal: Some(principal.clone()),
                msg_type: Some(NodeKind::Notice),
                content: Some(MessageContent::Text(reasoning)),
                status: Some(status.clone()),
                timestamp: Some(e.ts),
                ..Default::default()
            }
            .with_style_tag(CHAT_STYLE_TAG_REASONING),
        );
    }

    if !text.trim().is_empty() {
        // 正文子节点的 id 与写侧同源：优先取执行体交出的 `answer_id`（含
        // reasoning-only 那条分支），旧事实没有该键时按 core 的
        // [`event_produced_text_node_id`] 派生。
        let child_id =
            payload_str(e, "answer_id").unwrap_or_else(|| event_produced_text_node_id(e));
        // 形状经 `llm_build_text_node`（单点定义），只覆写「此刻」为事件时刻。
        let mut m = llm_build_text_node(&group, &text, None, child_id, &principal);
        m.timestamp = Some(e.ts);
        m.status = Some(status.clone());
        // 对话面标记（`surface` / `reason` / `exclude_from_context`）随载荷复刻：
        // 与 [`user_node`] 的 `meta` 同一条纪律——节点上的判别位必须来自事实，
        // 否则同一条发言在实时面与重建面是两个身份。work 轮的正文没有这一格。
        if let Some(meta) = e.payload.get("meta").filter(|m| !m.is_null()) {
            m.meta = Some(meta.clone());
        }
        nodes.push(m);
    }

    // 工具轮的**中途产出**（每轮正文 / 思考）：`rounds[]` 各带自己的节点 id。
    // 它们与末轮产出同为 Turn 根的子节点（`tool_dispatch` 把中途正文定格在同一个根下）。
    if let Some(rounds) = e.payload.get("rounds").and_then(|v| v.as_array()) {
        for (i, r) in rounds.iter().enumerate() {
            push_step_nodes(&mut nodes, &group, i, r, &status, e.ts, &principal);
        }
    }

    nodes
}

/// 一次模型往返的中途产出（`steps[i]`）→ 最多两个子节点（Reasoning / Text）。
fn push_step_nodes(
    nodes: &mut Vec<ChatNode>,
    group: &str,
    i: usize,
    r: &serde_json::Value,
    status: &NodeStatus,
    ts: i64,
    principal: &Principal,
) {
    let get = |k: &str| {
        r.get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    let text = get("text").unwrap_or_default();
    let reasoning = get("reasoning").unwrap_or_default();
    let reasoning_only = !reasoning.trim().is_empty() && reasoning.trim() == text.trim();

    if !reasoning.trim().is_empty() && !reasoning_only {
        nodes.push(
            ChatNode {
                id: get("reasoning_id").unwrap_or_else(|| format!("{group}-r{i}")),
                parent_id: Some(group.to_string()),
                principal: Some(principal.clone()),
                msg_type: Some(NodeKind::Notice),
                content: Some(MessageContent::Text(reasoning)),
                status: Some(status.clone()),
                timestamp: Some(ts),
                ..Default::default()
            }
            .with_style_tag(CHAT_STYLE_TAG_REASONING),
        );
    }
    if !text.trim().is_empty() {
        let id = if reasoning_only {
            get("reasoning_id").or_else(|| get("text_id"))
        } else {
            get("text_id")
        }
        .unwrap_or_else(|| format!("{group}-t{i}"));
        let mut m = llm_build_text_node(group, &text, None, id, principal);
        m.timestamp = Some(ts);
        m.status = Some(status.clone());
        nodes.push(m);
    }
}

/// 压缩失败（`failed` + `stamp.failure = compression`）→ 一个 `Failed` 压缩叶（D-402 ①）。
///
/// 成功那笔同样带这枚章（章按债行声明盖），所以只认 `failed` 格：成功仍由快照折出
/// `Completed` 的压缩叶。节点身份取失败载荷的 `compression_id`（按前缀指纹派生，
/// 含重试纪元；旧 WAL 无该字段时回落 `compression:<final_id>`）——读面把
/// `message.removed` 当永久隐藏，重试产物必须换身份才可读；`meta.failure_kind` 是原因
/// 分类，`meta.fingerprint` 是债源的收束键（诊断用）。节点被 `message.removed` 判死后
/// 由主循环统一剔除（重试入口）。
fn compression_failure_node(e: &Event) -> Option<ChatNode> {
    if e.kind != EVENT_FAILED {
        return None;
    }
    let failure = e.payload.get(STAMP_KEY)?.get(STAMP_FAILURE)?.as_str()?;
    if failure != "compression" {
        return None;
    }
    let id = match e
        .payload
        .get("compression_id")
        .and_then(serde_json::Value::as_str)
    {
        Some(id) => id.to_string(),
        None => format!("compression:{}", e.payload.get("final_id")?.as_str()?),
    };
    let detail = e.payload.get("detail").and_then(serde_json::Value::as_str);
    let text = e
        .payload
        .get("text")
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .or(detail)
        .unwrap_or("上下文压缩失败");
    let class = e
        .payload
        .get("failure_class")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("failed");
    let mut meta = serde_json::json!({ "failure_kind": class });
    if let Some(fingerprint) = e
        .payload
        .get("fingerprint")
        .and_then(serde_json::Value::as_str)
    {
        meta["fingerprint"] = serde_json::json!(fingerprint);
    }
    Some(
        ChatNode {
            id,
            principal: Some(actor_principal(e)),
            msg_type: Some(NodeKind::Notice),
            content: Some(MessageContent::Text(text.to_string())),
            status: Some(NodeStatus::Failed),
            timestamp: Some(e.ts),
            error: detail.map(str::to_string),
            meta: Some(meta),
            ..Default::default()
        }
        .with_style_tag(CHAT_STYLE_TAG_COMPRESSION),
    )
}

/// 一次模型往返（`response` / `failed`）→ 最多两个子节点（Reasoning / Text）。
///
/// 载荷自带这一轮的产出身份（`output_fields` 在落格前算定）：
/// - 有工具调用 ⇒ 正文取 `text`（思考另有节点）；
/// - 没有工具调用 ⇒ 正文取 `answer`（收束轮折出的那段）。
///
/// 只有**本轮自己的**往返（端口是词表里的模型端口、无 `via`）产节点：内部往返
/// （措辞 / 裁判 / 摘要）的 `response` 同样带正文，但那些话不是这一轮对读者说的。
/// 状态取载荷 `status`（`failed` 的停法）——`response` 没有该键 ⇒ `Completed`。
///
/// `speech_denied`：这一轮的发言被写侧闸拒 ⇒ **答复那一次**（无工具调用）不产节点。
/// 正文与思考仍是网格里的事实，但从未获准进入对话（fail-closed）；工具轮的中途产出
/// 不受影响——它们属于已经准许的循环。
fn output_nodes(e: &Event, speech_denied: bool) -> Vec<ChatNode> {
    if !is_reader_reply(&e.payload) || e.payload.get("via").is_some() {
        return Vec::new();
    }
    let Some(root) = payload_str(e, "root_id") else {
        return Vec::new();
    };
    // 节点主体 = **写这条事实的人**（`actor`），不是载荷里的 `principal`。
    // 后者是计费口径（谁发起这一轮，`model` 的两本账读它，`D-461`），
    // 拿它当发言人会把助手正文署成用户（t10：正文节点折出 `person/local`，
    // 实时面按「用户说的话」呈现，且与收束轮的答话节点主体不一致）。
    let principal = actor_principal(e);
    // 一次 `failed` 没有 `calls`：与「这次往返没有请求工具」同一条判据。
    let calls_empty = e
        .payload
        .get("calls")
        .and_then(|v| v.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(true);
    // 被拒的答复：不产节点（见本函数的 `speech_denied` 段）。
    if speech_denied && calls_empty {
        return Vec::new();
    }
    let mut step = serde_json::Map::new();
    for key in ["text_id", "reasoning_id", "reasoning"] {
        if let Some(value) = e.payload.get(key) {
            step.insert(key.to_string(), value.clone());
        }
    }
    let content = if calls_empty { "answer" } else { "text" };
    if let Some(value) = e.payload.get(content) {
        step.insert("text".to_string(), value.clone());
    }
    let mut nodes = Vec::new();
    push_step_nodes(
        &mut nodes,
        &root,
        0,
        &serde_json::Value::Object(step),
        &status_of(e),
        e.ts,
        &principal,
    );
    nodes
}

/// 端口债键 / 应答 `calls[].id` → **模型交回的调用 id**（节点身份）。
///
/// **兼容解析，只对旧数据生效**：历史 WAL 里应答的 `calls[].id` 是 `call_key`
/// 合成键（`…-c{idx:03}-{model_id}`），需要把尾段还原成模型 id。新数据里
/// `calls[].id` 就是 provider 的 wire id，不再经过任何剥键逻辑（原样返回），
/// 也不得参与新数据的匹配——匹配只认 wire id（见 `symbio-plugin-model` 的装配）。
///
/// 端口没给 id 时合成键没有尾段，此时原样返回合成键。
fn call_identity(id: &str) -> &str {
    if let Some((_, tail)) = id.rsplit_once("-c") {
        if let Some((digits, model)) = tail.split_once('-') {
            if digits.len() == 3 && digits.chars().all(|c| c.is_ascii_digit()) && !model.is_empty()
            {
                return model;
            }
        }
    }
    id
}

/// 把端口债键对回某次模型应答交回的调用；对不回的债（环侧另行开账的合成键）
/// 返回 `None`——投影面不从合成键里凭空造节点。
fn issued_call_id(events: &[Event], debt: &str) -> Option<String> {
    let id = call_identity(debt);
    events
        .iter()
        .rev()
        .any(|e| {
            e.kind == EVENT_RESPONSE
                && is_reader_reply(&e.payload)
                && e.payload.get("via").is_none()
                && e.payload
                    .get("calls")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter().any(|c| {
                            c.get("id")
                                .and_then(|x| x.as_str())
                                .is_some_and(|x| call_identity(x) == id)
                        })
                    })
                    .unwrap_or(false)
        })
        .then(|| id.to_string())
}

/// 未执行工具调用的投影补位：盖了 `stamp.failure = "tool_call"` 的 `failed`
/// 事实 → `[ToolCall 父?, 结果子]`。
///
/// ## 为什么需要它
///
/// 一笔工具债只按事实记账：`request(port=tool)` 说「这里有一次调用」，
/// `failed{why}` 说「它没跑到」。节点是事实的投影，因此调用卡与「未执行」结果子项
/// 都必须能由读侧折出来——写侧不再补占位（父节点、子节点都不是可写对象）。
///
/// ## 只补没有子节点的调用
///
/// 真正跑过的调用已由 `artifact.added` 产出调用卡与结果子节点（`existing` 里已有），
/// 这里不重复渲染。判据是**子节点**而不是「结果 id 在不在」：结果 id 由调用 id
/// 派生，而失败事实的债键可能另有形态，只按 id 判会把同一笔调用画出两张结果卡。
///
/// `why` 与端口执行体 `failed{why}` 同一份词表（`not_executed` / `aborted` / …），
/// 文案按**单笔**口径（环不引入批概念，也不再声称「同批前序」）。
fn unfinished_nodes(
    events: &[Event],
    existing: &HashSet<String>,
    projected: &[ChatNode],
) -> Vec<ChatNode> {
    let mut nodes = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for e in events {
        // 时序上「没跑到」只会以盖了 `failure = "tool_call"` 的 `failed` 出现；
        // 实体格位由事实自身带，这里不再重甄别（甄别写在 `nodes_of` 的格位判据里，
        // 避免同一判据两处）。
        if e.kind != EVENT_FAILED {
            continue;
        }
        if e.payload
            .get(STAMP_KEY)
            .and_then(|stamp| stamp.get(STAMP_FAILURE))
            .and_then(serde_json::Value::as_str)
            != Some("tool_call")
        {
            continue;
        }
        let Some(call_id) = payload_str(e, "id") else {
            continue;
        };
        // 同一笔的多次尝试各有 `failed`：只认首次出现的终局（与「同 id 只认首次」
        // 同一条纪律），重试链不该在投影面上画出多张结果卡。
        if !seen.insert(call_id.clone()) {
            continue;
        }
        // 端口债键 → **模型交回的调用 id**（节点身份）。开账兜底键是环侧合成的
        // （`…-c{idx:03}-{model_id}`），它与模型 id 不相等；节点面只认模型 id，
        // 因为那才是用户在流式面看到、并且下一轮请求要原样回灌的那一个。
        let Some(node_id) = issued_call_id(events, &call_id) else {
            continue;
        };
        // 已有子节点（真正跑过）⇒ 调用卡与结果卡都归 `artifact.added`，这里不补。
        let has_child = projected
            .iter()
            .any(|n| n.parent_id.as_deref() == Some(node_id.as_str()));
        if has_child {
            continue;
        }
        let why = text_of(e, "why");
        let req = events.iter().rev().find(|r| {
            r.kind == EVENT_REQUEST
                && r.payload.get("id").and_then(|v| v.as_str()) == Some(call_id.as_str())
        });
        let name = req
            .and_then(|r| {
                r.payload
                    .get("name")
                    .or_else(|| r.payload.get("tool"))
                    .and_then(|v| v.as_str())
            })
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        let args_text = req
            .and_then(|r| r.payload.get("args"))
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            })
            .unwrap_or_default();

        if !existing.contains(&node_id) {
            // 调用卡的父节点是**产生这批调用的那一次模型应答**的 `root_id`；
            // 按调用标识在 `calls[]` 里回找，才能把失败事实对回它所属的那一轮。
            // 回找不到 ⇒ 这不是一次模型发起的调用，不投影（见 `issued_call_id`）。
            let Some(resp) = events.iter().rev().find(|r| {
                r.kind == EVENT_RESPONSE
                    && is_reader_reply(&r.payload)
                    && r.payload.get("via").is_none()
                    && r.payload
                        .get("calls")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter().any(|c| {
                                c.get("id")
                                    .and_then(|x| x.as_str())
                                    .is_some_and(|x| call_identity(x) == node_id.as_str())
                            })
                        })
                        .unwrap_or(false)
            }) else {
                continue;
            };
            let root = payload_str(resp, "root_id").unwrap_or_else(|| format!("root-{node_id}"));
            let mut call = llm_build_tool_call_node(
                &root,
                node_id.clone(),
                name.clone(),
                args_text,
                None,
                &actor_principal(e),
            );
            call.timestamp = Some(e.ts);
            call.status = Some(NodeStatus::Completed);
            let mut meta = call.meta.clone().unwrap_or_else(|| serde_json::json!({}));
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("success".into(), serde_json::json!(false));
                obj.insert("failure_kind".into(), serde_json::json!(why));
            }
            call.meta = Some(meta);
            nodes.push(call);
        }

        let result_id = crate::event::result_node_id(&node_id);
        if !existing.contains(&result_id) {
            let mut result = llm_build_tool_message(
                &node_id,
                &unfinished_text(&why),
                Some(false),
                Some(result_id),
                name.as_deref(),
            );
            result.timestamp = Some(e.ts);
            // 「没跑到」不是「跑失败」：标 Failed 会被上下文过滤并多渲染一条 ⚠，
            // 与写侧占位同口径地以 Completed 交代（成因在 `meta.failure_kind`）。
            result.status = Some(NodeStatus::Completed);
            if let Some(obj) = result.meta.as_mut().and_then(|m| m.as_object_mut()) {
                obj.insert("failure_kind".into(), serde_json::json!(why));
            }
            nodes.push(result);
        }
    }
    nodes
}

/// 「没跑到」的人读文案（单笔口径）：成因码留在 `meta.failure_kind`，正文不重复批语义。
fn unfinished_text(why: &str) -> String {
    match why {
        "aborted" => "本次调用已中止（用户终止），未执行，未产生结果。".to_string(),
        other => format!("本次调用未执行（{other}），未产生结果。需要时请重新发起这一次调用。"),
    }
}

/// 工具格（`artifact × asserted`）→ `[本轮中途产出子（可选）, ToolCall 子, 结果子]`。
///
/// 载荷自带两张卡的身份：`tool_call_id`（provider 的 wire id；结果节点 id 由它派生）/
/// `args`（**已定稿的参数文本**）/ `success`。参数走 `llm_build_tool_call_node` 的
/// 文本入口——**不经 `Value` 重序列化**（键序一变，同一次调用在重建前后就是两个
/// 内容，见该函数的文档）。工具卡节点 id 不能取 wire id：它是流式面那帧的身份
/// （`tool_calls[].id`），由 `node_of_call` 反查（见 [`rebuild`] 的同名注释）。
///
/// 分组标识**不在产物载荷里复制**：它由 `group_of_call` 按 `tool_call_id` 反查
/// 发起这批调用的那条事实。查不到 ⇒ 这条产物不对应本切片的任何一次已发起调用，
/// 一个节点都不建。
fn artifact_nodes(
    e: &Event,
    group_of_call: &std::collections::HashMap<String, String>,
    node_of_call: &std::collections::HashMap<String, String>,
) -> Vec<ChatNode> {
    let call_id = payload_str(e, "tool_call_id");
    let Some(group) = call_id
        .as_deref()
        .and_then(|id| group_of_call.get(id))
        .cloned()
    else {
        return Vec::new();
    };
    // 这次调用的中途产出（思考 / 正文）**排在工具卡片之前**：它发生在模型请求
    // 工具之前，而落在这张卡片所属的格里正是为了拿到这个位置（写侧见
    // `turn_runner` 的产物收尾）。载荷没有中途产出（恢复路、以及把中途产出只记在
    // 收束格的老会话）⇒ 一个节点都不多发。
    let mut nodes = match e.payload.get("round") {
        Some(step) if step.is_object() => {
            let mut v = Vec::new();
            push_step_nodes(
                &mut v,
                &group,
                0,
                step,
                &NodeStatus::Completed,
                e.ts,
                &actor_principal(e),
            );
            v
        }
        _ => Vec::new(),
    };
    let call_id = call_id.unwrap_or_else(|| format!("{group}-call"));
    // 工具卡节点 id 用**节点身份**（`tool_calls[].id`），不是携带的 wire id：
    // 实时面那张卡是按节点身份发帧的，拿 wire id 建卡会让它永远停在 streaming。
    let call_node_id = node_of_call
        .get(&call_id)
        .cloned()
        .unwrap_or_else(|| call_id.clone());
    let success = e
        .payload
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let tool = text_of(e, "tool");
    // 卡上同时留下**wire id**（`tool_call_id`）：节点 id 是节点身份，产物 /
    // `user.answer` / 授权事实认的都是 wire id。丢了它，恢复类动作就只能在
    // 节点 id 与 wire id 之间猜（T27 的错配现场）。实时面构造时本就带着它，
    // 重建必须同形（`skip_serializing` 只挡落盘，不挡重建回放）。
    let mut call = llm_build_tool_call_node(
        &group,
        call_node_id.clone(),
        Some(tool.clone()),
        text_of(e, "args"),
        Some(call_id.clone()),
        &actor_principal(e),
    );
    call.timestamp = Some(e.ts);
    call.status = Some(NodeStatus::Completed);

    // 「没跑」类结果的**成因机读码**（not_executed / aborted / overflow_dropped…）：
    // 随载荷入格（`tool_dispatch` 从结果 meta 提取）就必须随重建出格——只在实时面可见、
    // 重启后消失 = 成因排查靠猜。父节点的 meta 与执行器落格口径一致
    // （`emit_parent_finalized`：success + failure_kind）。pending 类
    // （needs_user_action）走下面的复刻分支，不在这里。
    let failure_kind = e
        .payload
        .get("failure_kind")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if !failure_kind.is_empty() {
        // **并入**而非整格替换：构造侧的 `meta.style_tag` / `meta.tool_name` 是
        // 节点身份的一部分（后骨架与工具名只活在 meta 里），
        // 整格覆盖 = 重建一次丢一次形态词，前端折叠退回 `turn`（e2e t27 抓到）。
        let mut meta = call.meta.clone().unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("success".into(), serde_json::json!(success));
            obj.insert("failure_kind".into(), serde_json::json!(failure_kind));
        }
        call.meta = Some(meta);
    }

    // 结果节点 id 由 core 按调用 id 派生（`{call}-result`），与写侧同一份规则：
    // 结果与调用是两张卡，共用 id 会让前端把它们合成一条。
    let result_id = event_result_node_id(e);
    // 等待用户的结果节点是**待用户动作的卡**（`user_prompt` + `waiting_user_action`），
    // 不是普通结果文本：它的形状（`msg_type` / `status` / `meta.prompt`）随载荷入格，
    // 重建照复刻——否则实时面看得见的审批卡，重启后会变成一条普通工具结果
    // （`resume` 也就找不到可恢复的提问节点）。
    let needs_user_action = e
        .payload
        .get("needs_user_action")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let tool_principal = if tool.is_empty() {
        None
    } else {
        Some(symbio_kernel::chat::principal::Principal::tool_principal(
            &tool,
        ))
    };
    let mut result = if needs_user_action {
        ChatNode {
            id: result_id,
            parent_id: Some(call_node_id.clone()),
            principal: tool_principal,
            msg_type: Some(NodeKind::Interaction),
            content: Some(MessageContent::Text(text_of(e, "text"))),
            status: Some(NodeStatus::WaitingUserAction),
            meta: Some(serde_json::json!({
                "prompt": e.payload.get("prompt").cloned().unwrap_or(serde_json::Value::Null),
                "failure_kind": e
                    .payload
                    .get("failure_kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                "tool_name": tool,
            })),
            ..Default::default()
        }
    } else {
        let mut msg = llm_build_tool_message(
            &call_node_id,
            &text_of(e, "text"),
            Some(success),
            Some(result_id),
            if tool.is_empty() {
                None
            } else {
                Some(tool.as_str())
            },
        );
        // 「没跑」不是「跑失败」：带成因码（非 pending）的结果按 Completed 重建，
        // 与执行器落格时的口径一致（`not_executed_result` / 超限丢弃分支都标
        // Completed——标 Failed 会被上下文过滤并多渲染一条 ⚠）。成因码随行入
        // meta（机读），正文本身已是人读的提示。
        if !failure_kind.is_empty() {
            msg.status = Some(NodeStatus::Completed);
            if let Some(obj) = msg.meta.as_mut().and_then(|m| m.as_object_mut()) {
                obj.insert("failure_kind".into(), serde_json::json!(failure_kind));
            }
        }
        msg
    };
    result.timestamp = Some(e.ts);

    nodes.push(call);
    nodes.push(result);
    nodes
}

// ── 载荷取值（缺失一律静默给默认值：重建必须对**任意**历史事件容错）────────────

/// 取字符串字段（允许空串——正文可以是空串，那是「答了空话」的事实）。
fn text_of(e: &Event, key: &str) -> String {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// 载荷 `status` → 消息状态（未知 / 缺失按 `Completed`——网格里能落格的产出，
/// 默认语义就是「发生过且结束了」）。
fn status_of(e: &Event) -> NodeStatus {
    match e.payload.get("status").and_then(|v| v.as_str()) {
        Some("aborted") => NodeStatus::Aborted,
        Some("failed") => NodeStatus::Failed,
        Some("pending") => NodeStatus::WaitingUserAction,
        _ => NodeStatus::Completed,
    }
}

#[cfg(test)]
#[path = "rebuild.test.rs"]
mod tests;
