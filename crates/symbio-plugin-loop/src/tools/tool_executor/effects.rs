//! 效果输入的**记账口**：一次工具调用的终态（成功 / 失败 / 没跑到）在这里变成一条
//! [`EffectInput`]，交回轮末写口求值（DE-025 D-207）。
//!
//! ## 本模块不认识任何工具，也不认识任何效果
//!
//! 它只做三件事：问声明（`ToolDecls::behaviors_of`）、按调用 id 去重、把纯值收进
//! 出参。「这次调用改变了什么」由能力包的行为实现回答（`ToolBehavior::after`），
//! 「这些格怎么落」由宿主在唯一写入口回答。三者各自只有一份。
//!
//! ## 为什么在**观察点**就收，而不是等写口回看节点
//!
//! 工具结果节点只留正文与成败，`args` 与调用 id 在节点上有，但「哪一次调用属于
//! 本批」只有执行层知道。写口拿到的是本批的调用清单——那正是本函数产出的东西。
//!
//! ## 先到先得
//!
//! 同一次调用可能在两个分支各记一笔（终态广播找不到节点 ⇒ `parent_updates` 里没有
//! 它 ⇒ 末尾收口会再记一次「未执行」）。先记下的那次是**更接近事实**的判决（它就
//! 发生在该分支里），后来的收口只该覆盖「一次都没跑到」的调用，不能把跑成功的
//! 改写成没跑。

use serde_json::Value;
use symbio_plugin_api::effects::EffectInput;

/// 把一次调用的终态收进效果出参。
///
/// - `why`：没成功的原因（成功时忽略）。**由调用点给**——「为什么没跑到」只有
///   它知道（协议失败 / 超限丢弃 / 闸门拒止 / 批中止各有各的话）。
/// - 工具没声明任何行为 ⇒ 直接返回：没有效果可记，出参里不留空壳。
#[allow(clippy::too_many_arguments)]
pub fn record_effect(
    out: &mut Vec<EffectInput>,
    decls: &super::ToolDecls,
    id: &str,
    name: &str,
    args: &Value,
    success: bool,
    result: &str,
    why: &str,
) {
    let declared = decls.behaviors_of(name);
    if declared.is_empty() {
        return;
    }
    if out.iter().any(|e| e.call_id == id) {
        return;
    }
    out.push(EffectInput {
        tool: name.to_string(),
        call_id: id.to_string(),
        args: args.clone(),
        success,
        result: result.to_string(),
        why: if success {
            String::new()
        } else {
            why.to_string()
        },
        declared,
    });
}

#[cfg(test)]
#[path = "effects.test.rs"]
mod tests;
