//! 轮首检索面的判据：owner 序合并、`degraded` 取任一、空是合法结局（DE-026 D-264）。

use super::*;

fn hit(seq: u64, content: &str, tag: &str) -> Retrieved {
    Retrieved {
        seq,
        content: content.to_string(),
        tag: tag.to_string(),
    }
}

/// 两条只为测试存在的检索者：owner 字典序决定合并次序，与提交序无关。
struct Alpha;

impl Retriever for Alpha {
    fn owner(&self) -> &'static str {
        "tests/alpha"
    }

    fn retrieve(&self, at: &RetrievalRequest<'_>) -> Option<Retrieval> {
        // 读到的只是两个值：切片给条数、发言决定有没有语义维——都交不回句柄（D-252 的形状）。
        // 返回的次序**就是包内判据的结论**：宿主不再排第二遍。
        let _used_query = at.query.is_some();
        Some(Retrieval {
            degraded: false,
            entries: vec![hit(1, "甲先", "判断"), hit(2, "甲后", "经验")],
        })
    }
}

struct BetaDegraded;

impl Retriever for BetaDegraded {
    fn owner(&self) -> &'static str {
        "tests/beta"
    }

    fn retrieve(&self, _at: &RetrievalRequest<'_>) -> Option<Retrieval> {
        Some(Retrieval {
            degraded: true,
            entries: vec![hit(9, "乙", "经验")],
        })
    }
}

// 判据用**进程内**的提交：本 crate 的单测里没有别的插件，`inventory` 里只有这几条。
crate::submit_retriever!(BetaDegraded);
crate::submit_retriever!(Alpha);

fn at<'a>(events: &'a [Event], query: Option<&'a str>) -> RetrievalRequest<'a> {
    RetrievalRequest { events, query }
}

/// 检索者按 owner 字典序收集，与提交序无关（装配次序不构成判据）。
#[test]
fn retrievers_are_ordered_by_owner() {
    let owners: Vec<&str> = retrievers().iter().map(|r| r.owner()).collect();
    assert_eq!(owners, ["tests/alpha", "tests/beta"]);
}

/// 合并的两条机械规则：条目按 owner 序拼接（包内次序保留、宿主不排第二遍），
/// `degraded` 取任一——任何一副折叠没扫完，这一轮就必须标降级。
#[test]
fn merge_concatenates_in_owner_order_and_or_degraded() {
    let merged = retrieve_all(&at(&[], Some("问句"))).expect("有读数");
    assert!(merged.degraded, "tests/beta 没扫完 ⇒ 整轮标降级");
    let seqs: Vec<u64> = merged.entries.iter().map(|e| e.seq).collect();
    assert_eq!(
        seqs,
        [1, 2, 9],
        "owner 序拼接：alpha 的两条（包内次序原样保留）在前，beta 的一条在后"
    );
    let tags: Vec<&str> = merged.entries.iter().map(|e| e.tag.as_str()).collect();
    assert_eq!(tags, ["判断", "经验", "经验"]);
}

/// 缺省实现是 `None`：不检索的能力不必实现 `retrieve`（空是合法结局）。
#[test]
fn default_retrieval_is_none() {
    struct Silent;
    impl Retriever for Silent {
        fn owner(&self) -> &'static str {
            "tests/silent"
        }
    }
    assert!(Silent.retrieve(&at(&[], None)).is_none());
}

/// 全部检索者为空 ⇒ 整轮没有读数（`None`，与「本轮没召回」同一个形状）；
/// 有读数但一条没有，不合并成空壳。
#[test]
fn no_entries_at_all_yields_none() {
    struct Empty;
    impl Retriever for Empty {
        fn owner(&self) -> &'static str {
            "tests/empty"
        }
        fn retrieve(&self, at: &RetrievalRequest<'_>) -> Option<Retrieval> {
            // 没有新发言、切片里也没有活记忆 ⇒ 交空。
            at.query.is_none().then_some(Retrieval {
                degraded: false,
                entries: Vec::new(),
            })
        }
    }
    // 只验这条实现自身的形状：`retrieve_all` 走的是进程内全体提交者，
    // 上面两条测试夹具必有产出，空集路径在能力包的随迁用例里逐字钉（D-264 第 5 条）。
    assert!(Empty
        .retrieve(&at(&[], None))
        .is_some_and(|r| r.entries.is_empty()));
    assert!(Empty.retrieve(&at(&[], Some("有发言"))).is_none());
}
