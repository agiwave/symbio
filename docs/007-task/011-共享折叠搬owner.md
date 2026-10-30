---
kind: task
id: T-011-共享折叠搬owner
title: S8：共享折叠搬 owner——第二读者改名字读
status: todo
track: arch
prio: next
created: 2026-10-29
parent: T-001-能力全面拆分
deps: [T-010-名册五十四条处置]
where: `crates/symbio-sdk/src/projection`, `crates/symbio-plugin-api/src/views.rs`, `crates/symbio-plugin-loop/src`, `crates/symbio-plugin-session/src/stats.rs`
exit: 成本 / 校准 / 声誉 / 凸显四条构造器（覆盖名册 18/19/20/22/23/28 六行）离开 core，各归 owner 能力包；第二个读者改 `ViewRead::named`；迁移前后读数逐字不变；`PP-004` 的 files / ctors 两格只降不升
---

# S8：共享折叠搬 owner——第二读者改名字读

## 内容

名册里的 D 类六行（#18 成本控制、#19 自我模型、#20 校准、#22 信任账本、#23 凸显仲裁、
#28 声誉与互信）今天住 core，理由是「读者 ≥2 个插件」。但 D-221 已经给出合法路径：
折叠由 owner 插件声明 `ViewDecl`，第二个读者经 `ViewRead::named` **只按名字读**。
于是 D 不再是「不可拆」，只是「还没改读法」。

搬的是**折叠**，不是事实：事实仍由各自的写口落格，折叠函数跟着声明它的能力包走。
第二读者看到的值必须逐字不变——这是本段唯一的正确性判据。

## 处理过程

1. 一行为单位：先把它在 core 的构造器与类型搬到 owner crate，声明 `ViewDecl` 并进
   `shared_views` 清单。
2. 同一提交把第二读者的类型读改成 `ViewRead::named`，删掉 core 的导出。
3. 跑该折叠的两侧读数用例，确认迁移前后逐字相同。
4. 四条构造器搬完后核对 `PP-004`：文件和构造器两格都应下降；基线只许跟着代码往下改。
5. 更新名册账（S1）对应行，绿灯后提交。
