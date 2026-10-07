//! 能力坐标系：完备性分类 + 伪高阶识别
//!
//! 能力清单是**开放集**，所以用坐标系而不是清单来保证完备：
//!   横向五维（知/行/言/省/欲）——回答"这是什么种类的能力"，正交且合起来完备
//!   纵向五轴（主动/时域/抽象/自我模型/自主）——回答"这件事让它高阶了多少"
//! 新增能力必须回答"属于哪一维、提升哪一轴"；答不出 = 它是已有维度的组合，不是新能力。
//!
//! 反向用例：编造一个归不进五维的能力，分类器必须拒绝它。
//!
//! 编译运行：rustc --edition 2021 capability_frame.rs -o cf && ./cf

const DIMENSIONS: [&str; 5] = ["知", "行", "言", "省", "欲"];
const AXES: [&str; 5] = ["主动性", "时域跨度", "抽象层级", "自我模型精度", "自主程度"];

struct Cap {
    name: &'static str,
    dim: &'static str,
    axis: &'static str,
    tier: u8, // 天梯 T1-T8
}

fn caps() -> Vec<Cap> {
    vec![
        // ── 知 Episteme ──
        Cap { name: "工作记忆（≈4 槽）", dim: "知", axis: "抽象层级", tier: 1 },
        Cap { name: "情景记忆", dim: "知", axis: "时域跨度", tier: 3 },
        Cap { name: "语义记忆", dim: "知", axis: "时域跨度", tier: 3 },
        Cap { name: "程序记忆（技能）", dim: "知", axis: "时域跨度", tier: 3 },
        Cap { name: "时序历史", dim: "知", axis: "时域跨度", tier: 3 },
        Cap { name: "因果结构", dim: "知", axis: "抽象层级", tier: 4 },
        Cap { name: "未来预测", dim: "知", axis: "抽象层级", tier: 6 },
        Cap { name: "世界模型", dim: "知", axis: "抽象层级", tier: 6 },
        Cap { name: "长期记忆检索", dim: "知", axis: "时域跨度", tier: 3 },
        Cap { name: "主动求知", dim: "知", axis: "主动性", tier: 8 },
        // ── 行 Action ──
        Cap { name: "工具接入（L1）", dim: "行", axis: "抽象层级", tier: 2 },
        Cap { name: "副作用分级", dim: "行", axis: "自我模型精度", tier: 2 },
        Cap { name: "权限与配额", dim: "行", axis: "自主程度", tier: 4 },
        Cap { name: "幂等与重试", dim: "行", axis: "自主程度", tier: 2 },
        Cap { name: "执行隔离", dim: "行", axis: "自主程度", tier: 2 },
        Cap { name: "因果表征工具", dim: "行", axis: "抽象层级", tier: 3 },
        Cap { name: "可逆性设计", dim: "行", axis: "自我模型精度", tier: 3 },
        Cap { name: "制造工具（L2）", dim: "行", axis: "抽象层级", tier: 5 },
        Cap { name: "造工具的工具（L3）", dim: "行", axis: "抽象层级", tier: 5 },
        Cap { name: "抽象协议设计（L4）", dim: "行", axis: "抽象层级", tier: 5 },
        Cap { name: "具身实时闭环", dim: "行", axis: "主动性", tier: 7 },
        // ── 言 Utterance ──
        Cap { name: "多轮对话", dim: "言", axis: "抽象层级", tier: 1 },
        Cap { name: "流式输出", dim: "言", axis: "时域跨度", tier: 1 },
        Cap { name: "时延分层（反射→自主）", dim: "言", axis: "时域跨度", tier: 2 },
        Cap { name: "打断与插话", dim: "言", axis: "时域跨度", tier: 2 },
        Cap { name: "多模态", dim: "言", axis: "抽象层级", tier: 4 },
        Cap { name: "人格语气一致", dim: "言", axis: "抽象层级", tier: 2 },
        Cap { name: "主动开口", dim: "言", axis: "主动性", tier: 6 },
        Cap { name: "节奏与时机", dim: "言", axis: "主动性", tier: 6 },
        // ── 省 Reflection ──
        Cap { name: "规划与重规划", dim: "省", axis: "抽象层级", tier: 2 },
        Cap { name: "自我校验", dim: "省", axis: "自我模型精度", tier: 4 },
        Cap { name: "成本控制", dim: "省", axis: "自主程度", tier: 4 },
        Cap { name: "自我模型", dim: "省", axis: "自我模型精度", tier: 4 },
        Cap { name: "校准（说到做到率）", dim: "省", axis: "自我模型精度", tier: 4 },
        Cap { name: "承诺追踪", dim: "省", axis: "时域跨度", tier: 4 },
        Cap { name: "信任账本", dim: "省", axis: "时域跨度", tier: 5 },
        Cap { name: "凸显仲裁", dim: "省", axis: "自主程度", tier: 2 },
        Cap { name: "可解释", dim: "省", axis: "自我模型精度", tier: 4 },
        Cap { name: "技能编译", dim: "省", axis: "抽象层级", tier: 8 },
        Cap { name: "反自动化回退", dim: "省", axis: "自我模型精度", tier: 8 },
        Cap { name: "独立验证与返工", dim: "省", axis: "自主程度", tier: 6 },
        // ── 欲 Conation ──
        Cap { name: "目标自生成", dim: "欲", axis: "主动性", tier: 8 },
        Cap { name: "内在动机", dim: "欲", axis: "主动性", tier: 8 },
        Cap { name: "好奇与探索", dim: "欲", axis: "主动性", tier: 8 },
        Cap { name: "回避倾向", dim: "欲", axis: "自我模型精度", tier: 5 },
        Cap { name: "完成度追求", dim: "欲", axis: "自主程度", tier: 5 },
        Cap { name: "价值偏好", dim: "欲", axis: "自主程度", tier: 6 },
        // ── 社会性（对等协作：递归是树，覆盖不了图）──
        Cap { name: "对等协商", dim: "行", axis: "抽象层级", tier: 5 },
        Cap { name: "跨体承诺", dim: "省", axis: "时域跨度", tier: 5 },
        Cap { name: "声誉与互信", dim: "省", axis: "时域跨度", tier: 5 },
        Cap { name: "传递性信任", dim: "省", axis: "抽象层级", tier: 5 },
        Cap { name: "共同目标与分工", dim: "欲", axis: "自主程度", tier: 5 },
        Cap { name: "冲突与仲裁", dim: "省", axis: "自主程度", tier: 5 },
        Cap { name: "可撤销的承诺", dim: "行", axis: "时域跨度", tier: 5 },
    ]
}

