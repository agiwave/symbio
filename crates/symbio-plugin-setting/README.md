---
kind: design
id: crates/symbio-plugin-setting/README
title: Setting 插件
status: active
created: 2026-09-09
impl: done
---

# Setting 插件

**本智能体自身的信息设置** —— 我是谁（显示名 / 简介），以及我默认怎么说话
（回复语言 / 回答详略）。

## 职责

| 交出什么 | 机制 |
|---|---|
| 系统提示词【本智能体】 | `Registry::put`（提示词段行 `CapabilityRow::prompt`，`segment.rs`） |
| 指令注入段【工作区指令】/【本智能体指令】 | `instructions.rs`：按 `agents_file_name` 读两级指令文件，工作区级存在且非空即覆盖智能体级；两级都无则不注入 |
| 配置文档 `<根>/setting/PLUGIN.yml` | `PluginConfigMount`（四臂 dispatch 由机制提供）+ `capability_announce_configurable` |

## 与相邻插件的分工

| | 本插件 | `memory` | `manifest.yaml`（子智能体） |
|---|---|---|---|
| 内容 | 自述与偏好（结构化字段）+ 两级稳定指令 / 人格 | 动态记忆（自由文本） | 出厂身份与兼容门槛 |
| 形态 | `PLUGIN.yml`（表单编辑）+ `AGENTS.md`（正文，只读注入） | `MEMORY.md`（正文编辑） | `manifest.yaml`（包内，导入即校验） |
| 谁写 | 人（填字段 / 写指令） | 人与模型 | 打包的人 |
| 是否参与接入判定 | 否 | 否 | **是**（owner 插件的 §10 门槛） |

`AGENTS.md` 是**稳定指令 / 人格**，与 `memory` 管的动态记忆语义隔离、物理隔离：
指令两级（`{workdir}/AGENTS.md` + `<agentdir>/AGENTS.md`）是**覆盖**关系——工作区级
存在且非空时大模型不再看智能体级；动态记忆三层（会话 + 工作区 + 智能体）仍是
**叠加**关系、各层可独立开关，统一落在 `MEMORY.md`。指令文件名可由
`agents_file_name` 配置（默认 `AGENTS.md`），记忆文件名固定。

子智能体目录里既有 `manifest.yaml` 又有 `setting/`，两者不冲突：前者是**随包分发
的接入契约**（宿主不改），后者是**本实例运行期可改的自述与偏好**。

## 一个插件，两个实例（分形）

本插件不认识「系统智能体」与「子智能体」：它被装配在哪个智能体目录下，就描述哪个
智能体。

| 实例 | 配置落位 | 生效范围 |
|---|---|---|
| 系统树 | `<homedir>/setting/PLUGIN.yml` | 所有会话 |
| 子树 | `<agentdir>/setting/PLUGIN.yml` | 选中该智能体时 |

子树实例的注册经 owner 插件的作用域访问器加 `agent/<id>/` 前缀，
与系统侧并集且不撞名。

## 收哪些字段

判据是**有没有消费者**：自述四字段（`display_name` / `description` /
`reply_language` / `verbosity`）全都进系统提示词（本插件自己的出口）；
`agents_file_name` 由指令注入读取。图标、默认模型、默认工作区看起来也属于
「智能体的设置」，但它们的消费者在别的插件（前端图标表 / `model` / `work`）里，
本插件交出去没人读——那就是死配置。等消费方接好了再加，不预先发明字段。

四字段缺省全空 ⇒ **不注入【本智能体】段**；两级指令文件都不存在 ⇒ 不注入指令段
（刚装配完不改变任何一轮的提示词）。

## 关联

- 本智能体的记忆：`../memory/README.md`
- 子智能体目录与接入门槛：`../agent/README.md`
- 设置页入口（**不拥有任何设置内容**）：`../plugin_manager/README.md`
- 纯配置挂载点机制：`symbio_platform::sdk::plugin::dir::PluginConfigMount`
