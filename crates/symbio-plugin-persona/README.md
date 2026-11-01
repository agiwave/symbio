---
kind: design
id: crates/symbio-plugin-persona/README
title: 人格插件
status: active
created: 2026-11-02
impl: done
---

# 人格插件（persona）

一处定义「我是谁」，**两条输出面读同一段**：轮内正文 / 汇报（`loop` 的一轮）与对话面
措辞（`compose` 的静默往返）。名册 #47「人格语气一致」的能力包。

## 它交什么

| 面 | 交出物 | 谁读 |
|---|---|---|
| 提示词族 | 一条段 `persona-tone`（标题 `【人格】` + 正文） | `session` 的轮首装配与 `compose` 的措辞装配，二者都经 `capability_prompts` 按注册序取全部段 |
| 配置 | 一份配置文档（业务键 `persona`） | 设置页（可配置声明）与 VDFS |

**没有工具、没有事实、没有视图、没有自有路由、没有新端口。** 这正是本条要证明的事：
「同一个人在两条输出面上口吻一致」不需要宿主为它加一格影响类。

## 路由

**无自有路由**（`route` 恒 `NotFound` 并指路到 VDFS）。人格的编辑面是
`<根>/persona/PLUGIN.yml`（`ext = form`，由 `PluginConfigMount` 的四臂提供）——
与 `web` / `memory` 同一取舍：宿主与用户走同一条链路，不为一份配置再造一条协议。

## 平凡值

出厂**不带人格**（那是用户的话）。`persona` 缺席 / 空串 / 纯空白三者同义 ⇒
**一条段都不交** ⇒ 系统提示词与「没有本插件」逐字相同。停用本插件（装配位
`plugin_enabled: false`）同样是「段消失、系统照常运行」。

## 与 `model-persona` 的关系

`model` 插件按生效 provider 的 `system_prompt` 注册段 `model-persona`，它回答的是
「**这个模型**该怎么说话」。两者并存不冲突：人格本体排在前面（装配位 55），
`model-persona` 是对它的补充。把两者合并的代价是「换模型 = 换人格」，以及
「provider 没写 `system_prompt` 时系统里根本没有人格这个概念」。

## 为什么段名不叫 `persona`

能力表是**一张表**，键是行名、族不参与键（`Registry::put`）。本插件同时有一个挂载行
`persona`（`<根>/persona`）——段若同名，会把挂载行从表里顶掉，而**没有任何报错**：
设置页少一项、VDFS 少一个挂载点、配置再也改不了。`memory` 的 `agent-memory` 与
`model` 的 `model-persona` 是同一条纪律。

## 关联

- 提示词族的读写协议：`crates/symbio-sdk/src/capability/prompt.rs`
- 逐行处置账（#47）：`docs/003-design/026-能力拆分与影响类.md` §2.3
- 本条的验证账：`docs/007-task/012-A类能力插件化.md`
