//! `authz` 单测 —— 钉死**本机授权表的形状**（04 §3.1 批⑥ 的生产接线）。
//!
//! 这里不复述 core 的判定语义（`governance/mod.test.rs` 钉成对性、闭集校验与
//! 轮次→能力的映射），只钉四件只有这张表能回答的事：
//!
//! 1. `agent:main` 持 6 项、**不持验证**（C15 不自验），首响与追加发言都放行；
//! 2. **自主发起者那一行比主智能体窄**（只持 `define.work`、不持 `reply.*`，
//!    S12 §3/§5「自主行为不得冒充用户对话」）；
//! 3. **抢占判定者那一行比主智能体窄**（只持 `judge.intent`、不持 `reply.*`，
//!    S07 §5「判定者不得直接发言」）；
//! 4. **对等主体那一行比主智能体窄**（只持 `produce.artifact`，S08 §3——第一个
//!    对等/图侧主体，窄于主智能体那一行）；
//! 5. 取表口只有一条口径（[ADR-099]）：**表是权威**，登记过的主体拿自己那一行，
//!    未登记的主体在表里没有行 ⇒ 每个谓词为假（fail-closed）；
//! 6. 读写两侧的闸按**属主 / 矩阵成员**分：主智能体与属主各得其所，`user` 不入表
//!    （发言不是能力），矩阵外主体拒绝；表构造失败时的降级面（空矩阵）一概拒绝
//!    ——fail-closed 不因降级而松开；
//! 7. **工人 / 规划者 / 验证者三行比主智能体窄**（S02/S03 三角色，T-107 行与路径
//!    同批）：各自单能力独持，`production_matrix` 拿登记行，C15 两面（规划者不认证、
//!    验证者不产出）在表这一层可见；
//! 8. **绑定即登记**（[ADR-100]）：未部署的主体经绑定拿到来源行——能力随绑定来、
//!    **可见域不登记**、静态行不被放大、淘汰面只认部署表、委派上限两侧都在册；
//! 9. **委派取交集**（[T-195]）：子能力 = 父能力 ∩ 子主体的显式声明，父窄子宽时
//!    子只保留父也持有的能力，空交集登记为「什么都不许」而不是退回部署行。

use super::*;
use crate::symbio_core::governance::Capability;
use crate::symbio_core::view::is_agent_id;

/// 表的形状：主智能体持 5 项、缺 `assert.verification`，可见域 `thread_private`。
#[test]
fn main_agent_row_is_the_documented_shape() {
    let matrix = production_matrix();
    assert_eq!(
        matrix.grants_of(PRINCIPAL_MAIN).len(),
        5,
        "judge.intent / reply / define.work / produce.artifact / assign.work"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_MAIN),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );
}

/// 自主发起者那一行（S12 §3）：**比主智能体窄**——只持 `define.work`，不持任何 `reply.*`。
///
/// 这一行是本表里第一条「少给」的记录：它是这一主体的**全部能力来源**——
/// 「自主行为不得冒充用户对话」与断言同方向（ADR-099：未登记 = 拒，窄行是唯一入口）。
#[test]
fn the_autonomous_row_is_narrower_than_main() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_AUTONOMOUS);
    assert_eq!(caps.len(), 1, "只持 define.work：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_AUTONOMOUS, Capability::DefineWork),
        "自主层必须能定义工作"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_AUTONOMOUS) && !matrix.can_reply(PRINCIPAL_AUTONOMOUS),
        "自主行为不得冒充用户对话（首响与追加都不放行）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_AUTONOMOUS),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );
}

/// 抢占判定者那一行（S07 §5）：**比主智能体窄**——只持 `judge.intent`，不持任何 `reply.*`。
///
/// 判据落在**生产表**上（`production_matrix`），不再是自造矩阵：自造矩阵只证明
/// 「我拒我」，表里少一行它照样绿。ADR-099 之后方向与断言同向：表是权威，
/// 这一行就是判定者的全部能力来源。
#[test]
fn the_reflex_row_is_narrower_than_main() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_REFLEX);
    assert_eq!(caps.len(), 1, "只持 judge.intent：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_REFLEX, Capability::JudgeIntent),
        "判定者必须能判决意图，否则抢占判定这一步直接失效"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_REFLEX) && !matrix.can_reply(PRINCIPAL_REFLEX),
        "判定者不得直接发言（首响与追加都不放行，S07 §5）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_REFLEX),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );

    // 反向判据：`production_matrix` 拿到的必须是上面那一行（窄）。去掉 ROWS 里
    // 那一行，这条断言立刻变红（表里没有行 ⇒ 什么也拿不到，不是换一套能力）。
    assert_eq!(
        matrix.grants_of(PRINCIPAL_REFLEX).len(),
        1,
        "判定者这一行只有 judge.intent，不许多出 reply.*"
    );
    assert!(!matrix.can_reply(PRINCIPAL_REFLEX));
}

