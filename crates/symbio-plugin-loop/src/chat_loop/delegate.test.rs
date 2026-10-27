//! 三项真源的回归测试——证明它**会红**。
//!
//! 两处负向断言是本文件的重点，各自钉住一条**被否决的方案**：
//! - `length_zero_is_off`：阈值 0 若判成「任何长度都命中」，每条寒暄都会开后台会话
//!   （D-047 被否决方案第一条）；
//! - `empty_prefix_never_matches`：空前缀若不跳过，`starts_with("")` 恒真 ⇒ 一切
//!   消息都命中，判定权等于没有。

use super::*;
// 出厂值的真源在 core（`TurnSettings` 三旋钮与 session 配置面共用同一批常量）——
// 判定侧测试显式 import，不另持第二份字面量。
use symbio_core::{CapabilityCategory, WORKER_FORCE_PREFIX, WORKER_KEYWORDS, WORKER_MIN_CHARS};
use symbio_plugin_api::context::DelegateVerdict;

/// 出厂参数：前缀 `/work `、关键词空集、阈值 0。
fn factory(text: &str) -> Option<&'static str> {
    worker_start_reason(text, WORKER_FORCE_PREFIX, WORKER_KEYWORDS, WORKER_MIN_CHARS)
}

/// Q1 判定（配置化）：判据 → 判完的结论。生产路径在 `prepare_turn_inputs`，
/// 测试用同一函数体，避免「测的判定」与「跑的判定」分家。
fn judge(
    text: &str,
    force_prefix: &str,
    keywords: &str,
    min_chars: usize,
) -> DelegateVerdict<'static> {
    let keywords: Vec<&str> = keywords.split(',').collect();
    match worker_start_reason(text, force_prefix, &keywords, min_chars) {
        Some(reason) => DelegateVerdict::Start { reason },
        None => DelegateVerdict::Idle,
    }
}

/// 出厂**配置**下的委派段：判定按出厂三旋钮，渲染与生产同一个 `delegate_section`。
fn factory_section(
    utterance: Option<&str>,
    tools: &[CapabilityMeta],
    workers: &[WorkerProgress],
) -> Option<String> {
    let verdict = match utterance {
        None => DelegateVerdict::NotJudged,
        Some(text) => match factory(text) {
            Some(reason) => DelegateVerdict::Start { reason },
            None => DelegateVerdict::Idle,
        },
    };
    delegate_section(verdict, tools, workers)
}

// ── Q1 · 三条旋钮（判据可配：值取自配置面，出厂值只是默认）────────────────
mod knobs {
    use super::*;

    /// 改前缀 ⇒ 判定跟着改，且**出厂前缀此时不再命中**（配置是替换，不是叠加）。
    #[test]
    fn prefix_is_configurable_and_replaces_the_default() {
        let section =
            delegate_section(judge("#run 重构", "#run ", "", WORKER_MIN_CHARS), &[], &[]).unwrap();
        assert!(section.contains("reason=explicit_prefix"), "{section}");

        let replaced =
            delegate_section(judge("/work 重构", "#run ", "", WORKER_MIN_CHARS), &[], &[]).unwrap();
        assert!(replaced.contains("未命中任何委派判据"), "{replaced}");
    }

    /// 关键词与长度阈值同理：出厂是「空集 / 0」两层关闭，配了才开。
    #[test]
    fn keywords_and_min_chars_are_configurable() {
        let section = delegate_section(
            judge(
                "帮我把这个重构掉",
                WORKER_FORCE_PREFIX,
                "重构",
                WORKER_MIN_CHARS,
            ),
            &[],
            &[],
        )
        .unwrap();
        assert!(section.contains("reason=keyword_hit"), "{section}");

        let long = delegate_section(
            judge("这句话足够长了吧", WORKER_FORCE_PREFIX, "", 4),
            &[],
            &[],
        )
        .unwrap();
        assert!(long.contains("reason=topic_length"), "{long}");

        // 出厂值这两层是**关闭**的：同一条消息在默认配置下不命中（否则每条寒暄
        // 都开一个后台会话，D-047 被否决方案第一条）。
        let off = factory_section(Some("帮我把这个重构掉"), &[], &[]).unwrap();
        assert!(off.contains("未命中任何委派判据"), "{off}");
    }
}

