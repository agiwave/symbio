//! 巩固合并的**纯计算内核**：截断 / 分句 / 合并 + 保真度。
//!
//! 与 `plugins/loop/recall_score.rs` 同一条纪律：**生产与 verify
//! 共有同一份源码**。`docs/003-design/verify/consolidation_fidelity.rs` 以 `#[path]`
//! 包含本文件——它论证「质量约束（min_fidelity）必须先于形态约束（max_gen）生效」时，
//! 跑的是生产这条保真度算法本身，不是另写一份临摹件。
//! 临摹件会随生产漂移而静默失效：论证仍在绿，闸门已经换了算法。
//!
//! 三个函数都只依赖 `std`——这是能被 standalone `rustc` 编译的 verify 程序
//! `#[path]` 包含的前提。本模块不知道任何调用方的存在，更不 import 事件 / 存储：
//! **它只算数，入格永远是写方的事**
//! （[plan/01 §9.3](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")：架构只提供位置，不提供算法）。
//!
//! **保真度的比较基准由调用方显式选定**（[`merge_with_fidelity`] 的 `basis`）：
//! 当次留存率与跨代累积留存率是两个坐标系，不能靠函数默认值替调用方决定。
//! 生产巩固传各源**最初一代**的并集（[plan/01 §8](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
//! 「与源比，不与上一代比」），逐代与上一代比会把累积漂移读没。
//!
//! ## 为什么住 core（[D-082](../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 2/3/6/7 组）
//!
//! 它原先由会话插件的 `memory_events.rs` 以 `#[path]` 就地声明，另一处调用方
//! `skill_events.rs` 得回头够 `memory_events` 才拿得到——一份纯算法挂在某个写方
//! 名下，第二个写方就只得制造一条模块间反向引用。会话按
//! [plan/14 步 6](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 切成
//! 驱动器/留守两片之后，这条边会升级为跨插件引用（E-009 直接判红）。故词汇随
//! 「纯计算」归位 core 的文本工具域（与 [`crate::text`] 的字节安全
//! 截断同族），两片都经根别名 [`crate::merge_kernel`] 取用。

/// 按**字符数**截断（不切 UTF-8 字节——中文一字一字符，按字节切会把汉字切成
/// 两半）。记忆是提炼，不是转写副本：超长正文按这个口径收短。
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

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
///   的正文 = 跨代累积留存率（[plan/01 §8](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
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
