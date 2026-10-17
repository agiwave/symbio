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
//!    对等/图侧主体，窄于「派生整套」）；
//! 5. `matrix_for` 的两条分支：**登记过的主体走表**（表是权威，派生不覆盖它），
//!    未登记的 `agent:<id>` 才派生主智能体那一行；
//! 6. 读写两侧的闸按**属主 / 矩阵成员**分：主智能体与属主各得其所，`user` 不入表
//!    （发言不是能力），矩阵外主体拒绝；表构造失败时的降级面（空矩阵）一概拒绝
//!    ——fail-closed 不因降级而松开；
//! 7. **工人 / 规划者 / 验证者三行比主智能体窄**（S02/S03 三角色，T-107 行与路径
//!    同批）：各自单能力独持，`matrix_for` 拿登记行，C15 两面（规划者不认证、
//!    验证者不产出）在表这一层可见。

use super::*;
use crate::symbio_core::governance::Capability;

/// 表的形状：主智能体持 6 项、缺 `assert.verification`，可见域 `thread_private`。
#[test]
fn main_agent_row_is_the_documented_shape() {
    let matrix = production_matrix();
    assert_eq!(
        matrix.grants_of(PRINCIPAL_MAIN).len(),
        6,
        "judge.intent / reply.first / reply.append / define.work / produce.artifact / assign.work"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_MAIN),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );
}

/// 自主发起者那一行（S12 §3）：**比主智能体窄**——只持 `define.work`，不持任何 `reply.*`。
///
/// 这一行是本表里第一条「少给」的记录：若缺了它，`matrix_for` 对一切
/// `agent:*` 都派生主智能体的整套能力集（含 `reply.*`）——于是「自主行为不得冒充
/// 用户对话」在生产里被静默推翻，文档与验收断言却都还写着它成立。
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
        !matrix.can_reply(PRINCIPAL_AUTONOMOUS, true)
            && !matrix.can_reply(PRINCIPAL_AUTONOMOUS, false),
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
/// 与自主行同形的静默面：`agent:reflex` 缺表时，
/// `matrix_for` 对一切未登记 `agent:*` 都派生主智能体的整套能力（含 `reply.*`）——
/// 于是「判定者不得发言」在生产里被静默推翻。判据落在**生产表**上（`matrix_for`），
/// 不再是自造矩阵：自造矩阵只证明「我拒我」，表里少一行它照样绿。
#[test]
fn the_reflex_row_is_narrower_than_derivation() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_REFLEX);
    assert_eq!(caps.len(), 1, "只持 judge.intent：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_REFLEX, Capability::JudgeIntent),
        "判定者必须能判决意图，否则抢占判定这一步直接失效"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_REFLEX, true) && !matrix.can_reply(PRINCIPAL_REFLEX, false),
        "判定者不得直接发言（首响与追加都不放行，S07 §5）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_REFLEX),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );

    // 反向判据：**登记过的主体走表**，派生不得覆盖它——`matrix_for` 拿到的必须是
    // 上面那一行（窄），而不是派生出来的主智能体整套。去掉 ROWS 里那一行，这条断言
    // 立刻变红（派生分支会把 `reply.first` 塞回来）。
    let derived = matrix_for(PRINCIPAL_REFLEX);
    assert_eq!(
        derived.grants_of(PRINCIPAL_REFLEX).len(),
        1,
        "派生不得把主智能体的 reply.* 塞回判定者这一行"
    );
    assert!(!derived.can_reply(PRINCIPAL_REFLEX, true));
}

/// 对等主体那一行（S08 §3）：**第一个对等（图侧）主体的窄行**——只持 `produce.artifact`，
/// 严格窄于主智能体，也严格窄于 `matrix_for` 的派生整套。
///
/// S08 §4 的平凡值「身份分层 ≠ 权限分层」是给**子智能体**的（它要能写自己的收束格，
/// 否则什么都写不了）；对等体拿主智能体整套（含 `reply.*`）则是**静默超权**——
/// 与自主行 / 判定行同形的静默面。S08 §3.1 的纪律：对等主体出现必须**自带窄行同批落地**，
/// 不许靠派生。
#[test]
fn the_peer_row_is_narrower_than_the_derived_set() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_PEER);
    assert_eq!(caps.len(), 1, "只持 produce.artifact：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_PEER, Capability::ProduceArtifact),
        "对等体的权威动作是产生产物（S08 §3 的 ActorSpec 行），否则这一行没有理由存在"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_PEER, true) && !matrix.can_reply(PRINCIPAL_PEER, false),
        "对等体的协作经 commitment.* 而非 chat.*——不以本机主控的身份发言"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_PEER),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );

    // 「严格窄于派生整套」：主智能体持 6 项、对等行 1 项；且 `matrix_for` 必须拿**登记行**
    // 而非派生出来的整套——去掉 ROWS 那一行，派生分支会把 `reply.*` 与其余能力一起塞回来。
    let main_len = matrix.grants_of(PRINCIPAL_MAIN).len();
    assert!(
        caps.len() < main_len,
        "对等行必须严格窄于主智能体那一行（{main_len} 项）"
    );
    let derived = matrix_for(PRINCIPAL_PEER);
    assert_eq!(
        derived.grants_of(PRINCIPAL_PEER).len(),
        1,
        "派生不得把主智能体整套塞回对等行"
    );
    assert!(!derived.can_reply(PRINCIPAL_PEER, true));
    assert!(
        derived.can_write(PRINCIPAL_PEER, Capability::ProduceArtifact),
        "登记行是权威：对等体仍持它那一项能力"
    );
}

