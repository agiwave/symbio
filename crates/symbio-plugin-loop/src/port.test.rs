//! 端口引擎的用例：谁写哪一条（判据 2 / 3）、行不欠就不派发、幂等、
//! 租约的重试与判死（判据 7）。
//!
//! 反向用例是主菜——「引擎忘了落 `sent`」「失败静默挂起」「租约到期没人管」三种
//! 形态都不会自己报错，只有用例能把它们叫出来。

use std::sync::Arc;

use serde_json::json;

use super::super::loop_rows::{load, PORTS, ROWS};
use super::*;
use symbio_core::ring::{owed, PortDecl};
use symbio_core::{EventStore, Seq};

fn boot() -> EventStore {
    EventStore::new()
}

fn kinds(store: &EventStore) -> Vec<String> {
    store
        .range(Seq::new(0))
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

fn payload_of(store: &EventStore, kind: &str) -> serde_json::Value {
    store
        .range(Seq::new(0))
        .into_iter()
        .find(|e| e.kind == kind)
        .map(|e| e.payload)
        .unwrap_or(serde_json::Value::Null)
}

/// 一笔放行的前提事实 `authz.granted(port=tool)`。
///
/// 引擎在落 `sent` **之前**自己读授权行（D-136），而 `dispatch-tools` 的开账条件
/// 就是这条事实——工具端口的用例必须有它，否则派发会被拒。谁写它、载荷长什么
/// 样归 `tool_dispatch.test.rs` 的写方用例；这里只补前提。
fn grant(store: &EventStore, call: &Call) {
    append(
        store,
        Event::pending(
            format!("grant-1-{}", call.id),
            symbio_core::EVENT_AUTHZ_GRANTED,
            Entity::Turn,
            Verb::Asserted,
            1,
            "authorizer",
        )
        .with_produced_by(7)
        .with_payload(json!({
            "key": call.id,
            "port": "tool",
            "id": call.id,
            // 授权级联与派发关同一**次尝试**（D-138）：载荷带 `attempt`，
            // 缺了它 `authorize-tool` / `dispatch-tools` 的三元组键取不到实例。
            "attempt": call.attempt
        })),
    )
    .unwrap();
}

// ── 谁写哪一条 ───────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_writes_before_and_the_answer_itself() {
    // 判据 2 / 3 的正向：派发前后各一条事实，执行体自己不写任何协议事实。
    let store = boot();
    let call = Call::new("turn", "turn-1-r0", 1);
    announce(&store, &call, 1, 7).unwrap();

    let out = dispatch(
        &store,
        PORTS,
        Dispatch {
            call: &call,
            turn: 1,
            produced_by: 7,
            row: "dispatch-turn",
            now: 0,
            awaited: false,
        },
        || async { Ok("答完了") },
        |_: &()| unreachable!("成功路径不该问 why"),
    )
    .await
    .expect("派发成功");

    assert_eq!(out, "答完了");
    assert_eq!(kinds(&store), vec!["request", "sent", "response"]);
    // 租约与尝试上限随 `sent` 一起落：`stale_sent` 视图只吃事实，读不到 `PortDecl`。
    let sent = payload_of(&store, "sent");
    assert_eq!(sent["lease_ms"], json!(600_000));
    assert_eq!(sent["attempt_cap"], json!(1));
    let request = payload_of(&store, "request");
    assert_eq!(request["id"], json!("turn-1-r0"));
}

#[test]
fn announce_with_merges_the_authorization_payload_into_the_request() {
    // 授权行要读「请求了什么、风险多大、批过没有」——它们必须与 `request`
    // 同一条事实落盘。`announce_with` 只并入对象字段，协议三字段保持不动。
    let store = boot();
    let call = Call::new("tool", "tool-1-d1-b0-c0", 1);
    announce_with(
        &store,
        &call,
        1,
        7,
        json!({
            "tool": "read_file",
            "risk": "low",
            "threshold": "medium",
            "approved": false,
            "deferred": false,
        }),
    )
    .unwrap();

    let request = payload_of(&store, "request");
    assert_eq!(request["port"], json!("tool"));
    assert_eq!(request["id"], json!("tool-1-d1-b0-c0"));
    assert_eq!(request["attempt"], json!(1));
    assert_eq!(request["tool"], json!("read_file"));
    assert_eq!(request["risk"], json!("low"));
    assert_eq!(request["approved"], json!(false));
}

