---
kind: task
id: T-278-Agent权限四字段执行
title: Agent 权限四字段执行面——network / fs.read / fs.write / limits 同批接入（D-102 重开）
status: todo
created: 2026-10-24
track: impl
prio: next
deps: []
parent: T-272-设计实现总收口批
where: symbio/src/plugins/agent/host/manifest.rs · symbio/src/plugins/web/ · symbio/src/plugins/local/shell.rs · symbio/src/plugins/vdfs/ · docs/003-design/011-Agent目录规范.md
exit: '四字段同批有执行方：网络域名白名单在出站前判、fs 读写 glob 在 vdfs 路径入口判、资源限额在装配期判；每个字段一条反向用例（越界必拒且零副作用）；字段名回到 `permissions`（或线格式与 Rust 语义分开的注解保留）；DE-090 §4.1 与 D-102 的状态改写。'
---

## 内容

`manifest.yaml` 的 `permissions` 四个字段今天只是「包作者与宿主的沟通面」：
宿主不据它过滤路径、域名或工具数。规范与实现因此有一处**声明与执行脱节**，
而部分执行比不执行更坏——它让「哪些字段真管用」只能靠读代码。

本任务是 D-102 的重开兑现：**四字段同批**，不许先做 fs 看看效果。

## 判据

| # | 判据 | 承担者 |
|---|---|---|
| 1 | `network.domains` 拦在出站前，拒绝时错误指名主机、靶子留痕为零 | `web` 用例（复用 e2e t65 的守卫判据） |
| 2 | `fs.read/write` 的 glob 相对 Agent 根判定，`..` 一律拒 | `vdfs` 路径入口用例 |
| 3 | `limits` 在装配期判（超限即拒载，不是运行时截断） | `agent` 装配用例 |
| 4 | 四字段各一条反向用例，缺任何一条该字段视为未接入 | 同上 |
| 5 | 文档状态与 D-102 同步（「声明性质」→「执行面」） | DE-090 §4.1 · DE-011 §5.3 |

## 处理过程

- 2026-10-24：登记。字段名 `declared_permissions` 的改名理由（说清它没有执行方）
  随执行面落地一并回退，线格式 `permissions` 保持。
