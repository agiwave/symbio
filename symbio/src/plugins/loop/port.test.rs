//! 端口引擎的用例（T-223）：谁写哪一条（判据 2 / 3）、行不欠就不派发、幂等、
//! 租约的重试与判死（判据 7）。
//!
//! 反向用例是主菜——「引擎忘了落 `sent`」「失败静默挂起」「租约到期没人管」三种
//! 形态都不会自己报错，只有用例能把它们叫出来。

use std::sync::Arc;

use serde_json::json;

use super::super::loop_rows::{load, PORTS, ROWS};
use super::*;
use crate::symbio_core::ring::PortDecl;
use crate::symbio_core::{EventStore, Seq};

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
            owed: true,
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
    }
    for call in [&a, &b] {
        begin(
            &store,
            PORTS,
            Dispatch {
                call,
                turn: 1,
                produced_by: 7,
                owed: true,
            },
        )
        .unwrap();
    }
    assert_eq!(kinds(&store), vec!["request", "request", "sent", "sent"]);

    succeed(&store, &a, 1, 7).unwrap();
    succeed(&store, &b, 1, 7).unwrap();
    assert_eq!(
        kinds(&store),
        vec!["request", "request", "sent", "sent", "response", "response"]
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
            owed: true,
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
            owed: false,
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

    assert!(matches!(res, Err(DispatchError::NotOwed(_))));
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
            owed: true,
        },
        || async { Ok(()) },
        |_: &()| "unreachable".into(),
    )
    .await;
    assert!(matches!(res, Err(DispatchError::NoPort("ghost"))));
}

#[tokio::test]
async fn dispatching_twice_does_not_double_the_facts() {
    // 判据 6：幂等键 `(port, id, attempt)`（`failed` 是逻辑 id）。重复派发不重落。
    let store = boot();
    let call = Call::new("turn", "turn-1-r0", 1);
    announce(&store, &call, 1, 7).unwrap();
    for _ in 0..2 {
        let out: Result<(), DispatchError<()>> = dispatch(
            &store,
            PORTS,
            Dispatch {
                call: &call,
                turn: 1,
                produced_by: 7,
                owed: true,
            },
            || async { Ok(()) },
            |_: &()| "unreachable".into(),
        )
        .await;
        out.expect("第二次派发只该撞幂等键");
    }
    assert_eq!(kinds(&store), vec!["request", "sent", "response"]);
}

// ── 租约 ────────────────────────────────────────────────────────────────

/// 一条只发未答的派发事实（`sent` 之后进程死掉的样子）。
fn stranded(store: &EventStore, id: &str, attempt: u64, lease_ms: u64, cap: u64) {
    store
        .append(
            Event::pending(
                format!("sent-turn-{id}-{attempt}"),
                crate::symbio_core::EVENT_SENT,
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
            })),
        )
        .unwrap();
}

#[test]
fn the_lease_kills_a_turn_call_that_nobody_awaits() {
    // 判据 7（判死一侧）：`turn` 的 `max_attempts = 1` ⇒ 到期即判死。
    // 重试只会留下一条永不派发的 `request`——等它的 future 随进程一起没了。
    let store = boot();
    stranded(&store, "turn-9-r0", 1, 1_000, 1);
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
    // 判据 7（重试一侧）：有活着的等待方的端口（`max_attempts > 1`）重试，
    // 落 `retry{attempt}` + `request{attempt+1}`；旧尝试由 `retry` 收清。
    //
    // 端口声明**从真表来**（`loop_rows::PORTS`），只改 `max_attempts` 这一格：
    // 写一条独立的合成 `PortDecl` 时，`port` / `before` / `lease_ms` 与真表各长
    // 一套，真表漂了用例照样绿。这条用例证明的是**分支形状**——生产里它不可达，
    // 因为重试通道未启用（T-252 出口 ②，见下一条用例）。
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
    stranded(&store, "turn-9-r0", 1, turn.lease_ms, turn.max_attempts);
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
    assert_eq!(kinds(&store), vec!["sent", "retry", "request"]);
    assert_eq!(payload_of(&store, "retry")["attempt"], json!(1));
    assert_eq!(payload_of(&store, "request")["attempt"], json!(2));
}

#[test]
fn no_port_in_the_real_table_retries_today() {
    // 上面那条用例改写的只有 `max_attempts` 一格，这里钉住**真表此刻的取值**：
    // 三端口全为 1 ⇒ 生产里 `reap` 的重试分支不可达，到顶即判死（PL-016 §2.3）。
    // 这是「重试通道未启用」的机制侧一半（条文的另一半由 PTS-006 钉住）。抬上限
    // 不是顺手能做的事：行层债键是 `(port, id)`，上一 attempt 的出口格已经把那行
    // 关掉 ⇒ 重试出来的 `request` 不欠任何一行，没有执行体会派它（T-252）。
    // 这一条转红 = 有人抬了上限，那时重试出来的那笔必须已经有行会选中它。
    let caps: Vec<(&str, u64)> = PORTS.iter().map(|p| (p.port, p.max_attempts)).collect();
    assert!(
        caps.iter().all(|(_, n)| *n == 1),
        "有端口抬了 `max_attempts`：{caps:?}（先确认重试出来的 `request` 有执行体会派它）"
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

// ── 内部请求也走端口（T-238）─────────────────────────────────────────────

#[tokio::test]
async fn an_internal_request_is_a_full_model_round_trip() {
    // PL-017 判据 2 的全量口径：编排层替别人打的那次模型调用也要三格，
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

// ── 压缩摘要也走端口（T-247）─────────────────────────────────────────────

#[tokio::test]
async fn a_compression_summary_is_accounted_as_a_model_round_trip() {
    // 压缩摘要是一次**内部**模型往返（ADR-122 决策 2）：同样三格，同样带身份。
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
    // 但那次往返**发生过**——账上必须有它（ADR-122 决策 5）。
    let failed = facts
        .iter()
        .find(|e| e.kind == "failed")
        .expect("失败的摘要往返必须留下 `failed` 格");
    assert_eq!(failed.payload["via"], json!("compress/summary"));
    assert_eq!(failed.payload["id"], json!("model-3-d9-z0-1"));
}