// ── Q1 · 三层判据 ──────────────────────────────────────────────────────

#[test]
fn explicit_prefix_hits_and_returns_reason() {
    assert_eq!(factory("/work 帮我重构整个模块"), Some(REASON_PREFIX));
}

#[test]
fn prefix_keeps_its_trailing_space() {
    // 带尾空格是判据的一部分：`/worker`、`/workflow` 是词，不是指令。
    assert_eq!(factory("/worker 是什么"), None);
    assert_eq!(factory("/workflow 怎么写"), None);
    assert_eq!(factory("/work"), None, "没有尾空格不算前缀命中");
}

#[test]
fn empty_prefix_never_matches() {
    // `starts_with("")` 恒真——空前缀必须被跳过，否则一切消息都「该开 worker」。
    assert_eq!(worker_start_reason("随便聊聊", "", &["改造"], 0), None);
    assert_eq!(
        worker_start_reason("随便聊聊", "", &[], 0),
        None,
        "空白配置不该退化成「永远命中」"
    );
}

#[test]
fn keyword_hits_case_insensitively_as_substring() {
    let kw = ["重构", "REFactor"];
    assert_eq!(
        worker_start_reason("帮我重构一下这段", WORKER_FORCE_PREFIX, &kw, 0),
        Some(REASON_KEYWORD)
    );
    // 子串而非整句相等：大小写不同也要命中。
    assert_eq!(
        worker_start_reason("please refactor this", WORKER_FORCE_PREFIX, &kw, 0),
        Some(REASON_KEYWORD)
    );
}

#[test]
fn empty_keyword_set_is_off() {
    // 出厂空集 ⇒ 这一层关闭：普通消息不因为「有关键词层」而命中。
    assert_eq!(factory("帮我看看天气"), None);
    // 空串 / 纯空白关键词不许命中任何消息。
    assert_eq!(
        worker_start_reason("任意一句话", WORKER_FORCE_PREFIX, &["", "  "], 0),
        None
    );
}

#[test]
fn length_zero_is_off() {
    // 阈值 0 必须是「关闭」：若判成 >= 0，任何非空消息都命中。
    assert_eq!(worker_start_reason("嗯", WORKER_FORCE_PREFIX, &[], 0), None);
    assert_eq!(
        worker_start_reason("这是一段相当长的发言", WORKER_FORCE_PREFIX, &[], 0),
        None,
        "出厂阈值 0 不该让长消息命中"
    );
}

#[test]
fn length_hits_only_at_or_above_threshold() {
    let text = "一二三四五"; // 5 字
    assert_eq!(worker_start_reason(text, WORKER_FORCE_PREFIX, &[], 6), None);
    assert_eq!(
        worker_start_reason(text, WORKER_FORCE_PREFIX, &[], 5),
        Some(REASON_LENGTH)
    );
    // 按**字符**计，不是字节：中文一个字 3 字节，按字节算会提前命中。
    assert_eq!(
        worker_start_reason(text, WORKER_FORCE_PREFIX, &[], 4),
        Some(REASON_LENGTH)
    );
}

#[test]
fn empty_or_blank_input_is_none() {
    assert_eq!(factory(""), None);
    assert_eq!(factory("   \n\t "), None);
    assert_eq!(factory("///"), None, "纯标点不是「要干活」");
}

#[test]
fn prefix_short_circuits_before_keyword() {
    // 两层同时可命中 ⇒ 回**先判的那层**的理由码（短路顺序是判据的一部分）。
    let hit = worker_start_reason("/work 重构", WORKER_FORCE_PREFIX, &["重构"], 0);
    assert_eq!(hit, Some(REASON_PREFIX));
}

