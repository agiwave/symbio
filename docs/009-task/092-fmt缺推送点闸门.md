---
kind: task
id: T-092-fmt缺推送点闸门
title: fmt 只在整轮门禁与 CI 里判——推送点（pre-push）不判，未格式化源码能推到 origin
status: done
created: 2026-10-13
track: quality
deps: []
prio: next
where: scripts/git-hooks/pre-push
exit: 推送点也判 fmt（`--ci` 语义：只报红、不代改），未格式化源码不再能悄悄推到 origin；代价（工作区口径）已在注释里写明并被接受
evidence: `pre-push` 的调用由 `--only=facts --ci` 改为 `--only=fmt,facts --ci`；注释记下这次漏网的形态（`c8ed5ee7` 手工恢复 `stream.rs` 后未跑格式化，26 个文件推到 `origin/v2`，只在 CI 的 rust-checks 里以「cargo fmt --all 有 27 处差异」呈现）与已知代价（判据看工作区，手边有未格式化 `.rs` 脏改动时会多红一次）。验证：`node scripts/gate.mjs --only=fmt,facts --ci` 6/6 绿、4s（并入 fmt 前实测 0.3s，仍是秒级）。
---

## 内容

fmt 的判据只插在**两处**：整轮门禁（`05-fmt`，本地会自愈并暂存）与 CI 的 rust-checks
（`--only=fmt,backend --ci`）。提交点（pre-commit）按设计**不能**跑 fmt——`cargo fmt --all`
会重排**所有**脏文件，等于替人暂存别人的改动，与它「只暂存生成器自己改写的文件」的纪律
冲突；推送点（pre-push）此前只跑 `--only=facts --ci`。

后果（实测）：`c8ed5ee7` 手工恢复 `stream.rs`（编辑器按 GBK 读 UTF-8 致全文乱码）后没跑
格式化，26 个未格式化源码推到了 `origin/v2`，只在 CI 的 rust-checks 里以「cargo fmt --all
有 27 处差异」呈现——信号有，但迟了一整轮，且不归属造成它的那次提交。

## 处置

把 fmt 并入 pre-push 的 `--ci` 调用。选推送点而非提交点，是因为 pre-push 的既有语义正是
「**只报红、不代改**」（`--ci` 下 `autoWork` 有差异即红），与「不许替人暂存」的纪律相容。

## 已知代价（接受）

判据看的是**工作区**而不是「被推送的那些提交」：推送时若手边有未格式化的 `.rs` 脏改动，
也会红一次。此时正确动作本就是跑一次门禁（会自动格式化并暂存），故按本仓「漏跑远大于
多跑」的取舍接受；急着推可以 `git push --no-verify`。

## 不是本任务范围

- 不给 pre-commit 加 fmt（理由见上：会替人暂存别人的脏改动）。
- 不改 CI 的 rust-checks（它已经在判，本任务只是把同一判据前移到推送点）。

## 处理过程

- 2026-10-13 登记：清偿 `v2` 分支 CI 全红时，发现 rust-checks 的红是 26 个未格式化源码；
  `git show c8ed5ee7` 与 `git log -- scripts/git-hooks/pre-push` 定位为「fmt 无推送点判据」。
- 2026-10-13 清偿：`pre-push` 改为 `--only=fmt,facts --ci`；该调用本地实测 6/6 绿、4s。
