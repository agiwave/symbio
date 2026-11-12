---
kind: design
id: crates/symbio-plugin-hook/README
title: Hook 插件
status: active
created: 2026-09-09
impl: done
---

# Hook 插件

钩子插件：在关键点（工具调用前后、压缩前）注册并触发钩子。

## 路由

> **命名空间是目录名 `hook`**（不是 `hooks`）——容器按目录名建实例表并按它分发
> （`composite.rs`「目录名 = 实例名」）；`Plugin::meta` 的名字**从不参与路由**。
>
> 清单见 `docs/004-ref/003-ROUTES.md` §Hook 插件（**权威**）：`hook/register`、`hook/fire`、`hook/list`；
> 触发臂是 `fire`（无 `trigger`），调用侧唯一入口是 `crate::routes::ROUTE_HOOK_FIRE`。

## 机制

- 钩子是"在特定事件点执行附加逻辑"的扩展点：用户在配置里按**事件名**注册命令，宿主在关键点
  经 `hook/fire` 发一帧，结果只作信息性附加（`HookOutput` 的 `should_proceed` 只有前置拦截读）。
- **14 个事件名里只有三个会被触发**：`PreToolUse` / `PostToolUse`（工具端口执行体，调用前后）、
  `PreCompact`（压缩端口执行体，压缩前）。其余十一个没有产者——它们要通知的事情已经是一条事实，
  订阅方读事实（`D-478` 给了逐个对应的事实名）。加产者要新裁决，不靠"变体看起来该响"。

## 关联

- 触发面裁决：[DE-007-第三方插件体系](../../docs/003-design/007-第三方插件体系.md "DE-007-第三方插件体系") 的 `D-478`
- 压缩生命周期：`../plugin-compact/README.md`（`PreCompact` 的产者在它的端口执行体里）
