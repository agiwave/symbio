//! 能力目录折叠 —— **纯函数**：把能力清单折成一段给模型看的摘要。
//!
//! ## 为什么主会话需要它
//!
//! 主会话**不持有工具**（目标形态，见 `docs/plan/06` §10.2），但用户会问
//! "你能做什么"。它必须知道**存在哪些能力**才能：① 回答这个问题；② 判断
//! 一个请求值不值得开 worker。给模型看完整 JSON schema 太贵（那是执行面），
//! 所以这里只折出**按分类分组的名字目录**。
//!
//! ## 真相源与不重复
//!
//! 输入就是 `CapabilityVisitor::list_capability()` 的产物（`CapabilityMeta`），
//! 本模块**不维护第二份能力清单**：分类标签取 `category` 的**线格式词**
//! （serde snake_case，如 `file_operation`），名字取 `name`。新增工具 ⇒ 目录自动变长，
//! 零改动（这是"扩展只加数据"在提示词面的体现）。
//!
//! ## 平凡值与确定性
//!
//! - 无能力（空输入）⇒ **空串**：调用方据此**不注入**该段（不留"【可用能力】"空标题）；
//! - 超 `max` 条 ⇒ 截断并在末尾标 `…（共 N 项）`——截断必须可见；
//! - 输出按「分类字典序 + 名字字典序」排列：确定性（可双跑比对，A4）。

use crate::symbio_core::CapabilityMeta;
use std::collections::BTreeMap;

/// 段落标题（注入进提示词时的开头；空目录时整段不出现）
// dead-code-allow R-001: 消费方是 R1-b 的主会话提示词注入点，本期只有单测调用
#[allow(dead_code)]
pub const DIGEST_TITLE: &str = "【可用能力】";

/// 折叠：`{"分类": [名字…]}` → 标题 + 逐行目录。
///
/// 返回空串表示"没有可注入的内容"（调用方不要拼空段落）。
// dead-code-allow R-001: 消费方是 R1-b 的主会话提示词注入点（`prepare_turn_inputs`），本期只有单测调用
#[allow(dead_code)]
pub fn digest(caps: &[CapabilityMeta], max: usize) -> String {
    if caps.is_empty() || max == 0 {
        return String::new();
    }

    // ① 分组（BTreeMap ⇒ 输出确定）；名字去重后排序
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in caps {
        let name = c.name.trim();
        if name.is_empty() {
            continue;
        }
        let mut group = c
            .category
            .map(category_wire)
            .unwrap_or_else(|| "other".to_string());
        if group.is_empty() {
            group = "other".to_string();
        }
        groups.entry(group).or_default().push(name.to_string());
    }
    for names in groups.values_mut() {
        names.sort();
        names.dedup();
    }

    // ② 摊平后按上限截断（截断可见）
    let total: usize = groups.values().map(Vec::len).sum();
    let mut out = String::new();
    out.push_str(DIGEST_TITLE);
    out.push('\n');
    let mut shown = 0usize;
    for (group, names) in &groups {
        let mut line = Vec::new();
        for n in names {
            if shown >= max {
                break;
            }
            line.push(n.as_str());
            shown += 1;
        }
        if line.is_empty() {
            continue;
        }
        out.push_str(&format!("- {group}: {}\n", line.join(" / ")));
        if shown >= max {
            break;
        }
    }
    if shown < total {
        out.push_str(&format!("…（共 {total} 项，此处列出 {shown} 项）\n"));
    }
    out
}

/// `CapabilityCategory` 的线格式词（serde snake_case）。
///
/// 取**序列化结果**而不是自己写一张映射表：枚举加变体时这里零改动，
/// 也不会出现"目录里的分类名与线路上的不一样"这种第二份真相。
// dead-code-allow R-001: 随 digest 一起只有 R1-b 消费（reachability 仍判它死）
#[allow(dead_code)]
fn category_wire(k: crate::symbio_core::CapabilityCategory) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "digest.test.rs"]
mod tests;
