---
kind: task
id: T-243-Builder六口与按会话seal
title: Builder 六口与按会话 seal——行/端口/执行体经注册口进表，LazyLock 单例退役
status: todo
created: 2026-10-22
track: plan
parent: T-236-义务环符合性收口批
prio: next
deps: [T-241-ActorDecl执行体注册表, T-242-装载检查余四道]
where: symbio/src/symbio_core/decl/mod.rs · symbio/src/plugins/loop/loop_rows.rs
exit: 'PL-016 §3.2 六个注册口齐：`fact / view / actor / port / row / seal`（现为 3/6）；`loop_rows.rs` 的 static 行切片与端口切片改经 `row` / `port` 入口装载；「运行期不能注册是类型不是纪律」保持——`Session` 只持 `Arc<Frozen>`；进程级 `LazyLock` 单例改为按会话构造注入（或明示其等价性并给出类型依据）；判据 2「表里零代码」复查通过；全量门禁与 e2e 绿，单次提交 ≤ 300 行。'
---

## 内容

父任务：[T-236-义务环符合性收口批](236-义务环符合性收口批.md)。

Builder 现缺 `actor / port / row` 三口，执行体与行散落在插件 static 数据里靠
`LazyLock` 汇聚——声明层（五张注册表 + seal 七检）与数据面（行表）中间隔着一层
未成文的胶水。补齐后 §3.3「三个时点」在代码上可指认。

## 验收口径

1. `Builder` 方法面 = 6（fact / view / actor / port / row / seal），有编译期或测试断言；`loop_rows` 的 static 切片不再是行的唯一进表通道（或明文论证等价性并挂 ADR）。
2. 「运行期不能注册是类型」：Session 构造后无可达可变注册口——`verify/` 下 `should_not_compile` 用例或等价编译断言。
3. CLI 复验两幕全过 + 门禁全绿；CS-001 / PTS-003 不违规（变宽需 allow 行 + ADR）。

## 处理过程

- 2026-10-22 登记（PL-016 三面对账：方案 → 代码 → CLI 实证，见 T-236 内容节）。