#[tokio::test]
async fn begin_and_succeed_can_bracket_a_whole_batch() {
    // 工具批次的形态：N 笔先各自 `sent`，执行体整批跑完后再逐笔 `response`。
    // 事实顺序钉住这一点——「本批在途几笔」因此读得出来。
    let store = boot();
    let a = Call::new("tool", "tool-1-d1-b0-c0", 1);
    let b = Call::new("tool", "tool-1-d1-b0-c1", 1);
    for call in [&a, &b] {
        announce(&store, call, 1, 7).unwrap();
        grant(&store, call);
    }
    for call in [&a, &b] {
        begin(
            &store,
            PORTS,
            Dispatch {
                call,
                turn: 1,
                produced_by: 7,
                row: "dispatch-tools",
                now: 0,
                awaited: false,
            },
        )
        .unwrap();
    }
    assert_eq!(
        kinds(&store),
        vec![
            "request",
            "authz.granted",
            "request",
            "authz.granted",
            "sent",
            "sent"
        ]
    );

    succeed(&store, &a, 1, 7).unwrap();
    succeed(&store, &b, 1, 7).unwrap();
    assert_eq!(
        kinds(&store),
        vec![
            "request",
            "authz.granted",
            "request",
            "authz.granted",
            "sent",
            "sent",
            "response",
            "response"
        ]
    );
}

#[tokio::test]
async fn a_failed_actor_leaves_a_failed_fact_not_a_silent_hang() {
    // 判据 3：`run = Err` ⇒ 引擎落 `failed`，执行体自己的错误原样交回。
    let store = boot();
    let call = Call::new("turn", "turn-1-r0", 1);
    announce(&store, &call, 1, 7).unwrap();

    let res: Result<(), DispatchError<&'static str>> = dispatch(
        &store,
        PORTS,
        Dispatch {
            call: &call,
            turn: 1,
            produced_by: 7,
            row: "dispatch-turn",
            now: 0,
            awaited: false,
        },
        || async { Err("模型超时") },
        |e: &&'static str| format!("failed: {e}"),
    )
    .await;

    assert!(matches!(res, Err(DispatchError::Actor("模型超时"))));
    assert_eq!(kinds(&store), vec!["request", "sent", "failed"]);
    assert_eq!(
        payload_of(&store, "failed")["why"],
        json!("failed: 模型超时")
    );
}

#[tokio::test]
async fn a_row_that_is_not_owed_never_reaches_the_actor() {
    // 行不欠 ⇒ 拒绝派发。这不是防御式编程：`request` 没落成 / 行表装载失败
    // 都会从这里显形，而不是让一次没人承认的调用静默跑完。
    //
    // 这里**只**不写 `request`：结论由引擎现读（D-136），调用点给的只有行名。
    let store = boot();
    let call = Call::new("turn", "turn-1-r0", 1);
    let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let flag = ran.clone();
    let res: Result<(), DispatchError<()>> = dispatch(
        &store,
        PORTS,
        Dispatch {
            call: &call,
            turn: 1,
            produced_by: 7,
            row: "dispatch-turn",
            now: 0,
            awaited: false,
        },
        move || {
            let flag = flag.clone();
            async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        },
        |_: &()| "unreachable".into(),
    )
    .await;

    match res {
        Err(DispatchError::NotOwed(row)) => assert_eq!(
            row, "dispatch-turn",
            "拒派要说得出是哪一行不欠（行名由调用点声明、引擎读出）"
        ),
        other => panic!("没有账的调用不该派发：{other:?}"),
    }
    assert!(
        !ran.load(std::sync::atomic::Ordering::SeqCst),
        "没有账的调用不许发出"
    );
    assert!(kinds(&store).is_empty(), "一条事实都不该落");
}

#[tokio::test]
async fn an_undeclared_port_is_named_not_defaulted() {
    // 漏声明 ⇒ 显形。不静默拿一个默认租约——默认值会把「漏了」变成「看起来正常」。
    //
    // 端口名取一个**没声明过的**（`ghost`）：`model` / `turn` 已在 `PORTS` 里，
    // 拿它们当「未声明」会让这条判据随端口增加而失效。
    let store = boot();
    let call = Call::new("ghost", "m-1", 1);
    let res: Result<(), DispatchError<()>> = dispatch(
        &store,
        PORTS,
        Dispatch {
            call: &call,
            turn: 1,
            produced_by: 7,
            // 端口都没有声明，行名只是占位：`NoPort` 在读账之前判。
            row: "dispatch-ghost",
            now: 0,
            awaited: false,
        },
        || async { Ok(()) },
        |_: &()| "unreachable".into(),
    )
    .await;
    assert!(matches!(res, Err(DispatchError::NoPort("ghost"))));
}

