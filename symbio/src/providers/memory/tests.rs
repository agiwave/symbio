//! `providers/memory/mod.rs` 的单元测试 —— 三层记忆共用的那一份口径。
//!
//! 与实现**同级**分文件（约定：`mod.rs` + `tests.rs`）。
//!
//! 这里钉的是**所有层都必须一致**的行为，因此每条都对应一个「一旦分叉就会让用户
//! 困惑」的问题：
//!
//! - 写侧**拒绝**而不是截断（静默丢内容是最坏的失败）；
//! - 截断落在 UTF-8 字符边界（否则产出非法 UTF-8）；
//! - 「当前」报的是文件总量，不是这一轮给了多少；
//! - 无作用域是**正常状态**（不注入），不是错误；
//! - 片段头信息只占一行（它每轮都要付 token）。
//!
//! 文件名刻意取一个**不是** `AGENTS.md` 的值：本模块不认识文件名（各层自己定，
//! 见模块文档），测试用别的名字才**证明**这一点——若哪天有人把某个具体文件名
//! 硬编码回来，这里的用例会立刻红。

use super::{
    memory_render_segment, MemoryFile, MemoryInjection, MemoryNodeSpec, MemorySegmentSpec,
};
use crate::symbio_core::VdfsAccess;
use tempfile::TempDir;

/// 测试用**合成文件名**：本模块不认识任何真实层的名字，用例自己带一个。
const FILE_NAME: &str = "NOTES.md";

/// 测试用**合成地址**：本模块只负责「把调用方给的地址印进片段」，不认识任何地址方案
/// （虚拟根叫什么、工作目录在哪都不归它）。用一个明显不是真实根名的前缀，
/// 使「模块里混进了真实的挂载规则」这类回归在测试里必然暴露。
const ADDR: &str = "@vfs/work/NOTES.md";

/// 临时目录里的一个记忆文件
fn file_in(tmp: &TempDir, write_max: usize, inject_max: usize) -> MemoryFile {
    MemoryFile::new(Some(tmp.path().join(FILE_NAME)), write_max, inject_max)
}

fn spec() -> MemorySegmentSpec<'static> {
    MemorySegmentSpec {
        title: "工作区记忆",
        address: ADDR,
        note: None,
        empty_hint: "暂无内容，可写入长期有效的约定",
    }
}

// ==================== 截断 ====================

#[test]
fn cut_keeps_short_text_intact() {
    let m = MemoryInjection::cut("abc", 3);
    assert_eq!(m.text, "abc");
    assert!(!m.truncated);
    assert_eq!(m.total_bytes, 3);
    assert_eq!(m.budget_bytes, 3);
}

/// 预算落在多字节字符中间 → 回退到字符边界，不产出非法 UTF-8
#[test]
fn cut_falls_back_to_a_char_boundary() {
    // "汉字" = 6 字节；预算 4 落在第一个字符中间 → 回退到 3
    let m = MemoryInjection::cut("汉字", 4);
    assert_eq!(m.text, "汉");
    assert_eq!(m.text.len(), 3);
    assert!(m.truncated);
    assert_eq!(m.total_bytes, 6, "总量是原文，不是截断后");

    // 预算恰好落在字符末尾：整字保留
    let m = MemoryInjection::cut("汉字", 3);
    assert_eq!(m.text, "汉");
    assert!(m.truncated);
}

// ==================== 无作用域 ====================

/// 无作用域 = 「这层记忆此刻不适用」，是**正常状态**，不是故障
#[test]
fn absent_scope_is_a_normal_state() {
    let m = MemoryFile::absent(1024, 256);
    assert!(!m.has_scope());
    assert!(m.path().is_none());
    assert!(!m.exists());
    assert_eq!(m.size(), 0);
    assert_eq!(m.updated_at(), None);
    // 读 / 写是明确报错（调用方本就不该走到这里）
    assert!(m.read().is_err());
    assert!(m.write("x").is_err());
    // 注入是 None —— 静默跳过，不往收集期错误桶里塞东西
    assert_eq!(m.inject().unwrap(), None);
    assert_eq!(m.segment(&spec()).unwrap(), None);
    // 无文件 → 无名字（本模块没有兜底文件名）
    assert_eq!(m.file_name(), None);
}