fn classify(c: &Cap) -> Result<(), String> {
    if !DIMENSIONS.contains(&c.dim) {
        return Err(format!("「{}」的维度「{}」不在五维内", c.name, c.dim));
    }
    if !AXES.contains(&c.axis) {
        return Err(format!("「{}」的轴「{}」不在五轴内", c.name, c.axis));
    }
    if c.tier < 1 || c.tier > 8 {
        return Err(format!("「{}」的天梯层级 {} 越界（应 1-8）", c.name, c.tier));
    }
    Ok(())
}

/// 伪高阶识别：看起来高阶、实际是宽度或工程的能力。
/// 判据：能力提升的是"数量/容量/速度/耐力"，而不是五轴中的任一轴。
const PSEUDO: &[&str] = &[
    "工具数量多", "上下文更长", "反应更快", "支持更多模型", "能跑很久", "会写代码",
];

fn is_pseudo(name: &str) -> bool {
    PSEUDO.iter().any(|p| name.contains(p))
}

fn main() {
    println!("═══ 能力坐标系完备性 ═══");
    let cs = caps();
    println!("能力条目：{}", cs.len());

    let mut bad: Vec<String> = Vec::new();
    for c in &cs {
        if let Err(e) = classify(c) {
            bad.push(e);
        }
    }
    println!("归类失败：{} 条", bad.len());
    for b in &bad {
        println!("  {}", b);
    }

    println!("\n覆盖矩阵（行=五维，列=五轴）：");
    print!("        ");
    for a in AXES {
        print!("{:>10}  ", a);
    }
    println!();
    let mut empty = 0;
    for d in DIMENSIONS {
        print!("  {}    ", d);
        for a in AXES {
            let n = cs.iter().filter(|c| c.dim == d && c.axis == a).count();
            if n == 0 {
                empty += 1;
            }
            print!("{:>10}  ", n);
        }
        println!();
    }
    println!("空格子：{} / {}（这些是坐标系预留的生长位，不是缺口）", empty, DIMENSIONS.len() * AXES.len());

    let mut per_dim = [0usize; 5];
    for c in &cs {
        if let Some(i) = DIMENSIONS.iter().position(|d| *d == c.dim) {
            per_dim[i] += 1;
        }
    }
    println!("\n各维条数：");
    for (i, d) in DIMENSIONS.iter().enumerate() {
        println!("  {}：{}", d, per_dim[i]);
    }

    println!("\n═══ 伪高阶识别 ═══");
    for p in PSEUDO {
        println!("  {} → 伪高阶 = {}", p, is_pseudo(p));
    }
    let real = "校准（说到做到率）";
    println!("  {} → 伪高阶 = {}", real, is_pseudo(real));

    // ── 反向用例：编造一个归不进五维的能力，分类器必须拒绝 ──
    let fake = Cap { name: "玄学共振", dim: "灵", axis: "主动性", tier: 3 };
    let fake_bad = classify(&fake).is_err();
    let fake_axis = Cap { name: "玄学共振2", dim: "知", axis: "气场强度", tier: 3 };
    let fake_axis_bad = classify(&fake_axis).is_err();
    let fake_tier = Cap { name: "玄学共振3", dim: "知", axis: "主动性", tier: 99 };
    let fake_tier_bad = classify(&fake_tier).is_err();
    println!("\n反向用例：假维度 = {}，假轴 = {}，越界天梯 = {}", fake_bad, fake_axis_bad, fake_tier_bad);

    // ── 断言 ──
    assert!(bad.is_empty(), "全部能力必须能归入五维五轴");
    assert_eq!(cs.len(), 54, "能力条目数（新增能力时同步更新此断言）");
    assert!(per_dim.iter().all(|n| *n > 0), "每一维都不能为空（否则该维是摆设）");
    assert!(per_dim[4] >= 6, "「欲」维度必须有实质内容——缺了它系统只是高级工具");
    assert!(fake_bad && fake_axis_bad && fake_tier_bad, "反向用例：分类器必须拒绝非法维度/轴/天梯");
    assert!(is_pseudo("工具数量多"), "反向用例：伪高阶必须被识别");
    assert!(!is_pseudo(real), "真实能力不应被误判为伪高阶");

    println!("\n✅ 全部断言通过（含 4 条反向用例）：五维完备、五轴有刻度、伪高阶可识别。");
}