#[tokio::test]
async fn dispatching_twice_does_not_double_the_facts() {
    // 判据 6 的第一半：一笔账只还得一次。第一次派发落完 `sent`，`dispatch-turn`
    // 就不再欠——第二次连执行体都不会跑（D-136：结论由引擎现读，调用点不重复主张）。
    let store = boot();
    let call = Call::new("turn", "turn-1-r0", 1);
    announce(&store, &call, 1, 7).unwrap();
    let out: Result<(), DispatchError<()>> = dispatch(
        &store,
        PORTS,
        Dispatch {
            call: &call,
            turn: 1,
            produced_by: 7,
            row: "dispatch-turn",
            now: 0,
            awaited: false,
        },
        || async { Ok(()) },
        |_: &()| "unreachable".into(),
    )
    .await;
    out.expect("第一次派发");
    let second: Result<(), DispatchError<()>> = dispatch(
        &store,
        PORTS,
        Dispatch {
            call: &call,
            turn: 1,
            produced_by: 7,
            row: "dispatch-turn",
            now: 0,
            awaited: false,
        },
        || async { Ok(()) },
        |_: &()| "不该派发".into(),
    )
    .await;
    assert!(
        matches!(second, Err(DispatchError::NotOwed(_))),
        "已还过的账不该被派第二次：{second:?}"
    );
    assert_eq!(kinds(&store), vec!["request", "sent", "response"]);
}

#[test]
fn a_repeated_fact_write_collapses_on_its_idempotency_key() {
    // 判据 6 的第二半：幂等键 `(port, id, attempt)`（`failed` 是逻辑 id）。
    // 行**仍然欠**时重复写同一格，引擎吞掉 `Duplicate` 而不是多落一格——
    // 「同一个动作被跑两次」与「账上出现两条」是两件事，得分别钉住。
    let store = boot();
    let call = Call::new("turn", "turn-1-r0", 1);
    announce(&store, &call, 1, 7).unwrap();
    announce(&store, &call, 1, 7).unwrap();
    assert_eq!(kinds(&store), vec!["request"], "重复的 `request` 不重落");

    succeed(&store, &call, 1, 7).unwrap();
    succeed(&store, &call, 1, 7).unwrap();
    assert_eq!(
        kinds(&store),
        vec!["request", "response"],
        "重复的应答撞 `(port, id, attempt)`，不重落"
    );
}

// ── 租约 ────────────────────────────────────────────────────────────────

/// 一条只发未答的派发事实（`sent` 之后进程死掉的样子）。
fn stranded(store: &EventStore, id: &str, attempt: u64, lease_ms: u64, cap: u64, awaited: bool) {
    store
        .append(
            Event::pending(
                format!("sent-turn-{id}-{attempt}"),
                symbio_core::EVENT_SENT,
                Entity::Turn,
                Verb::Asserted,
                1,
                "system",
            )
            .with_ts(0)
            .with_payload(json!({
                "port": "turn",
                "id": id,
                "attempt": attempt,
                "lease_ms": lease_ms,
                "attempt_cap": cap,
                // 重试的两个前提之一（D-138）：只有「有活着的等待方」的尝试
                // 才会被租约看护重试，其余到期即判死。
                "awaited": awaited,
            })),
        )
        .unwrap();
}

#[test]
fn the_lease_kills_a_turn_call_that_nobody_awaits() {
    // 判据 7（判死一侧）：`turn` 的 `max_attempts = 1` ⇒ 到期即判死。
    // 重试只会留下一条永不派发的 `request`——等它的 future 随进程一起没了。
    let store = boot();
    stranded(&store, "turn-9-r0", 1, 1_000, 1, false);
    let frozen = load(ROWS).expect("真表装载：看护者读的表 = 会话开幕用的那张");

    // 还没到期：不欠、不动。
    assert!(
        reap(&store, &frozen, PORTS, &store.range(Seq::new(0)), 1, 1, 999)
            .unwrap()
            .is_empty()
    );
    assert_eq!(kinds(&store), vec!["sent"]);

    let out = reap(
        &store,
        &frozen,
        PORTS,
        &store.range(Seq::new(0)),
        1,
        1,
        1_000,
    )
    .unwrap();
    assert_eq!(
        out,
        vec![Reaped::Killed {
            id: "turn-9-r0".into(),
            attempt: 1,
        }]
    );
    assert_eq!(kinds(&store), vec!["sent", "failed"]);
    assert_eq!(payload_of(&store, "failed")["why"], json!("lease_expired"));

    // 关账之后不再欠：第二次收租什么都不做（幂等）。
    assert!(reap(
        &store,
        &frozen,
        PORTS,
        &store.range(Seq::new(0)),
        1,
        1,
        9_999
    )
    .unwrap()
    .is_empty());
}

