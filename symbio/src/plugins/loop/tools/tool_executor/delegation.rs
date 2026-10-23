//! 代际立约出参的**记账口**（`agent_run` 那一次调用）：工具执行层把「向上游承诺了什么、
//! 履行了没有」交出来，由轮末收束的 `derived_facts::record_derived` 落成 `commitment.*` 格。
//!
//! 记录类型 [`Delegation`] 本身不住这里——它是隔着驱动器与留守存储两侧都要用的
//! 词汇，住在 [`crate::symbio_core::facts`]（[D-082](../../../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 4 组）。
//! 本文件只留**判据与门**：哪一次调用算承诺、算不算履约——那是机制，随观察点住插件侧。

use crate::symbio_core::facts::Delegation;

/// 承诺出参的**唯一写入点**（见 [`Delegation`]）：只有声明了
/// `records_delegation_promise` 的工具会被记账，且在每个终态分支各记
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