/// 对等主体那一行（S08 §3）：**第一个对等（图侧）主体的窄行**——只持 `produce.artifact`，
/// 严格窄于主智能体。
///
/// S08 §3.1 的纪律：对等主体出现必须**自带窄行同批落地**——窄行是它唯一的入口
/// （ADR-099：表是权威，未登记 = 拒）。
#[test]
fn the_peer_row_is_narrower_than_main() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_PEER);
    assert_eq!(caps.len(), 1, "只持 produce.artifact：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_PEER, Capability::ProduceArtifact),
        "对等体的权威动作是产生产物（S08 §3 的 ActorSpec 行），否则这一行没有理由存在"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_PEER) && !matrix.can_reply(PRINCIPAL_PEER),
        "对等体的协作经 commitment.* 而非 chat.*——不以本机主控的身份发言"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_PEER),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );

    // 「严格窄于主智能体那一行」：主智能体持 6 项、对等行 1 项。去掉 ROWS 那一行，
    // 断言变红（表里没有行 ⇒ 什么也拿不到）。
    let main_len = matrix.grants_of(PRINCIPAL_MAIN).len();
    assert!(
        caps.len() < main_len,
        "对等行必须严格窄于主智能体那一行（{main_len} 项）"
    );
    assert!(!matrix.can_reply(PRINCIPAL_PEER));
    assert!(
        matrix.can_write(PRINCIPAL_PEER, Capability::ProduceArtifact),
        "登记行是权威：对等体仍持它那一项能力"
    );
}

/// 组织形态三角色（S13 §3，T-073）：市场 / 仲裁 / 守门三行**各自单能力独持**，
/// 且都严格窄于主智能体那一行。
///
/// 与自主 / 判定 / 对等三行同一条纪律：主体出现必须自带窄行同批落地——窄行是唯一
/// 入口（ADR-099），「市场不得冒充主控发言」「仲裁者不自证」「守门者先判后发声」
/// 与断言同方向。
/// 另钉 exit ① 的表半边：arbiter 持 `assert.verification` **不碰** C15（不持
/// `define.work` / `produce.artifact`，构造期双持检查见 `governance`）。
#[test]
fn the_three_society_rows_are_narrower_than_main() {
    let matrix = production_matrix();
    let main_len = matrix.grants_of(PRINCIPAL_MAIN).len();

    // 市场：只派活（assign.work），公告是公共域事实（vis_scope = public）。
    let market = matrix.grants_of(PRINCIPAL_MARKET);
    assert_eq!(market.len(), 1, "只持 assign.work：{market:?}");
    assert!(matrix.can_write(PRINCIPAL_MARKET, Capability::AssignWork));
    assert!(
        !matrix.can_reply(PRINCIPAL_MARKET) && !matrix.can_reply(PRINCIPAL_MARKET),
        "市场只派活、不冒充主控发言"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_MARKET),
        Some(VisScope::Public),
        "S13 §3：市场公告 vis_scope = public——取这个值的第一行"
    );

    // 仲裁者：只盖章（assert.verification），本表验证能力的持有者之一（另一行 = verifier，T-107）。
    let arbiter = matrix.grants_of(PRINCIPAL_ARBITER);
    assert_eq!(arbiter.len(), 1, "只持 assert.verification：{arbiter:?}");
    assert!(matrix.can_write(PRINCIPAL_ARBITER, Capability::AssertVerification));
    assert!(
        !matrix.can_write(PRINCIPAL_ARBITER, Capability::DefineWork)
            && !matrix.can_write(PRINCIPAL_ARBITER, Capability::ProduceArtifact),
        "C15：仲裁者不持产出/定义 ⇒ 不成对自验（构造期同款检查）"
    );
    assert!(!matrix.can_reply(PRINCIPAL_ARBITER));
    assert_eq!(
        matrix.sees_of(PRINCIPAL_ARBITER),
        Some(VisScope::Public),
        "裁决是集体决策的产物：public"
    );

    // 守门者：只判来者意图（judge.intent），判定是私有工作记录。
    let keeper = matrix.grants_of(PRINCIPAL_GATEKEEPER);
    assert_eq!(keeper.len(), 1, "只持 judge.intent：{keeper:?}");
    assert!(matrix.can_write(PRINCIPAL_GATEKEEPER, Capability::JudgeIntent));
    assert!(
        !matrix.can_reply(PRINCIPAL_GATEKEEPER) && !matrix.can_reply(PRINCIPAL_GATEKEEPER),
        "判定者不得直接发言（与 reflex 行同款独持先例）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_GATEKEEPER),
        Some(VisScope::ThreadPrivate),
        "守门判定是判定者自己的工作记录，不是公共域公告"
    );

    // 三行都严格窄于主智能体那一行，`production_matrix` 拿的是**登记行**。
    for (p, cap) in [
        (PRINCIPAL_MARKET, Capability::AssignWork),
        (PRINCIPAL_ARBITER, Capability::AssertVerification),
        (PRINCIPAL_GATEKEEPER, Capability::JudgeIntent),
    ] {
        assert!(
            matrix.grants_of(p).len() < main_len,
            "{p} 必须严格窄于主智能体那一行（{main_len} 项）"
        );
        assert_eq!(
            matrix.grants_of(p).len(),
            1,
            "{p} 这一行只有它那一项能力，不许多出主智能体的其余能力"
        );
        assert!(
            matrix.can_write(p, cap),
            "登记行是权威：{p} 仍持它那一项能力"
        );
        assert!(!matrix.can_reply(p));
    }
}