#[test]
fn the_lease_retries_while_the_port_has_attempts_left() {
    // 判据 7（重试一侧，D-138）：重试的两个前提——`sent.awaited`（有活着的等待方）
    // 与 `attempt < attempt_cap`——都成立时，落 `retry{attempt}` + `request{attempt+1}`，
    // 旧尝试由 `retry` 收清。
    //
    // 端口声明**从真表来**（`loop_rows::PORTS`），只改 `max_attempts` 这一格：
    // 写一条独立的合成 `PortDecl` 时，`port` / `before` / `lease_ms` 与真表各长
    // 一套，真表漂了用例照样绿。这条用例证明的是**分支形状**——生产里 `turn`
    // 的上限是 1 且普通批次的等待方随进程消失，这条路径由 `tool` 端口的唤醒批次
    // （等待方能由 `wake_batch` 从事实重建）走。
    let retrying: Vec<PortDecl> = PORTS
        .iter()
        .map(|p| PortDecl {
            max_attempts: if p.port == "turn" { 3 } else { p.max_attempts },
            ..*p
        })
        .collect();
    let turn = retrying
        .iter()
        .find(|p| p.port == "turn")
        .expect("真表里有 `turn` 端口");
    let store = boot();
    // 在途格的租约 / 尝试上限与判定用的 `now` 都从**同一份真表**推出来：
    // 换端口声明不必再改这里的数字，改了数字也不会让用例假绿。
    stranded(
        &store,
        "turn-9-r0",
        1,
        turn.lease_ms,
        turn.max_attempts,
        true,
    );
    let frozen = load(ROWS).expect("真表装载：看护者读的表 = 会话开幕用的那张");

    let out = reap(
        &store,
        &frozen,
        &retrying,
        &store.range(Seq::new(0)),
        1,
        1,
        turn.lease_ms as i64 + 1,
    )
    .unwrap();
    assert_eq!(
        out,
        vec![Reaped::Retried {
            id: "turn-9-r0".into(),
            attempt: 1,
        }]
    );
    assert_eq!(kinds(&store), vec!["sent", "request", "retry"]);
    // 写序是故意的（D-138 落地注释）：先写下一笔 `request`、再写 `retry`——两步
    // 之间崩溃时，下一次 `reap` 看到 `request{attempt+1}` 已存在，只补 `retry`，
    // 旧尝试不会被重复作废，新尝试也不会丢。
    assert_eq!(payload_of(&store, "retry")["attempt"], json!(1));
    assert_eq!(payload_of(&store, "request")["attempt"], json!(2));
}

#[test]
fn only_the_tool_port_has_headroom_and_only_for_rebuildable_waiters() {
    // 真表此刻的取值（D-138）：`turn` / `model` 的上限是 1——它们的等待方
    // （轮内同步等待）随进程消失，重试只会留下永不派发的 `request`，到顶即判死；
    // `tool` 是 2 = 首次 + 一次重试，且**逐次尝试是否真能重试由 `sent.awaited` 决定**
    // （普通批次 `awaited = false` 仍判死；唤醒批次能由 `wake_batch` 从事实重建
    // ⇒ `awaited = true` ⇒ 可重试一次）。
    //
    // 「重试出来的 request 有行会派它」不再靠这句注释担保：行层债键已含
    // `attempt`（D-138），`a_retried_request_reopens_authorization_and_dispatch`
    // 在行层钉住「第二次尝试重新欠授权与派发」。
    let caps: Vec<(&str, u64)> = PORTS.iter().map(|p| (p.port, p.max_attempts)).collect();
    assert_eq!(
        caps,
        vec![("turn", 1), ("model", 1), ("tool", 2)],
        "端口上限漂了：抬 `turn` / `model` 前先回答「等待方是谁」，抬 `tool` 到 3 前先回答「还需要哪种等待方形态」"
    );
}

