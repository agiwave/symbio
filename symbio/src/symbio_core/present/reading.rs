//! 读数面板的**宿主方言**（definition-driven reading）
//!
//! 与 [`super::detail`] 同一族哲学：**后端下发描述，前端零业务知识地渲染**。
//! 差别只在用途——`detail` 服务「可编辑的详情页」，本模块服务「**只读**的指标读数」。
//!
//! ## 为什么读数也要一份「方言」而不是前端自己拼
//!
//! 「这些数是什么意思、该叫什么、单位与精度怎么取」只有**产者**知道。前端自己拼
//! 就等于把口径抄进第二处（D-044 的反面：实测与判据不同源），而读数一旦改动，
//! 抄本不会报错、只会静静显示旧标签。本模块让前端只剩一件事：**按描述排版**。
//!
//! ## 形状里没有任何业务语义
//!
//! 分区 / 行 / 标签 / 值 / 口径提示 / 条形几何——不含「会话」「水位」「档位」「兜底」
//! 任何一个词。产者要说「到这条线就该接手」，就下发一根参考线，而不是让渲染器
//! 去猜哪一个是分母。
//! 一期只有 `session/stats` 一个产者，但换一个产者（成本、记忆、任何只读路由）
//! 前端**零改动**。这是它与「给 stats 写一个专用组件」的分界。
//!
//! ## 值与提示的分工
//!
//! - `value` 已由产者**排版成字符串**（数字的单位、精度、百分比都归它）——前端不做
//!   任何数值格式化，否则「P99 显示 ms 还是 s」又成了两处；
//! - `hint` 是**口径说明**（这个数怎么算的），与值同现但视觉次一级；
//! - `tone` 只分「平常 / 需留意」两档，且由产者判——它拿着数，才知道 2 个未收束
//!   轮是不是问题。前端不解析值的语义。

use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "reading.test.rs"]
mod tests;

/// 一列读数的**语气**（产者判，前端只换样式）。
///
/// 只有两档而不是四档（error / warn / info / success）：读数面板的回答是
/// 「现在的状况」，不是「操作的结果」——后者才需要四种状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingTone {
    /// 平常（默认）。
    #[default]
    Normal,
    /// 需留意：产者判「这个数值得被看一眼」，但不构成失败。
    Warn,
}

/// 一行读数：标签 + 已排版的值 + 可选的口径提示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingRow {
    /// 标签（「兜底率」/「未收束轮」…）。
    pub label: String,
    /// 已排版的值的文本（含单位与精度）。
    pub value: String,
    /// 口径提示（「fallback / user.message 之比」…）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// 语气（缺省 = 平常）。
    #[serde(default)]
    pub tone: ReadingTone,
}

/// 一条**几何呈现**（分区可选的一格）：填充位置 + 若干参考线位置。
///
/// 里面全是产者**已经除好的 0..1 比率**，不是留给渲染器再除一次的分子分母——
/// 除法一旦有两处，改系数就只有一半界面跟着变（表现为「界面上还没到线，后端
/// 已经动手」）。
///
/// ## 它是呈现提示，不是新事实
///
/// 不认这一格的渲染器照常只排 [`ReadingSection::rows`]，条与参考线**不出现**。
/// 渲染器因此不需要认识「水位」「成本」任何一个词——它只画「填到这儿」和
/// 「在这儿画一根线」；每根线**是什么**由同分区的某一行说明，对应关系住在产者。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReadingBar {
    /// 填充到的位置。缺省 = 没有分母，条不表态（画成空，而不是补一个数）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
    /// 参考线位置（零到多条，按产者给的顺序画）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<f64>,
    /// 语气（缺省 = 平常）：越线与否只有拿着数的产者知道，渲染器不解析值的语义。
    #[serde(default)]
    pub tone: ReadingTone,
}

/// 一个分区：标题可选，行必填。
///
/// 标题可选是因为存在「不该有标题的扁平读数」——为一个三行的面板硬造
/// 「概要」二字的分区标题，是排版噪声不是信息。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReadingSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rows: Vec<ReadingRow>,
    /// 条形几何（可选）——见 [`ReadingBar`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<ReadingBar>,
}

/// 一份读数面板：标题 + 若干分区。
///
/// **空 sections 是合法值**（「没有可报的」），不是错误——前端渲染空态，
/// 不渲染错误。这与 `has_wal = false` 时各列为「有据的零」是同一条判定：
/// 没有数与读不到是两件事，形状必须分得开。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReadingPanel {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub sections: Vec<ReadingSection>,
}

impl ReadingPanel {
    /// 行数（含全部分区）——诊断与判据用，前端不依赖它。
    pub fn row_count(&self) -> usize {
        self.sections.iter().map(|s| s.rows.len()).sum()
    }

    /// 有没有「需留意」的行。
    pub fn has_warn(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|s| &s.rows)
            .any(|r| r.tone == ReadingTone::Warn)
    }
}
