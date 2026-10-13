---
kind: task
id: T-091-verify并发工具链竞态
title: verify 阶段并发跑裸 rustc 撞上 rustup 按需安装——CI 上 19 个验证项里 18 个假红，C29 判据被误带
status: done
created: 2026-10-13
track: quality
deps: []
prio: now
where: scripts/gate.d/55-verify.mjs（串行预热）+ scripts/gate.d/_shared.test.mjs（判据钉）+ .github/workflows/ci.yml（verify-evidence 注释订正）
exit: verify 阶段在「工具链冷」的环境（CI runner / 新机器跑 `--only=verify`）下不再有并发安装竞态；「预热必须先于并发批、且自身串行」有机器判据（含反向用例）
evidence: `55-verify.mjs` 在 facts 生成之后、逐程序并发批之前插入一个**非 `parallel`** 的 `rustc --version` 预热项（`timeoutMs 300_000`；失败单独归因为「rustc 不可用」，不再与 18 个同形的「编译失败 exit=1」混在一起）。判据：`_shared.test.mjs` 新增用例钉「预热存在 ∧ 位于 `for (… of entries)` 之前 ∧ 自身不标 `parallel: true` ∧ 调的是 rustc」。反向判据已实测——给预热项加 `parallel: true` ⇒ 该用例红（52 → 51 pass / 1 fail）。`ci.yml` 的 verify-evidence 注释由「不装额外工具链，rustup 会自动解析」订正为记录这次竞态与「修在门禁侧」的理由。验证：`node --test scripts/gate.d/_shared.test.mjs` 52/52 绿；`node scripts/gate.mjs --only=verify` 21/21 绿。
---

## 内容

`e9c04076`（2026-10-09，门禁并发化收尾）给 verify 阶段加了 `concurrency: 8` 与逐程序
`parallel: true`。这个阶段是全仓**唯一用裸 `rustc` 并发跑**的地方（backend 走 `cargo`），
而 `rust-toolchain.toml` 钉的 1.93.1 在 CI runner 上并不预装——rustup 在**首次**调用时
按需下载安装。

CI 的 `verify-evidence` job 跑 `--only=verify`，跳过了 fmt 阶段（`cargo fmt --all` 会先
串行解析一次工具链，这正是本机与整轮门禁不暴露此问题的原因）⇒ 无人预热。8 个 `rustc`
同时撞上那次安装：实测 19 个程序里只有 1 个成功（它耗时 11s，正是那次安装），其余 18 个
以「toolchain 未安装」退出。

**危害不在红，在于红得错**：那条错误没有 `error[E…]` 编号，而 C29 的反向判据正是靠
「有没有编号」区分「类型级拒绝」与「文件被改坏」——于是 3 个反例档全部被判成
「失败原因不是类型错误」，报出来的原因是错的。

## 处置

在并发批之前插一个**串行**预热项（`rustc --version`）：① 把按需安装做完，此后并发批
不再有竞争；② 工具链解析失败单独归因，不再与 18 个同形的「编译失败」混在一起。

**不在 `ci.yml` 侧装第二份工具链**：那会造出第二个版本真相源，且只堵住 CI 这一处——
新机器本机跑 `--only=verify` 是同一个竞态。故判据钉在门禁侧，并由 `_shared.test.mjs` 钉住。

## 不是本任务范围

- 不改 verify 阶段的并发度（`concurrency: 8` 本身没问题，问题是**冷启动**与并发同时发生）。
- 不动 `docs/plan/verify/` 的 16 个程序与 3 个编译期反例。

## 处理过程

- 2026-10-13 登记：排查 `v2` 分支 CI 全红时（run 38372186256 / 38358236876）发现
  verify-evidence 19 项全红、且「编译失败 exit=1」里没有 `error[E…]`；对照 2026-10-08 的
  绿色 run（37721749855：程序**串行**、首个 rustc 耗时 8s）与 `git log -S"concurrency: 8"`
  定位到 `e9c04076`。
- 2026-10-13 清偿：加串行预热 + `_shared.test.mjs` 判据（含反向用例）；`--only=verify`
  21/21 绿。
