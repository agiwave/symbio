---
kind: design
id: crates/symbio-plugin-reversibility/README
title: 可逆性设计插件
status: active
created: 2026-11-08
impl: done
---

# 可逆性设计插件（reversibility）

一条领域事实、一个工具、一张视图：模型可以给动作标注可逆等级与补偿做法，每次成功
标注落成一条 `reversibility.assessed`（`domain × asserted`），那些事实再被折成一张
当前登记表。名册 #40「可逆性设计」的能力包。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 工具 | `reversibility_annotate(action, level, undo?)` | 模型（经工具面调用） |
| 行为 | 一次成功标注 ⇒ 一条 `reversibility.assessed` 草稿 | 宿主唯一的写口（I-1，D-213） |
| 事实 | `reversibility.assessed`，格位 `domain × asserted`，字段 `action` / `level` / `undo` | 写闸按声明校验名字与格位；`seal` 按归宿行判它有没有人看护 |
| 视图 | `reversibility.register` → `{ count, observations, by_level, actions }` | 本包私有（没有第二个读者；名字不进 `shared_views`，D-221） |

**没有自有路由、没有配置、没有 VDFS 挂载点、没有新端口。**

## 三个等级

| 等级 | 含义 | 读方该做什么 |
|---|---|---|
| `reversible` | 执行后能按原路退回 | 无需额外准备 |
| `compensable` | 退不回，但有**说得出做法**的补偿 | 执行前先确认补偿可用 |
| `irreversible` | 退不回、也没有补偿 | 需要人确认或干脆不做 |

`compensable` 的 `undo` 在工具与行为两层都要求非空：一个「有补偿」却说不清怎么补偿的
标注比 `irreversible` 更危险——它会让执行方误以为后路存在。缺 `undo` 时**直接拒**，
不静默降级成 `irreversible`，让误判当场暴露。

## 视图的口径

- **同动作后写覆盖**：登记表回答「这个动作现在被判成什么」，等级可以升降；
  两次判断都留在转写里，视图不保存判断史。
- `by_level` 只统计**当前**等级（「现在有多少不可逆动作」）；`observations` 才是证据量。
- 动作按首次标注序输出，超过上限保留最早登记的动作。

## 幂等与拒止

- 幂等键 = 轮锚 + 调用 id + 被标注的动作：同一次调用重放不会记两次，修正判断各自成键；
- **不设执行前拒绝**：同一动作再标注一次是修正判断，不是错误；
- 空动作、不在册的等级、无补偿做法的 `compensable` 在工具与行为两层都拒。

## 路由

**无自有路由**（`route` 恒 `NotFound` 并指路到工具与视图）。标注的入口是工具（模型侧）、
读出口是视图（宿主侧），两条都比一条自造协议更短，且都不需要本插件认识调用方。

## 关联

- 同类全链样本：`../symbio-plugin-causal/README.md`
- 逐行处置账（#40）：`docs/003-design/026-能力拆分与影响类.md` §2.3
