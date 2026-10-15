// 生成物：由 `scripts/gen-verify-facts.mjs` 从 docs/004-plan 抽取，**不要手改**。
// 要改这些数据就改文档（01 §8 / 02 坐标系 / 路线图总表 / 各阶 §3·§4 / S10 §2），再重跑生成脚本。
//
// 每个 verify 程序都是一个**独立的 crate**，各自只 `use` 下面的一部分；
// 没被某个程序读到的那些不是死码，是另一个程序在读。这里判死码只会制造噪音，
// 所以整模块关掉这条 lint。模块里唯一的逻辑是 §8 取值域语法的 Rust 镜像（`in_domain`）——
// 它必须只有一份：判据抄进第二个程序，同一个输入就可能只翻红一处。
#![allow(dead_code)]

/// 01 §8 的顶层机制键（`projection.param` 这类子键不计，判据见生成器注释）
pub const MECHANISMS: &[&str] = &[
    "store",
    "projection",
    "actor.pattern",
    "actor.capability",
    "actor.budget_ms",
    "event.entity",
    "event.verb",
    "scope",
    "vis_scope",
    "principal",
];

/// 01 §8 参数表里登记的全部键（含子键）——赋值块的合法词表
pub const PARAM_KEYS: &[&str] = &[
    "store",
    "projection",
    "projection.param",
    "actor.pattern",
    "actor.capability",
    "actor.budget_ms",
    "event.entity",
    "event.verb",
    "scope",
    "vis_scope",
    "principal",
];

/// 01 §8 一行的**取值域**。`open` = 表里写成 `*` 的开放值域（数据，不判取值）；
/// `forms` 里的 `<…>` 是通配段（`child:<id>` 匹配任何 `child:x`），标了 `↳ §N` 的
/// 集合已由生成器从那张表读成逐项形态。
pub struct Domain {
    pub key: &'static str,
    pub forms: &'static [&'static str],
    pub open: bool,
}

pub const DOMAINS: &[Domain] = &[
    Domain {
        key: "store",
        forms: &["memory", "wal", "sharded"],
        open: false,
    },
    Domain {
        key: "projection",
        forms: &["snapshot", "display", "readyset", "turnstate", "checkpoint", "eval", "recall", "consolidate", "reputation", "calibration", "budget", "replay", "skill_compile", "conation"],
        open: false,
    },
    Domain {
        key: "projection.param",
        forms: &["consolidate:max_gen=<n>", "consolidate:min_fidelity=<f>", "recall:tag=<label>", "recall:density=<n>", "recall:layer=<n>", "recall:spread=on", "recall:decay=<fn>", "recall:min_generality=<0–4>", "readyset:cap=<n>", "conation:pref=<p>"],
        open: false,
    },
    Domain {
        key: "actor.pattern",
        forms: &["decider", "reasoner", "translator"],
        open: false,
    },
    Domain {
        key: "actor.capability",
        forms: &["judge.intent", "reply.first", "reply.append", "define.work", "produce.artifact", "assert.verification", "assign.work"],
        open: false,
    },
    Domain {
        key: "actor.budget_ms",
        forms: &[],
        open: true,
    },
    Domain {
        key: "event.entity",
        forms: &["turn", "task", "artifact", "verdict", "memory", "commitment", "conation", "thread", "control", "system", "message"],
        open: false,
    },
    Domain {
        key: "event.verb",
        forms: &["opened", "progressed", "held", "closed", "asserted"],
        open: false,
    },
    Domain {
        key: "scope",
        forms: &["root", "child:<id>"],
        open: false,
    },
    Domain {
        key: "vis_scope",
        forms: &["thread_private", "shared", "public"],
        open: false,
    },
    Domain {
        key: "principal",
        forms: &[],
        open: true,
    },
];

