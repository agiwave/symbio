//! 字符口径的文本内核：按**字符数**截断。
//!
//! ## 为什么留在这里（D-387 Q3 普查）
//!
//! 「一个纯算法进共享库」的最小准入有一条是 **≥2 个不同 owner 的生产消费者**。
//! [`truncate_chars`] 满足：`symbio-plugin-memory` 的情景编码与 `symbio-plugin-skill`
//! 的技能编译各有一处非测试调用（都经 `symbio_plugin_api` 门面取用），因此留在共享
//! 文本域。
//!
//! 同族的分句 / 合并 / 保真度只有巩固一个消费者，按同一条准入**一律私有**，已搬进
//! `symbio-plugin-memory` 的 `src/consolidate/merge.rs`；本文件不再持有那份算法。
//! 文件名保留 `merge_kernel` 是路径事实，不是第二份实现。
//!
//! ## 与 verify 共有同一份源码
//!
//! `docs/003-design/verify/consolidation_fidelity.rs` 以 `#[path]` 包含本文件。
//! 因此本文件**只依赖 `std`、不得出现任何 `use`**——这是能被 standalone `rustc`
//! 编译的 verify 程序 `#[path]` 包含的前提。

/// 按**字符数**截断（不切 UTF-8 字节——中文一字一字符，按字节切会把汉字切成
/// 两半）。记忆是提炼，不是转写副本：超长正文按这个口径收短。
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}
