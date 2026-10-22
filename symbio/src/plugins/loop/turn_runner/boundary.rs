//! 阶段④ 轮边界判定（T-179 · B3，PL-015 §5 动作 B3）。
//!
//! 一次响应之后环要做什么，以及末轮答话取哪一段。两者都是**纯函数**——环的
//! 唯一决策点与唯一答话取值点，因此可以单测钉住（判据见
//! `turn_runner.test.rs` 的 `round_decision_tests`）。

use super::*;

/// 一次响应之后环要做什么——PL-014 §3 五值里的**两个决策值**。
///
/// `Ask` 是环头（每次迭代都做，不是决策），`Emit` / `Wait` 由执行段产生，
/// 所以环里真正需要「选下一步」的只有本枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoundDecision {
    /// 收束：本轮不再请求工具（落 final，或空答话落兜底）。
    Halt,
    /// 派发：模型请求了工具，且本轮有分发通道。
    Act,
    /// 配置缺口：请求了工具却没有分发通道——按失败诚实回报，不是「正常收束」。
    ConfigGap,
}

/// 纯决策：只看「欠不欠一批工具」与「有没有分发通道」，不读任何外部状态。
///
/// ## 第一维来自表，不来自响应（T-227 步 5a）
///
/// `tools_due` 由调用点问 `turn_tools_due` 视图得到（末次模型应答请求了工具、
/// 那之后还没有工具应答、且没有在途的工具批）。这一维**从内存里的响应搬到事实面**
/// 之后，「该不该跑一批工具」在回放时也答得出——而读 `rt.tool_calls` 只有当场
/// 那一次答得出。
///
/// ## 第二维为什么留在代码里
///
/// 「本轮有没有分发通道」不在事实面：它是这次装配的**入参**（`dispatch: Option<&dyn
/// DispatchPort>`），不是日志里发生过的事。把配置也落成事实是另一件事（`session.config`
/// 已经在做，但通道存在性是装配侧的知识），本步不动它。于是这一维诚实地留在这里，
/// 而不是假装它也在表里。
pub(crate) fn classify_round(tools_due: bool, has_dispatch: bool) -> RoundDecision {
    match (tools_due, has_dispatch) {
        (false, _) => RoundDecision::Halt,
        (true, true) => RoundDecision::Act,
        (true, false) => RoundDecision::ConfigGap,
    }
}

/// 末轮答话的取值：正文 → （截断则空）→ 端口思考 → 桥上的思考。
///
/// 四条顺序都是**判据**，不是偏好（T-142 实测：`finish=length`、正文 0 字符的一轮，
/// 13045 字的思考草稿被原样当答话交付，退出码还是 0）：
///
/// 1. 正文非空 ⇒ 正文（半截正文照常交付，自动续写在 v2 未接上）；
/// 2. `finish = length` ⇒ **空**：被 `max_tokens` 掐断的半段思考是草稿；
/// 3. 端口思考非空 ⇒ 思考即正文（reasoning 模型常常想清楚但不复述）；
/// 4. 桥上那份思考 ⇒ 端口没带思考的适配器只能靠它。
pub(crate) fn final_answer(rt: &LlmTurn, round_facts: &StreamNodeFacts) -> String {
    if !rt.text.trim().is_empty() {
        rt.text.clone()
    } else if rt.finish.is_length() {
        String::new()
    } else if !rt.reasoning.trim().is_empty() {
        rt.reasoning.clone()
    } else {
        round_facts.reasoning.clone().unwrap_or_default()
    }
}
