//! 事实落格即投影：一次往返 / 一次调用的**终态节点**在事实落格那一刻折出并下发。
//!
//! ## 缺口
//!
//! 两条通道（`D-314`）各只覆盖半段：执行期 `sink` 发的是**瞬时帧**——模型轮首帧把
//! 思考 / 正文 / 工具卡建成 `streaming`，工具侧只补 `meta.started_at`；而**终态**
//! （结果子节点、工具卡 `completed`、思考 / 正文 `completed`）原先只有轮收尾的
//! `Convergence::project_final_turn` 才由 `rebuild` 折出。于是只要一轮里有工具调用，
//! 界面在整轮跑完前既看不到工具结果，思考与工具卡也一直停在运行态。
//!
//! ## 办法：让投影器在事实落格时跑（`D-424`）
//!
//! `D-312` 已定「节点是事实的投影，不是可写对象」——形状的唯一 owner 是 `rebuild`。
//! 因此补齐终态不是让执行体学会写节点，而是让**同一个投影器**早一步跑：事实落格后，
//! 取这条事实与它的**配对事实**（同一次调用的派发清单 / 同一轮的拒言事实）组成最小
//! 切片，`rebuild` 折出节点，清掉重建号后交给转写唯一写入点。实时面与重建面因此
//! 逐字同形——它们本来就是同一个函数。
//!
//! ## 为什么是「配对切片」而不是「本轮整片」
//!
//! 投影一条事实只需要它自己与配对事实：`artifact.added` 靠派发清单拿分组与卡片身份
//! （`D-382`），`response` / `failed` 靠拒言事实判本轮那句答复是否获准（`D-390`）。
//! 首响（`chat.assistant.opening`）没有配对，自己成切片——它是说给正在等的用户的
//! 第一句话，必须早于工具结果帧到达。全片重折既贵，也会把已被抑制 / 已删除的节点
//! 重新吐出来。
//!
//! ## 配对事实从哪来
//!
//! 同一轮的所有写方都经会话事实源的唯一写口，因此索引随写口更新（[`LiveIndex`]）；
//! 未命中时回读事实源兜底（索引与写入不同源句柄、或事实早于本进程落格的情形），
//! 不猜、不造。

use std::collections::HashMap;

use symbio_kernel::chat::chat_node as cm;
use symbio_kernel::event::{Event, Seq};
use symbio_kernel::store::Store;
use symbio_plugin_api::debt::{ReplyRole, STAMP_KEY, STAMP_ROLE};
use symbio_sdk::projection::rebuild::rebuild;

use super::super::store::EventWalStore;
use crate::declarations::{CHAT_VOCAB, MESSAGE_VOCAB};

/// 事实落格时的**配对索引**：实时投影只问「这条事实的伙伴在哪」。
///
/// 只收两类事实——收它就要问它的伙伴在两个地方：
/// - 派发清单（`tool.requested`）：产物格的工具卡身份与分组都由它给（`D-382`）；
/// - 拒言事实：`response` / `failed` 折正文节点前要知道本轮那句答复是否获准（`D-390`）。
#[derive(Default)]
pub(crate) struct LiveIndex {
    /// wire id（`calls[].id`）→ 最近一条含它的派发清单。同一 wire id 跨轮复用时后到的为准。
    requests: HashMap<String, Event>,
    /// `root_id`（本轮开端事实 id）→ 拒言事实。
    denials: HashMap<String, Event>,
}

impl LiveIndex {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 这条事实是否参与实时投影的配对；参与则收进索引（**不论它自己是否产节点**）。
    pub(crate) fn observe(&mut self, event: &Event) {
        if event.kind == MESSAGE_VOCAB.tool_requested {
            self.remember_request(event);
        }
        if CHAT_VOCAB.denies_speech(event) {
            if let Some(root) = root_of(event) {
                self.denials.insert(root, event.clone());
            }
        }
    }

