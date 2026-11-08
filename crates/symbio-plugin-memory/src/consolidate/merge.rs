//! 巩固合并的**纯计算内核**：分句 / 合并 + 保真度。
//!
//! ## 为什么是私有模块，不是共享库（D-387 Q3 普查）
//!
//! 「一个纯算法进共享库」的最小准入有四条，第一条是 **≥2 个不同 owner 的生产消费者**。
//! 全仓普查后，`merge_with_fidelity` 只有一个生产消费者——本包的巩固写口
//! （[`crate::consolidate`]），`split_sentences` 只被它自己调用。**只有一个消费者
//! ⇒ 一律私有，不许库化**：抽成 crate 只会把一个调用点变成一条依赖边，再请回一份
//! 「禁止第二份实现」的判据。
//!
//! 同族的 `truncate_chars` 不在这里：它有两个不同 owner 的生产消费者（memory 的情景
//! 编码、skill 的技能编译），留在 `symbio_sdk::text::merge_kernel`，不跟着搬。
//!
//! ## 与 verify 共有同一份源码
//!
//! `docs/003-design/verify/consolidation_fidelity.rs` 以 `#[path]` 包含本文件——它论证
//! 「质量约束（min_fidelity）必须先于形态约束（max_gen）生效」时，跑的是生产这条
//! 保真度算法本身，不是另写一份临摹件。临摹件会随生产漂移而静默失效：论证仍在绿，
//! 闸门已经换了算法。
//!
//! 因此本文件**只依赖 `std`、不得出现任何 `use`**——这是能被 standalone `rustc` 编译的
//! verify 程序 `#[path]` 包含的前提。本模块不知道任何调用方的存在，更不 import 事件 /
//! 存储：**它只算数，入格永远是写方的事**
//! （[DE-001 §9.3](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")：架构只提供位置，不提供算法）。
//!
//! **保真度的比较基准由调用方显式选定**（[`merge_with_fidelity`] 的 `basis`）：
//! 当次留存率与跨代累积留存率是两个坐标系，不能靠函数默认值替调用方决定。
//! 生产巩固传各源**最初一代**的并集（[DE-001 §8](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
//! 「与源比，不与上一代比」），逐代与上一代比会把累积漂移读没。

/// 句子切分：中英文句末标点与换行为界（标点留在句内），空白段丢弃。
///
/// 刻意**不**把裸 `.` 当界：`1.5` 会被切成 `1.` / `5`，合并后印成 `1.\n5`——
/// 不但失真，还把数字改坏了。英文的句号只在其后是空白或结尾时才算界。
pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    for (i, ch) in chars.iter().enumerate() {
        buf.push(*ch);
        let next = chars.get(i + 1).copied();
        let boundary = matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n')
            || (*ch == '.' && next.is_none_or(char::is_whitespace));
        if boundary {
            let unit = buf.trim().to_string();
            if !unit.is_empty() {
                out.push(unit);
            }
            buf.clear();
        }
    }
    let tail = buf.trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// 合并两条来源，并把合并结果对**比较基准**的留存率作为保真度。
///
/// - `sources`：本次要合并的正文（当次的输入）；
/// - `basis`：保真度的比较基准。传 `sources` = 当次留存率；传各源**最初一代**
///   的正文 = 跨代累积留存率（[DE-001 §8](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
///   「与源比，不与上一代比」）。坐标系由调用方显式选定，不再藏在函数默认里。
///
/// 保真度 = `basis` 中**确实保留在合并结果里**的句字数 / `basis` 总字数。
/// 同一件事两份都写了 ⇒ 两份的句子都算保留（去重省的是位置，不是信息）；
/// 装不下被丢掉的才算失真——那正是 `accept` 的 0.7 下界要拦的东西。
pub fn merge_with_fidelity(sources: &[&str], basis: &[&str], max_chars: usize) -> (String, f64) {
    let mut units: Vec<String> = Vec::new();
    for s in sources {
        for unit in split_sentences(s) {
            if !units.contains(&unit) {
                units.push(unit);
            }
        }
    }

    let mut kept: Vec<String> = Vec::new();
    let mut used = 0usize;
    for unit in &units {
        let add = unit.chars().count() + if kept.is_empty() { 0 } else { 1 }; // '\n' 分隔
        if used + add > max_chars {
            break;
        }
        used += add;
        kept.push(unit.clone());
    }

    let (mut retained, mut total) = (0usize, 0usize);
    for s in basis {
        for unit in split_sentences(s) {
            total += unit.chars().count();
            if kept.contains(&unit) {
                retained += unit.chars().count();
            }
        }
    }
    let fidelity = if total == 0 {
        1.0
    } else {
        retained as f64 / total as f64
    };
    (kept.join("\n"), fidelity)
}
