---
kind: design
id: crates/symbio-plugin-sandbox/README
title: 安全运行环境插件（sandbox）
status: active
created: 2026-11-23
impl: partial
---

# 安全运行环境插件（sandbox）

沙箱是**被调用的执行环境**，不是反过来依赖工具实现。本插件拥有具名档案、能力约束、
审批策略与按档案隔离的永久授权；平台保留所有工具不可绕过的执行前闸门。

## 交出什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| ctx 句柄 | `Arc<dyn SandboxFace>`，兼容许可、风险与拦截查询 | 编排层与工具环预检 |
| ctx 策略 | `Arc<dyn ApprovalPolicy>`，单次求值放行、拒绝、拦截或需审批 | 平台强制闸门 |
| VDFS 资源 | `<根>/sandbox/<id>` 具名档案 | 管理界面 |
| Control 路由 | `sandbox/grant`、`sandbox/grants`、`sandbox/revoke` | 可信管理界面 |

接口形状定义在 `symbio_platform::sdk::sandbox` 与 `sdk::collectors::approval_policy`，
门面转出为 `symbio_plugin_api`。SDK 与门面**不含任何工具名、op 常量、黑白名单配置项**。

## 策略与授权

- 未选择档案使用内置 `default`；显式 `none` 才表示不限制。显式缺失或损坏失败关闭。
- 工具提供意图事实与动态风险；本插件按开放事实 kind 匹配规则，不维护工具清单。
- 有效风险取工具风险与规则风险下限的最大值；会话偏好与永久授权都不能突破档案阈值。
- `once` 始终可用；档案决定 `session` / `permanent` 是否开放。超过档案阈值只允许单次批准。
- 一次性审批绑定 `(profile_id, revision)`；当前档案变化或同名重建后旧绑定失效。
- 永久授权按档案隔离存入 `approval-grants.json`，只抬不降；旧会话授权仅迁入 `default`，
  非空迁移先备份，任何迁移错误都阻止策略发布。

## 管理边界

- 档案写入、删除与授权 Control 路由只接受可信 `VdfsCaller::Manager`。
- Control 路由修改授权事实，**不是工具执行面**；实际执行仍经平台 `GatedExecutor`。
- 前端通过通用表单与审批卡消费定义，不理解沙箱内部规则，也不自行选择授权档案。

## 不做什么

- **不认识工具**：没有工具清单、没有 op 常量、注册收口没有沙箱分支；
- **不进架构层**：策略、具名沙箱配置、拦截规则都住在本插件；
- **不声称真隔离**：没有进程、容器或网络隔离，审批控制的是是否允许执行，不是隔离执行；
- **不写第二份策略**：宿主不回写会话 mode / risk 作为最终有效策略。