/// S02/S03 三角色行（T-107，行与执行路径同批）：工人 / 规划者 / 验证者三行
/// **各自单能力独持**，严格窄于主智能体那一行。
///
/// 与自主 / 判定 / 对等 / 市场 / 仲裁 / 守门六行同一条纪律：主体出现必须自带窄行
/// 同批落地——窄行是唯一入口（ADR-099），「工人不以主控身份发言」「规划者不自我
/// 认证」「产出者不得既产出又认证」与断言同方向。
///
/// 另钉两条 C15 的结构面：planner 持 `define.work` 而不持 `assert.verification`
/// （不自我认证），verifier 持 `assert.verification` 而不持 `define.work` /
/// `produce.artifact` （不既产出又认证）——「二者互不可写」在**表**这一层可见，
/// 构造期的双持拒绝在 `governance` 那一侧。
#[test]
fn the_three_role_rows_are_narrower_than_main() {
    let matrix = production_matrix();
    let main_len = matrix.grants_of(PRINCIPAL_MAIN).len();

    // 工人：只产生产物（S02 worker 行），树侧委派链上以产物收束的子主体。
    let worker = matrix.grants_of(PRINCIPAL_WORKER);
    assert_eq!(worker.len(), 1, "只持 produce.artifact：{worker:?}");
    assert!(matrix.can_write(PRINCIPAL_WORKER, Capability::ProduceArtifact));
    assert!(
        !matrix.can_reply(PRINCIPAL_WORKER) && !matrix.can_reply(PRINCIPAL_WORKER),
        "工人以产物收束（收束闸的产物臂，ADR-067），不以主控身份发言"
    );
    assert!(
        !matrix.can_write(PRINCIPAL_WORKER, Capability::DefineWork)
            && !matrix.can_write(PRINCIPAL_WORKER, Capability::AssertVerification),
        "工人不立任务也不认证——立与验归工作权威（C15 的精神在任务表层）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_WORKER),
        Some(VisScope::ThreadPrivate),
        "工人的产物是会话自己的工作记录，不是公共域公告"
    );

    // 规划者：只立任务（S03 规划者行），与自主行同款独持先例。
    let planner = matrix.grants_of(PRINCIPAL_PLANNER);
    assert_eq!(planner.len(), 1, "只持 define.work：{planner:?}");
    assert!(matrix.can_write(PRINCIPAL_PLANNER, Capability::DefineWork));
    assert!(
        !matrix.can_write(PRINCIPAL_PLANNER, Capability::AssertVerification),
        "C15：规划者不持验证能力 ⇒ 不自我认证"
    );
    assert!(!matrix.can_reply(PRINCIPAL_PLANNER));
    assert_eq!(
        matrix.sees_of(PRINCIPAL_PLANNER),
        Some(VisScope::ThreadPrivate)
    );

    // 验证者：只独立判定（S03 验证者行），与仲裁行同持验证能力而不同角色。
    let verifier = matrix.grants_of(PRINCIPAL_VERIFIER);
    assert_eq!(verifier.len(), 1, "只持 assert.verification：{verifier:?}");
    assert!(matrix.can_write(PRINCIPAL_VERIFIER, Capability::AssertVerification));
    assert!(
        !matrix.can_write(PRINCIPAL_VERIFIER, Capability::DefineWork)
            && !matrix.can_write(PRINCIPAL_VERIFIER, Capability::ProduceArtifact),
        "C15：验证者不持产出/定义 ⇒ 不成对自验（构造期同款检查）"
    );
    assert!(!matrix.can_reply(PRINCIPAL_VERIFIER));
    assert_eq!(
        matrix.sees_of(PRINCIPAL_VERIFIER),
        Some(VisScope::ThreadPrivate),
        "验证判定是判定者自己的工作记录（与守门行同一条理由），不是公共域公告"
    );

    // 三行都严格窄于主智能体那一行，`production_matrix` 拿的是**登记行**。
    for (p, cap) in [
        (PRINCIPAL_WORKER, Capability::ProduceArtifact),
        (PRINCIPAL_PLANNER, Capability::DefineWork),
        (PRINCIPAL_VERIFIER, Capability::AssertVerification),
    ] {
        assert!(
            matrix.grants_of(p).len() < main_len,
            "{p} 必须严格窄于主智能体那一行（{main_len} 项）"
        );
        assert_eq!(
            matrix.grants_of(p).len(),
            1,
            "{p} 这一行只有它那一项能力，不许多出主智能体的其余能力"
        );
        assert!(
            matrix.can_write(p, cap),
            "登记行是权威：{p} 仍持它那一项能力"
        );
        assert!(!matrix.can_reply(p));
    }
}

