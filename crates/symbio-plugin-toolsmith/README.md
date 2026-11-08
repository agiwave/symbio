---
kind: design
id: crates/symbio-plugin-toolsmith/README
title: 制造工具插件（L2 受限）
status: active
created: 2026-11-08
impl: done
---

# 制造工具插件（toolsmith）

一条领域事实、一个工具、一张视图：模型可以登记**本插件自己的**工具定义（名字、说明、
入参 schema），每次成功登记落成一条 `toolsmith.defined`（`domain × asserted`），那些事实
再被折成一张当前定义清单。名册 #41「制造工具（L2）」的受限能力包。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 工具 | `toolsmith_define(name, description, input_schema)` | 模型（经工具面调用） |
| 行为 | 一次成功登记 ⇒ 一条 `toolsmith.defined` 草稿 | 宿主唯一的写口（I-1，D-213） |
| 事实 | `toolsmith.defined`，格位 `domain × asserted`，字段 `name` / `description` / `input_schema` / `owner` | 写闸按声明校验名字与格位；`seal` 按归宿行判它有没有人看护 |
| 视图 | `toolsmith.recipes` → `{ count, observations, tools }` | 任何按名字读的插件（`shared_views`，D-221） |

**没有自有路由、没有配置、没有 VDFS 挂载点、没有新端口。**

## 限制：只能造自己插件的工具

- 工具入参里**没有** `owner` / `target`：调用方没有地方填别人的插件；
- 行为把 owner 写成常量 `toolsmith`：夹带 `{"owner": "vdfs"}` 也落不成别人的名字；
- 视图按 owner 再过滤一遍：即便别的写方塞进一条 owner 不同的 `toolsmith.defined`，
  也读不出来。

限制在两个方向都堵上，才不依赖调用方守规矩。

## 限制：本片只交定义与清单

运行期装配不存在（D-217），因此定义**不改本轮工具面**；进入可调用工具面要等下一次
装配——那是名册 #42（造工具的工具，L3）的事。本包对那条路的承诺只有一句：定义带着
owner，将来装配期按 owner 过滤即可，不需要改本包的形状。

## 名字与幂等

- 名字规则：小写字母开头、只含 `[a-z0-9_]`、3–40 字符——「将来能被工具面唯一解析」的最小前提；
- 幂等键 = 轮锚 + 调用 id + 工具名：同一次调用重放不会记两次，定义修订各自成键；
- **不设执行前拒绝**：同名再登记按后写覆盖（说明或 schema 可以不同），不是错误；
- 空说明、非法名字、非对象 schema 在工具与行为两层都拒。

## 路由

**无自有路由**（`route` 恒 `NotFound` 并指路到工具与视图）。

## 关联

- 同类全链样本：`../symbio-plugin-causal/README.md`
- 逐行处置账（#41）：`docs/003-design/026-能力拆分与影响类.md` §2.3
