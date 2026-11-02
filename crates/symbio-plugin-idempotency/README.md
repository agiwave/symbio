---
kind: design
id: crates/symbio-plugin-idempotency/README
title: 幂等台账插件
status: active
created: 2026-11-02
impl: done
---

# 幂等台账插件（idempotency）

一条领域事实、一个工具、一张视图：模型可以把一个键与它的值记进幂等台账；同一个键
在同一会话里只记一次，重复调用在执行前被拒。名册 #37「幂等与重试」的**幂等半边**。

重试不在本片：重放身份与续写幕的口径属后续片，本插件不假装已经具备。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 工具 | `idempotent_record(key, value)` | 模型（经工具面调用） |
| 行为 | 一次成功记录 ⇒ 一条 `idempotency.recorded` 草稿 | 宿主唯一的写口（I-1，D-213） |
| 事实 | `idempotency.recorded`，格位 `domain × asserted`，字段 `key` / `value` | 写闸按声明校验名字与格位；`seal` 按归宿行判它有没有人看护 |
| 视图 | `idempotency.ledger` → `{ count, entries, keys }` | 任何按名字读的插件（`shared_views`，D-221） |

**没有自有路由、没有配置、没有 VDFS 挂载点、没有新端口。**

## 幂等与拒止

- 幂等键 = 轮锚 + 键（`i-<anchor>-<key>`）：**不带调用 id**，同一轮里同一键的两次调用
  撞同一个事件 id，第二条被存储丢掉；
- 跨轮的同一键在**执行前**被拒（`before` 只看本插件自己的 `idempotency.recorded` 事实，
  不看工具名），模型拿到的工具结果带拒因；
- 同一个键换一个值仍是重复：幂等判的是键，不是值。

## 视图的口径

台账按**首写为准**去重：同一个键只留第一次记录的那条。写入侧本就在 `before` 拒重复，
折叠再按键去重是第二条防线——它让「同轮同键由写口去重」的形态在读数上也只算一格。

## 路由

**无自有路由**（`route` 恒 `NotFound` 并指路到工具与视图）。

## 关联

- 能力包的形状来源（夹具）：`../symbio-plugin-notes/README.md`
- 同类全链样本：`../symbio-plugin-explain/README.md`
- 逐行处置账（#37）：`docs/003-design/026-能力拆分与影响类.md` §2.3
