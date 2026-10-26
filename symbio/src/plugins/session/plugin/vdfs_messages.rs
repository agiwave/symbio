//! 会话地址面的**转写域**：`<根>/session/<id>/message[/mid]` 的读、改、截断、清空。
//!
//! 从 `vdfs_provider` 分出来的判据是「这一段错了谁会红」：本文件出错，红的是
//! **消息面**——转写列不出来 / 缺了在途的那条 / 编辑或截断之后变更帧没投出去 /
//! 清空把不该动的动了。它同时是写侧最多的一域，因此和只读的目录域
//! （`vdfs_catalog`）分开：一处红的是「看不见」，一处红的是「改坏了」。
//!
//! 写语义本身不在这里：本文件只把 `Write` / `Action` 翻译成对
//! `commands` 的调用，再把变更投进订阅表。

use super::*;
use crate::symbio_core::chat_node as cm;
use crate::symbio_core::{
    vdfs_from_plugin_error, vdfs_host_ctx, VdfsActionResult, VdfsContent, VdfsContext, VdfsError,
    VdfsNode, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
    VDFS_ACTION_EXPAND_SKELETON, VDFS_ACTION_SWITCH_BRANCH, VDFS_ACTION_TRUNCATE,
};

impl SessionPlugin {
    /// 转写区段（`<id>/message[/mid]`）：列表与单条消息。
    pub(super) async fn messages_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        id: &str,
        mid: Option<&str>,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => match mid {
                // 转写列表：每一条消息是一个列表项，顺序由 `seq` 决定。
                // 含**在途**消息——流式期间列表就是活的，不必等落库。
                None => {
                    let msgs = self.transcript_of(id).await?;
                    let (limit, before) = window_params(ctx);
                    // 有界窗口：**只在调用方显式给参数时**生效。不给参数 = 全量，
                    // 与从前逐字节一致（流式期间前端要的是完整列表）。
                    Ok(VdfsResponse::list(
                        transcript_window(&msgs, limit, before)
                            .iter()
                            .map(message_node)
                            .collect::<Vec<VdfsNode>>(),
                    ))
                }
                Some(_) => Err(VdfsError::not_found(format!(
                    "消息是列表项，没有子项：{path}"
                ))),
            },
            VdfsRequest::Stat => match mid {
                // 列表本身是个目录（`l` 位：可列，不参与树遍历——会话整体是叶子）。
                // 只需存在性校验，不必取全量转写。
                None => {
                    self.session_of(id).await?;
                    Ok(VdfsResponse::Stat(messages_dir_node()))
                }
                Some(mid) => Ok(VdfsResponse::Stat(message_node(message_of(
                    &self.transcript_of(id).await?,
                    mid,
                )?))),
            },
            VdfsRequest::Read => match mid {
                // 单条消息的正文（列表项内容）——流式追加的正是它。
                // 同样取含在途的转写：追加型变更的消费者读到的必须是**已含该增量**的正文。
                Some(mid) => {
                    let msgs = self.transcript_of(id).await?;
                    Ok(VdfsResponse::Read(VdfsContent::text(message_text(
                        message_of(&msgs, mid)?,
                    ))))
                }
                None => Err(VdfsError::invalid(format!("该路径不可读取内容：{path}"))),
            },
            VdfsRequest::Write { content } => match mid {
                Some(mid) => {
                    if content.create {
                        return Err(VdfsError::invalid(
                            "消息不支持 create 意图：新增消息即发言，请走聊天协议\
                             （一次发言触发一整轮编排）",
                        ));
                    }
                    if content.binary {
                        return Err(VdfsError::invalid(
                            "消息是文本（JSON 字段补丁），不接受二进制内容",
                        ));
                    }
                    // 补丁缺 `id` 时由**地址**补上：地址是消息身份的唯一权威，
                    // 因此「不给 id」（`{"content":"..."}`）是合法用法，而「给了别的
                    // id」是错误（由 `patch_message` 报出）。
                    //
                    // 必须在这里补而不是靠 `ChatNode` 的 serde 默认值：`id` 在
                    // 该结构里是必填字段（它也是存储层的主键），反序列化会**先一步**
                    // 拒绝掉不带 id 的补丁——那样「补丁是字段子集、未提供的保持不变」
                    // 这条承诺在 `id` 上就是假的，调用方被迫把地址里已有的信息
                    // 再抄一遍。
                    let raw = content.text.as_deref().unwrap_or("").trim();
                    let mut value: Value = serde_json::from_str(raw).map_err(|e| {
                        VdfsError::invalid(format!(
                            "消息补丁需要合法 JSON（ChatNode 字段子集）：{e}"
                        ))
                    })?;
                    let obj = value.as_object_mut().ok_or_else(|| {
                        VdfsError::invalid("消息补丁需要 JSON 对象（ChatNode 字段子集）")
                    })?;
                    if !obj.contains_key("id") {
                        obj.insert("id".to_string(), Value::String(mid.to_string()));
                    }
                    let patch: cm::ChatNode = serde_json::from_value(value).map_err(|e| {
                        VdfsError::invalid(format!(
                            "消息补丁需要合法 JSON（ChatNode 字段子集）：{e}"
                        ))
                    })?;
                    self.patch_message(id, mid, &patch)
                        .await
                        .map_err(vdfs_from_plugin_error)?;
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        name: None,
                        created: false,
                    }))
                }
                // 转写列表本身：**仍只读**。往里放一条 = 发言 = 动作，不是写入。
                None => Err(VdfsError::Forbidden(
                    "转写列表只读：发言请走聊天协议（一次发言触发一整轮编排）".to_string(),
                )),
            },
            VdfsRequest::Delete { .. } => {
                // 转写区段（列表与单条）**不可 delete**：`delete` 的全局语义是
                // 「**这一个**节点没了」，而「从这条删到末尾」是一种**区间**语义，
                // 有它自己的动作——[`crate::symbio_core::VDFS_ACTION_TRUNCATE`]（落在起始消息上）。
                // 动作承载这类集合操作——理由见 `VDFS_ACTION_TRUNCATE` 的文档。
                Err(VdfsError::Forbidden(format!(
                    "转写区段不可 delete：删除某条及其之后请用 action(\"{truncate}\")：{path}",
                    truncate = VDFS_ACTION_TRUNCATE,
                )))
            }
            // 节点动作：转写区段上只有两类动词——**区间删除**
            // （`truncate`，落在单条消息上）与**恢复**（见下）。
            VdfsRequest::Action { action, payload } => {
                match (mid, action.as_str(), payload.as_ref()) {
                    (Some(mid), VDFS_ACTION_TRUNCATE, _) => {
                        let deleted_ids = self
                            .truncate_messages(id, mid)
                            .await
                            .map_err(vdfs_from_plugin_error)?;
                        // 回执带**权威**的被删 id 列表：消费方本地若因锚点缺失而删窄了，
                        // 据它补齐（`vdfs/delete` 只回 `{path}`，带不回这个）。
                        let data = serde_json::to_value(&deleted_ids)
                            .map_err(|e| VdfsError::internal(format!("截断回执序列化失败：{e}")))?;
                        let message = if deleted_ids.is_empty() {
                            // 目标不存在 ⇒ 什么都没删。这是**结果**不是错误，但也不发变更
                            // ——「什么都没删」不该留下痕迹：发一条「从 <mid> 起截断」的
                            // 通知会让消费者从一条并不存在的节点起截断，把整个列表清空。
                            format!("目标消息不存在，未做任何修改：{mid}")
                        } else {
                            format!("已从 {mid} 起截断 {} 条消息", deleted_ids.len())
                        };
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: action.clone(),
                            ok: true,
                            message,
                            data: Some(data),
                        }))
                    }
                    // ── 惰性重取子树（`expand_skeleton`，只读）──
                    //
                    // 落点与 `truncate`/恢复同一个位置：`<sid>/message/<mid>`，`mid` 就是
                    // 被折叠的**骨架自身**。前端本地释放了它的子节点、展开时按需拉回——
                    // 这条读出口只回那棵子树的窄帧，**不改存储**（与恢复不同，它不触发起
                    // 新一轮）。必须排在下面的恢复 catch-all **之前**：那一臂拿 `word` 去
                    // `ResumeAction` 反序列化，`"expand_skeleton"` 不在词表里会掉进
                    // `NotImplemented`。
                    (Some(mid), VDFS_ACTION_EXPAND_SKELETON, payload) => {
                        // 入参校验先行（判据 1）：非法 depth / since_seq 有错误码，
                        // 不静默按缺省处理——静默忽略会让「以为只要直接子」的调用方
                        // 实际拿到整棵树，而他无从知道。
                        let (depth, since_seq) = expand_params(payload)?;
                        let msgs = self.transcript_of(id).await?;
                        // 骨架不存在 ⇒ 空子树（只读动作，「没有」就是空，不是错误）。
                        let subtree = subtree_nodes(&msgs, mid, depth.as_deref(), since_seq);
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: action.clone(),
                            ok: true,
                            message: format!("已重取 {mid} 的子树（{} 个节点）", subtree.len()),
                            data: Some(json!({
                                "session_id": id,
                                "root": mid,
                                "nodes": serde_json::to_value(&subtree)
                                    .map_err(|e| VdfsError::internal(format!(
                                        "子树帧序列化失败：{e}"
                                    )))?,
                            })),
                        }))
                    }
                    // ── 分支切换（`switch_branch`，载荷 `{to}`）──
                    //
                    // 落点与 `truncate`/`expand_skeleton` 同一个位置，语义却相反：那两个
                    // 改的是**节点**，这个只改**读法**——落一条 `turn.branch_selected`，
                    // 不开轮、不产生消息（[D-173](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
                    //
                    // 必须排在下面的恢复 catch-all **之前**：`switch_branch` 不在
                    // `ResumeAction` 的词表里，掉进那一臂会被读成 `NotImplemented`——
                    // 而「这个动词不存在」是假话，它存在，只是不开轮。
                    (Some(mid), VDFS_ACTION_SWITCH_BRANCH, payload) => {
                        let to = switch_branch_target(payload)?;
                        let host = vdfs_host_ctx(ctx)?;
                        let actor = crate::symbio_core::request_principal(&*host);
                        self.switch_branch(id, mid, &to, &actor)
                            .await
                            .map_err(vdfs_from_plugin_error)?;
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: action.clone(),
                            ok: true,
                            message: format!("已把 {mid} 所在的分支组切到 {to}"),
                            data: Some(json!({ "session_id": id, "selected": to })),
                        }))
                    }
                    // ── 恢复（retry_subtree / retry / approve / reject / supply /
                    //    answer / retry_compaction）──
                    //
                    // 落点是**目标消息自身**（`<sid>/message/<mid>`）——与 `truncate`
                    // 同一个位置：「对这条消息做点什么」的地址就是它自己。动作名直接
                    // 取 `ResumeAction` 的线上词形（snake_case，跨栈契约已由
                    // `resume_action_wire_words_are_snake_case` 钉住），**不另造一套**
                    // ——同一批语义多一套名字就多一处漂移。
                    //
                    // 为什么它是动作而不是"写一条消息"：恢复不产生新用户消息，它
                    // **落在当时那条消息上**（删除-重建）。入队会让它排到一堆新消息
                    // 之后，等到被消费时候选早已被后续轮次改写，恢复语义当场失效。
                    (Some(mid), word, payload) => {
                        let Ok(resume_action) = serde_json::from_value::<cm::ResumeAction>(
                            Value::String(word.to_string()),
                        ) else {
                            return Err(VdfsError::NotImplemented);
                        };
                        self.session_of(id).await?;
                        let p = payload.cloned().unwrap_or(Value::Null);
                        let obj = p.as_object();
                        let get_str = |k: &str| {
                            obj.and_then(|o| o.get(k))
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                        };
                        // 上下文**自己造**（与收件箱消费者的 `run_inbox_turn` 同款）：
                        // 只带目标会话 id 与恢复请求。发起者的请求上下文刻意不沿用——
                        // 它的 `SESSION_ID` 是发起者自己的会话（跨空间时二者不同）。
                        let host = vdfs_host_ctx(ctx)?;
                        let req_ctx = host.fork();
                        req_ctx.set(SESSION_ID, id.to_string());
                        let _ = req_ctx.set_payload(session_chat::Request {
                            session_id: Some(id.to_string()),
                            resume: Some(cm::ResumeRequest {
                                target_id: mid.to_string(),
                                action: resume_action,
                                args: obj.and_then(|o| o.get("args")).cloned(),
                                reason: get_str("reason"),
                                answer: obj.and_then(|o| o.get("answer")).cloned(),
                            }),
                            mode: get_str("mode"),
                            risk_level: get_str("risk_level"),
                            ..session_chat::Request::default()
                        });
                        let resp = self
                            .turn
                            .start_turn(req_ctx)
                            .await
                            .map_err(vdfs_from_plugin_error)?;
                        let status = resp
                            .get::<Value>()
                            .ok()
                            .and_then(|v| v.get("status").and_then(Value::as_str).map(String::from))
                            .unwrap_or_default();
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: action.clone(),
                            // 忙碌时**明确回绝**：http 层是 200，只有 `ok` 能把这个
                            // 事实带给调用方（前端据此把乐观置的 working 复位）。
                            ok: status != "session_busy",
                            message: if status == "session_busy" {
                                format!("会话正在处理中，无法{action}：{path}")
                            } else {
                                format!("已在 {mid} 上执行 {action}")
                            },
                            data: Some(json!({ "session_id": id, "status": status })),
                        }))
                    }
                    _ => Err(VdfsError::NotImplemented),
                }
            }
            VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// `switch_branch` 的入参校验：载荷必须是 `{"to": "<同组候选 id>"}`。
