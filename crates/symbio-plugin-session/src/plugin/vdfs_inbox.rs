//! 会话地址面的**收件箱域**：`<根>/session/<id>/inbox`（运行中会话的在途事件）。
//!
//! 从 `vdfs_provider` 分出来的判据是「这一段错了谁会红」：本文件出错，红的是
//! **正在跑的这一轮看得见什么**——补充没进箱、中止没被接住、Expand 骨架读空。
//! 它与转写域的区别是生命周期：收件箱是**运行态小状态机**的读数（会话一停就清空），
//! 转写是事实源的渲染；把两者混在一处，最先被碰坏的是「停了还在不在」这条判据。

use super::*;
use symbio_kernel::vdfs::{
    VdfsActionResult, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsRequest, VdfsResponse,
    VdfsResult, VdfsWriteResponse, VDFS_ACTION_ABORT, VDFS_ACTION_CLEAR,
};
use symbio_sdk::keys::SESSION_ID;
use symbio_sdk::plugin::fact_source::FACT_SOURCE;
use symbio_sdk::vdfs::host::vdfs_host_ctx;

impl SessionPlugin {
    /// 收件箱（`<id>/inbox[/iid]`）：**待消费**的用户消息。
    pub(super) async fn inbox_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        id: &str,
        iid: Option<&str>,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => match iid {
                // 收件箱：**待消费**的用户消息（已消费的那些在转写里，不在这）
                None => {
                    self.session_of(id).await?;
                    Ok(VdfsResponse::list(
                        self.inbox_items(id)
                            .await
                            .iter()
                            .map(inbox_item_node)
                            .collect::<Vec<VdfsNode>>(),
                    ))
                }
                Some(_) => Err(VdfsError::not_found(format!(
                    "收件箱条目是列表项，没有子项：{path}"
                ))),
            },
            VdfsRequest::Stat => match iid {
                // 收件箱目录：形状与 `list` 同源；条目：队列里那一条（出队即消失）
                None => {
                    self.session_of(id).await?;
                    Ok(VdfsResponse::Stat(inbox_dir_node()))
                }
                Some(iid) => {
                    self.session_of(id).await?;
                    self.inbox_items(id)
                        .await
                        .iter()
                        .find(|i| i.id == iid)
                        .map(inbox_item_node)
                        .map(VdfsResponse::Stat)
                        .ok_or_else(|| {
                            VdfsError::not_found(format!(
                                "收件箱里没有待消费条目：{iid}（已出队的那条在转写里）"
                            ))
                        })
                }
            },
            VdfsRequest::Read => match iid {
                // 收件箱条目：正文就是那条待发消息（与消息节点同一投影口径，
                // 它就是一条消息——差的只是"还没被消费"）
                Some(iid) => {
                    self.session_of(id).await?;
                    let message = self
                        .inbox_items(id)
                        .await
                        .iter()
                        .find(|i| i.id == iid)
                        .map(|i| i.message.clone())
                        .ok_or_else(|| {
                            VdfsError::not_found(format!("收件箱里没有待消费条目：{iid}"))
                        })?;
                    Ok(VdfsResponse::Read(VdfsContent::text(message_text(
                        &message,
                    ))))
                }
                None => Err(VdfsError::invalid(format!("该路径不可读取内容：{path}"))),
            },
            VdfsRequest::Write { content } => {
                // 收件箱：**写即入队**（`<id>/inbox` 与 `<id>/inbox/<iid>` 同义）。
                //
                // ## 为什么这里不存在"发言不是一次写入"那个叉
                //
                // 转写列表（`<id>/message`）拒绝写入，因为"往列表里放一条"= 发言 =
                // 一次**动作**；而收件箱要表达的不是"跑一轮"，而是"把这条消息交给
                // 这个空间"——它本来就是一次写入，跑不跑、何时跑由空间自己决定
                // （见 `inbox` 模块）。两个地址因此一个只读、一个可写，这不是不对称，
                // 而是它们本来在说两件事。
                //
                // `create` 在这里**没有区分度**：两种形态都是新建一条队列项，因此既不
                // 要求也不拒绝——回执统一 `created: true`（它确实是一条新条目）。
                if content.binary {
                    return Err(VdfsError::invalid(
                        "收件箱条目是文本（ChatNode 字段子集或纯文本），不接受二进制内容",
                    ));
                }
                // 存在性校验：会话不在，就没有"它的收件箱"可写。
                // 返回的那份 `Session` 在**触发声明**那条路上要用（自主轮的运行参数
                // 按会话自己的 metadata 推导，见下）——普通写入路径仍然只把它当校验。
                let session = self.session_of(id).await?;
                let raw = content.text.as_deref().unwrap_or("");
                let message = parse_inbox_message(raw)?;
                // 写体声明了 `meta.trigger` ⇒ 这条写入在说「这是一次自主触发」，
                // 于是它**先落事实才准入队**：存储所有者的写窗口纪律住在
                // `transcript/autonomous.rs`（[D-076](../../../symbio-plugin-heartbeat/README.md#d-076-心跳片的八个宿主触点逐条落位调度随片走触发事实落进收件箱写臂 "crates/symbio-plugin-heartbeat/src/README")），
                // 识别与判据住在这条臂上。落格没成 ⇒ `Conflict` 且**队列没碰**——
                // 收件箱是 FIFO，一条永远跑不起来的队首会把后面的用户消息一起堵死。
                let idle_ms = trigger_declaration(&message)?;
                let item = match idle_ms {
                    Some(idle_ms) => self
                        .request_autonomous_round(
                            id,
                            iid.map(str::to_string),
                            &session.metadata,
                            message,
                            idle_ms,
                        )
                        .await
                        .ok_or_else(|| {
                            VdfsError::Conflict(format!(
                                "自主触发未能落进事实源，本轮不开（重试归调度侧）：{path}"
                            ))
                        })?,
                    None => {
                        // 工作目录取自**请求上下文**（与 `chat/send` 入队时同一来源）：
                        // 它是 `start_turn` 回退链的**第一档**（ctx > 会话 metadata > 报错），
                        // 丢了它就只能靠会话 metadata——而「会话还没绑过 workdir」正是新建
                        // 会话那一刻的常态。
                        let workdir = vdfs_host_ctx(ctx)
                            .ok()
                            .and_then(|h| h.get(symbio_sdk::keys::WORKDIR));
                        // 来源主体（D-067）：同取**请求上下文**（线上入口 =
                        // `metadata.source_principal`）。随条目寄存进 `params`，消费时经
                        // `run_inbox_turn` 挂回执行 ctx——「这条消息是谁发的」是**数据**，
                        // 轮末 `commitment.*` 的 `to` 据它取值，缺省保持本机用户。
                        let source_principal = vdfs_host_ctx(ctx)
                            .ok()
                            .and_then(|h| h.get(symbio_sdk::keys::SOURCE_PRINCIPAL))
                            .filter(|s| !s.trim().is_empty());
                        self.enqueue_inbox(
                            id,
                            iid.map(str::to_string),
                            message,
                            session_chat::Request {
                                source_principal,
                                ..Default::default()
                            },
                            workdir,
                        )
                        .await
                    }
                };
                // **入队之后由本臂开轮**（同一插件内，不跨插件依赖）。
                //
                // 没有这一下，「入队」与「开轮」之间就没有人接：`agent_run` 起的子会话
                // 只写这份 inbox、从不调 `session/consume`，队列项会永远停在队首。
                // 写入返回前必须把开轮做完——调用方拿到的回执对应的是「这一轮已经开」，
                // 不是「这条消息被人签收了」。
                //
                // **两条分支不分家**：自主触发的分支先落触发事实（`request_autonomous_round`
                // 只入队、不落 `user.message`），开轮同样归本臂——心跳侧没有任何消费者，
                // 少这一下则那一轮永远不开（事实链里只有触发、没有 `user.message`）。
                // 开轮单元的固定形状：`user.message` → `session.config`（溯源指前者）。
                {
                    // 事实源按**收件箱的会话 id** 取，不按调用方上下文取：
                    // `agent_run` 写的是子会话的 inbox，而调用方上下文里的
                    // `FACT_SOURCE` 是**外层**的。用上下文那一份会把子会话的
                    // `user.message` / `session.config` 落进外层 WAL——子会话
                    // 永远没有事实、也就永远不会被写口踢起驱动。
                    let source = self
                        .fact_source(id)
                        .await
                        .map_err(|e| VdfsError::Internal(format!("开轮取事实源失败: {e}")))?;
                    // 能力收集与开轮都按**这条消息的会话**算：fork 出独立上下文，
                    // 把会话键与事实源重挂到子会话（子插件的能力可见域据此计算）。
                    let inbox_ctx = vdfs_host_ctx(ctx)?.fork();
                    inbox_ctx.set(SESSION_ID, id.to_string());
                    inbox_ctx.set(FACT_SOURCE, Arc::clone(&source));
                    let seq = self
                        .consume_inbox_turn(&inbox_ctx, id, &*source)
                        .await
                        .map_err(|e| VdfsError::Internal(format!("开轮失败: {e}")))?;
                    // 收尾：等这轮驱动空闲后投影终答（见 `spawn_turn_closer`）。
                    // 本臂的调用方可能是 CLI（自己会 `session/project`，幂等），也可能
                    // 是 `agent_run`（子会话没有投影方——缺席就永远停在 `working`）。
                    if let Some(seq) = seq {
                        // 收尾任务用本轮开出的能力视图上下文（`consume_inbox_turn` 已把
                        // 能力视图挂到 `inbox_ctx` 上）去开下一轮；父插件未挂载时按既有
                        // 路径降级（只投影、不抽干）。
                        let next_ctx = self.get_parent().map(|_| inbox_ctx.clone());
                        self.spawn_turn_closer(
                            id.to_string(),
                            seq,
                            source,
                            self.drive_entry(),
                            next_ctx,
                        );
                    }
                }
                // 回执的 `name` 是**条目自己的名字**（不是请求地址的末段）：写收件箱
                // 目录自身时 id 由 provider 生成，调用方只能从回执得知它落成了什么
                // （与新建会话回执返回生成的 id 同一手法）；写具名条目
                // （`<inbox>/<iid>`）时那个名字本来就是调用方给的，回传没有信息量。
                let anonymous = iid.is_none();
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: anonymous.then(|| item.id.clone()),
                    created: true,
                }))
            }
            VdfsRequest::Delete { .. } => match iid {
                // 收件箱条目：**取消排队**（尚未被消费的那条）。
                //
                // 找不到就是"已出队"（它已经变成一条真消息了）或本来就没有——两种
                // 都报 `NotFound`：`delete` 的语义是"这条队列项没了"，不能报成成功。
                Some(iid) => {
                    self.session_of(id).await?;
                    if self.cancel_inbox_item(id, iid).await {
                        Ok(VdfsResponse::Unit)
                    } else {
                        Err(VdfsError::not_found(format!(
                            "收件箱里没有待消费条目 {iid}\
                             （已出队的那条已是会话里的消息，中止请走 \
                             action(会话地址, \"{abort}\")）：{path}",
                            abort = VDFS_ACTION_ABORT
                        )))
                    }
                }
                None => Err(VdfsError::Forbidden(format!(
                    "收件箱不可整体删除：清空待消费队列请用 action(\"{clear}\")：{path}",
                    clear = VDFS_ACTION_CLEAR,
                ))),
            },
            // 收件箱的 `clear`：**只取消还没被消费的**。已出队的那条已经是会话里的
            // 消息，中止它走 `action(<会话地址>, "abort")`（与删除条目同款分界）。
            VdfsRequest::Action { action, .. } if iid.is_none() && action == VDFS_ACTION_CLEAR => {
                self.session_of(id).await?;
                let ids = self.clear_inbox(id).await;
                Ok(VdfsResponse::Action(VdfsActionResult {
                    action: action.clone(),
                    ok: true,
                    message: format!("已取消 {} 条待消费消息", ids.len()),
                    data: Some(json!(ids)),
                }))
            }
            req @ (VdfsRequest::Watch { .. } | VdfsRequest::Unwatch) => {
                self.dispatch_watch(path, req).await
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}
