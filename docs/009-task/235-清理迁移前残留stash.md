---
kind: task
id: T-235-清理迁移前残留stash
title: 清理 v2 迁移前的残留 autostash
status: todo
created: 2026-10-22
track: meta
prio: next
deps: []
where: 仓库根（`git stash list`）
exit: '`refs/stash` 里那条迁移前（旧目录 `docs/plan/` · `docs/registry/`）的 autostash 被判定并处置：内容已在 v2 分支以新目录重做 ⇒ 丢弃并留一句处置理由；仍有独有内容 ⇒ 转成有编号的任务或补丁，不留在无名 stash 里。收口时 `git stash list` 为空，或每条 stash 都有明确归属（写出谁 / 为什么留）。'
---

## 内容

`git stash list` 残留一条：

```
stash@{0}: On v2-plan: [Checkout-autostash] before branch switch
```

`git stash show --stat` 显示它动的是 `docs/plan/` 与 `docs/registry/`——这两条目录
在 v2 已重定位为 `docs/004-plan/` 与 `docs/009-task/` 等。它是一次目录搬迁前的
自动暂存，**很可能只是搬迁过程的中间态**，不是待落地的改动。

留着的代价：下一个人看到一条无名 stash，不知道它是不是丢了一份未合并的工作；
`git stash` 又不在任何门禁里，只能靠人记得。

本任务把它判定并清掉（或转正），不是「顺手 drop」——先读一遍确认没有独有的
工作，再处置。

## 处理过程

- 2026-10-22 立项。来源：本轮收口核对时 `git stash list` 发现的历史残留
  （`v2-plan` 分支的 checkout autostash），确认与当前 `v2` 的目录形状不符。
- 处置动作本身是 `git stash drop` / `apply`，**不产生提交**；收口证据写
  「读过的判断 + 最终 `git stash list`」。
