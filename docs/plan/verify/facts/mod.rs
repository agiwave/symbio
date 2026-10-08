// 生成物：由 `scripts/gen-verify-facts.mjs` 从 docs/plan 抽取，**不要手改**。
// 要改这些数据就改文档（01 §8 / 02 坐标系 / 路线图总表 / 各阶 §3·§4），再重跑生成脚本。
//
// 每个 verify 程序都是一个**独立的 crate**，各自只 `use` 下面的一部分；
// 没被某个程序读到的那些不是死码，是另一个程序在读。这里判死码只会制造噪音，
// 所以整模块关掉这条 lint——纯数据模块没有逻辑，关掉不掩盖任何真实缺陷。
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
        forms: &["memory", "wal", "sharded", "distributed"],
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
        forms: &["80", "300", "60000", "86400000"],
        open: false,
    },
    Domain {
        key: "event.entity",
        forms: &["turn", "task", "artifact", "verdict", "memory", "commitment", "conation", "thread", "control", "system"],
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

/// 02 §3.1 天梯的最高级——名册「天梯」列的取值上界
pub const TIER_MAX: usize = 8;

/// 02 §6.1 名册的一行——坐标系审计的**输入**（全仓唯一一份能力名册就在文档里）
pub struct Capability {
    pub name: &'static str,
    pub dim: &'static str,
    pub axis: &'static str,
    pub tier: usize,
}

pub const CAPABILITIES: &[Capability] = &[
    Capability {
        name: "工作记忆（≈4 槽）",
        dim: "知",
        axis: "抽象层级",
        tier: 1,
    },
    Capability {
        name: "情景记忆",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
    },
    Capability {
        name: "语义记忆",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
    },
    Capability {
        name: "程序记忆（技能）",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
    },
    Capability {
        name: "时序历史",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
    },
    Capability {
        name: "因果结构",
        dim: "知",
        axis: "抽象层级",
        tier: 4,
    },
    Capability {
        name: "未来预测",
        dim: "知",
        axis: "抽象层级",
        tier: 6,
    },
    Capability {
        name: "世界模型",
        dim: "知",
        axis: "抽象层级",
        tier: 6,
    },
    Capability {
        name: "长期记忆检索",
        dim: "知",
        axis: "时域跨度",
        tier: 3,
    },
    Capability {
        name: "主动求知",
        dim: "知",
        axis: "主动性",
        tier: 8,
    },
    Capability {
        name: "工具接入（L1）",
        dim: "行",
        axis: "抽象层级",
        tier: 2,
    },
    Capability {
        name: "副作用分级",
        dim: "行",
        axis: "自我模型精度",
        tier: 2,
    },
    Capability {
        name: "权限与配额",
        dim: "行",
        axis: "自主程度",
        tier: 4,
    },
    Capability {
        name: "幂等与重试",
        dim: "行",
        axis: "自主程度",
        tier: 2,
    },
    Capability {
        name: "执行隔离",
        dim: "行",
        axis: "自主程度",
        tier: 2,
    },
    Capability {
        name: "因果表征工具",
        dim: "行",
        axis: "抽象层级",
        tier: 3,
    },
    Capability {
        name: "可逆性设计",
        dim: "行",
        axis: "自我模型精度",
        tier: 3,
    },
    Capability {
        name: "制造工具（L2）",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
    },
    Capability {
        name: "造工具的工具（L3）",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
    },
    Capability {
        name: "抽象协议设计（L4）",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
    },
    Capability {
        name: "具身实时闭环",
        dim: "行",
        axis: "主动性",
        tier: 7,
    },
    Capability {
        name: "多轮对话",
        dim: "言",
        axis: "抽象层级",
        tier: 1,
    },
    Capability {
        name: "流式输出",
        dim: "言",
        axis: "时域跨度",
        tier: 1,
    },
    Capability {
        name: "时延分层（反射→自主）",
        dim: "言",
        axis: "时域跨度",
        tier: 2,
    },
    Capability {
        name: "打断与插话",
        dim: "言",
        axis: "时域跨度",
        tier: 2,
    },
    Capability {
        name: "多模态",
        dim: "言",
        axis: "抽象层级",
        tier: 4,
    },
    Capability {
        name: "人格语气一致",
        dim: "言",
        axis: "抽象层级",
        tier: 2,
    },
    Capability {
        name: "主动开口",
        dim: "言",
        axis: "主动性",
        tier: 6,
    },
    Capability {
        name: "节奏与时机",
        dim: "言",
        axis: "主动性",
        tier: 6,
    },
    Capability {
        name: "规划与重规划",
        dim: "省",
        axis: "抽象层级",
        tier: 2,
    },
    Capability {
        name: "自我校验",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
    },
    Capability {
        name: "成本控制",
        dim: "省",
        axis: "自主程度",
        tier: 4,
    },
    Capability {
        name: "自我模型",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
    },
    Capability {
        name: "校准（说到做到率）",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
    },
    Capability {
        name: "承诺追踪",
        dim: "省",
        axis: "时域跨度",
        tier: 4,
    },
    Capability {
        name: "信任账本",
        dim: "省",
        axis: "时域跨度",
        tier: 5,
    },
    Capability {
        name: "凸显仲裁",
        dim: "省",
        axis: "自主程度",
        tier: 2,
    },
    Capability {
        name: "可解释",
        dim: "省",
        axis: "自我模型精度",
        tier: 4,
    },
    Capability {
        name: "技能编译",
        dim: "省",
        axis: "抽象层级",
        tier: 8,
    },
    Capability {
        name: "反自动化回退",
        dim: "省",
        axis: "自我模型精度",
        tier: 8,
    },
    Capability {
        name: "独立验证与返工",
        dim: "省",
        axis: "自主程度",
        tier: 6,
    },
    Capability {
        name: "目标自生成",
        dim: "欲",
        axis: "主动性",
        tier: 8,
    },
    Capability {
        name: "内在动机",
        dim: "欲",
        axis: "主动性",
        tier: 8,
    },
    Capability {
        name: "好奇与探索",
        dim: "欲",
        axis: "主动性",
        tier: 8,
    },
    Capability {
        name: "回避倾向",
        dim: "欲",
        axis: "自我模型精度",
        tier: 5,
    },
    Capability {
        name: "完成度追求",
        dim: "欲",
        axis: "自主程度",
        tier: 5,
    },
    Capability {
        name: "价值偏好",
        dim: "欲",
        axis: "自主程度",
        tier: 6,
    },
    Capability {
        name: "对等协商",
        dim: "行",
        axis: "抽象层级",
        tier: 5,
    },
    Capability {
        name: "跨体承诺",
        dim: "省",
        axis: "时域跨度",
        tier: 5,
    },
    Capability {
        name: "声誉与互信",
        dim: "省",
        axis: "时域跨度",
        tier: 5,
    },
    Capability {
        name: "传递性信任",
        dim: "省",
        axis: "抽象层级",
        tier: 5,
    },
    Capability {
        name: "共同目标与分工",
        dim: "欲",
        axis: "自主程度",
        tier: 5,
    },
    Capability {
        name: "冲突与仲裁",
        dim: "省",
        axis: "自主程度",
        tier: 5,
    },
    Capability {
        name: "可撤销的承诺",
        dim: "行",
        axis: "时域跨度",
        tier: 5,
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
    pub tier: &'static str,
    /// 总表声明的「新增机制键」
    pub claimed_new_keys: usize,
    /// 总表声明的「参数变化」——等于该阶 §3 赋值块的行数
    pub claimed_param_changes: usize,
}

pub const STAGE_CLAIMS: &[StageClaim] = &[
    StageClaim {
        id: "S01",
        name: "最小闭环（对话基线）",
        tier: "T1",
        claimed_new_keys: 6,
        claimed_param_changes: 6,
    },
    StageClaim {
        id: "S02",
        name: "工具调用与产物",
        tier: "T1–T2",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S03",
        name: "多步任务与返工",
        tier: "T2",
        claimed_new_keys: 1,
        claimed_param_changes: 5,
    },
    StageClaim {
        id: "S04",
        name: "并发调度与租约",
        tier: "T3",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S05",
        name: "长会话与断点恢复",
        tier: "T3",
        claimed_new_keys: 1,
        claimed_param_changes: 3,
    },
    StageClaim {
        id: "S06",
        name: "长期记忆与语义检索",
        tier: "T3",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S07",
        name: "插话与实时打断",
        tier: "T3–T4",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S08",
        name: "多主体与对等承诺",
        tier: "T4–T5",
        claimed_new_keys: 2,
        claimed_param_changes: 5,
    },
    StageClaim {
        id: "S09",
        name: "外部执行与熔断",
        tier: "T5–T6",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S10",
        name: "个人认知体系注入",
        tier: "T4–T6",
        claimed_new_keys: 0,
        claimed_param_changes: 5,
    },
    StageClaim {
        id: "S11",
        name: "技能编译与自我改进",
        tier: "T6",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S12",
        name: "自主层与长期目标",
        tier: "T7",
        claimed_new_keys: 0,
        claimed_param_changes: 4,
    },
    StageClaim {
        id: "S13",
        name: "多智能体社会",
        tier: "T8",
        claimed_new_keys: 0,
        claimed_param_changes: 5,
    },
];

/// 一阶的 §3 赋值块 + §4 退路口——阶梯审计的**输入**
pub struct StageDoc {
    pub id: &'static str,
    /// roadmap 目录里的那篇（文件名即 owner 声明）
    pub file: &'static str,
    pub assigns: &'static [(&'static str, &'static str)],
    /// §4 里平凡值加粗的那一行：`(机制键, 平凡值)`
    pub fallback: (&'static str, &'static str),
}

pub const STAGE_DOCS: &[StageDoc] = &[
    StageDoc {
        id: "S01",
        file: "S01-最小闭环.md",
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
        file: "S02-工具调用与产物.md",
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
        file: "S03-多步任务与返工.md",
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
        file: "S04-并发调度与租约.md",
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
        file: "S05-长会话与断点恢复.md",
        assigns: &[
            ("event.entity", "thread"),
            ("projection", "checkpoint"),
            ("store", "wal"),
        ],
        fallback: ("store", "memory"),
    },
    StageDoc {
        id: "S06",
        file: "S06-长期记忆与语义检索.md",
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
        file: "S07-插话与实时打断.md",
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
        file: "S08-多主体与对等承诺.md",
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
        file: "S09-外部执行与熔断.md",
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
        file: "S10-个人认知体系注入.md",
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
        file: "S11-技能编译与自我改进.md",
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
        file: "S12-自主层与长期目标.md",
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
        file: "S13-多智能体社会.md",
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
