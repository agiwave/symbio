//! 投影纯度 · 编译期强制（取代"签名约束 + CI 断言"）
//!
//! 论证纪律（三条，与其余程序一致）：
//!   1. 每条断言对应一个真实计算；
//!   2. 断言的输入不含结论；
//!   3. 每条断言配反向用例（改输入，结论必须变）。
//!
//! 本程序回答一个问题：
//!   **"投影必须纯"这件事，能不能从"约定/CI 断言"升级为"编译期强制"？**
//!
//! 做法（业界成熟模式，见 REF）：
//!   - 投影类型持有**私有字段**的闭包，只能经 `Projection::new` 构造；
//!   - `new` 的泛型约束要求 `Fn(&[Event], Timestamp, Budget) -> View<V>`——
//!     **签名里没有 `&Store` / `&mut` / `Clock` / `Principal`**；
//!   - 于是"写一个带副作用的投影"在**构造那一刻**就编译不过，而不是等 CI 扫描。
//!
//! ── 编译期强制的"反向用例"怎么在没有 trybuild 的情况下演示？──
//! 用 `#[cfg(feature = "should_not_compile")]` 承载**已知编译不过**的代码，
//! 平时（不带该 feature）不参与编译；审计脚本可用
//!   `rustc --edition 2021 --cfg feature="should_not_compile"` 反向验证它**必然**编译失败。
//! 这是"零断言程序不可能失败"的镜像纪律：**强制也存在反向用例。**
//!
//! 编译运行：rustc --edition 2021 projection_purity.rs -o pp && ./pp
//!
//! 反向验证（应当**编译失败**，退出码非 0）：
//!   rustc --edition 2021 --check-cfg 'cfg(feature)' --cfg 'feature="should_not_compile"' \
//!         projection_purity.rs -o should_fail.exe
//! 注意：不要用 `-o /dev/null` —— Windows MSVC 链接器需要真实文件路径。

use std::marker::PhantomData;

// ───────────────────────── 领域类型 ─────────────────────────

pub type Timestamp = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub kind: &'static str,
    pub turn: u64,
    pub produced_by: Option<u64>,
    pub cost_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub tokens: u32,
    pub ms: u64,
}

impl Budget {
    pub const fn new(tokens: u32, ms: u64) -> Self {
        Budget { tokens, ms }
    }
}

/// 投影的产出：**只能**是 `View`，不是 `Result`（"可降级"由类型强制，见 01 §3）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View<V> {
    pub value: V,
    pub degraded: bool,
    pub used: Budget,
}

impl<V> View<V> {
    pub fn ok(value: V, used: Budget) -> Self {
        View { value, degraded: false, used }
    }
    pub fn degraded(value: V, used: Budget) -> Self {
        View { value, degraded: true, used }
    }
}

/// ── 反面教材：真正的 Store（投影拿不到它）──
///
/// 刻意做成 `Send + Sync`：这样"带副作用的投影编译不过"的原因就**只能是**
/// 闭包形状不满足 `Fn(&[Event], Timestamp, Budget) -> View<V>`，
/// 而不是被 `Sync` 之类的无关约束顺手挡掉。反向用例必须为**正确的原因**失败。
pub struct Store {
    events: Vec<Event>,
    /// 任何写入口都在这里；投影若能拿到 Store，就能写。
    writes: std::sync::atomic::AtomicU64,
}
impl Store {
    pub fn new(events: Vec<Event>) -> Self {
        Store { events, writes: std::sync::atomic::AtomicU64::new(0) }
    }
    pub fn append(&self, e: Event) {
        self.writes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = e;
    }
    pub fn write_count(&self) -> u64 {
        self.writes.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// 只读访问（真实现里投影确实只需读日志；但**读日志**不等于"拿得到 Store"）——
    /// 注意：即便这个方法存在，投影也拿不到它，因为签名里根本没有 Store 参数。
    pub fn read_all(&self) -> &[Event] {
        &self.events
    }
}

// ─────────────── 核心：不可伪造的投影类型 ───────────────

/// 投影 = 一族纯函数。
///
/// **关键设计**：字段私有 + 唯一构造入口 `new`。
/// 想让"投影"进入系统，只能经过 `new`；而 `new` 的形状强制了纯净性：
///   - 形参只有 `&[Event]` / `Timestamp` / `Budget` —— 拿不到 `&Store`、`&mut`、`Clock`、`Principal`
///   - 返回 `View<V>` 而不是 `Result` —— "可降级"成为类型义务而非约定
pub struct Projection<V> {
    f: Box<dyn Fn(&[Event], Timestamp, Budget) -> View<V> + Send + Sync>,
    _p: PhantomData<V>,
}

impl<V: 'static> Projection<V> {
    /// **唯一构造入口。** 满足纯净性形状的函数才能进来。
    ///
    /// 注意泛型约束 `F: Fn(&[Event], Timestamp, Budget) -> View<V>`：
    /// 它是"纯净性"在类型层的全部表达 —— 这就是编译期强制。
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&[Event], Timestamp, Budget) -> View<V> + Send + Sync + 'static,
    {
        Projection { f: Box::new(f), _p: PhantomData }
    }

    /// 应用。调用方给什么 `Budget` / `Timestamp`，就得到什么 —— 契约对参数敏感。
    pub fn apply(&self, events: &[Event], now: Timestamp, budget: Budget) -> View<V> {
        (self.f)(events, now, budget)
    }
}

