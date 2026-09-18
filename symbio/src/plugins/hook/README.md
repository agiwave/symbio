# Hook 插件

钩子插件：在生命周期关键点（如会话压缩前 PreCompact）注册并触发钩子。

## 路由

> **命名空间是目录名 `hook`**（不是 `hooks`）：容器按目录名建实例表并按它分发
> （`composite.rs`「目录名 = 实例名」）。本 README 与 `ROUTES.md` 曾据
> `PluginMeta::new("hooks", …)` 写成 `hooks/*`——而 `Plugin::meta()` 全仓无生产消费方，
> 那个名字**从不参与路由**，于是两处文档写的是**不存在的路径**。已修正（2026-09-18）。
>
> 清单见 `docs/reference/ROUTES.md` §Hook 插件（**权威**）：`hook/register`、`hook/fire`、`hook/list`。
> 触发臂是 `fire`（无 `trigger`）。调用侧唯一入口是 `symbio_core::paths::HOOK_FIRE`
> （`session/tool_executor.rs::fire_hook`）。

## 机制

- 钩子是"在特定事件点执行附加逻辑"的扩展点；session 在主动压缩前会触发 PreCompact 钩子（见 `../session/README.md` 策略⑥）。
- 钩子执行结果不影响主流程终态，仅作信息性附加。

## 关联

- 压缩生命周期：`../session/README.md`
