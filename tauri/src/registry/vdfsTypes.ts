/**
 * VDFS 前端渲染注册表 —— **纯 UI 映射**（机制允许前端持有的唯一部分）
 *
 * 规范 §7：后端下发节点的 `ext`（扩展名）与资源类别名，前端据此选择
 * 详情渲染器与图标。**不得**在此硬编码资源类型清单、标签、能力或路径模板。
 *
 * 对应后端：symbio/src/symbio_core/vdfs_provider.rs（ext 约定）
 */

import type { Component } from 'vue'
import { getEntityIcon } from './entityTypes'
import {
  VFDS_EXT_DIR,
  VFDS_EXT_FORM,
  VFDS_EXT_JSON,
  VFDS_EXT_MARKDOWN,
  VFDS_EXT_SESSION,
  VFDS_EXT_TEXT,
  isVdfsDir,
  vdfsExtOf,
  type VdfsNode,
} from '../schemas/vdfs'

/**
 * 详情渲染器标识（机制级的呈现形态，不含场景语义）。
 *
 * - `dir`        目录 → 中栏列表 / 树
 * - `form`       定义驱动表单（解析 node.schema）
 * - `session`    会话工作区
 * - `markdown` / `json` / `text` 文本类编辑器
 * - `appearance` / `about` 前端状态自持的专属面板（外观设置 / 关于）
 * - `fallback`   通用只读兜底
 */
export type VdfsRenderer =
  | 'dir'
  | 'form'
  | 'session'
  | 'markdown'
  | 'json'
  | 'text'
  | 'appearance'
  | 'about'
  | 'fallback'

/** 扩展名 → 渲染器（**唯一的硬编码表**，纯 UI 约定） */
const EXT_RENDERERS: Record<string, VdfsRenderer> = {
  [VFDS_EXT_FORM]: 'form',
  [VFDS_EXT_SESSION]: 'session',
  [VFDS_EXT_MARKDOWN]: 'markdown',
  [VFDS_EXT_JSON]: 'json',
  [VFDS_EXT_TEXT]: 'text',
  [VFDS_EXT_DIR]: 'dir',
  // 前端状态自持的专属面板（ext 即语义类型名，由 provider 声明）
  appearance: 'appearance',
  about: 'about',
  txt: 'text',
  log: 'text',
  yaml: 'text',
  yml: 'text',
}

/** 渲染器组件注册表（由视图层按需注入；未注册的渲染器由视图兜底） */
const RENDERER_COMPONENTS: Record<string, Component> = {}

/** 为某个渲染器登记组件（UI 资产，与数据契约严格分离） */
export function registerVdfsRenderer(renderer: VdfsRenderer, component: Component): void {
  RENDERER_COMPONENTS[renderer] = component
}

/** 取已注册的渲染器组件（未注册返回 undefined） */
export function getVdfsRenderer(renderer: VdfsRenderer): Component | undefined {
  return RENDERER_COMPONENTS[renderer]
}

/**
 * 解析节点的详情渲染器（**前端选择详情页面的唯一入口**）。
 *
 * 目录优先（访问位含 `l`）；其余按扩展名查表；未命中回退 `fallback`。
 */
export function resolveVdfsRenderer(node: VdfsNode | null | undefined): VdfsRenderer {
  if (!node) return 'fallback'
  if (isVdfsDir(node)) return 'dir'
  const ext = vdfsExtOf(node)?.toLowerCase()
  if (!ext) return 'fallback'
  return EXT_RENDERERS[ext] ?? 'fallback'
}

/**
 * `.vdfs` 子目录图标：复用实体图标注册表（子目录名与实体 kind 同名的场景，如
 * `setting` / `model` / `session` / `agent` / `skill` / `mcp`）。
 * 未命中返回 undefined（视图回退默认图标）。
 */
export function dirIconOf(name: string): Component | undefined {
  return getEntityIcon(name)
}
