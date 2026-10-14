---
kind: arch
id: scripts/facts/README
title: '`scripts/facts/` —— 事实生成器目录'
status: active
created: 2026-10-16
---

# `scripts/facts/` —— 事实生成器目录

**目录即接线**：`scripts/gen-facts.mjs` 遍历本目录下每个非 `_` 前缀的 `.mjs`，
逐个调用它导出的 `render()`。新增一份事实 = 在这里建一个文件——
不需要改门禁、不需要在任何名单里登记。

由 `scripts/gate.d/60-facts.mjs` 的 `autoWork` 自动执行（生成并暂存）；
CI 侧跑 `node scripts/gen-facts.mjs --check`，任一份漂移即红。

## 一份事实长什么样

```js
export default {
  id: 'current',                                   // 必须等于文件名
  title: '读侧结构事实表',                          // 人读的一句话，进横幅与日志
  out: 'docs/CURRENT.md',                          // 仓根相对；不含 `..`
  stamp: true,                                     // 可选：是否带溯源行
  render: (ctx) => string,                         // 纯函数：同一份代码 ⇒ 同一份输出
}
```

契约由 runner 逐条机检（`facts/_lib.mjs` 的 `validateFact`），违约即红。

## 两条硬约定

1. **`render` 必须是纯函数**。门禁把生成当确定性的机械工作（`autoWork`）：
   跑成功即通过，CI 侧把「跑完仍有差异」判红。掺了时间或随机数，它就会因为
   「每次内容都不同」而永远报红，而红的原因不是漂移。溯源行是唯一例外，
   已由 `stamp` 单列处理（比对时剔除）。
2. **只放离线可重生成的代码投影**，不放「跑起来才产出」的事实——
   `docs/ref/PROMPT_SHAPE.md` 的生成点是 e2e `t46`（真实链路抓包），
   `docs/ref/PROMPT_AB.md` 是门外实验。搬进来会让 runner 在没有运行时的
   场合（CI 的 facts 阶段）静默产出一份错的。

## 不放什么

- **登记汇总页**（`docs/INDEX.md` / 各 kind 的 `INDEX.md`）——它们的输入是
  条目 front-matter，不是代码，生成方是 `registry-audit.mjs --write`。
- **文档引用地址**（`[[编号]]` 展开）——那是改写而不是生成，生成方是
  `gen-doc-refs.mjs`。两者都留在 `60-facts` 阶段里各占一格，
  因为它们与事实生成是**不同种类**的工作：前两者是判据的产物（判定），
  事实生成是投影（改写）。