/// 01 §8 取值域语法的 Rust 镜像（本生成物里唯一的逻辑，供各 verify 程序共用——
/// 判据抄两份，同一个输入就可能只翻红一处）。三条放行路径：
/// ① 键是**开放值域**（§8 那格写 `*`，如 `principal` = 身份是数据）；
/// ② 值匹配该键的某个形态（`<…>` 是通配段：`child:<id>` 匹配任何 `child:x`）；
/// ③ **带子键参数的取值**——`projection = recall:tag=judgment` 合法的条件是「`recall`
///    是 `projection` 的取值」**且**「整串是 `projection.*` 某个子键的取值」。
/// ③ 的判据是「§8 里登记了 `key.*` 这一行」，不是「这一格恰好叫 projection」。
pub fn form_matches(form: &str, v: &str) -> bool {
    match (form.find('<'), form.find('>')) {
        (Some(i), Some(j)) if j > i => {
            let (head, tail) = (&form[..i], &form[j + 1..]);
            v.starts_with(head) && v.ends_with(tail) && v.len() >= head.len() + tail.len()
        }
        (Some(_), _) => panic!("取值域形态 {form} 的通配段没有闭合"),
        _ => form == v,
    }
}

/// §8 里这个键的值域行。键没登记时返回 `None`——那不是越界**取值**，是越界**键**，
/// 由调用方按自己问的那个问题分辨（同一处红不该有两种说法）。
pub fn domain_of(key: &str) -> Option<&'static Domain> {
    DOMAINS.iter().find(|d| d.key == key)
}

/// 赋值 `key = value` 是否落在 01 §8 声明的值域内；键没登记 ⇒ 判不了 ⇒ false。
pub fn in_domain(key: &str, value: &str) -> bool {
    let Some(d) = domain_of(key) else { return false };
    if d.open || d.forms.iter().any(|f| form_matches(f, value)) {
        return true;
    }
    let base = value.split(':').next().unwrap_or(value);
    d.forms.iter().any(|f| form_matches(f, base))
        && DOMAINS.iter().any(|sub| {
            sub.key.starts_with(&format!("{key}."))
                && !sub.open
                && sub.forms.iter().any(|f| form_matches(f, value))
        })
}

/// 02 §2 的五维（名册「维」列的合法取值）
pub const DIMENSIONS: &[&str] = &[
    "知",
    "行",
    "言",
    "省",
    "欲",
];

/// 02 §3 的五轴（名册「轴」列的合法取值）
pub const AXES: &[&str] = &[
    "主动性",
    "时域跨度",
    "抽象层级",
    "自我模型精度",
    "自主程度",
];

/// 02 §3 每条轴的**极性**：它要求什么性质、禁止什么性质。判据（`axis_orthogonality.rs`）：
/// 要求与禁止都相同 ⇒ 同一条轴（必须合并）；A 的要求命中 B 的禁止 ⇒ 两条轴互相抵消。
#[derive(Clone, Copy)]
pub struct AxisPolarity {
    pub axis: &'static str,
    pub requires: &'static str,
    pub forbids: &'static str,
}

pub const AXIS_POLARITIES: &[AxisPolarity] = &[
    AxisPolarity {
        axis: "主动性",
        requires: "unprompted",
        forbids: "prompt_only",
    },
    AxisPolarity {
        axis: "时域跨度",
        requires: "cross_session",
        forbids: "turn_local",
    },
    AxisPolarity {
        axis: "抽象层级",
        requires: "generalize",
        forbids: "literal_only",
    },
    AxisPolarity {
        axis: "自我模型精度",
        requires: "calibrate_self",
        forbids: "silent_confidence",
    },
    AxisPolarity {
        axis: "自主程度",
        requires: "unsupervised",
        forbids: "ask_each_step",
    },
];

/// 02 §3.1 刻度表的**列数** = 等级的格数（L1..Ln）。名册「等级」列的上界取自这里，
/// 程序不写死「四级」——刻度加一级而判据不动，新一级就静默落在判据之外。
pub const LEVEL_MAX: usize = 4;

/// 02 §6.1「类型」列的闭集（那张小表拥有它，程序不另抄一份）
pub const KINDS: &[&str] = &[
    "认知",
    "工程",
    "治理",
];

