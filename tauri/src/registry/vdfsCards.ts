/**
 * VDFS 列表卡片呈现注册表 —— 「节点 → 状态点 / 状态文案 / 徽标 / 标签 / 图标」
 * 的**唯一来源**（前端纯 UI 映射）
 *
 * 动机：这五条映射原先内联在 `components/vdfs/VdfsWorkbench.vue` 里，各自带一段
 * 「不许这么改」的注释（徽标只给目录、状态文案不泄漏后端枚举、不渲染机制字段…）。
 * 注释钉不住回归——它们既不在能直接测的地方，也随组件一起膨胀。搬到这里之后：
 * 规则与规则的实现在同一处，且可单测（见 `registry/__tests__/vdfsCards.spec.ts`）。
 *
 * ## 分层（与 registry/vdfsTypes.ts / vdfsIcons.ts 同一套）
 *
 * - **数据/能力**（存在、访问位、顺序、类型清单）一律来自后端，**不在本模块**；
 * - **机制映射**（`ext` → 渲染器）在 `vdfsTypes.ts`，**不在本模块**；
 * - **图标映射**（kind / kind:ext → 图标组件）在 `vdfsIcons.ts`，本模块只**查**它；
 * - 本模块只做最后一跳：**节点 → 卡片上那几个可见元素取什么值**。
 *
 * ## 三条不许回退的约定
 *
 * 1. **徽标只给目录**（子项数）。文件的 `ext` 是**渲染器键**（`form` / `session`），
 *    属机制细节，不该出现在给用户看的列表里。
 * 2. **状态文案只做文案映射**：后端 `status` 枚举（working / active / …）直接当
 *    tooltip 就是把机制词摆给用户看；未知取值返回**空串**——宁可不提示，
 *    也不要把后端枚举漏出去。文案不参与任何判据（能力判据只认访问位）。
 * 3. **不渲染机制级字段**：访问位（`w` = 可写）是能力判据，决定「能不能保存 /
 *    删除 / 新建」，不是给用户看的标签；`ext` / `path` / `kind` 同理。
 *
 * 规范：docs/design/vdfs-frontend.md · docs/design/vdfs.md
 */

import type { Component } from 'vue'
import { isVdfsDir, type VdfsNode } from '@/schemas/vdfs'
import { relativeTime } from '@/utils/time'
import { dirIconOf } from '@/registry/vdfsTypes'
import { getVdfsIcon, getVdfsIconFor } from '@/registry/vdfsIcons'

/** 卡片状态点取值（与 `VdfsCard` 的 `status` prop 同域） */
export type CardStatus = 'active' | 'working' | 'disabled' | 'warning' | 'error' | 'muted'

/** 卡片标签（`VdfsCard` 的 `tags` 元素） */
export interface CardTag {
  label: string
  kind?: 'muted' | 'primary'
}

/**
 * 节点 `status` → 状态点取值。
 *
 * 后端取值是开放枚举（后续可加），故未知一律回落 `muted`（画一个灰点），
 * 而不是不画点——「有状态但认不出」与「没有状态」是两件事：
 * 后者由 `showStatus`（节点 `status` 为空串）单独表达。
 */
const STATUS_OF: Record<string, CardStatus> = {
  working: 'working',
  active: 'active',
  disabled: 'disabled',
  error: 'error',
  warning: 'warning',
}

/**
 * 状态点的 hover 提示（纯 UI 文案）。
 *
 * ⚠️ 未知取值返回**空串**：宁可不显示提示，也不要把后端枚举漏出去。
 * 颜色语义由 `cardStatusOf` 决定，文案不参与任何判据。
 */
const STATUS_TEXT: Record<string, string> = {
  working: '进行中',
  active: '就绪',
  disabled: '已停用',
  error: '出错',
  warning: '需注意',
}

export function cardStatusOf(n: VdfsNode): CardStatus {
  return STATUS_OF[n.status] ?? 'muted'
}

export function cardStatusTextOf(n: VdfsNode): string {
  return STATUS_TEXT[n.status] ?? ''
}

/**
 * 徽标：**只有目录**给（子项数），文件一律不给。
 *
 * `children` 缺省（后端没给子项数）时也不给——不给徽标好过给出一个错的 `0`。
 */
export function cardBadgeOf(n: VdfsNode): string | undefined {
  if (!isVdfsDir(n)) return undefined
  return typeof n.children === 'number' ? String(n.children) : undefined
}

/**
 * 标签：后端声明的类型特有标签（`meta_tags`，VDFS 只透传）+ 相对时间。
 *
 * `meta_tags` 是后端决定的语义（会话的工作目录名 / 消息数之类），前端原样渲染、
 * 不加解释；非字符串与空串一律丢弃（脏数据不该变成空白小方块）。
 */
export function cardTagsOf(n: VdfsNode): CardTag[] {
  const out: CardTag[] = []
  const tags = n.meta_tags
  if (Array.isArray(tags)) {
    for (const label of tags) {
      if (typeof label === 'string' && label) out.push({ label, kind: 'muted' })
    }
  }
  const t = relativeTime(n.updated_at)
  if (t) out.push({ label: t, kind: 'muted' })
  return out
}

/**
 * 图标：目录按**目录名**映射；文件按「kind + 项级扩展名」查项级图标，
 * 再回退 kind 级。全部是纯 UI 映射（VDFS 不下发图标）。
 *
 * 项级标识读节点顶层的 `config_type`（后端 flatten 下发），缺省回落节点名。
 * 查不到时返回 `undefined`——由 `VdfsCard` 决定此时不画图标。
 */
export function cardIconOf(n: VdfsNode): Component | undefined {
  if (isVdfsDir(n)) return dirIconOf(n.name) ?? undefined
  const ext = typeof n.config_type === 'string' && n.config_type ? n.config_type : n.name
  return (
    getVdfsIconFor({ kind: n.kind, config_type: ext }) ??
    getVdfsIcon(n.kind) ??
    dirIconOf(n.kind) ??
    undefined
  )
}
