---
kind: design
id: scripts/README
title: '''`scripts/` —— 脚本目录的布局与机制'''
status: active
created: 2026-11-13
impl: done
---

# `scripts/` —— 脚本目录的布局与机制

**每个子目录是一种机制，根目录只放入口与共享库。** 判据不是「文件放在哪看着顺眼」，
而是「新增一份要不要改某个阶段的源码」：要改的就不合格——手写清单的失效形态是静默
（脚本在仓里、门禁全绿、它从没被执行过），而这种事没人会记得去查。

## 子目录 = 机制

| 目录 | 谁遍历它 | 新增一份要做什么 |
|---|---|---|
| `audits/` | `gate.d/30-docs.mjs` 按 `// @gate guard` 收编 | 建 `x-audit.mjs` + 同名 `.test.mjs` + 声 `@gate guard` |
| `gate.d/` | `gate.mjs` 按文件名序加载 | 建 `<序号>-<名>.mjs` 并 `export default { id, tasks }` |
| `gens/` | `06-facts-write`（`write`）与 `60-facts`（`check`） | 建 `x.mjs` + 声 `// @gens <序号> write check` |
| `facts/` | `gens/gen-facts.mjs` 逐个调 `render()` | 见 [facts/README.md](facts/README.md "scripts/facts/README") |
| `baselines/` | 由各自的 owner 脚本读 | 不是机制面：数据文件，随 owner 走 |
| `exemptions/` | 由各自的 owner 守卫读 | 同上 |
| `git-hooks/` | git 的 `core.hooksPath` | 钩子文件，入口在 `check-commit-msg.mjs` |

四条「目录即清单」的读法各自只有**一处实现**：`gate.d/_shared.mjs` 的
`gateDeclarations` / `gensDeclarations` / `facts/_lib.mjs` / `gate.mjs` 的阶段加载。
声明是自述，所以都有反向对账（`gate-wiring-audit` 的 GW-001…008）：声明了却没被跑到，
在那里红。

## 根目录 = 入口 + 共享库

根目录**不许再放静态审计脚本或生成器**——它们有家。留下的是两类：

- **入口**（人直接敲的）：`gate.mjs`、`registry-board.mjs`、`doc-find.mjs`、
  `windows-restart.mjs` / `-prepare`、`cargo-offline-refresh.mjs`、
  `obligation-ring-accept.mjs`、`check-commit-msg.mjs`、`cli-binary.mjs` / `tauri-binary.mjs`
  （后两者是「按指纹决定要不要重建」的构建入口）。
- **共享库**（只被 import，声明 `@gate lib`）：`color`、`md-table`、`fs-walk`、`rust-scan`、
  `line-count`、`decision-anchors`、`registry-lib`、`route-facts`、`commit`、
  `core-change-backfill`、`e2e-order`、`e2e-concurrency`。

库不再分目录：它们**本来就不需要接线**（没有清单、没有阶段会跑它们），
按 kind 分子目录只会把所有 `import` 路径重写一遍——那是纯改写，没有收益。

## 为什么不是「一个大目录 + 前缀」

`*-audit.mjs` / `gen-*.mjs` 的名字前缀**不是**分组依据：名字是给人读的，判据的输入是
声明。把名字变成判据的输入，改名就成了一次判据变更（`GW-007` 因此只做单向：
`-audit` 名字必须在 `audits/` 下，反过来不成立——`projection-purity` 就在里面）。