/// 02 §3.2 天梯每一级的「核心能力」——必须逐条是名册里**该级**的行
pub struct TierCore {
    pub tier: usize,
    pub names: &'static [&'static str],
}

pub const TIER_CORES: &[TierCore] = &[
    TierCore {
        tier: 1,
        names: &["多轮对话", "工作记忆（≈4 槽）", "流式输出"],
    },
    TierCore {
        tier: 2,
        names: &["工具接入（L1）", "时延分层（反射→自主）", "打断与插话", "人格语气一致"],
    },
    TierCore {
        tier: 3,
        names: &["情景记忆", "语义记忆", "时序历史", "长期记忆检索"],
    },
    TierCore {
        tier: 4,
        names: &["因果结构", "权限与配额", "自我校验", "自我模型", "校准（说到做到率）"],
    },
    TierCore {
        tier: 5,
        names: &["制造工具（L2）", "造工具的工具（L3）", "抽象协议设计（L4）", "对等协商", "共同目标与分工"],
    },
    TierCore {
        tier: 6,
        names: &["未来预测", "世界模型", "主动开口", "节奏与时机", "独立验证与返工", "价值偏好"],
    },
    TierCore {
        tier: 7,
        names: &["具身实时闭环"],
    },
    TierCore {
        tier: 8,
        names: &["技能编译", "反自动化回退", "目标自生成", "内在动机", "好奇与探索", "主动求知"],
    },
];

/// 02 §3.3 的共线规则：同一对轴在多少阶里同时被抬起算**超限**（上限是判断，写在文档里）
pub const OVERLAP_LIMIT: usize = 4;

/// 02 §3.3 登记的超限轴对与它声明的阶数——**声明**，与程序算出的同集合才对
pub const DECLARED_OVERLAP: &[(&str, &str, usize)] = &[
    ("时域跨度", "抽象层级", 5),
    ("抽象层级", "自我模型精度", 5),
];

/// 02 §3.1 天梯的最高级——名册「天梯」列的取值上界
pub const TIER_MAX: usize = 8;

/// 02 §6.1 名册的一行——坐标系审计的**输入**（全仓唯一一份能力名册就在文档里）
#[derive(Clone, Copy)]
pub struct Capability {
    pub name: &'static str,
    pub dim: &'static str,
    pub axis: &'static str,
    pub tier: usize,
    /// 把这条轴抬到第几级（§3.1 的刻度），不是"抬了几级"——起点是该轴当时的已知等级
    pub level: usize,
    /// 认知 / 工程 / 治理（闭集见 `KINDS`）
    pub kind: &'static str,
    /// 由哪几阶点亮（阶梯的阶号，见 T-120 §1 总表）——坐标系与实施阶梯的咬合处
    pub stages: &'static [&'static str],
}