// ==================== 读写 ====================

#[test]
fn missing_file_reads_as_empty_not_as_error() {
    let tmp = TempDir::new().unwrap();
    let m = file_in(&tmp, 1024, 256);
    assert!(!m.exists());
    assert_eq!(m.read().unwrap(), "");
    assert_eq!(m.size(), 0);
}

#[test]
fn write_creates_dirs_and_roundtrips() {
    let tmp = TempDir::new().unwrap();
    let m = file_in(&tmp, 1024, 256);
    m.write("第一条长期事实\n").unwrap();

    assert!(m.exists());
    assert_eq!(m.read().unwrap(), "第一条长期事实\n");
    assert_eq!(m.size(), "第一条长期事实\n".len() as u64);
    assert!(m.updated_at().is_some(), "节点要用它做排序");
}

// ==================== 写入闸门：拒绝而非截断 ====================

#[test]
fn oversized_write_is_rejected_and_leaves_the_file_untouched() {
    let tmp = TempDir::new().unwrap();
    let m = file_in(&tmp, 1024, 256);
    m.write("原有内容").unwrap();

    let narrow = file_in(&tmp, 16, 256);
    let err = narrow.write(&"x".repeat(64)).unwrap_err();
    assert!(err.contains("超出容量上限"), "要点明原因：{err}");
    assert!(err.contains("16"), "要带上限值：{err}");
    assert_eq!(
        m.read().unwrap(),
        "原有内容",
        "被拒绝的写入不得留下半截内容"
    );
}

/// 上限按**字节**判定：多字节文本不能因为「字符数少」而蒙混过关
#[test]
fn write_limit_counts_bytes_not_chars() {
    let tmp = TempDir::new().unwrap();
    let m = file_in(&tmp, 11, 256);
    assert!(m.write("四个汉字").is_err(), "4 个汉字 = 12 字节 > 11");
    let m = file_in(&tmp, 12, 256);
    assert!(m.write("四个汉字").is_ok());
}

// ==================== 注入闸门与写入闸门独立 ====================

#[test]
fn inject_budget_is_independent_from_the_write_limit() {
    let tmp = TempDir::new().unwrap();
    // 写入上限 1024、注入预算 4 —— 文件可以比注入预算大得多
    let m = file_in(&tmp, 1024, 4);
    m.write("0123456789").unwrap();

    let body = m.inject().unwrap().unwrap();
    assert_eq!(body.text, "0123");
    assert!(body.truncated, "超出注入预算要明确标记，模型才知道去读全文");
    assert_eq!(
        body.total_bytes, 10,
        "「当前」描述的是文件，不是这一轮给了多少"
    );
    assert_eq!(body.budget_bytes, 4);
    assert_eq!(m.read().unwrap(), "0123456789", "文件本身不受注入预算影响");
}

#[test]
fn inject_reports_no_truncation_when_budget_is_enough() {
    let tmp = TempDir::new().unwrap();
    let m = file_in(&tmp, 1024, 1024);
    m.write("0123456789").unwrap();

    let body = m.inject().unwrap().unwrap();
    assert_eq!(body.text, "0123456789");
    assert!(!body.truncated);
    assert_eq!(body.total_bytes, 10);
}

// ==================== 片段渲染 ====================

fn body(text: &str, truncated: bool) -> MemoryInjection {
    MemoryInjection {
        text: text.to_string(),
        truncated,
        total_bytes: text.len() as u64,
        budget_bytes: 256,
    }
}

/// 头信息四要素缺一不可：地址（能改）、上限（写超会被拒）、当前（还剩多少）、
/// 写法（先读后写）
#[test]
fn segment_carries_address_capacity_and_write_discipline() {
    let s = memory_render_segment(&spec(), &body("用户偏好中文回答。", false), 16384);

    assert!(s.contains("【工作区记忆】"), "要有一眼认出的标题: {s}");
    assert!(s.contains(ADDR), "要给出可编辑地址: {s}");
    assert!(s.contains("16384"), "要给出写入上限: {s}");
    assert!(s.contains("用户偏好中文回答。"), "正文要原样带上: {s}");
    assert!(s.contains("vdfs_read"), "要给出先读后写的做法: {s}");
    assert!(s.contains("vdfs_write"), "要指出改写用哪个工具: {s}");
}

