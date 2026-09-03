/**
 * 资源类型注册表（前端纯展示层）—— editor 组件 + icon 按 kind 注册
 *
 * 分层原则（见 composables/useResourceProviders.ts）：
 * - 类型的**存在性/能力/前缀/顺序/标签**全部来自后端 `resources/providers` 下发的
 *   ProviderInfo（是权威，前端不再硬编码类型清单）；
 * - 本模块只维护**前端 UI 专属**的映射：某 kind 的专属编辑表单（editor 组件）与
 *   SVG 图标。后端不参与下发 Vue 组件 / SVG。
 *
 * ## 新增一种资源类型的两步扩展位（如未来的 setting）
 *
 * 1. 后端：实现 ResourceProvider + 插件 route 接 dispatch + `provider_registry()`
 *    登记一条——前端即可自动发现（生成导航、进入统一资源页）；
 * 2. 前端（可选）：`registerResourceEditor(kind, 表单组件)` 与
 *    `registerResourceIcon(kind, SVG 组件)`——未注册的走通用兜底
 *    （zip 面板 / JSON 编辑器 / 只读详情）。
 */

import { markRaw, shallowReactive, type Component } from 'vue'
import ModelProviderForm from '@/components/resources/ModelProviderForm.vue'

/** 前端展示登记：某 kind 的专属编辑器（form 组件），未登记走通用兜底 */
const editors = shallowReactive<Record<string, Component>>({})

/** 某 kind 的图标组件（SVG），未登记走默认图标，见 Icons.description☆ */
export function registerResourceEditor(kind: string, component: Component): void {
  editors[kind] = component
}

export function getResourceEditor(kind: string): Component | undefined {
  return editors[kind]
}

/** 前端展示预留：kind → 自定义图标组件（v-for 后有默认图标兜底） */
const icons = shallowReactive<Record<string, Component>>({})

export function registerResourceIcon(kind: string, icon: Component): void {
  icons[kind] = icon
}

export function getResourceIcon(kind: string): Component | undefined {
  return icons[kind]
}

// 内置注册：model 使用独立表单
registerResourceEditor('model', markRaw(ModelProviderForm))

/** 构造资源路径唯一标识：`[provider]/[id].[kind]`（如 `model/openai.model`） */
export function resourcePath(provider: string, id: string, kind: string): string {
  return `${provider}/${id}.${kind}`
}