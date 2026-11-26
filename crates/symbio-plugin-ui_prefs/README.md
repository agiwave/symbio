---
kind: design
id: crates/symbio-plugin-ui_prefs/README
title: UI_prefs 插件（界面偏好）
status: active
created: 2026-11-21
impl: done
---

# UI_prefs 插件（界面偏好）

界面偏好的**后端权威数据**：外观（主题 / 字号）与会话结束提示音（开关 / 分类型 / 音量）。

## 定位

此前「外观」是 `plugin_manager` 的自有分区，数据在前端 localStorage，后端没有归属。本插件把这些数据接成配置文档 `<根>/ui_prefs/PLUGIN.yml`，由 `PluginConfigMount` 提供读写、校验与落盘；前端 localStorage 退为**首屏快照 + 离线降级**。

- 入口：插件管理 →「外观」分区（读写转发到本插件）
- 不声明 `configurable`：避免 `ui_prefs` 条目里出现第二张通用外观表单
- 不进侧边栏：`PluginMeta.hidden = true`
- 核心插件：默认加载且不可删除（`plugin_undisablable: true`）

## 「关于」的归属

关于页没有用户数据（版本来自编译期、引导是 UI 流程），因此不落盘。它的节点归属记在本插件名下，正文恒为空。

## 数据形状

| 字段 | 取值 | 缺省 |
|---|---|---|
| `theme` | `light` / `dark` / `auto` | `light` |
| `font_size` | `small` / `medium` / `large` | `medium` |
| `sound_enabled` | bool | `true` |
| `sound_completed` / `sound_aborted` / `sound_failed` | bool | `true` |
| `volume` | 0..1 | `0.6` |

缺省值与前端 `stores/appearance.ts` / `stores/soundSettings.ts` / `services/completionChime.ts` 同源。