/// **简洁是硬指标**：头信息只有一行，整体开销远小于注入预算
#[test]
fn head_info_is_one_line_and_overhead_stays_small() {
    let text = "正文";
    let s = memory_render_segment(&spec(), &body(text, false), 16384);

    assert_eq!(s.lines().count(), 2, "一行头信息 + 正文，不加别的: {s}");
    let overhead = s.len() - text.len();
    assert!(
        overhead < 256,
        "头信息开销要小（实际 {overhead} 字节）：{s}"
    );
}

#[test]
fn note_is_optional_and_lands_in_the_head() {
    let plain = memory_render_segment(&spec(), &body("x", false), 1024);
    assert!(!plain.contains("相互独立"));

    let noted = memory_render_segment(
        &MemorySegmentSpec {
            note: Some("与【工作区记忆】相互独立"),
            ..spec()
        },
        &body("x", false),
        1024,
    );
    assert!(noted.contains("与【工作区记忆】相互独立"));
    assert_eq!(noted.lines().count(), 2, "附加说明不得撑出第二行头信息");
}

/// 空内容恰恰是最需要「你可以往里写」的时候 —— 但只用一句话
#[test]
fn empty_memory_still_teaches_how_to_remember() {
    let s = memory_render_segment(&spec(), &body("", false), 1024);
    assert!(s.contains("暂无内容"), "要点明现在还没有内容: {s}");
    assert!(s.contains("vdfs_write"), "要教它怎么写入: {s}");
    assert!(!s.contains("已截断"), "空内容不得谎报截断: {s}");
    assert_eq!(s.lines().count(), 2);
}

/// 截断必须**明确告知**：否则模型会以为看到的就是全部
#[test]
fn truncated_memory_says_so_and_points_at_the_address() {
    let s = memory_render_segment(&spec(), &body("前一半", true), 1024);
    assert!(s.contains("已截断"), "要说明只看到了一部分: {s}");
    assert!(s.contains("256"), "要说清截到了多少字节: {s}");
    assert!(s.contains("vdfs_read"), "要指路去读全文: {s}");
    assert_eq!(s.lines().count(), 3, "截断提示也只占一行: {s}");
}

// ==================== 组合：inject → segment ====================

#[test]
fn segment_reads_writes_and_reports_in_one_step() {
    let tmp = TempDir::new().unwrap();
    let m = file_in(&tmp, 1024, 4);
    m.write("0123456789").unwrap();

    let s = m.segment(&spec()).unwrap().unwrap();
    assert!(s.contains("0123"), "只带预算内的前缀: {s}");
    assert!(!s.contains("0123456789"), "超预算的部分不注入: {s}");
    assert!(s.contains("当前：10字节"), "「当前」要是文件总量: {s}");
    assert!(s.contains("已截断至 4 字节"));
}

// ==================== 节点 ====================

#[test]
fn node_shape_is_shared_by_list_and_stat() {
    let tmp = TempDir::new().unwrap();
    let m = file_in(&tmp, 1024, 256);
    m.write("内容").unwrap();

    let n = m.node(&MemoryNodeSpec {
        title: "工作区记忆",
        kind: "work",
        description: "本工作区的长期记忆",
    });

    assert_eq!(n.name, FILE_NAME, "节点名 = 真实文件名");
    assert_eq!(n.title, "工作区记忆");
    assert_eq!(n.kind, "work");
    assert_eq!(n.size, Some("内容".len() as u64));
    assert!(n.updated_at.is_some());
    assert_eq!(n.access, VdfsAccess::READ_WRITE, "记忆恒可读写");
    // 缺省 ext 由文件名字面扩展名推导（`AGENTS.md` → `md`），渲染器据此分发
    assert_eq!(n.ext, None, "不显式声明，交给机制推导");
}