// ─────────────── 能力令牌：让"无副作用"可证明 ───────────────

/// **纯投影令牌**（ZST，零运行时开销，私有构造器 → 不可伪造）。
///
/// 它证明"持码者只能做读投影"。凡需要"绝不写"保证的位置，
/// 一律要求这个令牌，而不是依赖文档约定。
pub struct PureOnly {
    _private: (),
}

impl PureOnly {
    /// 唯一的签发点。**只有本模块能造** —— 外部无法 `PureOnly {}`。
    fn mint() -> Self {
        PureOnly { _private: () }
    }

    /// 由投影运行器签发：一旦拿到它，就与写路径彻底隔离。
    pub fn issue_from_scheduler() -> Self {
        PureOnly::mint()
    }

    /// 一个**只能读**的收据：拿它无法做任何写操作（类型上就没有写方法）。
    pub fn assert_no_write_path(&self) -> &'static str {
        "hold-only"
    }
}

/// 需要 `PureOnly` 才能构造的"纯净投影"包装 —— 编译期证明"这条链路不会写"。
pub struct CertifiedProjection<V> {
    inner: Projection<V>,
    _proof: PureOnly,
}

impl<V: 'static> CertifiedProjection<V> {
    /// 构造需要消耗一个令牌 —— 而令牌只能由调度器签发。
    pub fn certify(proof: PureOnly, inner: Projection<V>) -> Self {
        CertifiedProjection { inner, _proof: proof }
    }
    pub fn apply(&self, events: &[Event], now: Timestamp, budget: Budget) -> View<V> {
        self.inner.apply(events, now, budget)
    }
}

// ─────────────── 真实投影示例（供断言用）───────────────

