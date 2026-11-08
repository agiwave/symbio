---
kind: design
id: crates/symbio-plugin-world_model/README
title: 世界模型插件
status: active
created: 2026-11-08
impl: done
---

# 世界模型插件（world_model）

一条领域事实、一个工具、一张视图：模型可以记下「某个实体的某个属性当前是什么值」，
每次成功观察落成一条 `world.observed`（`domain × asserted`），那些事实再被折成一张
可读的当前世界视图。名册 #35「世界模型」的能力包。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 工具 | `world_observe(entity, attribute, value, confidence?)` | 模型（经工具面调用） |
| 行为 | 一次成功观察 ⇒ 一条 `world.observed` 草稿 | 宿主唯一的写口（I-1，D-213） |
| 事实 | `world.observed`，格位 `domain × asserted`，字段 `entity` / `attribute` / `value` / `confidence` | 写闸按声明校验名字与格位；`seal` 按归宿行判它有没有人看护 |
| 视图 | `world.model` → `{ count, observations, entities }` | 任何按名字读的插件（`shared_views`，D-221） |

**没有自有路由、没有配置、没有 VDFS 挂载点、没有新端口。**

## 视图的口径

- **同实体同属性后写覆盖**：视图回答「现在是什么」，不保存时间线；时间线留在
  转写里，视图不替读方保存。
- `observations` 是证据量（含被覆盖的），`count` 是实体数：两者不互相推导。
- 实体按首次观察序输出；世界模型是长期累积的读数，先进入世界的实体更可能是读方要找的。

## 幂等与拒止

- 幂等键 = 轮锚 + 调用 id + 被观察的键：同一次调用重放不会记两次，不同观察各自成立；
- **不设执行前拒绝**：同一实体同一属性再观察一次不是错误，是它变了的证据；
- 空实体 / 空属性 / 空值在工具与行为两层都拒：读数里没有信息量的格子只会让读方多滤一次。

## 路由

**无自有路由**（`route` 恒 `NotFound` 并指路到工具与视图）。观察的入口是工具（模型侧）、
读出口是视图（宿主侧），两条都比一条自造协议更短，且都不需要本插件认识调用方。

## 关联

- 同类全链样本：`../symbio-plugin-causal/README.md`
- 逐行处置账（#35）：`docs/003-design/026-能力拆分与影响类.md` §2.3