/// 收束格按**种类**判（[ADR-067](../../../docs/010-adr/067.md "ADR-067")，T-072 的单测判据）：
/// 产物收束问 `produce.artifact`、发言收束仍按 `reply.*`——同一张表的两道闸。
///
/// 钉的是谓词面的**分派**，不是每个主体自己的窄行（那些在上面各测里）：对等主体
/// 产物闸放行、发言闸仍拒；主智能体两道都放；判定者 / 自主层产物闸也拒（它们
/// 持的不是 `produce.artifact`）。没有这道分派，对等主体跑一轮永远收束不了
/// （ADR-065 的批②③ 合并理由）；反过来若产物闸误用了 `can_reply`，对等主体的
/// 产物也落不了格——两种错位都被这条钉住。
#[test]
fn the_closure_gate_is_dispatched_by_kind() {
    let matrix = production_matrix();

    // 对等主体：产物收束放行、发言收束仍拒（「没有发言权」≠「留不下任何事实」）。
    assert!(
        matrix.can_produce_artifact(PRINCIPAL_PEER),
        "对等体以产物收束一轮（ADR-065 裁决的落地前提）"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_PEER),
        "产物闸放行不改变发言闸：对等体仍无发言权"
    );

    // 主智能体：两道都放（发言与产物都是它的权威动作）。
    assert!(matrix.can_reply(PRINCIPAL_MAIN));
    assert!(matrix.can_produce_artifact(PRINCIPAL_MAIN));

    // 判定者 / 自主层：两道都拒——它们持的能力与收束无关（fail-closed 的方向不变）。
    for p in [PRINCIPAL_REFLEX, PRINCIPAL_AUTONOMOUS] {
        assert!(
            !matrix.can_produce_artifact(p) && !matrix.can_reply(p),
            "{p} 既不产生产物也不发言（各自的窄行）"
        );
    }

    // 未知主体 fail-closed：两道闸同拒。
    assert!(!matrix.can_produce_artifact("session/ghost"));
}

/// 取表口只有一条口径（[ADR-099](../../../docs/010-adr/099.md "ADR-099")）：
/// **登记过的主体走表，未登记的主体什么也拿不到**——少一行是一条拒绝，不是一条漏判。
#[test]
fn an_unregistered_subject_gets_no_row_at_all() {
    // 登记过：拿自己那一行——窄行不被主智能体那套覆盖（覆盖就成「表授予 A、闸门判 B」）。
    let autonomous = production_matrix();
    assert_eq!(
        autonomous.grants_of(PRINCIPAL_AUTONOMOUS).len(),
        1,
        "窄行就是这一行的全部能力集"
    );
    assert!(!autonomous.can_reply(PRINCIPAL_AUTONOMOUS));

    // 未登记的子智能体：表里没有它的行 ⇒ 每个谓词为假（fail-closed）。
    // 反向判据：哪天有人把派生加回来，这条断言立刻变红。
    let sub = "session/sub";
    assert!(
        production_matrix().grants_of(sub).is_empty(),
        "未登记主体在表里没有行（ADR-099：未登记 = 拒）"
    );
    assert!(!production_matrix().can_reply(sub), "写不了收束格");
    assert!(
        !production_matrix().can_produce_artifact(sub),
        "也产不了产物"
    );

    // 要写收束格的子智能体必须自己登记一行——窄行是唯一入口。
    // 非 agent 命名空间同款：表里没有它 ⇒ fail-closed。
    assert!(!production_matrix().can_reply(PRINCIPAL_USER));
}

/// 写侧闸：收束的两种位置都放行；矩阵外主体两种位置都拒。
#[test]
fn only_the_main_agent_may_close_a_turn() {
    let matrix = production_matrix();
    assert!(
        matrix.can_reply("session/main") && matrix.can_reply("session/main"),
        "首响与追加都必须放行，否则生产对话直接断"
    );
    assert!(
        !matrix.can_reply("session/ghost") && !matrix.can_reply("session/ghost"),
        "未知主体 fail-closed"
    );
    assert!(
        !matrix.can_reply("user"),
        "人不入能力表（发言不是能力），故人没有权威收束"
    );
}

