---
kind: task
id: T-011-共享折叠搬owner
title: S8：共享折叠搬 owner——第二读者改名字读
status: doing
track: arch
prio: next
created: 2026-10-29
parent: T-001-能力全面拆分
deps: []
where: `docs/003-design/026-能力拆分与影响类.md` §2.3, `crates/symbio-sdk/src/projection`, `crates/symbio-plugin-api/src/views.rs`, `crates/symbio-plugin-loop/src`, `crates/symbio-plugin-session/src/stats.rs`
exit: 成本 / 校准 / 声誉 / 凸显四条构造器（覆盖名册 18/19/20/22/23/28 六行）离开 `symbio-sdk`，各归 owner 能力包；第二个读者改 `ViewRead::named`；迁移前后读数逐字不变（唯一例外是 `skill.retreat` 的 as-of 半边，按 `DE-026` D-286 改判随视图收到的 `now`）；`PP-004` 的 files / ctors 两格只降不升；每一批都是「包内声明 + 读者改名字 + `sdk` 删除」同一提交（`EX-007`：机制面新增行为 0，删除行不限——`D-283`）
---

# S8：共享折叠搬 owner——第二读者改名字读

## 内容

名册里的 D 类六行（#18 成本控制、#19 自我模型、#20 校准、#22 信任账本、#23 凸显仲裁、
#28 声誉与互信）今天住 `symbio-sdk/src/projection`，理由是「读者 ≥2 个插件」。但 D-221 已经给出合法路径：
折叠由 owner 插件声明 `ViewDecl`，第二个读者经 `ViewRead::named` **只按名字读**。
于是 D 不再是「不可拆」，只是「还没改读法」。

搬的是**折叠**，不是事实：事实仍由各自的写口落格，折叠函数跟着声明它的能力包走。
第二读者看到的值必须逐字不变——这是本段唯一的正确性判据。

### 进度

| 行 | 状态 | 落点 |
|---|---|---|
| #18 成本控制 | 已落（S8a） | `crates/symbio-plugin-cost/src/decl.rs::ledger` 以 `ViewDecl` 交 `cost.ledger`；读者 `loop/breaker.rs::session_cost_ms` 与 `session/stats.rs` 的成本列同批改名字读，`sdk/src/projection/cost.rs` 随批删除（D-284 去掉扫描上界、D-285 落地）。`PP-004` 两格 6/6 → 5/5 |
| #19 / #20 自我模型与校准 | 已落（S8b） | 两行是同一条折叠 ⇒ 一个提交，落点是词汇 owner `crates/symbio-plugin-skill/src/decl.rs::calibration_ledger`，以 `ViewDecl` 交 `skill.calibration`（`D-286` 改判：不立 `symbio-plugin-calibration`——校准没有自己的事实与行为）。读者 `session/stats.rs` 的校准列改名字读，`sdk/src/projection/calibration.rs` 与 `plugin-api` 的 `pub use calibration_for` 随批删除 ⇒ `PP-004` 两格 5/5 → 4/4 |
| #22 / #28 信任账本与互信 | 已落（S8c） | 两行是同一条折叠 ⇒ 一个提交，落点是新建的只读能力包 `crates/symbio-plugin-reputation/src/decl.rs::ledger`，以 `ViewDecl` 交 `reputation.ledger`（`D-288`；判定者先收纯 `score` 那次解缆是独立一批 `D-287`）。读者 `loop/derived_facts.rs::record_elimination` 与 `session/stats.rs` 的声誉列同批改名字读，`sdk/src/projection/reputation.rs` 随批删除，承诺四条的词汇面（`loop/control_decl.rs::COMMITMENT_VOCAB`）一起离开控制面 ⇒ `PP-004` 两格 4/4 → 3/3。`reads` 走断言格通道（`D-282`）：不为它给 `loop` 补 `FactDecl` |
| #23 凸显仲裁 | 未起 | `symbio-plugin-readyset`；调度（`loop` 候选集）与读数（`session` 的 `readyset` 列）两个读者都在，改完 `readyset_for` 才离开 core |

## 处理过程

1. 一行为单位：先把它在 `sdk/src/projection` 的构造器与类型搬到 owner crate，声明 `ViewDecl` 并进
   `shared_views` 清单。
2. 同一提交把第二读者的类型读改成 `ViewRead::named`，删掉 `sdk` 的导出与文件本身
   （机制面只删不加——`D-283` 让这一半和绑定切换留在同一个迁移提交里）。
3. 跑该折叠的两侧读数用例，确认迁移前后逐字相同。
4. 四条构造器搬完后核对 `PP-004`：文件和构造器两格都应下降；基线只许跟着代码往下改。
5. 更新名册账（`DE-026` §2.3）对应行，绿灯后提交。