///
/// 与 [`expand_params`] 同一条判据（非法输入有错误码，不静默按缺省处理）：这里
/// **没有缺省**——「切到哪一支」是这个动作唯一要说的一句话，缺了它就只能猜，
/// 而猜错的后果是用户的转写换了一条历史，他还看不见区别。
fn switch_branch_target(payload: Option<&Value>) -> VdfsResult<String> {
    let obj = match payload {
        Some(Value::Object(obj)) => obj,
        Some(other) => {
            return Err(VdfsError::invalid(format!(
                "switch_branch 的载荷需要 JSON 对象（{{\"to\": 候选 id}}），实得：{other}"
            )));
        }
        None => {
            return Err(VdfsError::invalid(
                "switch_branch 缺少 `to`：切分支必须指明目标候选",
            ));
        }
    };
    match obj
        .get("to")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(to) => Ok(to.to_string()),
        None => Err(VdfsError::invalid(format!(
            "switch_branch 的 `to` 需要是非空字符串（候选节点 id），实得：{}",
            obj.get("to").cloned().unwrap_or(Value::Null)
        ))),
    }
}

/// `expand_skeleton` 的入参校验（判据 1：入参与 DE-019 §6 一致，非法值有错误码）。
///
/// `depth` 只接受 `"1"` / `"all"`（缺省 = `all`）；`since_seq` 只接受整数
/// （seq 水位）。**给了但不合法 ⇒ 报错**：静默按缺省处理会让「以为只要直接子」
/// 的调用方实际拿到整棵树，而他从回执里看不出来——错误必须像 `truncate`
/// 的锚点缺失一样当场给。
///
/// 显式 `null` 与缺省同义（「不过滤」的线上写法），不算非法。
fn expand_params(payload: Option<&Value>) -> VdfsResult<(Option<String>, Option<i64>)> {
    let (depth, since_seq) = match payload {
        None | Some(Value::Null) => (None, None),
        Some(Value::Object(obj)) => {
            let depth = match obj.get("depth") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s == "1" || s == "all" => Some(s.clone()),
                Some(other) => {
                    return Err(VdfsError::invalid(format!(
                        "expand_skeleton 的 depth 只接受 \"1\" | \"all\"（缺省 all），实得：{other}"
                    )));
                }
            };
            let since_seq = match obj.get("since_seq") {
                None | Some(Value::Null) => None,
                Some(v) => match v.as_i64() {
                    Some(n) => Some(n),
                    None => {
                        return Err(VdfsError::invalid(format!(
                            "expand_skeleton 的 since_seq 只接受整数（seq 水位），实得：{v}"
                        )));
                    }
                },
            };
            (depth, since_seq)
        }
        Some(other) => {
            return Err(VdfsError::invalid(format!(
                "expand_skeleton 的载荷需要 JSON 对象（{{depth?, since_seq?}}），实得：{other}"
            )));
        }
    };
    Ok((depth, since_seq))
}