pub const CAPABILITIES: &[Capability] = &[
    Capability {
        name: "工作记忆（≈4 槽）",
        dim: "知",
        axis: "抽象层级",
        tier: 1,
        level: 1,
        kind: "认知",
        stages: &["S01"],
    },
    Capability {
        name: "情景记忆",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
        level: 3,
        kind: "认知",
        stages: &["S06"],
    },
    Capability {
        name: "语义记忆",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
        level: 3,
        kind: "认知",
        stages: &["S06"],
    },
    Capability {
        name: "程序记忆（技能）",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
        level: 3,
        kind: "认知",
        stages: &["S11"],
    },
    Capability {
        name: "时序历史",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
        level: 3,
        kind: "认知",
        stages: &["S05"],
    },
    Capability {
        name: "因果结构",
        dim: "知",
        axis: "抽象层级",
        tier: 4,
        level: 3,
        kind: "认知",
        stages: &["S10"],
    },
    Capability {
        name: "未来预测",
        dim: "知",
        axis: "抽象层级",
        tier: 6,
        level: 4,
        kind: "认知",
        stages: &["S12"],
    },
    Capability {
        name: "世界模型",
        dim: "知",
        axis: "抽象层级",
        tier: 6,
        level: 4,
        kind: "认知",
        stages: &["S06"],
    },
    Capability {
        name: "长期记忆检索",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
        level: 3,
        kind: "工程",
        stages: &["S06"],
    },
    Capability {
        name: "主动求知",
        dim: "知",
        axis: "主动性",
        tier: 8,
        level: 4,
        kind: "认知",
        stages: &["S12"],
    },
    Capability {
        name: "工具接入（L1）",
        dim: "行",
        axis: "抽象层级",
        tier: 2,
        level: 1,
        kind: "工程",
        stages: &["S02"],
    },
    Capability {
        name: "副作用分级",
        dim: "行",
        axis: "自我模型精度",
        tier: 2,
        level: 1,
        kind: "治理",
        stages: &["S02"],
    },
    Capability {
        name: "权限与配额",
        dim: "行",
        axis: "自主程度",
        tier: 4,
        level: 2,
        kind: "治理",
        stages: &["S09"],
    },
    Capability {
        name: "幂等与重试",
        dim: "行",
        axis: "自主程度",
        tier: 2,
        level: 1,
        kind: "工程",
        stages: &["S02"],
    },
    Capability {
        name: "执行隔离",
        dim: "行",
        axis: "自主程度",
        tier: 2,
        level: 1,
        kind: "工程",
        stages: &["S02"],
    },
    Capability {
        name: "因果表征工具",
        dim: "行",
        axis: "抽象层级",
        tier: 3,
        level: 2,
        kind: "认知",
        stages: &["S09"],
    },
    Capability {
        name: "可逆性设计",
        dim: "行",
        axis: "自我模型精度",
        tier: 3,
        level: 2,
        kind: "工程",
        stages: &["S09"],
    },
    Capability {
        name: "制造工具（L2）",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
        level: 3,
        kind: "认知",
        stages: &["S13"],
    },
    Capability {
        name: "造工具的工具（L3）",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
        level: 3,
        kind: "认知",
        stages: &["S13"],
    },
    Capability {
        name: "抽象协议设计（L4）",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
        level: 4,
        kind: "认知",
        stages: &["S13"],
    },
    Capability {
        name: "具身实时闭环",
        dim: "行",
        axis: "主动性",
        tier: 7,
        level: 3,
        kind: "工程",
        stages: &["S09"],
    },
    Capability {
        name: "对等协商",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
        level: 3,
        kind: "治理",
        stages: &["S13"],
    },
    Capability {
        name: "可撤销的承诺",
        dim: "行",
        axis: "时域跨度",
        tier: 5,
        level: 3,
        kind: "治理",
        stages: &["S09"],
    },
    Capability {
        name: "多轮对话",
        dim: "言",
        axis: "抽象层级",
        tier: 1,
        level: 2,
        kind: "认知",
        stages: &["S01"],
    },
    Capability {
        name: "流式输出",
        dim: "言",
        axis: "时域跨度",
        tier: 1,
        level: 1,
        kind: "工程",
        stages: &["S01"],
    },
    Capability {
        name: "时延分层（反射→自主）",
        dim: "言",
        axis: "时域跨度",
        tier: 2,
        level: 2,
        kind: "工程",
        stages: &["S07"],
    },
    Capability {
        name: "打断与插话",
        dim: "言",
        axis: "时域跨度",
        tier: 2,
        level: 2,
        kind: "工程",
        stages: &["S07"],
    },
    Capability {
        name: "多模态",
        dim: "言",
        axis: "抽象层级",
        tier: 4,
        level: 3,
        kind: "认知",
        stages: &["S09"],
    },
    Capability {
        name: "人格语气一致",
        dim: "言",
        axis: "抽象层级",
        tier: 2,
        level: 2,
        kind: "认知",
        stages: &["S10"],
    },
    Capability {
        name: "主动开口",
        dim: "言",
        axis: "主动性",
        tier: 6,
        level: 3,
        kind: "认知",
        stages: &["S12"],
    },
    Capability {
        name: "节奏与时机",
        dim: "言",
        axis: "主动性",
        tier: 6,
        level: 4,
        kind: "认知",
        stages: &["S10"],
    },
    Capability {
        name: "规划与重规划",
        dim: "省",
        axis: "抽象层级",
        tier: 2,
        level: 2,
        kind: "认知",
        stages: &["S03"],
    },
    Capability {
        name: "自我校验",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
        level: 2,
        kind: "治理",
        stages: &["S03", "S09"],
    },
    Capability {
        name: "成本控制",
        dim: "省",
        axis: "自主程度",
        tier: 4,
        level: 2,
        kind: "治理",
        stages: &["S04"],
    },
    Capability {
        name: "自我模型",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
        level: 3,
        kind: "认知",
        stages: &["S11"],
    },
    Capability {
        name: "校准（说到做到率）",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
        level: 3,
        kind: "认知",
        stages: &["S11"],
    },
    Capability {
        name: "承诺追踪",
        dim: "省",
        axis: "时域跨度",
        tier: 4,
        level: 4,
        kind: "治理",
        stages: &["S13"],
    },
    Capability {
        name: "信任账本",
        dim: "省",
        axis: "时域跨度",
        tier: 5,
        level: 3,
        kind: "治理",
        stages: &["S13"],
    },
    Capability {
        name: "凸显仲裁",
        dim: "省",
        axis: "自主程度",
        tier: 2,
        level: 1,
        kind: "认知",
        stages: &["S04"],
    },
    Capability {
        name: "可解释",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
        level: 3,
        kind: "治理",
        stages: &["S12"],
    },
    Capability {
        name: "技能编译",
        dim: "省",
        axis: "抽象层级",
        tier: 8,
        level: 4,
        kind: "认知",
        stages: &["S11"],
    },
    Capability {
        name: "反自动化回退",
        dim: "省",
        axis: "自我模型精度",
        tier: 8,
        level: 4,
        kind: "认知",
        stages: &["S11"],
    },
    Capability {
        name: "独立验证与返工",
        dim: "省",
        axis: "自主程度",
        tier: 6,
        level: 3,
        kind: "治理",
        stages: &["S03", "S09"],
    },
    Capability {
        name: "跨体承诺",
        dim: "省",
        axis: "时域跨度",
        tier: 5,
        level: 3,
        kind: "治理",
        stages: &["S08"],
    },
    Capability {
        name: "声誉与互信",
        dim: "省",
        axis: "时域跨度",
        tier: 5,
        level: 3,
        kind: "治理",
        stages: &["S08"],
    },
    Capability {
        name: "传递性信任",
        dim: "省",
        axis: "抽象层级",
        tier: 5,
        level: 3,
        kind: "认知",
        stages: &["S08"],
    },
    Capability {
        name: "冲突与仲裁",
        dim: "省",
        axis: "自主程度",
        tier: 5,
        level: 3,
        kind: "治理",
        stages: &["S08"],
    },
    Capability {
        name: "目标自生成",
        dim: "欲",
        axis: "主动性",
        tier: 8,
        level: 4,
        kind: "认知",
        stages: &["S12"],
    },
    Capability {
        name: "内在动机",
        dim: "欲",
        axis: "主动性",
        tier: 8,
        level: 3,
        kind: "认知",
        stages: &["S12"],
    },
    Capability {
        name: "好奇与探索",
        dim: "欲",
        axis: "主动性",
        tier: 8,
        level: 3,
        kind: "认知",
        stages: &["S12"],
    },
    Capability {
        name: "回避倾向",
        dim: "欲",
        axis: "自我模型精度",
        tier: 5,
        level: 4,
        kind: "认知",
        stages: &["S12"],
    },
    Capability {
        name: "完成度追求",
        dim: "欲",
        axis: "自主程度",
        tier: 5,
        level: 2,
        kind: "认知",
        stages: &["S03"],
    },
    Capability {
        name: "价值偏好",
        dim: "欲",
        axis: "自主程度",
        tier: 6,
        level: 4,
        kind: "治理",
        stages: &["S10"],
    },
    Capability {
        name: "共同目标与分工",
        dim: "欲",
        axis: "自主程度",
        tier: 5,
        level: 3,
        kind: "治理",
        stages: &["S08"],
    },
];

