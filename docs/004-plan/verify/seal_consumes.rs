//! # 注册口的封闭性：`seal` 消耗 `Builder`，会话只持 `Arc<Frozen>`
//!
//! 判据来源：PL-016 §3.3「三个时点」——声明在装载期进表，装载完就是**只读的表**；
//! 「运行期不能注册」不能是一条靠人自觉的纪律，它必须写在类型上。
//! [T-243] 验收口径 2 要的就是这一格的正反两面证据。
//!
//! ## 它证明什么
//!
//! 正面（本文件正常档跑的断言）：
//!   1. 注册面的**全部**方法就是那几个口（真实面是 `fact / view / actor / port / row /
//!      seal`，这里以 `fact / row / seal` 三格建模——形状同源，少两格不改变判据）；
//!   2. 装载有任何缺口 ⇒ `Err(Vec<Gap>)`，且**没有表产出**：`seal(self)` 把 `self`
//!      吃掉了，失败分支里那两张 `Vec` 随 `self` 一起 drop——「半张表继续跑」在
//!      这台机器上写不出来；
//!   3. 会话手里那份表是 `Arc<Frozen>`，`Frozen` 只有 `&self` 方法 ⇒ 连 `&mut Frozen`
//!      都拿不到（多会话共享下 `Arc::get_mut` 恒 `None`）。
//!
//! 反面（`should_not_compile` feature 里，必须编译失败）：
//!   (a) `seal()` 之后再注册一格 ⇒ use-after-move（E0382）；
//!   (b) 经会话持有的表调一个注册方法（`frozen.row(..)`）⇒ 没有这个方法（E0599）；
//!   (c) 即便独占拿到 `&mut Frozen`，直写表字段也不可达 ⇒ 字段私有（E0616）。
//!
//! ## 一条刻意保留的边界（**编译得过**，所以它不是本文件的断言对象）
//!
//! `SIDE_REGISTRY` 是一个绕开 `Builder` 的进程级注册通道。类型面**挡不住**它——
//! 一级强制只守「句柄进不来」，守不住「有人在别处自己开一张表」。那条账必须由二级
//! 强制结：`scripts/port-surface-audit.mjs` 与「表里零代码」（PL-016 §6 判据 2）的
//! 静态扫描。把它写在这里而不是删掉，是为了标出本文件的证明**止于何处**。
//!
//! 编译运行：rustc --edition 2021 seal_consumes.rs -o sc && ./sc
//!
//! 反向验证（(a)(b)(c) 应当让编译以非 0 退出）：
//!   rustc --edition 2021 --check-cfg 'cfg(feature)' --cfg 'feature="should_not_compile"' \
//!         seal_consumes.rs -o should_fail.exe
//! 注意：不要用 `-o /dev/null` —— Windows MSVC 链接器需要真实文件路径。

use std::sync::{Arc, Mutex};

/// 声明层——与 `symbio_core::decl` 同形：**类型 `pub`，字段私有**，
/// 注册口只有 `&mut self` 那几个，装载出口 `seal` 按值消耗自身。
///
/// 反例 (c) 依赖这一层嵌套：子模块能读父模块的私有字段，同级模块不能——
/// `must_fail` 因此必须与 `decl` 同级，而不是写在 `decl` 里面。
pub mod decl {
    use std::sync::Arc;