/// 读侧闸：`thread_private` 下只有属主本人；`shared` 只认矩阵成员。
#[test]
fn read_side_is_scoped_by_owner_and_membership() {
    let matrix = production_matrix();
    assert!(matrix.can_see("session/main", "session/main", VisScope::ThreadPrivate));
    assert!(!matrix.can_see("user", "session/main", VisScope::ThreadPrivate));
    assert!(!matrix.can_see("session/ghost", "session/main", VisScope::Shared));
}

/// 降级面：空矩阵（表构造失败时的 fail-closed 兜底）一概拒绝。
#[test]
fn the_degraded_empty_matrix_denies_everything() {
    let deny_all = PermissionMatrix::from_names(&[]).expect("空表无主体可校验");
    assert!(!deny_all.can_reply("session/main"));
    assert!(
        !deny_all.can_see("user", "session/main", VisScope::ThreadPrivate),
        "非属主读不到（thread_private 比的是属主，空矩阵不给任何人开例外）"
    );
    assert!(!deny_all.can_see("session/main", "session/main", VisScope::Shared));
}

/// **所有 `PRINCIPAL_*` 常量都是可路由地址 `<namespace>/<name>`**
/// （[ADR-069 §决策 2]）——id 第一段是归属 namespace，可路由到那个插件或概念；
/// 这条把「授权表与主体地址形态同源」钉在编译期。
///
/// 判据**逐条枚举**常量而不是 grep 字符串：加一个常量必须**同时加进本判据的列表**，
/// 忘了加即本测试红——比「grep 扫字符串」硬，因为 grep 会漏拼写近似的项。
#[test]
fn all_principals_are_routable_ids() {
    for (name, principal) in [
        ("PRINCIPAL_MAIN", PRINCIPAL_MAIN),
        ("PRINCIPAL_USER", PRINCIPAL_USER),
        ("PRINCIPAL_AUTONOMOUS", PRINCIPAL_AUTONOMOUS),
        ("PRINCIPAL_REFLEX", PRINCIPAL_REFLEX),
        ("PRINCIPAL_PEER", PRINCIPAL_PEER),
        ("PRINCIPAL_MARKET", PRINCIPAL_MARKET),
        ("PRINCIPAL_ARBITER", PRINCIPAL_ARBITER),
        ("PRINCIPAL_GATEKEEPER", PRINCIPAL_GATEKEEPER),
        ("PRINCIPAL_WORKER", PRINCIPAL_WORKER),
        ("PRINCIPAL_PLANNER", PRINCIPAL_PLANNER),
        ("PRINCIPAL_VERIFIER", PRINCIPAL_VERIFIER),
        ("PRINCIPAL_TOOL_VDFS_WRITE", PRINCIPAL_TOOL_VDFS_WRITE),
        ("PRINCIPAL_TOOL_WEB_FETCH", PRINCIPAL_TOOL_WEB_FETCH),
        (
            "PRINCIPAL_TOOL_CONTENT_SEARCH",
            PRINCIPAL_TOOL_CONTENT_SEARCH,
        ),
    ] {
        let ns = principal_namespace(principal);
        assert!(
            ns.is_some_and(|s| !s.is_empty()),
            "`{name} = {principal}` 不是 `<ns>/<name>` 可路由地址（第一段是归属，可解析）"
        );
    }
    // 关键锚点逐字锁死——漂移即红：id 形状就是 ADR-069 §决策 2 那张表里的取值。
    assert_eq!(PRINCIPAL_MAIN, "session/main");
    assert_eq!(PRINCIPAL_USER, "person/local");
    assert_eq!(PRINCIPAL_TOOL_VDFS_WRITE, "vdfs/write");
    // agent 命名空间登记表命中所有 `session/*`；其他三族不命中。
    for p in [PRINCIPAL_MAIN, PRINCIPAL_AUTONOMOUS, PRINCIPAL_REFLEX] {
        assert!(is_agent_id(p), "`{p}` 属 agent 命名空间");
    }
    for p in [PRINCIPAL_USER, PRINCIPAL_TOOL_VDFS_WRITE] {
        assert!(!is_agent_id(p), "`{p}` 不属 agent 命名空间");
    }
}

/// **三行工具主体登记进表**（[ADR-069 §决策 2 工具响应主体]）——今天写侧闸按会话主体判，
/// 这三行是**声明性**的；但表里没有行就等于「工具不是主体」，[T-147] 之后
/// `plugin::attributed` 按 kind=Tool 补上的 principal 就没底。
#[test]
fn tool_principals_have_declared_rows() {
    let matrix = production_matrix();
    for tool in [
        PRINCIPAL_TOOL_VDFS_WRITE,
        PRINCIPAL_TOOL_WEB_FETCH,
        PRINCIPAL_TOOL_CONTENT_SEARCH,
    ] {
        let grants = matrix.grants_of(tool);
        assert!(!grants.is_empty(), "`{tool}` 应在授权表里有一行（ADR-069）");
        assert!(
            grants.contains(&Capability::ProduceArtifact),
            "`{tool}` 的权威动作是产生产物（produce.artifact）"
        );
        // 工具主体的 namespace 不在 agent 登记表里——visible_to 第 2/3 条按此判：
        // 工具的响应在会话线程里对任何 viewer 都可见。
        assert!(
            !is_agent_id(tool),
            "`{tool}` 的 namespace 不在 agent 登记表里（[ADR-069 §决策 3]）"
        );
    }
}