    /// 落格事实的投影伙伴（按「先伙伴、后事实」的折序返回）。
    ///
    /// 索引先答；未命中时回读事实源——配对键（`tool_call_id` / `root_id`）是**载荷
    /// 里的身份**，回读只按它找，不拿「最近一条」顶替（顶替会把产物挂到别的一笔上）。
    pub(crate) fn partners(&mut self, event: &Event, wal: &EventWalStore) -> Vec<Event> {
        if event.kind == MESSAGE_VOCAB.artifact_added {
            let Some(call_id) = event.payload.get("tool_call_id").and_then(|v| v.as_str()) else {
                return Vec::new();
            };
            if let Some(request) = self.requests.get(call_id) {
                return vec![request.clone()];
            }
            let found = Store::range(wal, Seq::new(0)).into_iter().rev().find(|e| {
                e.kind == MESSAGE_VOCAB.tool_requested
                    && e.payload
                        .get("calls")
                        .and_then(|v| v.as_array())
                        .is_some_and(|calls| {
                            calls.iter().any(|call| {
                                call.get("id").and_then(|v| v.as_str()) == Some(call_id)
                            })
                        })
            });
            // 命中后收进索引：同一批调用的后续产物不必再扫一次。
            if let Some(request) = &found {
                self.remember_request(request);
            }
            return found.into_iter().collect();
        }
        // 其余（`response` / `failed`）：拒言事实可能不存在（没被拒），空切片就是答案。
        root_of(event)
            .and_then(|root| self.denials.get(&root).cloned())
            .into_iter()
            .collect()
    }

    fn remember_request(&mut self, event: &Event) {
        let Some(calls) = event.payload.get("calls").and_then(|v| v.as_array()) else {
            return;
        };
        for call in calls {
            if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                self.requests.insert(id.to_string(), event.clone());
            }
        }
    }
}

/// 这条事实是否需要过实时投影（要收进索引，或据它折帧）。
pub(crate) fn watches(event: &Event) -> bool {
    event.kind == MESSAGE_VOCAB.tool_requested || CHAT_VOCAB.denies_speech(event) || projects(event)
}

/// 这条事实（不含伙伴）是否产出实时终态帧。
///
/// 判据与 `rebuild` 的折叠入口同一份：产物格要 `tool_call_id`（否则建不出结果卡）；
/// 首响自成节点；`response` / `failed` 只认**对读者说的那次往返**（`stamp.role = reader`、
/// 无 `via`、有 `root_id`）——内部往返（措辞 / 裁判 / 摘要）与工具结果的协议事实不产节点。
pub(crate) fn projects(event: &Event) -> bool {
    if event.kind == CHAT_VOCAB.opening {
        return true;
    }
    if event.kind == MESSAGE_VOCAB.artifact_added {
        return event
            .payload
            .get("tool_call_id")
            .and_then(|v| v.as_str())
            .is_some();
    }
    if event.kind != symbio_plugin_api::EVENT_RESPONSE
        && event.kind != symbio_plugin_api::EVENT_FAILED
    {
        return false;
    }
    event.payload.get("via").is_none()
        && root_of(event).is_some()
        && event
            .payload
            .get(STAMP_KEY)
            .and_then(|stamp| stamp.get(STAMP_ROLE))
            .and_then(|v| v.as_str())
            == Some(ReplyRole::Reader.as_str())
}

/// 伙伴 + 事实本身 → 节点（序号清空：在途号由转写唯一写入点分配）。
pub(crate) fn frames(event: &Event, partners: Vec<Event>) -> Vec<cm::ChatNode> {
    let mut slice = partners;
    slice.push(event.clone());
    rebuild(&slice, &CHAT_VOCAB, &MESSAGE_VOCAB)
        .into_iter()
        .map(|mut node| {
            node.seq = None;
            node
        })
        .collect()
}

fn root_of(event: &Event) -> Option<String> {
    event
        .payload
        .get("root_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "live.test.rs"]
mod tests;
