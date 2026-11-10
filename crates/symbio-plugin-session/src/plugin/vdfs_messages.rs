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
use symbio_kernel::chat::chat_node as cm;
use symbio_kernel::event::{Entity, Verb};
use symbio_kernel::vdfs::{
    VdfsActionResult, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsRequest, VdfsResponse,
    VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPAND_SKELETON, VDFS_ACTION_RETRY_COMPACTION,
    VDFS_ACTION_RETRY_SUBTREE, VDFS_ACTION_SWITCH_BRANCH, VDFS_ACTION_TRUNCATE,
};
use symbio_plugin_api::events::{EVENT_MESSAGE_REMOVED, EVENT_USER_ANSWER};
use symbio_plugin_api::FactDraft;
use symbio_sdk::keys::SESSION_ID;
use symbio_sdk::plugin::fact_source::FACT_SOURCE;
use symbio_sdk::vdfs::host::{vdfs_from_plugin_error, vdfs_host_ctx};

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
                // 有它自己的动作——[`symbio_kernel::vdfs::VDFS_ACTION_TRUNCATE`]（落在起始消息上）。
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
                    // 不开轮、不产生消息（[D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
                    //
                    // 必须排在下面的恢复 catch-all **之前**：`switch_branch` 不在
                    // `ResumeAction` 的词表里，掉进那一臂会被读成 `NotImplemented`——
                    // 而「这个动词不存在」是假话，它存在，只是不开轮。
                    (Some(mid), VDFS_ACTION_SWITCH_BRANCH, payload) => {
                        let to = switch_branch_target(payload)?;
                        let host = vdfs_host_ctx(ctx)?;
                        let actor = symbio_sdk::authz::request_principal(&*host);
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
                    // ── 重试另一支（`retry_subtree`，无载荷）──
                    //
                    // 与 `switch_branch` 是一对，方向相反：那条只改读法、不开轮；这条
                    // 只开轮、不改既有节点。**动作 → 事实 → 环派发**：写侧落一条
                    // `turn.branch{of, into}`（D-173 的目标事实），随后**复用写收件箱
                    // 的那一个开轮单元**（`user.message` → `session.config`，由事实源
                    // 的写口踢驱动）——不另造第二种「开轮」接法，也不在这里重跑模型。
                    //
                    // 新支的请求因此天然不带旧支的产出：`of` 是旧支的轮锚，投影按
                    // `turn.branch` 折出「当前读哪一支」，旧支子树在请求装配时被裁掉。
                    (Some(mid), VDFS_ACTION_RETRY_SUBTREE, _payload) => {
                        let host = vdfs_host_ctx(ctx)?;
                        let actor = symbio_sdk::authz::request_principal(&*host);
                        let message = self
                            .retry_branch(id, mid, &actor)
                            .await
                            .map_err(vdfs_from_plugin_error)?;
                        // 新支的轮根 = 新一条用户消息的节点 id（`retry_branch` 已铸好，
                        // 并把它换到返回节点上）。入队用这个 id：地址末段即身份。
                        let new_id = message.id.clone();
                        let workdir = host.get(symbio_sdk::keys::WORKDIR);
                        let source_principal = host
                            .get(symbio_sdk::keys::SOURCE_PRINCIPAL)
                            .filter(|s| !s.trim().is_empty());
                        self.enqueue_inbox(
                            id,
                            Some(new_id.clone()),
                            message,
                            session_chat::Request {
                                source_principal,
                                ..Default::default()
                            },
                            workdir,
                        )
                        .await;
                        // 开轮：形状与写收件箱逐字相同（同上一条注释）。
                        let source = self
                            .fact_source(id)
                            .await
                            .map_err(|e| VdfsError::internal(format!("开轮取事实源失败: {e}")))?;
                        let inbox_ctx = vdfs_host_ctx(ctx)?.fork();
                        inbox_ctx.set(SESSION_ID, id.to_string());
                        inbox_ctx.set(FACT_SOURCE, Arc::clone(&source));
                        let seq = self
                            .consume_inbox_turn(&inbox_ctx, id, &*source)
                            .await
                            .map_err(|e| VdfsError::internal(format!("开轮失败: {e}")))?;
                        if let Some(seq) = seq {
                            let next_ctx = self.get_parent().map(|_| inbox_ctx.clone());
                            self.spawn_turn_closer(
                                id.to_string(),
                                seq,
                                source,
                                self.drive_entry(),
                                next_ctx,
                            );
                        }
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: VDFS_ACTION_RETRY_SUBTREE.to_string(),
                            ok: true,
                            message: format!("已重试 {mid}：新的一支 {new_id} 已开轮"),
                            data: Some(json!({
                                "session_id": id,
                                "of": mid,
                                "into": new_id,
                            })),
                        }))
                    }
                    // ── 压缩失败重试（D-402 ③）──
                    //
                    // 动作落在**失败的压缩节点**（`mid`）上：只落一条 `message.removed`
                    // （目标就是该节点），不在这里重跑（恢复只能从事实进环）。读面因此
                    // 丢弃失败叶；压缩债源看到「该节点的最新一次移除」就换一把带纪元的
                    // 前缀指纹，不欠水位也重跑失败时那段切片——驱动由事实源写口踢出。
                    (Some(mid), VDFS_ACTION_RETRY_COMPACTION, _payload) => {
                        let msgs = self.transcript_of(id).await?;
                        let node =
                            msgs.iter()
                                .find(|message| message.id == mid)
                                .ok_or_else(|| {
                                    VdfsError::not_found(format!("压缩节点不存在：{mid}"))
                                })?;
                        if !(node.is_compression() && node.status == Some(cm::NodeStatus::Failed)) {
                            return Err(VdfsError::invalid(format!(
                                "retry_compaction 只能落在失败的压缩节点上：{mid}"
                            )));
                        }
                        let source = self
                            .fact_source(id)
                            .await
                            .map_err(|e| VdfsError::internal(format!("打开事实源失败：{e}")))?;
                        let anchor = source
                            .range(0)
                            .last()
                            .and_then(|e| e.seq)
                            .map(|s| s.value())
                            .ok_or_else(|| {
                                VdfsError::internal(
                                    "重试未能入格：事实源为空，没有可用的溯源锚".to_string(),
                                )
                            })?;
                        let host = vdfs_host_ctx(ctx)?;
                        let actor = symbio_sdk::authz::request_principal(&*host);
                        source
                            .append(
                                actor,
                                FactDraft {
                                    key: format!("compaction-retry:{mid}"),
                                    kind: EVENT_MESSAGE_REMOVED.to_string(),
                                    entity: Entity::Message,
                                    verb: Verb::Closed,
                                    payload: json!({ "ids": [mid] }),
                                    produced_by: Some(anchor),
                                },
                            )
                            .map_err(|e| VdfsError::internal(format!("压缩重试落格失败：{e}")))?;
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: VDFS_ACTION_RETRY_COMPACTION.to_string(),
                            ok: true,
                            message: format!("已重试压缩 {mid}"),
                            data: Some(json!({
                                "session_id": id,
                                "target": mid,
                            })),
                        }))
                    }
                    // ── 用户答复 / 批准 / 拒绝（载荷 `{answer, mode}`）──
                    //
                    // **只落一条事实，不在这里续跑。** 动作闭集是
                    // `{answer, approve, reject}`，落成 `user.answer`（事实里的 `kind`
                    // 取 `answer | approve | deny`）：宿主看到事实源 head 前进后由写口
                    // 踢一次驱动（契约 `symbio_sdk::plugin::drive_entry`），工具侧那一行
                    // 按 `kind` 分岔——`answer` 把答复原文落成结果，`approve` 合并
                    // `approved:true` 走正常执行，`deny` 不执行、落 `user_rejected`。
                    // 挂起的调用**不在这里执行**（D-121：审批 / 答复恢复只能从事实进环）。
                    (Some(mid), action @ ("answer" | "approve" | "reject"), payload) => {
                        let kind = match action {
                            "approve" => "approve",
                            "reject" => "deny",
                            _ => "answer",
                        };
                        let answer = payload
                            .and_then(|p| p.get("answer"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        let mode = payload
                            .and_then(|p| p.get("mode"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        // 动作落在**卡片节点**（`mid`）上，但答复事实要认的是
                        // **调用的 wire id**：工具侧的产物 / 续接行都按
                        // `tool_call_id` 配对（t27 的现场：两套身份混用 ⇒ 永远找不到
                        // 产物）。解不出 wire id 就拒绝，不静默拿节点 id 顶替。
                        let msgs = self.transcript_of(id).await?;
                        let wire_id = wire_call_id_of(&msgs, mid)?;
                        // 走**本部署根**的写口（不是通用开面）：驱动与转写出口都绑在
                        // 这个句柄上，答复落格后由它踢出续跑的一环（`user.answer`
                        // 只落事实，续跑由踢驱动发起）。
                        let source = self
                            .fact_source(id)
                            .await
                            .map_err(|e| VdfsError::internal(format!("打开事实源失败：{e}")))?;
                        // 溯源锚（I2：断言类事实必须带 `produced_by`，且不许悬空）：
                        // 答复不是对某条事实的**修改**，而是「站在当前这份状态上」的
                        // 一次外部断言；取落格前最后一条事实的 seq——与
                        // `message_events::provenance` 找不到承载事件时的退化口径一致。
                        let anchor = source
                            .range(0)
                            .last()
                            .and_then(|e| e.seq)
                            .map(|s| s.value())
                            .ok_or_else(|| {
                                VdfsError::internal(
                                    "答复未能入格：事实源为空，没有可用的溯源锚".to_string(),
                                )
                            })?;
                        let draft = FactDraft {
                            // 事实身份 = 它回答的那次调用（wire id 全局唯一）：
                            // 工具侧的续接行按原债键（`tool:<request_id>`）开账，只把这
                            // 条事实的身份放进载荷，答复原文 / `kind` 由执行体自己读。
                            key: format!("user-answer:{wire_id}"),
                            kind: EVENT_USER_ANSWER.to_string(),
                            entity: Entity::Scope,
                            verb: Verb::Asserted,
                            payload: json!({
                                "kind": kind,
                                "call_id": wire_id,
                                "answer": answer,
                                "mode": mode,
                                "by": symbio_sdk::authz::PRINCIPAL_USER,
                            }),
                            produced_by: Some(anchor),
                        };
                        source
                            .append(symbio_sdk::authz::PRINCIPAL_USER.to_string(), draft)
                            .map_err(|e| {
                                VdfsError::internal(format!("user.answer 落格失败：{e}"))
                            })?;
                        let label = match kind {
                            "approve" => "批准",
                            "deny" => "拒绝",
                            _ => "答复",
                        };
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: action.to_string(),
                            ok: true,
                            message: format!("已记录对 {mid} 的{label}；由宿主再驱动一环"),
                            data: Some(json!({ "session_id": id, "call_id": wire_id })),
                        }))
                    }
                    _ => Err(VdfsError::NotImplemented),
                }
            }
            req @ (VdfsRequest::Watch { .. } | VdfsRequest::Unwatch) => {
                self.dispatch_watch(path, req).await
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

/// 动作目标（卡片节点 `mid`）→ 这次调用的 **wire id**（`ChatNode.tool_call_id`）。
///
/// 两套身份各有各的用处，不能混：
/// - `ChatNode.id` 是**节点地址**——动作、变更帧、前端选中都落在它上面；
/// - `tool_call_id` 是 **provider 的 wire id**——`tool.requested.calls[].id`、
///   `artifact.added.tool_call_id`、`authz.denied.id` 全按它配对。
///
/// 规则：自身是 `tool_call` ⇒ 直接取它的 `tool_call_id`；交互 / `user_prompt`
/// 之类子节点沿 `parent_id` 向上找最近的 `tool_call`。找不到（或历史里那段没留下
/// wire id）就**拒绝**——静默用节点 id 顶替，答复事实会落在一个工具侧永远配不上的
/// 键上（T27 的现场）。
fn wire_call_id_of(msgs: &[cm::ChatNode], mid: &str) -> VdfsResult<String> {
    let mut cursor: Option<&str> = Some(mid);
    // 父链最长不超过消息条数：防御坏数据里的环。
    for _ in 0..=msgs.len() {
        let Some(node) = cursor.and_then(|id| msgs.iter().find(|m| m.id == id)) else {
            break;
        };
        if node.is_tool_call() {
            return match node.tool_call_id.as_deref().map(str::trim) {
                Some(wire) if !wire.is_empty() => Ok(wire.to_string()),
                // 老数据 / 投影丢了 wire id：这里**不猜**（猜 = 落一条配不上的事实）。
                _ => Err(VdfsError::invalid(format!(
                    "动作目标 {mid} 是工具调用卡，但历史里没有它的 wire id（tool_call_id 缺失）"
                ))),
            };
        }
        cursor = node.parent_id.as_deref();
    }
    Err(VdfsError::invalid(format!(
        "动作目标 {mid} 解不出调用的 wire id：自身不是工具调用，父链上也没有最近的 tool_call"
    )))
}