/// **人不入能力表**（[ADR-069 §后果] 与 [authz.rs 模块文档]「两张表里的人」）——
/// `PRINCIPAL_USER` 归一为 `person/local` 之后仍**不立行**；判据：
/// `production_matrix().can_reply(PRINCIPAL_USER)` 依旧为假。
#[test]
fn person_local_is_not_in_the_capability_table() {
    assert!(
        !production_matrix().can_reply(PRINCIPAL_USER),
        "人不入能力表——`person/local` 没有 reply.* 权限（与归一前 `user` 同）"
    );
    assert!(
        production_matrix().grants_of(PRINCIPAL_USER).is_empty(),
        "`person/local` 在授权表里没有行——归一常量值不改变『人不入表』这条纪律"
    );
    // 归一后 user 也不是 agent 命名空间成员（visible_to 会放行，但 can_reply 仍拒）。
    assert!(!is_agent_id(PRINCIPAL_USER));
}

// ── 绑定即登记（[ADR-100]）──────────────────────────────────────────────
//
// 运行期绑定面（`BOUND`）是**进程级**登记：下面五条用例只碰自己独有的主体名
// （`session/t-bind-*`），不碰别处引用的 `session/main` / `ghost` / `worker` / `sub`
// ——Rust 单测在同一进程里并行跑，共享这一张表；用独有名是这里的纪律。

/// 绑定即登记：未部署的主体经 [`bind_inheriting`] 拿到来源行，收束闸随之放行。
///
/// 这条钉的是 [ADR-100] 取代「派生回滚」的核心断言：`agent:<id>` 身份会话
/// **不再靠 `matrix_for` 静默派生主行**，而是随会话装配点的绑定拿到一行——
/// 拿得到与拿不到都由可见的登记动作决定（少一行 = 一条拒绝）。
#[test]
fn a_bound_subject_takes_the_source_row() {
    let subject = "session/t-bind-root";
    // 绑定前：既未部署、也未绑定 ⇒ 每个谓词为假（与 [ADR-099] 同向）。
    assert!(
        production_matrix().grants_of(subject).is_empty(),
        "未绑定主体在合流面没有行"
    );
    assert!(!production_matrix().can_reply(subject));
    // 根会话绑定 = 继承会话面缺省行（主智能体行）。
    bind_root("t-bind-root-sess", subject);
    assert_eq!(
        production_matrix().grant_names_of(subject).len(),
        5,
        "拿到会话面行五项能力"
    );
    assert!(
        production_matrix().can_reply(subject),
        "绑定来的 reply 就是收束闸的判据"
    );
    assert!(production_matrix().can_produce_artifact(subject));
    assert!(
        !production_matrix().is_deployed(subject),
        "绑定 ≠ 部署事实（淘汰面与可见域面都不认它）"
    );
}

/// **静态行不被放大**：主体已在 [`ROWS`] 里时，委派绑定取父行 ∩ 声明行——窄行不会
/// 被悄悄放大成会话面。
///
/// 反判据：把交集换成「整行取自父」（旧的继承语义），工人会拿到主行五项，
/// `can_reply(worker)` 立刻变真。
#[test]
fn binding_never_widens_a_deployed_row() {
    let before = production_matrix().grants_of(PRINCIPAL_WORKER).len();
    bind_inheriting("t-bind-widen-sess", PRINCIPAL_WORKER, PRINCIPAL_MAIN);
    assert_eq!(
        production_matrix().grants_of(PRINCIPAL_WORKER).len(),
        before,
        "工人那一行（单能力独持）不因绑定调用变宽"
    );
    assert!(
        !production_matrix().can_reply(PRINCIPAL_WORKER),
        "工人仍无发言权（步 3 的行形状是静态事实，运行期绑定改不动它）"
    );
}

/// **绑定不带可见域**：绑定主体不是矩阵成员，读不到他人 `shared`（t76 的反判据）。
///
/// 能力与可见域必须分家——把可见域也随绑定登记，任何 `agent:<id>` 都会变成
/// 矩阵成员、读到他人 shared，t76「ghost 读不到他人 shared」当场破。
#[test]
fn binding_grants_no_visibility_scope() {
    let subject = "session/t-bind-vis";
    bind_root("t-bind-vis-sess", subject);
    assert!(production_matrix().can_reply(subject), "能力随绑定来了……");
    assert!(
        !production_matrix().can_see(subject, PRINCIPAL_MAIN, VisScope::Shared),
        "……但可见域不随绑定来：读不到他人 shared"
    );
    assert_eq!(
        production_matrix().sees_of(subject),
        None,
        "绑定主体的可见域仍为空（只认部署表）"
    );
}

