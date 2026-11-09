//! `view` 域的单元测试——主体 id 的归属段解析
//! （[D-069 §决策 2](../../../../docs/003-design/005-协议规范.md#d-069-principal-三字段kind-与-id-分家id-是可路由地址 "DE-005-协议规范")）。
//!
//! 判据**只有一条**（[`principal_namespace`]），本文件把它钉在地址形状上：任何一次
//! 「顺手放宽 / 收紧」都得先改这里的断言——规则的改动必须是一次显式的决定，而不是
//! 某个调用点顺手写下的第二个条件。

use super::*;

/// **可路由地址的归属段解析**：`<ns>/<name>` 或更长的路径都能取到第一段——
/// 与 VDFS 地址、route 常量同族形状；这是 [D-069 §决策 2] 的"地址"含义。
#[test]
fn principal_namespace_is_the_first_route_segment() {
    assert_eq!(principal_namespace("session/main"), Some("session"));
    assert_eq!(principal_namespace("vdfs/write"), Some("vdfs"));
    assert_eq!(principal_namespace("person/local"), Some("person"));
    // 多段路径也取首段——将来 `<ns>/<sub>/<name>` 的形态不破坏判据。
    assert_eq!(principal_namespace("session/s42/worker"), Some("session"));
    // 边界：首段空（`/foo`）→ 返回 Some("")；地址必须以归属段起头才能路由。
    assert_eq!(principal_namespace("/foo"), Some(""));
    // 未带 namespace 的裸 id：没有归属段，返回 None。
    assert_eq!(principal_namespace("user"), None);
    assert_eq!(principal_namespace(""), None);
    // 大小写敏感：`Session` 与 `session` 是两个归属段。
    assert_eq!(
        principal_namespace("Session/main"),
        Some("Session"),
        "归属段按字面比较，不做大小写归一"
    );
}
