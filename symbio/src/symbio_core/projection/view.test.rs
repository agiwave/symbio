//! `View` 自测 —— 平凡值可区分与 `map` 不丢标记。

use super::*;

#[test]
fn new_is_not_trivial() {
    let v = View::new(42);
    assert_eq!(v.value, 42);
    assert!(!v.trivial);
}

#[test]
fn trivial_is_marked() {
    let v = View::<u32>::trivial(0);
    assert!(v.trivial, "平凡值必须可区分");
}

#[test]
fn map_preserves_trivial_flag() {
    let v = View::trivial(vec![1, 2, 3]);
    let m = v.map(|xs| xs.len());
    assert_eq!(m.value, 3);
    assert!(m.trivial, "map 不得丢失平凡值标记");
}
