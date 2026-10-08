// 生成物：由 `scripts/gen-verify-facts.mjs` 从 docs/plan 抽取，**不要手改**。
// 要改这些数据就改文档（01 §8 / 路线图总表 / 各阶 §3·§4），然后重跑生成脚本。
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