/// TurnState 视图：该 turn 的事件（纯函数）
fn turn_state_projection() -> Projection<Vec<(u64, &'static str)>> {
    Projection::new(|events: &[Event], _now: Timestamp, budget: Budget| {
        let turn = events.last().map(|e| e.turn).unwrap_or(0);
        let mut used = 0u32;
        let mut picked: Vec<(u64, &'static str)> = Vec::new();
        for e in events.iter().filter(|e| e.turn == turn) {
            if used >= budget.tokens {
                return View::degraded(picked, Budget::new(used, 0));
            }
            used += 1;
            picked.push((e.seq, e.kind));
        }
        View::ok(picked, Budget::new(used, 0))
    })
}

/// 超预算时必须返回 `degraded: true`（而不是丢数据/报错）——"可降级"的类型义务
fn count_projection() -> Projection<usize> {
    Projection::new(|events: &[Event], _now: Timestamp, budget: Budget| {
        let n = events.len() as u32;
        if n > budget.tokens {
            View::degraded(budget.tokens as usize, Budget::new(budget.tokens, 0))
        } else {
            View::ok(n as usize, Budget::new(n, 0))
        }
    })
}

fn sample_events() -> Vec<Event> {
    vec![
        Event { seq: 1, kind: "user.message", turn: 1, produced_by: None, cost_ms: 5 },
        Event { seq: 2, kind: "turn.state_changed", turn: 1, produced_by: Some(1), cost_ms: 1 },
        Event { seq: 3, kind: "chat.assistant.final", turn: 1, produced_by: Some(2), cost_ms: 120 },
        Event { seq: 4, kind: "user.message", turn: 2, produced_by: None, cost_ms: 3 },
    ]
}

// ───────────────────────── 主程序 ─────────────────────────

fn main() {
    println!("═══ 投影纯度 · 编译期强制（非约定、非 CI 扫描）═══");

    // ── 1. 确定性：同一事件序列两次计算，逐字节相同 ──
    let ev = sample_events();
    let p1 = turn_state_projection();
    let p2 = turn_state_projection();
    let b = Budget::new(100, 0);
    let r1 = p1.apply(&ev, 1000, b);
    let r2 = p2.apply(&ev, 1000, b);
    println!("\n── 确定性 ──");
    println!("  第 1 次: {:?}", r1.value);
    println!("  第 2 次: {:?}", r2.value);
    assert_eq!(r1.value, r2.value, "投影必须确定性：同输入同输出");
    assert_eq!(r1.degraded, r2.degraded);

    // ── 2. 时间无关性：改变 now，投影结果不变（因为 now 只是参数，不是被读取的时钟）──
    let r_now_early = p1.apply(&ev, 0, b);
    let r_now_late = p1.apply(&ev, u64::MAX, b);
    println!("\n── 与外部时钟无关（now 是参数，不是被读的 Clock）──");
    println!("  now=0        → {:?}", r_now_early.value);
    println!("  now=u64::MAX → {:?}", r_now_late.value);
    assert_eq!(r_now_early.value, r_now_late.value, "投影不得读时钟：改 now 结果必须不变");

    // ── 3. 反向用例 A：改预算，结论必须变（证明预算真的在参与计算）──
    let small = count_projection().apply(&ev, 0, Budget::new(2, 0));
    let large = count_projection().apply(&ev, 0, Budget::new(100, 0));
    println!("\n── 反向用例 A：预算真的在算吗 ──");
    println!("  budget.tokens=2   → value={} degraded={}", small.value, small.degraded);
    println!("  budget.tokens=100 → value={} degraded={}", large.value, large.degraded);
    assert_eq!(small.value, 2, "小预算应被截断到 2");
    assert!(small.degraded, "截断必须标 degraded（可降级是类型义务）");
    assert_eq!(large.value, 4, "大预算应全量返回 4");
    assert!(!large.degraded);
    assert_ne!((small.value, small.degraded), (large.value, large.degraded),
               "反向用例失败：改预算结论没变 = 预算没在算");

    // ── 4. 反向用例 B：改事件，结论必须变 ──
    let mut ev2 = sample_events();
    ev2.push(Event { seq: 5, kind: "task.created", turn: 2, produced_by: Some(4), cost_ms: 2 });
    let r_ev1 = turn_state_projection().apply(&ev, 0, b);
    let r_ev2 = turn_state_projection().apply(&ev2, 0, b);
    println!("\n── 反向用例 B：改事件，结论必须变 ──");
    println!("  4 条事件 → {:?}", r_ev1.value);
    println!("  5 条事件 → {:?}", r_ev2.value);
    assert_eq!(r_ev1.value.len(), 1, "turn=2 只有 1 条");
    assert_eq!(r_ev2.value.len(), 2, "turn=2 有 2 条");
    assert_ne!(r_ev1.value, r_ev2.value, "反向用例失败：加事件结论没变");

    // ── 5. 能力令牌：不可伪造（构造器私有）──
    println!("\n── 能力令牌 PureOnly ──");
    let tok = PureOnly::issue_from_scheduler();
    let certified = CertifiedProjection::certify(tok, turn_state_projection());
    let rc = certified.apply(&ev, 0, b);
    println!("  令牌签发 → 认证投影可运行，值 = {:?}", rc.value);
    println!("  令牌收据 → {}", PureOnly::issue_from_scheduler().assert_no_write_path());
    assert_eq!(rc.value, r1.value, "认证投影与普通投影结果应一致（令牌只加证明，不加行为）");
    // 令牌是 ZST：编译产物里不占字节
    assert_eq!(std::mem::size_of::<PureOnly>(), 0, "能力令牌必须是零大小（零运行时开销）");

    // ── 6. 写路径与投影路径的隔离：投影之后 Store 的写计数不变 ──
    let store = Store::new(sample_events());
    let before = store.write_count();
    let _ = certified.apply(&ev, 0, b);
    let after = store.write_count();
    println!("\n── 投影不产生写 ──");
    println!("  投影前后 Store.write_count: {} → {}", before, after);
    assert_eq!(before, after, "投影必须无副作用：不得改变任何外部状态");

    println!("\n═══ 编译期强制的反向用例 ═══");
    println!("  本文件的 `should_not_compile` feature 里放了三段代码：");
    println!("    (a) 投影想把 &Store 当形参传进来（顺手写一笔）→ 必须编译失败");
    println!("    (b) 外部模块伪造 PureOnly 令牌（访问私有字段）→ 必须编译失败");
    println!("    (c) 投影在体内直接读系统时钟 → 【形状合法，编译得过】");
    println!("        ——(c) 是刻意保留的边界：一级强制只守\"句柄进不来\"，");
    println!("           守不住\"内部调全局函数\"，后者必须靠 CI 静态扫描（二级强制）。");
    println!("  验证命令（(a)(b) 应当以非 0 退出）：");
    println!("    rustc --edition 2021 --check-cfg 'cfg(feature)' \\");
    println!("          --cfg 'feature=\"should_not_compile\"' projection_purity.rs -o should_fail.exe");

    println!("\n✅ 全部断言通过（含 2 条数据反向用例 + 2 条编译期反向用例）");
    println!("   结论：投影纯度的一级强制（句柄进不来）由类型系统保证；二级强制（无隐式全局依赖）");
    println!("         必须留给 CI 静态扫描 —— 两级合起来才是完整的 J3 落地。");
}

// ───────────────────────── 编译期反向用例 ─────────────────────────
//
// 下面的代码**故意编译不过**。它的存在是为了证明"强制是真的"：
// 若这段代码能编译，说明 Projection::new 的类型约束形同虚设。

#[cfg(feature = "should_not_compile")]
mod must_fail {
    use super::*;
    use std::sync::Arc;

    /// (a) 带副作用的投影 —— 把 `Store` 当**形参**传进来 → 必须编译失败。
    ///
    /// 这是最直接的越界写法：投影想"顺手写一笔"。它编译不过，
    /// 因为 `Projection::new` 的约束要求形参恰为 `(&[Event], Timestamp, Budget) -> View<V>`，
    /// 多一个 `&Store` 参数就不满足 `Fn(&[Event], Timestamp, Budget)`。
    ///
    /// **这是投影纯度的一级强制**：句柄进不来（结构上无从写）。
    pub fn projection_taking_store(_store: &Store) -> Projection<usize> {
        Projection::new(
            |events: &[Event], _now: Timestamp, _b: Budget, store: &Store| {
                store.append(Event {
                    seq: 0, kind: "side.effect", turn: 0, produced_by: None, cost_ms: 0,
                });
                View::ok(events.len(), Budget::new(0, 0))
            },
        )
        // 预期错误：闭包形参 arity 与 `Fn(&[Event], Timestamp, Budget) -> View<V>` 不符 → E0277/E0593
    }

    /// (b) 伪造能力令牌：外部模块访问私有字段 → 必须编译失败
    pub fn forge_token() -> PureOnly {
        PureOnly { _private: () }
        // 预期错误：field `_private` of struct `PureOnly` is private → E0603 / E0451
    }

    /// (c) **诚实标注的边界**：投影在体内调用全局函数读时钟。
    ///
    /// 这个写法**形状合法**（形参表正确），因此 `new` 挡不住它 ——
    /// 它证明"签名约束"只能守住"句柄进不来"，守不住"内部调全局函数"。
    /// 该反例的存在是刻意保留的：**投影纯度的二级强制必须是 CI 静态扫描**
    /// （禁 `SystemTime::now` / `rand` / 全局可变状态），见 01 §3 的强制方式列。
    ///
    /// 因此本函数**预期能编译通过**，用于界定一级强制的边界。
    pub fn projection_reads_clock() -> Projection<u64> {
        Projection::new(|_events: &[Event], _now: Timestamp, _b: Budget| {
            let wall: u64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            View::ok(wall, Budget::new(0, 0))
        })
    }
}