/// **淘汰面只认部署表**：绑定登记不构成「部署授予过 `assign.work`」。
///
/// 判据取两口之差：合流面（`can_write_name`）看得见绑定来的能力，部署面
/// （`deployed_can_write_name`）看不见——`derived_facts::record_elimination` 走后者，
/// 运行期绑定的主体不会因此被回头淘汰（它本就不是部署事实）。
#[test]
fn the_elimination_face_ignores_bindings() {
    let subject = "session/t-bind-elim";
    bind_root("t-bind-elim-sess", subject);
    assert!(
        production_matrix().can_write_name(subject, CAP_ASSIGN_WORK),
        "合流面：绑定来的 assign.work 能力可见"
    );
    assert!(
        !production_matrix().deployed_can_write_name(subject, CAP_ASSIGN_WORK),
        "部署面：绑定主体不算部署事实，不进淘汰面"
    );
}

/// **委派两侧都在册**：`from` 无行时先给 `from` 登记会话面缺省，再让子主体继承。
///
/// `turn_runner::within_delegation_ceiling` 判**父主体**的行，父侧缺行会让子会话
/// 因「父也不持该能力」而收束不了——所以委派上限两侧都要在册，缺一侧整条链断。
#[test]
fn delegated_binding_registers_both_sides() {
    let parent = "session/t-bind-parent";
    let child = "session/t-bind-child";
    bind_inheriting("t-bind-child-sess", child, parent);
    assert!(
        !production_matrix().grants_of(parent).is_empty(),
        "父侧也在册（否则委派上限判不到父行）"
    );
    assert!(
        !production_matrix().grants_of(child).is_empty(),
        "子侧继承父行（委派不放大权威：子 ≤ 父）"
    );
    assert_eq!(
        production_matrix().grants_of(child).len(),
        production_matrix().grants_of(parent).len(),
        "子拿到的是父那一行的能力集"
    );
    assert!(
        production_matrix().can_reply(child),
        "子会话收束闸放行（父行继承下来）"
    );
}

// ── 创建即登记与同寿回收（[T-182]）─────────────────────────────────────
//
// 下面的用例钉的是**生命周期**而不是判定口径：登记以会话为持有人，会话结束
// （`unbind_session`，生产由 `delete_session_internal` 调用）时整册消失。
// 判定面（交集 / 替换 / 空行）仍由上面的 ADR-100 边界用例钉住。

/// **创建即登记 + 同寿回收**：登记行的寿命就是会话的寿命——会话结束，行随之消失；
/// 进程级绑定面不再为「出现过的每个 agent 身份」留一行。
#[test]
fn a_binding_is_reclaimed_when_its_session_ends() {
    let subject = "session/t-bind-life";
    let session = "t-bind-life-sess";
    bind_root(session, subject);
    assert!(
        production_matrix().can_reply(subject),
        "会话创建即声明主体行"
    );
    unbind_session(session);
    assert!(
        production_matrix().grants_of(subject).is_empty(),
        "会话结束 ⇒ 登记随之消失（同寿）"
    );
}

/// **同一主体的两条会话登记互不覆盖寿命**：最近一次登记为准；较新的会话结束时，
/// 较老的那条登记重新生效，而不是连它一起消失。
#[test]
fn bindings_of_two_live_sessions_outlive_each_other() {
    let subject = "session/t-bind-shared";
    let reader = "t-bind-shared-reader";
    let worker_holder = "t-bind-shared-worker";

    bind_root(reader, subject); // 会话面缺省行（5 项）
    assert!(production_matrix().can_reply(subject));

    bind_inheriting(worker_holder, subject, PRINCIPAL_WORKER); // 父 worker ⇒ 只剩产物能力
    assert!(
        !production_matrix().can_reply(subject),
        "同一主体多会话登记时取最近一次"
    );

    unbind_session(worker_holder);
    assert!(
        production_matrix().can_reply(subject),
        "较新的登记回收后，较老的一条重新生效"
    );
    unbind_session(reader);
    assert!(production_matrix().grants_of(subject).is_empty());
}

/// **同一会话改写身份 ⇒ 旧行不留**：绑定的生命周期单位是会话，不是主体；
/// 会话从 A 改到 B 后，A 的行不能挂到会话结束。
#[test]
fn rebinding_a_session_replaces_its_previous_identity_row() {
    let session = "t-bind-switch-sess";
    let first = "session/t-bind-switch-a";
    let second = "session/t-bind-switch-b";
    bind_root(session, first);
    bind_root(session, second);
    assert!(
        production_matrix().grants_of(first).is_empty(),
        "会话改写身份后，旧主体的行被撤掉"
    );
    assert!(production_matrix().can_reply(second), "新身份拿到会话面行");
    unbind_session(session);
    assert!(production_matrix().grants_of(second).is_empty());
}