#[test]
fn every_port_row_is_in_the_table() {
    // 加一个端口 = 1 条 `PortDecl` + 2 行模板（dispatch / await）。租约行**不**
    // 按端口写（一个 `lease` 行管所有端口），少了它 `sent` 之后崩溃就永远挂着。
    for p in PORTS {
        // 工具端口的行名沿用方案里的复数（`dispatch-tools` / `await-tools`）：
        // 一个工具端口的债单位是**单次调用**，但一次模型响应是一批；行名读的是
        // 批次语义。模板仍是两条（dispatch / await），不是第三条。
        let stem = if p.port == "tool" { "tools" } else { p.port };
        for row in [format!("dispatch-{stem}"), format!("await-{stem}")] {
            assert!(
                ROWS.iter().any(|d| d.name == row),
                "端口 {} 缺 {row} 行",
                p.port
            );
        }
    }
    assert_eq!(ROWS.iter().filter(|d| d.name == "lease").count(), 1);
    // `await` 行就是 `Wait` 的折叠：恒 blocked，永不选中。
    let await_row = ROWS.iter().find(|d| d.name == "await-turn").unwrap();
    assert_eq!(await_row.by, None);
}

// ── 内部请求也走端口─────────────────────────────────────────────

#[tokio::test]
async fn an_internal_request_is_a_full_model_round_trip() {
    // DE-015 判据 2 的全量口径：编排层替别人打的那次模型调用也要三格，
    // 而且顺序是 request → sent → response（`sent` 夹在中间 = 引擎在执行体
    // **之前**记账）。缺这条形状时的形态是「打了模型而日志一格没有」：
    // 那次调用既没有租约可收，也没有失败事实可记。
    let store = boot();
    let out = round_trip(
        &store,
        Round {
            turn: 3,
            produced_by: 2,
            drive_seq: 9,
            now: 0,
            step: "w0",
            via: "compose/wording",
        },
        || async { Ok::<_, String>(Some("答话")) },
    )
    .await
    .expect("内部请求应当按端口 `model` 派发");
    assert_eq!(out, Some("答话"));

    let facts = store.range(Seq::new(0));
    assert_eq!(
        facts.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        vec!["request", "sent", "response"],
        "三格齐、且 `sent` 排在应答之前"
    );
    let seqs: Vec<u64> = facts.iter().map(|e| e.seq.unwrap().value()).collect();
    assert!(
        seqs[0] < seqs[1] && seqs[1] < seqs[2],
        "`sent` 的 seq 必须介于 `request` 与 `response` 之间：{seqs:?}"
    );
    // 逻辑 id 带**幕锚**（`model-{turn}-d{drive}-{step}`）：续写幕复用同一轮号，
    // 不带幕锚就会与首幕撞成同一笔账，而首幕已经有过 `sent`。
    assert_eq!(payload_of(&store, "request")["id"], json!("model-3-d9-w0"));
    assert_eq!(payload_of(&store, "request")["port"], json!("model"));
    assert_eq!(
        payload_of(&store, "request")["via"],
        json!("compose/wording")
    );
    assert_eq!(payload_of(&store, "sent")["id"], json!("model-3-d9-w0"));
    assert_eq!(payload_of(&store, "response")["id"], json!("model-3-d9-w0"));
    assert_eq!(
        payload_of(&store, "response")["via"],
        json!("compose/wording")
    );
    // 这一笔不带工具调用 ⇒ `open-tools` 的集合展开开出**零笔**子债。
    assert_eq!(
        store
            .range(Seq::new(0))
            .iter()
            .filter(|e| e.kind == "request")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_second_internal_request_in_the_same_act_is_a_separate_call() {
    // 同一幕里两次内部请求（步名不同）= 两笔独立的账。
    let store = boot();
    round_trip(
        &store,
        Round {
            turn: 3,
            produced_by: 2,
            drive_seq: 9,
            now: 0,
            step: "w0",
            via: "compose/wording",
        },
        || async { Ok::<_, String>("第一次") },
    )
    .await
    .unwrap();
    round_trip(
        &store,
        Round {
            turn: 3,
            produced_by: 2,
            drive_seq: 9,
            now: 0,
            step: "w1",
            via: "compose/wording",
        },
        || async { Ok::<_, String>("第二次") },
    )
    .await
    .unwrap();
    let facts = store.range(Seq::new(0));
    assert_eq!(facts.iter().filter(|e| e.kind == "request").count(), 2);
    assert_eq!(facts.iter().filter(|e| e.kind == "response").count(), 2);
}

// ── 压缩摘要也走端口─────────────────────────────────────────────

#[tokio::test]
async fn a_compression_summary_is_accounted_as_a_model_round_trip() {
    // 压缩摘要是一次**内部**模型往返（D-122 决策 2）：同样三格，同样带身份。
    // 身份是判据侧把「这次内部往返」与「这一轮自己答完了」分开的唯一依据——
    // 混进来会让判据读到一条 `response` 就以为本轮已收束。
    let store = Arc::new(boot());
    let compress = Compress::new(store.clone(), 3, 2, 9, 0, 0);
    let out = compress
        .summary(|| async { Ok::<_, String>("摘要") })
        .await
        .expect("压缩摘要应当按端口 `model` 派发");
    assert_eq!(out, "摘要");
    assert_eq!(compress.calls(), 1, "打了一次摘要模型，次数就是 1");

    let facts = store.range(Seq::new(0));
    assert_eq!(
        facts.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        vec!["request", "sent", "response"]
    );
    for e in &facts {
        assert_eq!(
            e.payload["via"],
            json!("compress/summary"),
            "{} 这一格丢了身份",
            e.kind
        );
    }
    // 逻辑 id 带**幕锚**与请求轮：`model-{turn}-d{drive}-z{round}-{attempt}`。
    assert_eq!(
        payload_of(&store, "request")["id"],
        json!("model-3-d9-z0-0")
    );
    // 内部往返不带工具 ⇒ 应答格的 `calls` 是空数组（`open-tools` 展开出零笔子债）。
    assert_eq!(payload_of(&store, "response")["calls"], json!([]));
}

#[tokio::test]
async fn the_correction_retry_is_a_separate_call_and_a_failure_still_has_its_cell() {
    // 压缩可能打两次模型（快照校验不过时纠正重试）：两笔账，两个逻辑 id。
    // 并成一笔会让「三格 = 一次模型往返」失真，判据侧数不出真实调用次数。
    let store = Arc::new(boot());
    let compress = Compress::new(store.clone(), 3, 2, 9, 0, 0);
    compress
        .summary(|| async { Ok::<_, String>("第一次") })
        .await
        .unwrap();
    let second = compress
        .summary(|| async { Err::<String, _>("网关 500") })
        .await;
    assert!(second.is_err(), "执行体失败要原样交回");

    assert_eq!(compress.calls(), 2, "两次摘要往返就是两笔账");
    let facts = store.range(Seq::new(0));
    let ids: Vec<&str> = facts
        .iter()
        .filter(|e| e.kind == "request")
        .map(|e| e.payload["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["model-3-d9-z0-0", "model-3-d9-z0-1"]);
    // 失败那笔也有带身份的 `failed` 格：压缩失败不收束这一轮（它是优化项），
    // 但那次往返**发生过**——账上必须有它（D-122 决策 5）。
    let failed = facts
        .iter()
        .find(|e| e.kind == "failed")
        .expect("失败的摘要往返必须留下 `failed` 格");
    assert_eq!(failed.payload["via"], json!("compress/summary"));
    assert_eq!(failed.payload["id"], json!("model-3-d9-z0-1"));
}

// ── 窗口边界：看护只看见递给它的那批（DE-015 §2.6 第 7 条的边界段）──────

/// 收租窗口是**切片**，不是 `reap` 内部的过滤：切点由调用方
/// （`chat_loop::drive` 的 `lease_tail`）按 `LEASE_WINDOW` 决定。
///
/// 这条用例把后果钉住——窗口外那笔陈旧 `sent` **不被判死**，账仍开着，
/// 由 `waiting` 行挂着；同一笔放进视野则到期即判死。两半合起来才说明
/// 「窗口外靠 `waiting`」是真的降级路径，不是一句猜测。
#[test]
fn reap_only_sees_the_facts_it_is_handed() {
    let store = boot();
    // 第一笔 = 窗口外的陈旧派发；后面两笔是把它挤出窗口的「新事实」。
    stranded(&store, "turn-9-r0", 1, 1_000, 1, false);
    for (i, kind) in ["turn.stage", "turn.stage"].iter().enumerate() {
        store
            .append(
                Event::pending(
                    format!("filler-{i}"),
                    *kind,
                    Entity::Turn,
                    Verb::Asserted,
                    1,
                    "system",
                )
                .with_ts(2_000),
            )
            .unwrap();
    }
    let frozen = load(ROWS).expect("真表装载：看护者读的表 = 会话开幕用的那张");
    let all = store.range(Seq::new(0));

    // 切片不含第一笔 ⇒ 那笔过期账看不到，什么都不收（不静默当它死了：`sent` 还在）。
    let out = reap(&store, &frozen, PORTS, &all[1..], 1, 1, 10_000).unwrap();
    assert!(out.is_empty(), "窗口外的陈旧 `sent` 不该被收：{out:?}");
    assert_eq!(
        kinds(&store),
        vec!["sent", "turn.stage", "turn.stage"],
        "收租不动账：没有 `failed` 也没有 `retry`"
    );

    // 同一笔放进视野 ⇒ 到期即判死（`turn` 端口 `max_attempts = 1`）。
    let out = reap(&store, &frozen, PORTS, &all, 1, 1, 10_000).unwrap();
    assert_eq!(
        out,
        vec![Reaped::Killed {
            id: "turn-9-r0".into(),
            attempt: 1,
        }],
        "窗口内看到它就必须收"
    );
    assert_eq!(payload_of(&store, "failed")["why"], json!("lease_expired"));
}

// ── 重放的粒度是「一次调用」（判据 ③）────────────────────────────────
//
// 粗端口的过渡形状留下的那句「崩溃恢复只能按整轮重放」，说的是**没有事实**可依：
// 一轮的内存草稿一死，重启只能从头再跑一遍。端口接净之后这句话不再成立——
// 每一笔调用的身份 `(port, id, attempt)` 都在日志里，已经回来的那几笔的 `response`
// 也在，于是「谁还欠着」与「卡住的那笔怎么办」都是事实的函数。`reap` 的输入里
// **没有内存状态**（不看 `in_flight`，只看切片），所以用例里「杀掉再重放」就等于
// 「把同一份日志再喂一遍」——这不是模拟，是同一条代码路径。

/// 落一条 `request`：`extra` 是端口特有的字段（工具的 `tool` / `risk` 等）。
fn request_cell(store: &EventStore, port: &str, id: &str, extra: serde_json::Value) {
    let mut payload = json!({ "port": port, "id": id, "attempt": 1 });
    if let (Some(obj), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            obj.insert(k.clone(), v.clone());
        }
    }
    append(
        store,
        Event::pending(
            format!("{port}-{id}-request"),
            "request",
            Entity::Turn,
            Verb::Asserted,
            1,
            "executor",
        )
        .with_produced_by(7)
        .with_payload(payload),
    )
    .unwrap();
}

/// 授权级联夹在 `request` 与 `sent` 之间：没有它，工具那两行永远不开账。
fn granted_cell(store: &EventStore, id: &str) {
    append(
        store,
        Event::pending(
            format!("tool-{id}-granted"),
            symbio_core::EVENT_AUTHZ_GRANTED,
            Entity::Turn,
            Verb::Asserted,
            1,
            "authorizer",
        )
        .with_produced_by(7)
        .with_payload(json!({
            "key": id,
            "port": "tool",
            "id": id,
            "attempt": 1,
            "tool": "read_file",
            "risk": "low",
            "deferred": false,
        })),
    )
    .unwrap();
}

/// 引擎在跑执行体**之前**落的那一格（判据 2：写方不变）。
fn sent_cell(store: &EventStore, port: &str, id: &str) {
    append(
        store,
        Event::pending(
            format!("{port}-{id}-sent"),
            "sent",
            Entity::Turn,
            Verb::Asserted,
            1,
            "system",
        )
        .with_ts(0)
        .with_payload(json!({
            "port": port,
            "id": id,
            "attempt": 1,
            "lease_ms": 600_000,
            "attempt_cap": 1,
            "awaited": false,
        })),
    )
    .unwrap();
}

fn response_cell(store: &EventStore, port: &str, id: &str, calls: serde_json::Value) {
    append(
        store,
        Event::pending(
            format!("{port}-{id}-response"),
            "response",
            Entity::Turn,
            Verb::Asserted,
            1,
            "executor",
        )
        .with_ts(1_000)
        .with_payload(json!({
            "port": port,
            "id": id,
            "attempt": 1,
            "calls": calls,
        })),
    )
    .unwrap();
}

/// 判据 ③：**重放只补那一笔**。
///
/// 现场摆的是一轮跑了三分之二的日志：幕的往返派出去了、第一次模型往返回来了
/// （带两个工具调用）、那两个工具跑完了、第二次模型往返派出去了而应答没回来。
/// 三件事一起才叫判据：
///
/// 1. **表这一侧**：重放时已经还过的债一条都不重新开账——`dispatch-model` 不该
///    再欠（两笔都有 `sent`）、工具那三行也不该再欠（两笔都有 `response`）；
/// 2. **看护这一侧**：`reap` 只对「派出去了、没回来」的两笔下判决，一笔是幕自己
///    那一笔，一笔是第二次往返；
/// 3. **不动账那一侧**：判死只写它自己那一格，没回来的两笔之外一个 `failed`
///    都不多、一条 `retry` 都不写。
///
/// 第 2 条同时钉住「粗端口的粒度在哪」：幕自己那一笔就是**一整轮**，它在日志上
/// 与模型往返同形（`request` / `sent` / `response`），所以恢复能逐笔判而不是整轮重跑。
#[test]
fn a_replay_makes_up_only_the_call_that_never_came_back() {
    let store = boot();
    // 幕的往返：派出去了，`response` 没有——这一轮没跑完就死了。
    request_cell(&store, "turn", "turn-1-d3-r0", serde_json::Value::Null);
    sent_cell(&store, "turn", "turn-1-d3-r0");

    // 第一次模型往返：回来了，还带回两个工具调用。
    request_cell(&store, "model", "model-1-d3-r0", json!({ "via": "turn" }));
    sent_cell(&store, "model", "model-1-d3-r0");
    response_cell(
        &store,
        "model",
        "model-1-d3-r0",
        json!([
            { "port": "tool", "id": "call-a", "name": "read_file", "arguments": {} },
            { "port": "tool", "id": "call-b", "name": "read_file", "arguments": {} },
        ]),
    );

    // 那一批工具：放行、派出、回来了。
    for id in ["call-a", "call-b"] {
        request_cell(
            &store,
            "tool",
            id,
            json!({
                "tool": "read_file",
                "risk": "low",
                "threshold": "medium",
                "approved": false,
                "deferred": false,
            }),
        );
        granted_cell(&store, id);
        sent_cell(&store, "tool", id);
        response_cell(&store, "tool", id, json!([]));
    }

    // 第二次模型往返：派出去了，应答没回来。
    request_cell(&store, "model", "model-1-d3-r1", json!({ "via": "turn" }));
    sent_cell(&store, "model", "model-1-d3-r1");

    let frozen = load(ROWS).expect("真表装载：重放读的表 = 会话开幕用的那张");
    let facts = store.range(Seq::new(0));
    // 重启之后时钟必然走过租约（时间只在事件上推进），所以 `now` 取租约之后。
    let now = 700_000;

    // 一、表这一侧：还过的债不重开。
    let debts = owed(ROWS, &frozen, &facts, now);
    for paid in [
        "dispatch-model",
        "open-tools",
        "authorize-tool",
        "dispatch-tools",
        "await-tools",
    ] {
        assert!(
            !debts.iter().any(|d| d.decl.name == paid),
            "重放不该让 `{paid}` 重新开账——那一笔早还过了：{:?}",
            debts
                .iter()
                .map(|d| (d.decl.name, d.key.trace()))
                .collect::<Vec<_>>()
        );
    }

    // 二、看护这一侧：只判这两笔。
    let out = reap(&store, &frozen, PORTS, &facts, 1, 7, now).unwrap();
    assert_eq!(
        out,
        vec![
            Reaped::Killed {
                id: "turn-1-d3-r0".into(),
                attempt: 1,
            },
            Reaped::Killed {
                id: "model-1-d3-r1".into(),
                attempt: 1,
            },
        ],
        "只有这两笔没人接：第一次往返与两个工具调用早就答过了"
    );

    // 三、不动账：判死只落它自己那一格，其余一笔不碰。
    let failed: Vec<String> = store
        .range(Seq::new(0))
        .iter()
        .filter(|e| e.kind == "failed")
        .map(|e| e.payload["id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        failed,
        vec!["turn-1-d3-r0", "model-1-d3-r1"],
        "多一个 `failed` 就是把已经回来的那笔又判了一次死"
    );
    assert!(
        !kinds(&store).iter().any(|k| k == "retry"),
        "`turn` / `model` 的 `max_attempts = 1`：到期即判死，不写 `retry`"
    );
}