/// 02 §2「条数」列的**声明**——由名册算出后被验的结论，不是输入
pub const DECLARED_DIM_COUNTS: &[(&str, usize)] = &[
    ("知", 10),
    ("行", 13),
    ("言", 8),
    ("省", 16),
    ("欲", 7),
];

/// 02 §5 表里的伪高阶说法——词表本体在文档，程序不再抄第二份
pub const PSEUDO: &[&str] = &[
    "工具数量多",
    "上下文更长",
    "反应更快",
    "支持更多模型",
    "能跑很久",
    "会写代码",
    "加了护栏 / 加了仪表盘",
];

/// 02 §6.2 声明的空格子（生长位）——必须与名册算出的空格子**同集合**
pub const DECLARED_GROWTH: &[(&str, &str)] = &[
    ("知", "自我模型精度"),
    ("知", "自主程度"),
    ("言", "自我模型精度"),
    ("言", "自主程度"),
    ("欲", "时域跨度"),
    ("欲", "抽象层级"),
    ("省", "主动性"),
];

/// 02 §7 声明的社会性能力（名 → 提升的轴）——名册里那些行的声明
pub const DECLARED_SOCIAL: &[(&str, &str)] = &[
    ("对等协商", "抽象层级"),
    ("跨体承诺", "时域跨度"),
    ("声誉与互信", "时域跨度"),
    ("传递性信任", "抽象层级"),
    ("共同目标与分工", "自主程度"),
    ("冲突与仲裁", "自主程度"),
    ("可撤销的承诺", "时域跨度"),
];

