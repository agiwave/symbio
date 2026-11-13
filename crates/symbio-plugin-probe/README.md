---
kind: design
id: crates/symbio-plugin-probe/README
title: 验收探针（probe）
status: active
created: 2026-11-13
impl: done
---

# 验收探针（probe）

它不是一条产品能力，是一个**可执行的断言**：新增一个真实能力包，只需要建目录 + 写
`PLUGIN.yml` + 写代码，`kernel` / `sdk` / `plugin-api` / `loop` / `session` 零改动。
装配清单（`symbio-deploy`）、链接行与依赖行都由 `PLUGIN.yml` 与目录生成。

## 它交什么

| 面 | 交出物 | 说明 |
|---|---|---|
| 端口 | `probe_echo`（`before = EVENT_SENT`，租约 60s，1 次尝试，不限并发） | 把一段文本回显——最小可观测的世界面 |
| 债 | 一笔，按会话前缀惰性 | 证明「申请 → 派发 → 收束」这条链路对**新端口**同样成立 |
| 执行体 | `port.rs` | 只有世界侧的往返，不含任何业务判断 |

端口是**数据不是类型**：环不持有端口清单，只按端口名问冻结表。所以本文件就是「新增一个
世界端口」的全部声明成本——不出本 crate。

## 为什么它不进默认装配

`plugin_assembly: none`：本包没有工具、没有路由、没有挂载点，从 CLI 外面读不到任何用户
可见面，留在默认树里只会让生产多一个空插件。它**仍被链接、仍提交声明**，由通用判据覆盖：

- 工厂可构造 + 端口进冻结表且有受理行 ⇒ `crates/symbio/src/assembly_acceptance.test.rs`；
- 端口闭环的驱动断言属于 e2e 黑盒，**不在宿主 crate 里写专名用例**。

## 它没有 `pub` 项

注册入口是 `plugin.rs` 的 `submit_object_creator!`，不是导出根（E-014）。于是「探针做过什么」
只能通过装配与端口观测，不能被别的 crate import——否则它就成了一个会被依赖的假能力包。

## 关联

- 能力包的最小成本（本包证明的那句话）：`docs/003-design/045-能力包迁出与落地裁决.md`
- 装配与冻结表：`docs/003-design/025-能力包与扩展面.md`
- 端口收口判据：`docs/003-design/048-端口机制裁决.md`
