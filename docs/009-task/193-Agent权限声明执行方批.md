---
kind: task
id: T-193-Agent权限声明执行方批
title: Agent 权限声明执行方批——permissions 要么有闸，要么改名「声明即声明」
status: done
created: 2026-10-20
track: plan
prio: next
deps: []
parent: T-183-质量与机制清偿总批
where: symbio/src/plugins/agent/host/scope.rs · symbio/src/plugins/agent/host/plugin.rs · symbio/src/plugins/agent/host/manifest.rs · symbio/src/symbio_core/authz.rs · docs/010-adr/
exit: '二选一落地并留痕：① 补执行方——`FsPerms.read` / `write` 过滤子树暴露的 `vdfs` 族能力，`NetworkPerms.domains` 过滤 web 族出网域名，`Limits.max_prompt_bytes` 在提示词拼接处截断并显式告知模型，`Limits.max_tools` 在工具注册处截断，四个执行点各有用例且写法为「默认拒绝、显式放行」；② 降级——字段改名为声明性质（如 `declared_permissions`）或在 manifest 模块文档与规范里明写「本字段当前无执行方」；任一条都要有 ADR 登记；「少一行 = 一条拒绝」的方向不破；全量门禁与 e2e 全绿'
evidence: 提交 `20a5db18`。取 ②（降级）并留痕：[ADR-102](../010-adr/102.md "ADR-102") 裁决 Rust 字段 `permissions` → `declared_permissions`，线格式仍是 `permissions`（`permissions_stay_on_the_wire_while_the_rust_field_says_declared` 钉住双向）；规范 §5.3 与 §11-4 明写「当前无执行方」，§11-4 由「应当约束」改为如实陈述。不补执行方的理由（四个执行点单独接任一个都会产生新的假边界）与补齐四前提（作用域 provider / 出网单口 / 截断裁决 / 空表方向）落 ADR。RE-010 新增「声明·Agent 权限」③ 行，豁免期限指向 ADR-102。门禁：lib 全绿（manifest 新用例 5/5）、clippy 净、docs/facts 73/73、e2e 77/77。
---

## 内容

父设计：09 报告建议 S-1 · [PL-012](../004-plan/012-高阶智能体平台v2.md "PL-012-高阶智能体平台v2") §3 判据 J3（静默失效）

**问题**：`manifest.permissions` 是本机制**唯一一处 J3 违反**，且它是**面向第三方 Agent 包的
公开契约面**——包作者读到的是一份权限清单，运行期却没有任何一处代码读它。

| 字段 | 声明的意思 | 实际执行方 |
|---|---|---|
| `FsPerms.read` / `write` | 该 Agent 只能读写这些路径 | 无 |
| `NetworkPerms.domains` | 只能访问这些域名 | 无 |
| `Limits.max_prompt_bytes` | 提示词有上限 | 无 |
| `Limits.max_tools` | 工具数有上限 | 无 |

**「看起来像安全边界的假边界」比没有边界更糟**：包作者会据此设计自己的包（「我只申请了
读 `/data`，所以写不进去」），而系统实际给他全部能力。这不是「功能没做」，是**契约在骗人**。

**两条路，必须选一条并留痕**：

- **① 补执行方**（推荐）：四个执行点分布在能力过滤（`host/scope.rs`）、工具注册
  （`host/plugin.rs`）、提示词拼接、域名过滤。写法必须是**默认拒绝、显式放行**——
  反过来写（默认放行、显式拒绝）会漏，而漏一处 = 权限形同虚设，且比无声明更糟。
- **② 降级**：把字段改名或在文档里明写「本字段当前无执行方」。成本很低，但它把
  「假边界」变成「已知的声明」——这至少不再骗人。

**为什么是 next 而不是 later**：它是安全面 + 公开契约面，且是「J3 违反」在本仓的**样板案例**。
[T-184](184-机制强制等级清单批.md "T-184-机制强制等级清单批") 建清单时，这一条会被标成 ③ 类；
本批是它的第一个真实消项。

## 处理过程

- 2026-10-20 立批。来源：09 报告 §5 S-1（该报告标为「高价值」）。
- 2026-10-20 与 [T-195](195-委派权威交集批.md "T-195-委派权威交集批") 的分工：本批管
  **Agent 包声明的能力范围**（静态清单的执行），T-195 管**委派时的能力继承**（运行期绑定）。
  两者都在授权面，但一个是「包说自己能干什么」，一个是「子会话能干什么」。
- 2026-10-20 一处前置核查：先 grep 全仓 manifest，确认存量 Agent 包是否已经写了这些字段
  （若已写而无人执行，说明用户已经在按假边界设计，收紧时要有迁移提示）。