/// 路线图总表 §1 对某一阶的**声明**——是被验的结论，不是输入
pub struct StageClaim {
    pub id: &'static str,
    pub name: &'static str,
    /// 总表声明的「新增机制键」
    pub claimed_new_keys: usize,
    /// 总表声明的「参数变化」——等于该阶 §3 赋值块的行数
    pub claimed_param_changes: usize,
}

pub const STAGE_CLAIMS: &[StageClaim] = &[
    StageClaim {
        id: "S01",
        name: "最小闭环（对话基线）",
        claimed_new_keys: 6,
        claimed_param_changes: 6,
    },
    StageClaim {
        id: "S02",
        name: "工具调用与产物",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S03",
        name: "多步任务与返工",
        claimed_new_keys: 1,
        claimed_param_changes: 5,
    },
    StageClaim {
        id: "S04",
        name: "并发调度与租约",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S05",
        name: "长会话与断点恢复",
        claimed_new_keys: 1,
        claimed_param_changes: 3,
    },
    StageClaim {
        id: "S06",
        name: "长期记忆与语义检索",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S07",
        name: "插话与实时打断",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S08",
        name: "多主体与对等承诺",
        claimed_new_keys: 2,
        claimed_param_changes: 5,
    },
    StageClaim {
        id: "S09",
        name: "外部执行与熔断",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S10",
        name: "个人认知体系注入",
        claimed_new_keys: 0,
        claimed_param_changes: 5,
    },
    StageClaim {
        id: "S11",
        name: "技能编译与自我改进",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S12",
        name: "自主层与长期目标",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S13",
        name: "多智能体社会",
        claimed_new_keys: 0,
        claimed_param_changes: 5,
    },
];

/// 一阶的 §3 赋值块 + §4 退路口——阶梯审计的**输入**
pub struct StageDoc {
    pub id: &'static str,
    /// 任务目录里的那篇阶文件（文件名即 owner 声明）
    pub file: &'static str,
    pub assigns: &'static [(&'static str, &'static str)],
    /// §4 里平凡值加粗的那一行：`(机制键, 平凡值)`
    pub fallback: (&'static str, &'static str),
}

