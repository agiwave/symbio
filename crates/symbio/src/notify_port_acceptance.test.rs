//! 第三端口验收的断言（DE-048 D-302）。
//!
//! 验收的判据不是「测试夹具能跑」，而是三件事同时成立：
//! 1. 生产冻结表（链接 loop 与全部插件，经 `sealed_declarations`）接纳该端口的
//!    `PortDecl` 与行——否则装载期就会红（`PortWithoutAnswerExit` / `UnmannedRow` 等）；
//! 2. 环的通用受理执行体（`PORT_ACTOR`）出现在冻结的执行体声明里；
//! 3. 给一条 `request(port=notify)` 事实，环的选债器（`ring::owed`）会选中这一行。
//!
//! 本文件不改 loop 的任何文件——零改动接入的证明见提交的 `git diff --stat`。

use symbio_kernel::decl::ring::{owed, Lit};
use symbio_kernel::decl::Frozen;
use symbio_kernel::event::{Event, EVENT_REQUEST};
use symbio_kernel::{Entity, Verb};
use symbio_plugin_api::rows::PORT_ACTOR;
use symbio_plugin_api::{sealed_declarations, EVENT_SENT};

const NOTIFY: &str = "notify";

#[test]
fn production_table_accepts_the_third_port() {
    // 必须经本 crate 的门面触达一次，`lib.rs` 里的 `pub use <插件 crate>;` 链接行才会把
    // 全部插件 crate 拉进最终链接；否则本二进制里 inventory 为空，验收退化成空跑。
    let _ = crate::assembly::PLUGIN_ID_HOME;

    let frozen: std::sync::Arc<Frozen> =
        sealed_declarations().expect("生产声明表应能冻结（含 loop + 全部插件）");

    // 1. 端口声明进了表：`before` 是协议格 `sent`。
    let port = frozen
        .ports()
        .iter()
        .find(|p| p.port == NOTIFY)
        .expect("生产表里应有 notify 端口的 PortDecl");
    assert_eq!(port.before, EVENT_SENT);

    // 2. 通用受理执行体的声明在表上（它的动作在 loop 的绑定表里，编译期已接线）。
    assert!(
        frozen.actors().iter().any(|a| a.name == PORT_ACTOR),
        "端口行绑定后 `{PORT_ACTOR}` 应进执行体声明表"
    );

    // 3. 该端口的行绑在通用受理执行体上——行由插件声明，环不认识端口名。
    let row = frozen
        .rows()
        .iter()
        .find(|r| {
            r.by == Some(PORT_ACTOR)
                && r.owes.source == EVENT_REQUEST
                && r.owes
                    .eq
                    .iter()
                    .any(|(field, lit)| *field == "port" && *lit == Lit::Str(NOTIFY))
        })
        .expect("生产表里应有 by=port、owes=request(port=notify) 的行");

    // 4. 选债器：一条 request 事实就能让这一行欠账（可还、未被挂起）。
    let request = Event::pending(
        "request-notify-n1-1",
        EVENT_REQUEST,
        Entity::System,
        Verb::Asserted,
        7,
        "system",
    )
    .with_payload(serde_json::json!({
        "port": NOTIFY,
        "id": "n1",
        "attempt": 1,
    }));
    let debts = owed(frozen.rows(), &frozen, &[request], 0);
    assert!(
        debts.iter().any(|d| d.decl.name == row.name && !d.blocked),
        "notify 的 request 事实应选中 `{}`，实际欠账：{:?}",
        row.name,
        debts
            .iter()
            .map(|d| (d.decl.name, d.blocked))
            .collect::<Vec<_>>()
    );
}