/// 组织形态三角色（S13 §3，T-073）：市场 / 仲裁 / 守门三行**各自单能力独持**，
/// 且都严格窄于 `matrix_for` 的派生整套。
///
/// 与自主 / 判定 / 对等三行同一条纪律：主体出现必须自带窄行同批落地，不许靠派生
/// ——未登记的 `agent:market` 会拿主智能体整套（含 `reply.*`），「市场不得冒充
/// 主控发言」「仲裁者不自证」「守门者先判后发声」于是在生产里被静默推翻。
/// 另钉 exit ① 的表半边：arbiter 持 `assert.verification` **不碰** C15（不持
/// `define.work` / `produce.artifact`，构造期双持检查见 `governance`）。
#[test]
fn the_three_society_rows_are_narrower_than_derivation() {
    let matrix = production_matrix();
    let main_len = matrix.grants_of(PRINCIPAL_MAIN).len();

    // 市场：只派活（assign.work），公告是公共域事实（vis_scope = public）。
    let market = matrix.grants_of(PRINCIPAL_MARKET);
    assert_eq!(market.len(), 1, "只持 assign.work：{market:?}");
    assert!(matrix.can_write(PRINCIPAL_MARKET, Capability::AssignWork));
    assert!(
        !matrix.can_reply(PRINCIPAL_MARKET, true) && !matrix.can_reply(PRINCIPAL_MARKET, false),
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
    assert!(!matrix.can_reply(PRINCIPAL_ARBITER, true));
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
        !matrix.can_reply(PRINCIPAL_GATEKEEPER, true)
            && !matrix.can_reply(PRINCIPAL_GATEKEEPER, false),
        "判定者不得直接发言（与 reflex 行同款独持先例）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_GATEKEEPER),
        Some(VisScope::ThreadPrivate),
        "守门判定是判定者自己的工作记录，不是公共域公告"
    );

    // 三行都严格窄于主智能体那一行，且 `matrix_for` 拿的是**登记行**不是派生整套。
    for (p, cap) in [
        (PRINCIPAL_MARKET, Capability::AssignWork),
        (PRINCIPAL_ARBITER, Capability::AssertVerification),
        (PRINCIPAL_GATEKEEPER, Capability::JudgeIntent),
    ] {
        assert!(
            matrix.grants_of(p).len() < main_len,
            "{p} 必须严格窄于主智能体那一行（{main_len} 项）"
        );
        let derived = matrix_for(p);
        assert_eq!(
            derived.grants_of(p).len(),
            1,
            "派生不得把主智能体整套塞回 {p}"
        );
        assert!(
            derived.can_write(p, cap),
            "登记行是权威：{p} 仍持它那一项能力"
        );
        assert!(!derived.can_reply(p, true));
    }
}