#[test]
fn no_criteria_hit_returns_none() {
    assert_eq!(
        worker_start_reason("今天天气不错", WORKER_FORCE_PREFIX, &["改造"], 0),
        None
    );
}

// ── Q2 · 能力目录 ──────────────────────────────────────────────────────

fn meta(name: &str, category: Option<CapabilityCategory>) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        description: String::new(),
        input_schema: serde_json::json!({ "type": "object" }),
        category,
        examples: None,
        ..Default::default()
    }
}

#[test]
fn empty_tools_is_none() {
    // 空目录不产出段落（与就绪段「候选集空 ⇒ 整段不出现」同一条判据）。
    assert_eq!(capability_digest(&[]), None);
}

#[test]
fn groups_by_category_and_sorts_both_levels() {
    let tools = [
        meta("web_search", Some(CapabilityCategory::Network)),
        meta("file_write", Some(CapabilityCategory::FileOperation)),
        meta("file_read", Some(CapabilityCategory::FileOperation)),
    ];
    let digest = capability_digest(&tools).unwrap();
    let lines: Vec<&str> = digest.lines().collect();

    assert_eq!(lines[0], "【能力目录】按分类分组（只列名字，不列参数）");
    // BTreeMap ⇒ 分类段按字典序（FileOperation < Network）。
    assert_eq!(lines[1], "- FileOperation: file_read, file_write");
    assert_eq!(lines[2], "- Network: web_search");
}

#[test]
fn missing_category_falls_back_to_other() {
    // `category: None` 的兜底是 `Other`，不是「不进目录」——漏掉的能力等于不可见。
    let digest = capability_digest(&[meta("mystery", None)]).unwrap();
    assert!(digest.contains("- Other: mystery"), "实际：{digest}");
}

#[test]
fn short_name_strips_the_prefix() {
    // 与 `prepare_turn_inputs` 解析 `context_retention` 用同一条 `rsplit('/')` 口径。
    let digest = capability_digest(&[meta(
        "mcp/github/create_issue",
        Some(CapabilityCategory::Mcp),
    )])
    .unwrap();
    assert!(digest.contains("- Mcp: create_issue"), "实际：{digest}");
    assert!(
        !digest.contains("mcp/github"),
        "目录里不该留全路径：{digest}"
    );
}

#[test]
fn duplicate_short_names_collapse() {
    let digest = capability_digest(&[
        meta("a/read", Some(CapabilityCategory::FileOperation)),
        meta("b/read", Some(CapabilityCategory::FileOperation)),
    ])
    .unwrap();
    assert!(digest.contains("- FileOperation: read"), "实际：{digest}");
}

#[test]
fn digest_is_order_independent() {
    // `composite::traverse` 遍历的是 `HashMap`，**顺序每次不同**——所以「同一份能力
    // 集合」必须与它被遇上的先后无关。用两份**顺序相反**的清单钉这条：不排序的实现
    // 在这里必然产出两段不同的文本（分组内与分组间都会跟着入参走）。
    let a = [
        meta("zeta", Some(CapabilityCategory::Skill)),
        meta("alpha", Some(CapabilityCategory::Skill)),
        meta("beta", Some(CapabilityCategory::Core)),
    ];
    let b = [
        meta("beta", Some(CapabilityCategory::Core)),
        meta("alpha", Some(CapabilityCategory::Skill)),
        meta("zeta", Some(CapabilityCategory::Skill)),
    ];
    let first = capability_digest(&a).unwrap();
    for _ in 0..8 {
        assert_eq!(
            capability_digest(&b).unwrap(),
            first,
            "入参顺序换了，段落就变了"
        );
        assert_eq!(capability_digest(&a).unwrap(), first);
    }
}

// ── 合成段落（`prepare_turn_inputs` 的装配入口） ────────────────────────

#[test]
fn section_is_none_when_there_is_nothing_to_say() {
    // 无发言（resume / 心跳）且无能力 ⇒ 一个字节都不多。
    assert_eq!(factory_section(None, &[], &[]), None);
}