    /// 一条事实声明（这里只带判据要用的两格：名字 + 载荷字段）。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct FactDecl {
        pub name: &'static str,
        pub fields: &'static [&'static str],
    }

    /// 一行声明。`owes` 是它开账的**源事实名**——装载期必须在事实声明里找得到它
    /// （PL-016 §3.4 第 3 条「字段都在声明里」的同一族检查，这里取其事实名一格）。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RowDecl {
        pub name: &'static str,
        pub owes: &'static str,
    }

    /// 装载缺口。与 `ring::Gap` 同形：**报告型判据（§3.4 第 4 条）以外全部走这一个 `Err`**。
    #[derive(Debug, PartialEq, Eq)]
    pub enum Gap {
        UnknownSource { row: &'static str, source: &'static str },
    }

    /// 注册通道的**全部**可达面：两个 `&mut self` 口 + 一个消耗自身的 `seal`。
    ///
    /// 字段私有且类型不出模块 ⇒ 表除了经这几张口进，没有第二条路（判据 2「表里零代码」
    /// 在这一层的形状前提）。
    pub struct Builder {
        facts: Vec<FactDecl>,
        rows: Vec<RowDecl>,
    }

    /// 冻结的声明表：只有 `&self` 方法，没有内部可变性。
    pub struct Frozen {
        facts: Vec<FactDecl>,
        rows: Vec<RowDecl>,
    }

    impl Builder {
        pub fn new() -> Self {
            Self {
                facts: Vec::new(),
                rows: Vec::new(),
            }
        }

        pub fn fact(&mut self, f: FactDecl) {
            self.facts.push(f);
        }

        pub fn row(&mut self, r: RowDecl) {
            self.rows.push(r);
        }

        /// 装载的**唯一出口**：`self` 按值消耗。
        ///
        /// 「seal 之后再注册一格」因此不是一条被禁止的写法，而是一个编译不过的表达式
        /// （见 `must_fail` (a)）。缺口先于表返回：失败分支里 `self` 的两张 `Vec` 一起
        /// drop，调用方手里**没有任何表**可以带着跑。
        pub fn seal(self) -> Result<Arc<Frozen>, Vec<Gap>> {
            let mut gaps = Vec::new();
            for r in &self.rows {
                if !self.facts.iter().any(|f| f.name == r.owes) {
                    gaps.push(Gap::UnknownSource {
                        row: r.name,
                        source: r.owes,
                    });
                }
            }
            if !gaps.is_empty() {
                return Err(gaps);
            }
            Ok(Arc::new(Frozen {
                facts: self.facts,
                rows: self.rows,
            }))
        }
    }

    impl Frozen {
        pub fn fact(&self, name: &str) -> Option<FactDecl> {
            self.facts.iter().copied().find(|f| f.name == name)
        }

        pub fn rows(&self) -> &[RowDecl] {
            &self.rows
        }
    }

    /// 一个会话——它从构造那一刻起手里就只有一张 `Arc<Frozen>`（T-243 的「按会话注入」
    /// 与进程级记忆化只在开幕时机上有分歧，这一格类型两边相同）。
    pub struct Session {
        table: Arc<Frozen>,
    }

    impl Session {
        pub fn new(table: Arc<Frozen>) -> Self {
            Self { table }
        }

        /// 读侧唯一的出口：交出去的是 `&Arc<Frozen>`，不是可变句柄。
        pub fn table(&self) -> &Arc<Frozen> {
            &self.table
        }
    }
}

use decl::{Builder, FactDecl, Frozen, Gap, RowDecl, Session};

/// **它编译得过，而且本文件不判它红**——一个绕开 `Builder` 的进程级注册通道。
///
/// 一级强制（类型）守的是「句柄进不来」，守不住「有人在别处自己开一张表」。
/// 这条账由二级强制结（port-surface 审计 + 「表里零代码」扫描）。
static SIDE_REGISTRY: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn main() {
    println!("═══ 注册口的封闭性（PL-016 §3.3 · T-243 口径 2）═══");

    // ── 1. 方法面就是那几个口，一个都不能变宽 ──
    //
    // 与 `decl/mod.test.rs::the_six_registration_ports_are_the_whole_surface` 同形：
    // 每个口按**它应有的函数指针类型**绑一遍。签名一变宽（比如 `seal` 改成 `&mut self`），
    // 这一格当场编译不过。
    let _fact: fn(&mut Builder, FactDecl) = Builder::fact;
    let _row: fn(&mut Builder, RowDecl) = Builder::row;
    let _seal: fn(Builder) -> Result<Arc<Frozen>, Vec<Gap>> = Builder::seal;
    println!("  口面：fact(&mut self) · row(&mut self) · seal(self)（按值消耗）");

    // ── 2. 好表冻得出，读得到 ──
    let mut ok = Builder::new();
    ok.fact(FactDecl {
        name: "user.message",
        fields: &["text"],
    });
    ok.fact(FactDecl {
        name: "request",
        fields: &["port", "id"],
    });
    ok.row(RowDecl {
        name: "dispatch-model",
        owes: "user.message",
    });
    ok.row(RowDecl {
        name: "courier",
        owes: "request",
    });
    let frozen = ok.seal().expect("好表：装载不该有缺口");
    assert_eq!(frozen.rows().len(), 2, "冻出的表就是灌进去的那两行");
    assert_eq!(
        frozen.fact("request").map(|f| f.fields),
        Some(&["port", "id"][..]),
        "读表经 `&self` 访问器"
    );
    assert!(frozen.fact("typo.message").is_none(), "未声明的名字读不到（拼错即无）");
    println!("  正面：2 行 / 2 事实冻出，读侧只有 &self");

    // ── 3. 缺口先于表：一次报全，且不产出任何表 ──
    let mut bad = Builder::new();
    bad.fact(FactDecl {
        name: "user.message",
        fields: &["text"],
    });
    bad.row(RowDecl {
        name: "row-a",
        owes: "user.message",
    });
    bad.row(RowDecl {
        name: "row-b",
        owes: "typo.message",
    });
    bad.row(RowDecl {
        name: "row-c",
        owes: "also.a.typo",
    });
    let gaps = bad.seal().err().expect("两行引用未声明的事实，必须装载失败");
    assert_eq!(
        gaps,
        vec![
            Gap::UnknownSource {
                row: "row-b",
                source: "typo.message"
            },
            Gap::UnknownSource {
                row: "row-c",
                source: "also.a.typo"
            },
        ],
        "缺口一次报全（不是撞到第一个就返回），顺序随行表"
    );
    // 到这里 `bad` 已被消耗：既没有表，也没有剩下的注册口——「部分成功」无处可去。
    println!("  正面：装载失败带 {} 条缺口，表未产出（`Builder` 随 seal 消耗）", gaps.len());

    // ── 4. 会话手里的表连 &mut 都拿不到 ──
    let mut b3 = Builder::new();
    b3.fact(FactDecl {
        name: "user.message",
        fields: &["text"],
    });
    b3.row(RowDecl {
        name: "execute-turn",
        owes: "user.message",
    });
    let shared = b3.seal().expect("单行表：装载不该有缺口");
    let sessions: Vec<Session> = (0..3)
        .map(|_| Session::new(Arc::clone(&shared)))
        .collect();
    assert_eq!(Arc::strong_count(&shared), 4, "1 份原表 + 3 场会话");
    // `Arc::get_mut` 是拿到 `&mut Frozen` 的**唯一**类型面通道；共享之下恒 `None`。
    // 会话侧同理：想把表改成可写的，第一步就迈不出去。
    let mut handle = Arc::clone(&shared);
    assert!(
        Arc::get_mut(&mut handle).is_none(),
        "多会话共享下拿不到 &mut Frozen ⇒ 注册口在运行期不可达"
    );
    for s in &sessions {
        assert_eq!(s.table().rows().len(), 1, "每一场会话读到的都是同一张冻结表");
    }
    println!("  正面：3 场会话共享一张表，&mut 通道恒不可达（strong_count=4）");

    // ── 5. 边界：类型面看不见侧表 ──
    {
        let mut side = SIDE_REGISTRY.lock().unwrap();
        side.push("a-row-that-never-went-through-the-builder");
        assert_eq!(side.len(), 1, "侧表自己写得动——这恰是类型挡不住的");
    }
    assert_eq!(
        frozen.rows().len(),
        2,
        "侧表进不了 `Frozen`：它不是注册通道，也就不会被装载读到"
    );
    println!("  边界：绕开 Builder 的进程级侧表编译得过 ⇒ 归二级强制（port-surface 审计）");

    println!("\n═══ 编译期强制的反向用例 ═══");
    println!("  本文件的 `should_not_compile` feature 里有三段代码：");
    println!("    (a) `seal()` 之后再注册一格 → 必须编译失败（E0382 use-after-move）");
    println!("    (b) 经会话持有的表调注册方法 `table().row(..)` → 必须编译失败（E0599 无此方法）");
    println!("    (c) 独占取到 `&mut Frozen` 后直写私有字段 `f.rows.push(..)` → 必须编译失败（E0616 字段私有）");
    println!("  验证命令（应当以非 0 退出）：");
    println!("    rustc --edition 2021 --check-cfg 'cfg(feature)' \\");
    println!("          --cfg 'feature=\"should_not_compile\"' seal_consumes.rs -o should_fail.exe");

    println!("\n✅ 全部断言通过（5 组正面 + 3 条编译期反向用例）");
}