/// S02/S03 三角色行（T-107，行与执行路径同批）：工人 / 规划者 / 验证者三行
/// **各自单能力独持**，严格窄于主智能体，也严格窄于 `matrix_for` 的派生整套。
///
/// 与自主 / 判定 / 对等 / 市场 / 仲裁 / 守门六行同一条纪律：主体出现必须自带窄行
/// 同批落地——未登记的 `agent:worker` 会走派生拿主智能体整套（含 `reply.*`），
/// 「工人不以主控身份发言」「规划者不自我认证」「产出者不得既产出又认证」于是在
/// 生产里被静默推翻，而文档与验收断言都还写着它们成立。
///
/// 另钉两条 C15 的结构面：planner 持 `define.work` 而不持 `assert.verification`
/// （不自我认证），verifier 持 `assert.verification` 而不持 `define.work` /
/// `produce.artifact` （不既产出又认证）——「二者互不可写」在**表**这一层可见，
/// 构造期的双持拒绝在 `governance` 那一侧。
#[test]
fn the_three_role_rows_are_narrower_than_derivation() {
    let matrix = production_matrix();
    let main_len = matrix.grants_of(PRINCIPAL_MAIN).len();

    // 工人：只产生产物（S02 worker 行），树侧委派链上以产物收束的子主体。
    let worker = matrix.grants_of(PRINCIPAL_WORKER);
    assert_eq!(worker.len(), 1, "只持 produce.artifact：{worker:?}");
    assert!(matrix.can_write(PRINCIPAL_WORKER, Capability::ProduceArtifact));
    assert!(
        !matrix.can_reply(PRINCIPAL_WORKER, true) && !matrix.can_reply(PRINCIPAL_WORKER, false),
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
    assert!(!matrix.can_reply(PRINCIPAL_PLANNER, true));
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
    assert!(!matrix.can_reply(PRINCIPAL_VERIFIER, true));
    assert_eq!(
        matrix.sees_of(PRINCIPAL_VERIFIER),
        Some(VisScope::ThreadPrivate),
        "验证判定是判定者自己的工作记录（与守门行同一条理由），不是公共域公告"
    );

    // 三行都严格窄于主智能体那一行，且 `matrix_for` 拿的是**登记行**不是派生整套。
    for (p, cap) in [
        (PRINCIPAL_WORKER, Capability::ProduceArtifact),
        (PRINCIPAL_PLANNER, Capability::DefineWork),
        (PRINCIPAL_VERIFIER, Capability::AssertVerification),
    ] {
        assert!(
            matrix.grants_of(p).len() < main_len,
            "{p} 必须严格窄于主智能体那一行（{main_len} 项）"
        );
        let derived = matrix_for(p);
        assert_eq!(
            derived.grants_of(p).len(),
            1,
            "派生不得把主智能体整套塞回 {p}"
        );
        assert!(
            derived.can_write(p, cap),
            "登记行是权威：{p} 仍持它那一项能力"
        );
        assert!(!derived.can_reply(p, true));
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
        !matrix.can_reply(PRINCIPAL_PEER, true),
        "产物闸放行不改变发言闸：对等体仍无发言权"
    );

    // 主智能体：两道都放（发言与产物都是它的权威动作）。
    assert!(matrix.can_reply(PRINCIPAL_MAIN, true));
    assert!(matrix.can_produce_artifact(PRINCIPAL_MAIN));

    // 判定者 / 自主层：两道都拒——它们持的能力与收束无关（fail-closed 的方向不变）。
    for p in [PRINCIPAL_REFLEX, PRINCIPAL_AUTONOMOUS] {
        assert!(
            !matrix.can_produce_artifact(p) && !matrix.can_reply(p, true),
            "{p} 既不产生产物也不发言（各自的窄行）"
        );
    }

    // 未知主体 fail-closed：两道闸同拒。
    assert!(!matrix.can_produce_artifact("session/ghost"));
}

/// `matrix_for` 两条分支：**登记过的主体走表**（表是权威，派生不覆盖它）；
/// 未登记的 `agent:<id>` 才派生主智能体那一行。
#[test]
fn matrix_for_prefers_the_registered_row_over_derivation() {
    // 登记过：拿自己那一行——窄行不被主智能体那套覆盖（覆盖就成「表授予 A、闸门判 B」）。
    let autonomous = matrix_for(PRINCIPAL_AUTONOMOUS);
    assert_eq!(
        autonomous.grants_of(PRINCIPAL_AUTONOMOUS).len(),
        1,
        "派生不得把主智能体的 reply.* 塞回这一行"
    );
    assert!(!autonomous.can_reply(PRINCIPAL_AUTONOMOUS, true));

    // 未登记的子智能体：派生主智能体那一行（身份分层 ≠ 权限分层，S08 §4 平凡值）。
    let sub = matrix_for("session/sub");
    assert_eq!(
        sub.grants_of("session/sub").len(),
        production_matrix().grants_of(PRINCIPAL_MAIN).len(),
        "未登记主体派生主智能体的整套能力"
    );
    assert!(
        sub.can_reply("session/sub", true),
        "子智能体要能写自己的收束格，否则它什么都写不了"
    );

    // 非 `agent:*` 的主体走表：表里没有它 ⇒ fail-closed。
    assert!(!matrix_for(PRINCIPAL_USER).can_reply(PRINCIPAL_USER, true));
}

/// 写侧闸：收束的两种位置都放行；矩阵外主体两种位置都拒。
#[test]
fn only_the_main_agent_may_close_a_turn() {
    let matrix = production_matrix();
    assert!(
        matrix.can_reply("session/main", true) && matrix.can_reply("session/main", false),
        "首响与追加都必须放行，否则生产对话直接断"
    );
    assert!(
        !matrix.can_reply("session/ghost", true) && !matrix.can_reply("session/ghost", false),
        "未知主体 fail-closed"
    );
    assert!(
        !matrix.can_reply("user", true),
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
    assert!(!deny_all.can_reply("session/main", true));
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
/// `matrix_for(PRINCIPAL_USER).can_reply(PRINCIPAL_USER, true)` 依旧为假。
#[test]
fn person_local_is_not_in_the_capability_table() {
    assert!(
        !production_matrix().can_reply(PRINCIPAL_USER, true),
        "人不入能力表——`person/local` 没有 reply.* 权限（与归一前 `user` 同）"
    );
    assert!(
        production_matrix().grants_of(PRINCIPAL_USER).is_empty(),
        "`person/local` 在授权表里没有行——归一常量值不改变『人不入表』这条纪律"
    );
    // 归一后 user 也不是 agent 命名空间成员（visible_to 会放行，但 can_reply 仍拒）。
    assert!(!is_agent_id(PRINCIPAL_USER));
}
