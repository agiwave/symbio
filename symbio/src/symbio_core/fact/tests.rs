//! `fact` 域自测 —— 信封与网格的**可执行判据**。

use super::*;

/// 网格全集：`wire()` 与 `ALL` 必须一一对应（漏一个 = 机制表与代码分叉）。
#[test]
fn wire_and_all_agree() {
    assert_eq!(FactKind::ALL.len(), 24, "网格取值数应与 ALL 一致");
    for (i, k) in FactKind::ALL.iter().enumerate() {
        // 每个取值的 wire 形状必须是 `<实体>.<动词>`
        let w = k.wire();
        assert!(
            w.split_once('.').is_some(),
            "wire 形状应为 `<实体>.<动词>`：{w}"
        );
        assert!(!k.entity().is_empty(), "实体名不应为空：{w}");
        // ALL 内不得重复
        for other in &FactKind::ALL[i + 1..] {
            assert_ne!(k, other, "ALL 内有重复取值");
        }
    }
}

/// 反向用例：wire 词形不得重复（两个不同类型映射到同一线上词形 = 审计失效）。
#[test]
fn wire_is_unique() {
    for (i, a) in FactKind::ALL.iter().enumerate() {
        for b in &FactKind::ALL[i + 1..] {
            assert_ne!(a.wire(), b.wire(), "wire 词形重复：{}", a.wire());
        }
    }
}

/// **线上词形只有一个来源**：序列化输出必须**逐字等于** `wire()`。
///
/// 这条存在的原因是它曾经**不成立**：`FactKind` 一度 `derive` + `snake_case`，
/// 于是线上载荷是 `memory_encoded`，而 `wire()` 是 `memory.encoded`——同一事实
/// 两种写法。单测两侧都用 `wire()` 比较，因此**测不出来**；只有跨进程的真实
/// 载荷（e2e）才暴露。故在此钉死：序列化 = `wire()`，反序列化 = 按 `wire()` 反查。
#[test]
fn serde_uses_wire_verbatim() {
    for k in FactKind::ALL {
        // 序列化后的 JSON 字符串字面量 = wire 词形本身（含点号）
        let json = serde_json::to_string(k).expect("序列化应成功");
        assert_eq!(
            json,
            format!("\"{}\"", k.wire()),
            "序列化必须与 wire() 逐字一致"
        );
        // 往返：按 wire 反查回同一个取值
        let back: FactKind = serde_json::from_str(&json).expect("反序列化应成功");
        assert_eq!(&back, k, "往返应回到同一取值");
    }
}

/// 反序列化**不静默兜底**：认不出的词形必须报错，不能变成看似正常的错值。
#[test]
fn unknown_wire_is_rejected() {
    // snake_case 那套旧词形现在必须**认不出**（否则就是双词形复燃）
    let err = serde_json::from_str::<FactKind>(r#""memory_encoded""#);
    assert!(err.is_err(), "旧 snake_case 词形不应被接受");
    let err2 = serde_json::from_str::<FactKind>(r#""nope.nope""#);
    assert!(err2.is_err(), "未知词形应报错");
}

/// 溯源方向：指向更早的 seq 合法，指向自身 / 更晚不合法。
#[test]
fn provenance_direction() {
    let mk = |seq: u64, caused_by: Option<u64>| Fact {
        seq,
        kind: FactKind::TurnUserMessage,
        principal: FactPrincipal::new("u"),
        caused_by,
        at_ms: 0,
        payload: serde_json::Value::Null,
    };
    assert!(mk(5, None).provenance_points_back(), "无前驱恒合法");
    assert!(mk(5, Some(0)).provenance_points_back(), "0 = 无前驱，合法");
    assert!(mk(5, Some(3)).provenance_points_back(), "指向更早，合法");
    assert!(!mk(5, Some(5)).provenance_points_back(), "指向自身不合法");
    assert!(!mk(5, Some(9)).provenance_points_back(), "指向更晚不合法");
}

/// I2：断言类事实必须有溯源；非断言类不要求。
#[test]
fn assertion_requires_provenance() {
    assert!(FactKind::TaskVerified.is_assertion());
    assert!(FactKind::ArtifactAdded.is_assertion());
    assert!(!FactKind::TurnUserMessage.is_assertion());
    assert!(!FactKind::TurnAssistantFinal.is_assertion());

    let assertion = Fact {
        seq: 2,
        kind: FactKind::TaskVerified,
        principal: FactPrincipal::new("p"),
        caused_by: None,
        at_ms: 0,
        payload: serde_json::Value::Null,
    };
    assert!(!assertion.has_provenance(), "断言类无溯源应判违规");

    let ok = Fact {
        caused_by: Some(1),
        ..assertion.clone()
    };
    assert!(ok.has_provenance(), "断言类带溯源应通过");

    let plain = Fact {
        kind: FactKind::TurnUserMessage,
        ..assertion
    };
    assert!(plain.has_provenance(), "非断言类不要求溯源");
}

/// 反向用例：每个实体名必须落在已知 7 实体里（拼错实体 = 归不进网格）。
#[test]
fn entity_is_known() {
    let known = [
        "turn",
        "task",
        "artifact",
        "memory",
        "commitment",
        "conation",
        "system",
    ];
    for k in FactKind::ALL {
        assert!(
            known.contains(&k.entity()),
            "未知实体 {}（来自 {}）",
            k.entity(),
            k.wire()
        );
    }
}
