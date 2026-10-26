//! `attributed`（落库补身份的唯一执行点）的判据。
//!
//! 为什么这条测试住在转写域而不是宿主外壳：它钉的是 [`attributed`] 这一个判定点，
//! 而那个点随落库回包出口一起归位到了 `transcript/publish.rs`
//! （[D-075](../../../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P3 第一刀）。

use super::*;

/// plan/11 批 1 ②：
/// 人的话是 `user`（不随会话变），其余是**本会话主体**；**已带主体的一律不覆盖**——
/// 覆盖转播进来的别的会话 / 子智能体的消息就是串主体。
///
/// 为什么断言在 `attributed` 而不是「落库后的文件」：这条规则的判定点只有一个
/// （[`append_and_publish`] 里那次映射），文件断言是它的效果、还得搭一整条链路；
/// 判定点钉死之后，效果由 T30（e2e）在真实落盘上验。
#[test]
fn attributed_fills_by_role_and_never_overwrites() {
    use symbio_core::principal::Principal;
    let base = |role| cm::ChatNode {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        ..Default::default()
    };
    let main = symbio_core::PRINCIPAL_MAIN;
    // 判据看的是 **id 层**（`<ns>/<name>` 可路由地址）——kind 由 `from_legacy` 依 namespace
    // 派生，测试不重复这条规则。
    let of = |m: cm::ChatNode| attributed(m, main).principal.map(|p| p.id);

    // 人说的话不随会话变。
    assert_eq!(
        of(base(Some(cm::NodeRole::User))),
        Some(symbio_core::PRINCIPAL_USER.to_string())
    );
    // 助手正文、工具结果、压缩记录、系统说明都是本会话主体说的。
    assert_eq!(
        of(base(Some(cm::NodeRole::Assistant))),
        Some(main.to_string())
    );
    assert_eq!(of(base(None)), Some(main.to_string()));

    // 已带主体 ⇒ 原样交还（转播的身份优先，这是「不串主体」的写侧那一半）。
    let mut relayed = base(Some(cm::NodeRole::Assistant));
    relayed.principal = Some(Principal::from_legacy("session/reviewer"));
    assert_eq!(
        of(relayed),
        Some("session/reviewer".to_string()),
        "不得把子智能体的发言改成本会话主体"
    );

    // 换个会话主体，人的话仍是 `user`——它是会话外的那一方。
    let mut user = base(Some(cm::NodeRole::User));
    user.principal = None;
    assert_eq!(
        attributed(user, "session/reviewer").principal.map(|p| p.id),
        Some(symbio_core::PRINCIPAL_USER.to_string())
    );
}