// ───────────────────────── 编译期反向用例 ─────────────────────────
//
// 下面三段**故意编译不过**。若它们能编译，上面断言的封闭性就是假的：
// (a) 成立意味着 seal 不消耗注册通道，(b) 成立意味着表可以自己长行，
// (c) 成立意味着「只读」只是一句注释。

#[cfg(feature = "should_not_compile")]
mod must_fail {
    use super::decl::{Builder, FactDecl, RowDecl, Session};
    use std::sync::Arc;

    /// (a) 装载完了又要注册一格——E0382：`b` 已在 `seal()` 处被移走。
    pub fn register_after_seal() {
        let mut b = Builder::new();
        b.fact(FactDecl {
            name: "user.message",
            fields: &["text"],
        });
        let _frozen = b.seal();
        b.row(RowDecl {
            name: "late-row",
            owes: "user.message",
        });
    }

    /// (b) 经会话持有的表注册——E0599：`Frozen` 没有 `row` 方法（读侧只有 `&self`）。
    pub fn register_through_session(s: &Session) {
        s.table().row(RowDecl {
            name: "late-row",
            owes: "user.message",
        });
    }

    /// (c) 拿到 `&mut Frozen` 后直写表字段——E0616：`rows` 是 `decl` 的私有字段。
    ///
    /// 这一格的前提（`strong_count == 1`）在正常档跑不到：会话表恒为共享。它要判的是
    /// 「就算拿到可变引用，字段也不可达」——只读不是靠约定，也不靠拿不到引用。
    pub fn write_the_private_table() {
        let mut b = Builder::new();
        b.fact(FactDecl {
            name: "user.message",
            fields: &["text"],
        });
        let mut frozen = b.seal().expect("单事实空表");
        let inner: &mut super::decl::Frozen = Arc::get_mut(&mut frozen).expect("独占，取到 &mut");
        inner.rows.push(RowDecl {
            name: "late-row",
            owes: "user.message",
        });
    }
}
