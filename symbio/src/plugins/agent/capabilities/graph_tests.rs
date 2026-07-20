//! graph.rs 单元测试
//!
//! 对应源文件: `graph.rs`

use super::*;

fn mock_cu(id: &str, rels: &[(&str, &[&str])]) -> CognitiveUnit {
    let mut cu = CognitiveUnit::new(id);
    for (rel, targets) in rels {
        cu.set(*rel, json!(targets));
    }
    cu
}

#[test]
fn build_adjacency_basic() {
    let units = vec![
        mock_cu("a", &[("causes", &["b", "c"])]),
        mock_cu("b", &[("depends", &["c"])]),
    ];
    let adj = build_adjacency(&units, &["causes".to_string(), "depends".to_string()]);
    assert_eq!(adj.get("a").unwrap().len(), 2);
    assert!(adj
        .get("b")
        .unwrap()
        .iter()
        .any(|(t, _)| t == "a" || t == "c"));
}

#[test]
fn build_adjacency_filters_relations() {
    let units = vec![mock_cu("a", &[("causes", &["b"]), ("opposite", &["c"])])];
    let adj = build_adjacency(&units, &["causes".to_string()]);
    assert!(adj.get("a").unwrap().iter().all(|(_, r)| r == "causes"));
}
