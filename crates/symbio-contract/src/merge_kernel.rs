//! 字符口径的文本内核：按**字符数**截断（D-387 / D-486）。
//!
//! ## 为什么在契约层
//!
//! 「一个纯算法进共享库」的最小准入是 **≥2 个不同 owner 的生产消费者**：本函数由
//! `symbio-plugin-memory` 的情景编码与 `symbio-plugin-skill` 的技能编译各有一处非测试
//! 调用（都经门面取用）。纯 std、零状态零 IO、内核不读 ⇒ 规则 B 全过。
//!
//! ## 与 verify 共有同一份源码
//!
//! `docs/003-design/verify/consolidation_fidelity.rs` 以 `#[path]` 包含本文件。
//! 因此本文件**只依赖 `std`、不得出现任何 `use`**——这是能被 standalone `rustc`
//! 编译的 verify 程序 `#[path]` 包含的前提。
//!
//! 文件名保留 `merge_kernel` 是路径事实（verify 的包含行与 memory 的注释都指它），
//! 不是第二份实现：同族的分句 / 合并 / 保真度只有巩固一个消费者，已私有化到
//! `symbio-plugin-memory` 的 `src/consolidate/merge.rs`，本文件不再持有那份算法。
//!
/// 按**字符数**截断（不切 UTF-8 字节——中文一字一字符，按字节切会把汉字切成
/// 两半）。记忆是提炼，不是转写副本：超长正文按这个口径收短。
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}