// ── 委派取交集（T-195）────────────────────────────────────────────────
//
// 这两条用**自造小表**（`EffectiveMatrix { deployed }`）：生产表里唯一的多能力行是
// `session/main`，拿它当「子」会污染同一进程里所有用例。小表证明的是判据本身；
// 生产接线（`request_actor_spec` 的根 / 子分派）由上层用例覆盖。

/// **父窄子宽 ⇒ 子取交集**：子主体的部署行是显式声明，父行是上限。
///
/// 反判据：改回「子拿自己那一行」（旧实现的 `is_deployed ⇒ return`），
/// `assign.work` 会留在子身上——父没有的能力被子继承了。
#[test]
fn a_delegated_child_gets_the_intersection_of_parent_and_declaration() {
    let deployed = PermissionMatrix::from_names(&[
        (
            "session/t-deploy-wide",
            &["reply", "assign.work"],
            VisScope::ThreadPrivate,
        ),
        (
            "session/t-deploy-narrow",
            &["reply"],
            VisScope::ThreadPrivate,
        ),
    ])
    .expect("自造小表合法");
    let matrix = EffectiveMatrix { deployed };

    bind_inheriting_in(
        &matrix,
        "t-intersect-wide-sess",
        "session/t-deploy-wide",
        "session/t-deploy-narrow",
    );
    assert_eq!(
        matrix.grants_of("session/t-deploy-wide"),
        vec![Capability::Reply],
        "子能力 = 父能力 ∩ 显式声明（委派不放大权威）"
    );
    assert!(
        !matrix.can_write("session/t-deploy-wide", Capability::AssignWork),
        "父不持 assign.work ⇒ 子也不持"
    );
    assert!(
        matrix.can_reply("session/t-deploy-wide"),
        "交集保留了双方都持有的 reply"
    );

    // 子未部署（没有显式声明）：整行来自父——交集退化为父行，不是空集。
    bind_inheriting_in(
        &matrix,
        "t-intersect-undep-sess",
        "session/t-un-deployed",
        "session/t-deploy-narrow",
    );
    assert_eq!(
        matrix.grants_of("session/t-un-deployed"),
        vec![Capability::Reply],
        "没有声明行时继承父行"
    );
}

/// **空交集是一条拒绝，不是退回部署行**：子声明与父行没有共同能力时，登记空行；
/// 合流面以空行为有效行——部署行不会「补回来」。
///
/// 反判据：若 `insert_binding` 把空行当作「不登记」跳过，`effective_grants` 会回退
/// 部署行，子立刻拿回父没有的 `assign.work`——委派放大权威。
#[test]
fn an_empty_intersection_registers_a_denying_row() {
    let deployed = PermissionMatrix::from_names(&[
        (
            "session/t-intersect-child",
            &["assign.work"],
            VisScope::ThreadPrivate,
        ),
        (
            "session/t-intersect-parent",
            &["reply"],
            VisScope::ThreadPrivate,
        ),
    ])
    .expect("自造小表合法");
    let matrix = EffectiveMatrix { deployed };

    bind_inheriting_in(
        &matrix,
        "t-intersect-empty-sess",
        "session/t-intersect-child",
        "session/t-intersect-parent",
    );
    assert!(
        matrix.grants_of("session/t-intersect-child").is_empty(),
        "父行与子声明无交集 ⇒ 子没有能力"
    );
    assert!(
        !matrix.can_write("session/t-intersect-child", Capability::AssignWork),
        "部署行不能因为交集为空而被退回"
    );
    assert!(
        matrix
            .grant_names_of("session/t-intersect-child")
            .is_empty(),
        "规格读到的能力集同样为空"
    );
}

/// **根不是委派**：已部署的根主体原样用自己的行，不拿会话面缺省去夹它。
///
/// 反判据：对 `Scope::Root` 也走交集 ⇒ verifier / arbiter 的
/// `assert.verification`（主智能体刻意不持，C15）会被夹成空行。
#[test]
fn a_deployed_root_keeps_its_own_row() {
    bind_root("t-bind-verifier-sess", PRINCIPAL_VERIFIER);
    assert_eq!(
        production_matrix().grant_names_of(PRINCIPAL_VERIFIER),
        vec![CAP_ASSERT_VERIFICATION.to_string()],
        "根主体不被会话面缺省夹窄"
    );
    bind_root("t-bind-arbiter-sess", PRINCIPAL_ARBITER);
    assert!(
        production_matrix().can_write(PRINCIPAL_ARBITER, Capability::AssertVerification),
        "仲裁者同样保留自己的行"
    );
}
