//! 代际立约出参（`agent_run` 的承诺记录）：工具执行层把「向上游承诺了什么、
//! 履行了没有」交出来，由轮末收束的 `derived_facts::record_derived` 落成 `commitment.*` 格。

/// 本轮的**代际立约记录**（[04 §3.1 批⑧](../../../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")，S08 §3）：
/// 工具执行层把「我向上游承诺了什么、履行了没有」交出来，由轮末收束的
/// `derived_facts::record_derived` 落成 `commitment.*` 格。
///
/// 只认**声明了 `records_delegation_promise` 的工具**（`agent_run` 即是；T-158 改了
/// 判据的住址，没改这条语义）——它是**跨主体**的那一次调用：本会话向上游承诺
/// 「会有另一个智能体接手这件事」。别的工具是本会话自办，办没办成是执行结果，
/// 不是承诺。
///
/// 工具执行层**没有事实源**（它只写节点、不碰 WAL），所以这里是纯出参：
/// 谁负责把承诺入格，谁负责开那扇门（轮末收束的 `derived_facts::record_derived`）。
#[derive(Debug, Clone)]
pub struct Delegation {
    /// 承诺号 = 本次调用在**转写里的 ToolCall 节点 id**（`c-offer-<id>` 由此成格）：
    /// 节点 id 由转写生成、跨轮不撞——模型给的 `tool_call` 编号只在一次响应内唯一，
    /// 直接拿来当事件号会在下一轮撞幂等键（`record_derived` 落格时还会给事件号
    /// 加 `v2t-{anchor_id}` 前缀、一轮一个，两层各自挡一种撞法）。
    pub id: String,
    /// 承诺内容 = 委托出去的那句话（`agent_run` 的 `prompt` 参数）。
    pub promise: String,
    /// 是否履行（`true` ⇒ `commitment.released`，`false` ⇒ `.broken` + `why`）。
    pub ok: bool,
    /// 违约原因（`ok = true` 时为空——违约必须带 why，守约不必）。
    pub why: String,
}

/// 承诺出参的**唯一写入点**（见 [`Delegation`]）：只有声明了
/// `records_delegation_promise` 的工具会被记账（T-158），且在每个终态分支各记
/// 一次——没跑到的（协议错 / 被拦 / 批次跳过）同样是**没履约**，记成违约比
/// "查无此约"诚实（S08 §5：违约可被观测是 T5 的全部前提）。
pub fn note_delegation(
    out: &mut Vec<Delegation>,
    decls: &super::ToolDecls,
    id: &str,
    name: &str,
    args: &serde_json::Value,
    ok: bool,
    why: &str,
) {
    if !decls.records_delegation_promise(name) {
        return;
    }
    let record = Delegation {
        id: id.to_string(),
        promise: args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        ok,
        why: if ok { String::new() } else { why.to_string() },
    };
    // **先到先得**：同一次调用可能在两个分支各记一笔（终态广播找不到节点 ⇒
    // `parent_updates` 里没有它 ⇒ 末尾收口会再记一次「未执行」）。先记下的那次
    // 是**更接近事实**的判决（它就发生在该分支里），后来的收口只该覆盖「一次都
    // 没跑到」的调用，不能把跑成功的改写成没跑。
    if !out.iter().any(|d| d.id == record.id) {
        out.push(record);
    }
}
