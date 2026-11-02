---
kind: design
id: crates/symbio-plugin-explain/README
title: 可解释插件
status: active
created: 2026-11-02
impl: done
---

# 可解释插件（explain）

一条领域事实、一个工具、一张视图：模型可以为一次决定或动作记下「为什么」，每次成功
记录落成一条 `explain.rationale`（`domain × asserted`），那些事实再被折成一张可读的
理由台账。名册 #49「可解释」的能力包。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 工具 | `explain_record(decision, why, evidence?)` | 模型（经工具面调用） |
| 行为 | 一次成功记录 ⇒ 一条 `explain.rationale` 草稿 | 宿主唯一的写口（I-1，D-213） |
| 事实 | `explain.rationale`，格位 `domain × asserted`，字段 `decision` / `why` / `evidence` | 写闸按声明校验名字与格位；`seal` 按归宿行判它有没有人看护 |
| 视图 | `explain.rationales` → `{ count, entries, decisions }` | 任何按名字读的插件（`shared_views`，D-221） |

**没有自有路由、没有配置、没有 VDFS 挂载点、没有新端口。**

## 幂等与拒止

- 幂等键 = 轮锚 + 调用 id + 决定的短句（`x-<anchor>-<call_id>-<decision>`）：
  同一次重放不会把一条理由记两次；
- 同一会话内相同 `decision` + `why` 的重复在**执行前**被拒（`before` 只看本插件自己的
  `explain.rationale` 事实，不看工具名），模型拿到的工具结果带拒因；
- 同一个 `decision` 换一条 `why` 不是重复：那是同一决定的新理由。

## 视图的口径

`decisions` 是 `entries` 里**去重后的 decided 短句列表**，按写入序保留首次出现。
它不是「哪些决定被解释过」之外的判断——本插件不认识任何领域，只交记录与读数。

## 路由

**无自有路由**（`route` 恒 `NotFound` 并指路到工具与视图）。记录的入口是工具（模型侧）、
读出口是视图（宿主侧），两条都比一条自造协议更短，且都不需要本插件认识调用方。

## 关联

- 能力包的形状来源（夹具）：`../symbio-plugin-notes/README.md`
- 同类全链样本：`../symbio-plugin-causal/README.md`
- 逐行处置账（#49）：`docs/003-design/026-能力拆分与影响类.md` §2.3