#[test]
fn section_carries_the_hit_reason_verbatim() {
    let section = factory_section(Some("/work 帮我重构"), &[], &[]).unwrap();
    assert!(
        section.starts_with("- 委派判定：本轮应启动 worker（reason=explicit_prefix）"),
        "实际：{section}"
    );
    // 理由码必须逐字可见：它是审计面，改一个字符就与观测口径脱钩。
    assert!(section.contains(REASON_PREFIX));
}

#[test]
fn section_records_a_miss_too() {
    // 「为什么这轮没动手」是 Q1 存在的理由——未命中也要落事实，否则分不清
    // 「判了不动」与「压根没判」。
    let section = factory_section(Some("今天天气不错"), &[], &[]).unwrap();
    assert!(
        section.contains("- 委派判定：未启动 worker（未命中任何委派判据）"),
        "实际：{section}"
    );
    assert!(
        !section.contains("reason="),
        "未命中不该凭空造理由码：{section}"
    );
}

#[test]
fn section_without_utterance_omits_the_verdict_line() {
    // resume / 心跳：没有用户新发言，判决无从谈起 ⇒ 只剩能力目录。
    let section = factory_section(
        None,
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
        &[],
    )
    .unwrap();
    assert!(!section.contains("委派判定"), "实际：{section}");
    assert!(section.contains("【能力目录】"), "实际：{section}");
}

#[test]
fn section_puts_verdict_before_digest() {
    // 判定在前、目录在后：模型先看到「本轮动不动手」，再看到「我有哪些能力」。
    let section = factory_section(
        Some("/work 重构"),
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
        &[],
    )
    .unwrap();
    let verdict = section.find("委派判定").unwrap();
    let digest = section.find("【能力目录】").unwrap();
    assert!(verdict < digest, "次序反了：{section}");
}

#[test]
fn section_survives_empty_capability_list_when_verdict_exists() {
    // 能力空 ⇒ 目录不出现，但判定仍在（两条真源各自独立决定自己在不在）。
    let section = factory_section(Some("/work 重构"), &[], &[]).unwrap();
    assert!(section.contains("reason=explicit_prefix"));
    assert!(!section.contains("【能力目录】"), "实际：{section}");
}

#[test]
fn section_carries_worker_fields_verbatim() {
    let p = WorkerProgress {
        id: "worker-1".to_string(),
        title: "重构".to_string(),
        in_flight: true,
        rounds: 3,
        steps: vec!["读代码".to_string()],
    };
    let section = factory_section(Some("/work 继续"), &[], &[p]).unwrap();
    // 三个字段名逐字可见：与 B2 状态行同名，两处漂移时一眼对得出来。
    assert!(section.contains("rounds=3"), "{section}");
    assert!(section.contains("in_flight=true"), "{section}");
    assert!(section.contains("steps=[读代码]"), "{section}");
    assert!(section.contains("`重构`"), "{section}");
}

#[test]
fn section_puts_workers_after_verdict_and_digest() {
    // 次序：模型先看「本轮动不动手」、再看「我有哪些能力」、最后看「worker 到哪了」。
    let p = WorkerProgress {
        id: "w".to_string(),
        title: "t".to_string(),
        ..Default::default()
    };
    let section = factory_section(
        Some("/work 重构"),
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
        &[p],
    )
    .unwrap();
    let verdict = section.find("委派判定").unwrap();
    let digest = section.find("【能力目录】").unwrap();
    let worker = section.find("worker `").unwrap();
    assert!(verdict < digest && digest < worker, "次序反了：{section}");
}

#[test]
fn section_omits_workers_when_there_are_none() {
    // 没开 worker ⇒ Q3 一个字节都不占（与 Q2 空目录同一条纪律）。
    let section = factory_section(Some("/work 继续"), &[], &[]).unwrap();
    assert!(!section.contains("worker `"), "{section}");
}
