/**
 * useVdfs 的纯 helper —— **不持有任何响应式状态**，只吃入参、给结果。
 *
 * 从 `useVdfs.ts` 抽出，目的只有两个：让那个上千行的 composable 少承担一份
 * 与响应式无关的判据逻辑；让这些判据能被直接单测（不必 mount composable、
 * 不必造一整套 service mock）。函数体与签名逐字保留，调用方一字未改。
 *
 * 分层不变：本文件与 `useVdfs.ts` 同在 composables 层，只向下依赖 schemas
 * （契约层），不引入任何新的跨层依赖方向。
 */

import {
  VDFS_STATUS_ACTIVE,
  vdfsParent,
  type VdfsItem,
  type VdfsNewType,
  type VdfsTreeNode,
} from '@/schemas/vdfs'

/** 在树里按完整地址找一项（树回执是扁平的，层级只体现在 `path`） */
export function findTreeNode(nodes: VdfsTreeNode[], path: string): VdfsTreeNode | null {
  for (const node of nodes) {
    if (node.item.path === path) return node
    const hit = findTreeNode(node.children, path)
    if (hit) return hit
  }
  return null
}

/**
 * 路由带来的选中项（`?select=`）归一化：只接受**一个直接子项名**。
 *
 * 带分隔符 / 上跳 / 空值的形状一律视为「没有选中项」——宁可落到目录页，
 * 也不拿一个能拼出界外地址的字符串去做 join。
 */
export function focusNameOf(raw: string | null): string | null {
  const name = (raw ?? '').trim()
  if (!name || name === '.' || name === '..') return null
  if (name.includes('/') || name.includes('\\')) return null
  return name
}

/** 条目是否命中筛选词（标题 / 名称 / 描述 + 标签，任一子串命中即算） */
export function nodeMatchesFilter(n: VdfsItem, q: string): boolean {
  if (n.name.toLowerCase().includes(q)) return true
  if (n.title?.toLowerCase().includes(q)) return true
  if (n.description?.toLowerCase().includes(q)) return true
  // `meta_tags` 走 VdfsNode 的索引签名（类型是 unknown），故先做形状判定再取值——
  // 后端脏数据（非数组 / 数组里混非字符串）必须被静默忽略，不能让它把整次筛选炸掉。
  const tags = n.meta_tags
  if (!Array.isArray(tags)) return false
  return tags.some((t) => typeof t === 'string' && t.toLowerCase().includes(q))
}

/**
 * 是否影响某目录**的条目集合**：变更路径是它自身 / 它的**直接**子项 / 它的祖先。
 *
 * 判据是「这次变更会不会改变该目录的列表」——只有**直接子项**的增删改会。
 * 更深的层级（孙辈及以下）不改变本目录的条目集合，为它重拉整目录纯属浪费：
 * 会话转写是在 `<sid>/message/<mid>` 这一层逐帧变更的，而浏览器常停在
 * `<根>/session` 或 `<根>`——「任意后代都算命中」会让每一条消息帧都挂一次
 * 防抖刷新。
 *
 * 祖先命中保留原语义：祖先节点变了，本目录节点的视图（`cwdNode`）与它在
 * 兄弟间的排序可能跟着变，而本目录自身的路径与条目不变。
 */
export function affects(dir: string, changePath: string): boolean {
  return changePath === dir || vdfsParent(changePath) === dir || dir.startsWith(`${changePath}/`)
}

/**
 * 草稿节点 = 「新建」的选中态：**没有 id、也没有名字**（判据见
 * `schemas/vdfs.isVdfsDraft`——没有路径，即还没落盘）。
 *
 * 它只需要两样东西就够渲染出**该类型真实的详情页**：
 *
 * - `ext` = 该类型落成后的节点扩展名（渲染器键）。它**不总是** `type.ext`：
 *   后者是呈现扩展名（地址末段后缀，如 `model`），而落成后的节点是
 *   `form`（配置型资源的详情是定义驱动表单）。后端用 `node_ext` 声明这个差
 *   异，缺省时两者相同（会话即如此）；
 * - `schema` = 该类型的呈现描述（form 定义）——没有它表单渲染不出任何字段。
 *
 * 其余呈现字段留空——「还没有的东西」不该假装有内容（用户第 2 点：新建时
 * 详情页是缺 id / 名字的，选中一项后才是带内容的那一页）。
 */
export function draftNodeOf(type: VdfsNewType): VdfsItem {
  return {
    // 草稿**没有地址**（还没落盘）：空串即 `isVdfsDraft` 的判据
    path: '',
    name: '',
    title: '',
    kind: type.ext,
    status: VDFS_STATUS_ACTIVE,
    access: 'w',
    ext: type.node_ext || type.ext,
    // 呈现描述随类型下发：草稿与落成后**同一张详情**（用户第 1 点）
    schema: type.schema,
    // 类型自己的说明（如「在详情页里填好，保存时一次写入」）——草稿页据此
    // 告诉用户这一步要做什么
    description: type.description,
    // 图标键读 `config_type`（与清单项同源）——纯 UI 映射，草稿照给
    config_type: type.ext,
  }
}