pub const STAGE_DOCS: &[StageDoc] = &[
    StageDoc {
        id: "S01",
        file: "121-最小闭环.md",
        assigns: &[
            ("event.entity", "turn"),
            ("event.verb", "closed"),
            ("actor.pattern", "reasoner"),
            ("actor.capability", "reply.first"),
            ("actor.budget_ms", "300"),
            ("projection", "snapshot"),
        ],
        fallback: ("actor.pattern", "decider"),
    },
    StageDoc {
        id: "S02",
        file: "122-工具调用与产物.md",
        assigns: &[
            ("event.entity", "artifact"),
            ("event.verb", "asserted"),
            ("actor.capability", "produce.artifact"),
            ("projection", "display"),
        ],
        fallback: ("actor.capability", "reply.append"),
    },
    StageDoc {
        id: "S03",
        file: "123-多步任务与返工.md",
        assigns: &[
            ("event.entity", "task"),
            ("event.verb", "progressed"),
            ("actor.pattern", "decider"),
            ("actor.capability", "define.work"),
            ("scope", "child:<id>"),
        ],
        fallback: ("scope", "root"),
    },
    StageDoc {
        id: "S04",
        file: "124-并发调度与租约.md",
        assigns: &[
            ("event.verb", "held"),
            ("actor.capability", "assign.work"),
            ("actor.budget_ms", "80"),
            ("projection", "readyset"),
        ],
        fallback: ("projection", "readyset:cap=1"),
    },
    StageDoc {
        id: "S05",
        file: "125-长会话与断点恢复.md",
        assigns: &[
            ("event.entity", "thread"),
            ("projection", "checkpoint"),
            ("store", "wal"),
        ],
        fallback: ("store", "memory"),
    },
    StageDoc {
        id: "S06",
        file: "126-长期记忆与语义检索.md",
        assigns: &[
            ("event.entity", "memory"),
            ("event.verb", "asserted"),
            ("actor.pattern", "translator"),
            ("projection", "recall"),
        ],
        fallback: ("projection", "snapshot"),
    },
    StageDoc {
        id: "S07",
        file: "127-插话与实时打断.md",
        assigns: &[
            ("event.entity", "control"),
            ("event.verb", "held"),
            ("actor.budget_ms", "80"),
            ("projection", "turnstate"),
        ],
        fallback: ("event.entity", "turn"),
    },
    StageDoc {
        id: "S08",
        file: "128-多主体与对等承诺.md",
        assigns: &[
            ("event.entity", "commitment"),
            ("principal", "agent:peer-b"),
            ("vis_scope", "thread_private"),
            ("projection", "reputation"),
            ("actor.pattern", "translator"),
        ],
        fallback: ("principal", "agent:main"),
    },
    StageDoc {
        id: "S09",
        file: "129-外部执行与熔断.md",
        assigns: &[
            ("event.entity", "control"),
            ("event.verb", "asserted"),
            ("actor.pattern", "decider"),
            ("projection", "budget"),
        ],
        fallback: ("actor.capability", "reply.append"),
    },
    StageDoc {
        id: "S10",
        file: "130-个人认知体系注入.md",
        assigns: &[
            ("event.entity", "memory"),
            ("event.verb", "asserted"),
            ("projection", "recall:tag=judgment"),
            ("projection", "recall:min_generality=2"),
            ("vis_scope", "shared"),
        ],
        fallback: ("projection", "recall:min_generality=0"),
    },
    StageDoc {
        id: "S11",
        file: "131-技能编译与自我改进.md",
        assigns: &[
            ("event.entity", "memory"),
            ("event.verb", "progressed"),
            ("actor.pattern", "decider"),
            ("projection", "skill_compile"),
        ],
        fallback: ("actor.pattern", "reasoner"),
    },
    StageDoc {
        id: "S12",
        file: "132-自主层与长期目标.md",
        assigns: &[
            ("event.entity", "system"),
            ("event.verb", "opened"),
            ("actor.pattern", "decider"),
            ("actor.budget_ms", "86400000"),
        ],
        fallback: ("event.entity", "turn"),
    },
    StageDoc {
        id: "S13",
        file: "133-多智能体社会.md",
        assigns: &[
            ("event.entity", "commitment"),
            ("event.verb", "asserted"),
            ("principal", "agent:market"),
            ("vis_scope", "public"),
            ("projection", "reputation"),
        ],
        fallback: ("vis_scope", "thread_private"),
    },
];

