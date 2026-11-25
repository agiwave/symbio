/**
 * VDFS 前端渲染注册表 —— **纯 UI 映射**（机制允许前端持有的唯一部分）
 *
 * 规范 §7：后端下发节点的 `ext`（扩展名）与资源类别名，前端据此选择
 * 详情渲染器与图标。**不得**在此硬编码资源类型清单、标签、能力或路径模板。
 *
 * 对应后端：crates/symbio-platform/src/kernel/vdfs/node.rs（ext 约定）
 */

import { createRendererRegistry } from './factory'
import {
  VDFS_EXT_DIR,
  VDFS_EXT_FORM,
  VDFS_EXT_JSON,
  VDFS_EXT_MARKDOWN,
  VDFS_EXT_MESSAGE,
  VDFS_EXT_SESSION,
  VDFS_EXT_TEXT,
  VDFS_KIND_INBOX,
  VDFS_KIND_MESSAGES,
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
 * - `message`    单条对话消息（**转写列表项**的只读视图）
 * - `markdown` / `json` / `text` 文本类编辑器
 * - `appearance` 配置型插件 ui_prefs 的专属面板（外观 + 提示音）
 * - `fallback`   通用只读兜底
 */
export type VdfsRenderer =
  | 'dir'
  | 'form'
  | 'session'
  | 'message'
  | 'markdown'
  | 'json'
  | 'text'
  | 'appearance'
  | 'market'
  | 'fallback'

/**
 * 「正文即**文本缓冲**」的渲染器子集 —— 正文原样呈现（不 parse 成结构），
 * 表单与二进制不在其列。
 *
 * 这个集合曾被写了三遍，且**其中一遍其实不是同一个集合**：
 * `useVdfs` 的读取守卫用的是「文本缓冲 ∪ `form`」（表单的字段值也来自正文，
 * 同样需要读一次），而 `VdfsWorkbench` 用的是纯文本缓冲。
 * 形状相同不等于同一件事——所以这里给出**两个**具名谓词，而不是硬凑一个。
 *
 * （原先还有第三处：追加守卫。它随 `appended` 变更一起删除——变更不再带增量
 * 载荷，没有本地增量会被在途 `read` 的旧快照覆盖。）
 */
const TEXTUAL_RENDERERS: ReadonlySet<VdfsRenderer> = new Set([
  'text',
  'markdown',
  'json',
  'message',
])

/** 该渲染器把节点正文当**文本缓冲**看待（原样呈现，不 parse） */
export function isTextualRenderer(r: VdfsRenderer): boolean {
  return TEXTUAL_RENDERERS.has(r)
}

/** 表单渲染器：字段值从正文 parse，保存走 `saveFields` 而不是整篇 `saveText`。 */
export function isFormRenderer(r: VdfsRenderer): boolean {
  return r === 'form'
}

/**
 * 数据来自**节点自述 `schema`**、而非正文的渲染器（目前只有插件市场）。
 *
 * 市场是「添加插件」点开的一张草稿页：草稿没有落盘、没有正文，候选由后端随
 * `new_type.schema` 下发（D-520）。把它排除在文本类之外，机制就不会去读一个
 * 不存在的节点正文。
 */
export function rendererReadsNodeSchema(r: VdfsRenderer): boolean {
  return r === 'market'
}

/**
 * 某个**落成扩展名**是不是「schema 驱动的选择器页」（目前即市场）。
 *
 * 供「进入目录自动开草稿」用：自动开的前提是「空目录唯一能做的就是填这张表单」，
 * 而选择器页的数据在 `schema` 里，进目录第一眼该是列表（D-520）。判据从注册表导出，
 * 前端不写资源类型名，也不改内核协议字段。
 */
export function isSchemaDrivenExt(ext: string | null | undefined): boolean {
  if (!ext) return false
  const r = EXT_RENDERERS[ext.toLowerCase()]
  return r ? rendererReadsNodeSchema(r) : false
}

/** 节点的项级类型是不是会话（ext 判据的唯一出处，框架层只问这个谓词）。 */
export function isSessionExt(ext: string | null | undefined): boolean {
  return ext === VDFS_EXT_SESSION
}

/** 该节点是不是一个会话（列表筛选 / 挂载点认领共用同一判据）。 */
export function isSessionNode(node: VdfsNode): boolean {
  return isSessionExt(vdfsExtOf(node))
}

/** 会话内部的转写段（按 `kind` 认，不按展示段名）。 */
export function isMessagesSeg(node: { kind?: string }): boolean {
  return node.kind === VDFS_KIND_MESSAGES
}

/** 会话内部的收件箱段（同上）。 */
export function isInboxSeg(node: { kind?: string }): boolean {
  return node.kind === VDFS_KIND_INBOX
}

/**
 * 该渲染器需要**读一次节点正文**才谈得上呈现。
 *
 * = 文本缓冲类 ∪ `form` ∪ `appearance`：表单字段值同样取自正文（`vdfs/read` 的
 * `text`，前端 parse 成对象后作为显式入参交给渲染器）；外观面板的偏好值来自
 * `ui_prefs` 的配置文档（一份 JSON 正文，D-521），同样必须先读。
 * 其余（`dir` / `session` / `market` / `fallback`）各有自己的数据来源——
 * 市场的候选随 `new_type.schema` 下发（见 `rendererReadsNodeSchema`），替它们读正文
 * 是白读。
 */
export function rendererReadsNodeText(r: VdfsRenderer): boolean {
  return isFormRenderer(r) || isTextualRenderer(r) || r === 'appearance'
}

/**
 * 保存载荷**本身就是正文文本**的渲染器（目前只有外观面板）。
 *
 * 外观面板把 `ui_prefs` 的偏好对象序列化成 JSON 文本后 `emit('save')`，工作台
 * 应原样写回节点，而不是拿本地 `nodeText` 缓冲——后者是读取时的快照，用户刚改的
 * 值还没落进它（D-521）。
 */
export function rendererPayloadIsText(r: VdfsRenderer): boolean {
  return r === 'appearance'
}

/** 扩展名 → 渲染器（**唯一的硬编码表**，纯 UI 约定） */
const EXT_RENDERERS: Record<string, VdfsRenderer> = {
  [VDFS_EXT_FORM]: 'form',
  [VDFS_EXT_SESSION]: 'session',
  // 消息是**只读列表项**（访问位只有 `r`）：专用只读视图按 `attributes` 展示
  // 角色 / 类型 / 状态 / 错误，正文取节点内容——与「正文在内容、结构在 attributes」
  // 的分工一一对应。正文是文本缓冲，由 `useVdfs` 的一次 `read` 填充。
  [VDFS_EXT_MESSAGE]: 'message',
  [VDFS_EXT_MARKDOWN]: 'markdown',
  [VDFS_EXT_JSON]: 'json',
  [VDFS_EXT_TEXT]: 'text',
  [VDFS_EXT_DIR]: 'dir',
  // 配置型插件 ui_prefs 的专属面板（ext 即语义类型名，由 provider 声明）
  appearance: 'appearance',
  market: 'market',
  txt: 'text',
  log: 'text',
  yaml: 'text',
  yml: 'text',
}

/**
 * VDFS 详情域的渲染器注册表（机制来自 `registry/factory`）。
 *
 * 「标识 → 组件 + 兜底」这条机制只有一份实现；本文件只声明**本域的两个约定**：
 * 标识的联合类型（`VdfsRenderer`）与兜底键（`fallback`）。
 */
const renderers = createRendererRegistry<VdfsRenderer>('fallback')

/** 为某个渲染器登记组件（UI 资产，与数据契约严格分离） */
export const registerVdfsRenderer = renderers.register

/** 取已注册的渲染器组件（未注册返回 undefined） */
export const getVdfsRenderer = renderers.get

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