/// 认知体系的一条需求（S10 §2 的映射表，130-个人认知体系注入.md）
#[derive(Clone, Copy)]
pub struct CognitiveDemand {
    pub id: &'static str,
    pub name: &'static str,
    /// 落在 01 §8 的哪些`(键, 取值)`上；**空 = 这条需求不占参数面**（文档里以 `—` 写明）
    pub assignments: &'static [(&'static str, &'static str)],
    /// 这条需求的落点在哪一阶——认知体系横跨多阶，不是单独一阶
    pub stage: &'static str,
}

pub const COGNITIVE_DEMANDS: &[CognitiveDemand] = &[
    CognitiveDemand {
        id: "C01",
        name: "七类认知内容入库（知识/经验/技能/判断/策略/直觉/情绪）",
        assignments: &[("event.entity", "memory"), ("event.verb", "asserted"), ("projection", "recall:tag=judgment")],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C02",
        name: "内容密度金字塔（原文/要点/摘要/模式）",
        assignments: &[("projection", "recall:density=summary")],
        stage: "S10",
    },
    CognitiveDemand {
        id: "C03",
        name: "Level 0-4 通用性分级与过滤（读侧，与遗忘同构）",
        assignments: &[("projection", "recall:min_generality=2")],
        stage: "S10",
    },
    CognitiveDemand {
        id: "C04",
        name: "五层存储（原始/情景/语义/技能/元）",
        assignments: &[("event.entity", "memory"), ("projection", "recall:layer=semantic")],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C05",
        name: "激活扩散检索（关系网络多跳）",
        assignments: &[("projection", "recall:spread=on"), ("actor.pattern", "translator"), ("actor.budget_ms", "500")],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C06",
        name: "向量索引 + 关系索引 + 时间分区",
        assignments: &[],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C07",
        name: "时间衰减 + 使用反馈调权",
        assignments: &[("projection", "recall:decay=exp"), ("event.entity", "memory"), ("event.verb", "asserted")],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C08",
        name: "内外双轨动作空间（Internal / External）",
        assignments: &[("actor.capability", "produce.artifact"), ("actor.pattern", "decider")],
        stage: "S09",
    },
    CognitiveDemand {
        id: "C09",
        name: "系统 2 → 系统 1 技能编译",
        assignments: &[("projection", "skill_compile"), ("actor.pattern", "decider")],
        stage: "S11",
    },
    CognitiveDemand {
        id: "C10",
        name: "个人认知归属与隔离",
        assignments: &[("principal", "person:zhangsan"), ("vis_scope", "thread_private")],
        stage: "S08",
    },
    CognitiveDemand {
        id: "C11",
        name: "跨主体借用他人认知（人 → 智能体）",
        assignments: &[("principal", "agent:helper"), ("vis_scope", "shared")],
        stage: "S08",
    },
    CognitiveDemand {
        id: "C12",
        name: "动态学习：巩固（压缩 + 反事实）",
        assignments: &[("projection", "consolidate"), ("event.entity", "memory"), ("event.verb", "progressed"), ("actor.pattern", "reasoner")],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C13",
        name: "动态学习：遗忘（投影排除，非物理删除）",
        assignments: &[("projection", "recall"), ("event.entity", "memory"), ("event.verb", "closed")],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C14",
        name: "认知注入上下文（检索结果进提示词）",
        assignments: &[("actor.pattern", "translator"), ("projection", "recall"), ("event.entity", "memory"), ("event.verb", "asserted")],
        stage: "S06",
    },
    CognitiveDemand {
        id: "C15",
        name: "认知置信度校准（元认知）",
        assignments: &[("projection", "calibration"), ("event.entity", "verdict"), ("event.verb", "asserted")],
        stage: "S11",
    },
    CognitiveDemand {
        id: "C16",
        name: "L1-L5 能力演进（自主层 + 子作用域）",
        assignments: &[("scope", "child:long-goal"), ("actor.budget_ms", "86400000")],
        stage: "S12",
    },
];
