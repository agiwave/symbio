/**
 * VDFS 服务层 —— 统一资源与数据访问的前端入口
 *
 * 与后端对齐：symbio/src/symbio_core/vdfs/host.rs（分发）+ plugins/vdfs（协议入口）
 * 规范：docs/design/vdfs.md（机制）、docs/design/vdfs-frontend.md（前端页面规范）
 *
 * 设计：本层只做「地址 → 请求」的机械翻译，不含任何资源类型知识。
 * 全部资源共用同一组函数；新增挂载点无需改动本文件。
 *
 * ## 口径翻译（本文件是**唯一**翻译点）
 *
 * 前端地址口径 = `.vdfs`（与 LLM 侧 ToolVdfs 的虚拟地址前缀同源）；
 * 线路协议口径 = 规范化全路径 `/…`（vdfs.md §3.1，机制不变）。
 * 出站前 `toWirePath`、入站后 `toVdfsPath`；页面与渲染器**不认识**线路口径。
 */

import { callPlugin } from './plugin'
import {
  VFDS_ACTION,
  VFDS_DELETE,
  VFDS_LIST,
  VFDS_MKDIR,
  VFDS_MOVE,
  VFDS_PROVIDERS,
  VFDS_READ,
  VFDS_STAT,
  VFDS_TREE,
  VFDS_UNWATCH,
  VFDS_WATCH,
  VFDS_WRITE,
  VFDS_ROOT,
  toVdfsPath,
  toWirePath,
  type VdfsActionResponse,
  type VdfsContent,
  type VdfsDeleteResponse,
  type VdfsListResponse,
  type VdfsMountInfo,
  type VdfsMoveResponse,
  type VdfsNode,
  type VdfsProvidersResponse,
  type VdfsTreeResponse,
  type VdfsWriteResponse,
} from '../schemas/vdfs'
import { logger } from '@/utils/logger'

// ==================== 入站口径翻译（线路 `/…` → 前端 `.vdfs/…`） ====================

/** 单个节点：改写 `path` 为前端口径（其余字段原样） */
function inboundNode(n: VdfsNode): VdfsNode {
  return { ...n, path: toVdfsPath(n.path) }
}

function inboundNodes(nodes: VdfsNode[] | undefined): VdfsNode[] {
  return (nodes ?? []).map(inboundNode)
}

/**
 * 拉取挂载点清单（虚拟根 `.vdfs` 的目录内容）。
 * 前端据此生成导航与资源类别，不硬编码任何资源类型。
 */
export async function fetchMounts(): Promise<VdfsMountInfo[]> {
  try {
    const resp = await callPlugin<VdfsProvidersResponse>(VFDS_PROVIDERS, {})
    return (resp?.providers ?? []).map((m) => ({ ...m, root: toVdfsPath(m.root) }))
  } catch (err) {
    logger.error('vdfs-service', 'fetchMounts failed:', err)
    return []
  }
}

/**
 * 列目录。`path` 缺省 = 虚拟根（挂载点清单）。
 * 失败返回空目录（含最小节点），不抛错——列表页永远可渲染。
 */
export async function listVdfs(path = VFDS_ROOT): Promise<VdfsListResponse> {
  try {
    const resp = await callPlugin<VdfsListResponse>(VFDS_LIST, { path: toWirePath(path) })
    if (!resp) return { path, node: emptyNode(path), items: [] }
    return {
      path: toVdfsPath(resp.path),
      node: inboundNode(resp.node),
      items: inboundNodes(resp.items),
    }
  } catch (err) {
    logger.error('vdfs-service', `listVdfs(${path}) failed:`, err)
    return { path, node: emptyNode(path), items: [] }
  }
}

/** 树状遍历（只下钻访问位含 t 的目录） */
export async function treeVdfs(
  path: string,
  opts?: { depth?: number; limit?: number }
): Promise<VdfsTreeResponse> {
  try {
    const resp = await callPlugin<VdfsTreeResponse>(VFDS_TREE, {
      path: toWirePath(path),
      depth: opts?.depth,
      limit: opts?.limit,
    })
    if (!resp) return { path, nodes: [], truncated: false }
    return {
      path: toVdfsPath(resp.path),
      nodes: inboundNodes(resp.nodes),
      truncated: resp.truncated,
    }
  } catch (err) {
    logger.error('vdfs-service', `treeVdfs(${path}) failed:`, err)
    return { path, nodes: [], truncated: false }
  }
}

/** 读元数据；失败返回 null */
export async function statVdfs(path: string): Promise<VdfsNode | null> {
  try {
    const node = await callPlugin<VdfsNode>(VFDS_STAT, { path: toWirePath(path) })
    return node ? inboundNode(node) : null
  } catch (err) {
    logger.debug('vdfs-service', `statVdfs(${path}) failed:`, err)
    return null
  }
}

/** 读内容；失败返回 null */
export async function readVdfs(path: string): Promise<VdfsContent | null> {
  try {
    const content = await callPlugin<VdfsContent>(VFDS_READ, { path: toWirePath(path) })
    return content ? { ...content, path: toVdfsPath(content.path) } : null
  } catch (err) {
    logger.error('vdfs-service', `readVdfs(${path}) failed:`, err)
    return null
  }
}

/**
 * 写内容（创建或覆盖）。
 *
 * 校验失败时**抛错**（错误带字段级信息，用 parseVdfsValidation 还原），
 * 调用方据此逐字段提示——这是 provider 自持校验的消费端约定。
 */
export async function writeVdfs(
  path: string,
  text: string,
  opts?: { create?: boolean; etag?: string }
): Promise<VdfsWriteResponse> {
  const resp = await callPlugin<VdfsWriteResponse>(VFDS_WRITE, {
    path: toWirePath(path),
    text,
    create: opts?.create,
    etag: opts?.etag,
  })
  return { ...resp, path: toVdfsPath(resp?.path ?? path) }
}

/**
 * ArrayBuffer → base64（二进制写入通道的载荷编码）。
 *
 * 分块拼接：整包（zip）可能上兆，一次性 `String.fromCharCode(...bytes)` 会爆栈。
 */
export function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf)
  const CHUNK = 0x8000
  let out = ''
  for (let i = 0; i < bytes.length; i += CHUNK) {
    out += String.fromCharCode(...bytes.subarray(i, i + CHUNK))
  }
  return btoa(out)
}

/** base64 → 字节（与 `arrayBufferToBase64` 互逆；动作带回的文件载荷用它落地） */
export function base64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
  const bin = atob(b64.trim())
  const out = new Uint8Array(new ArrayBuffer(bin.length))
  for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i)
  return out
}

/**
 * 触发一次文件下载（动作结果带回的文件落地）。
 *
 * 只用标准 DOM API：Tauri webview 与浏览器行为一致。文件**内容**由 provider
 * 产出（如「导出」打包的 zip），本层只负责把字节交给用户。
 */
export function downloadBlob(filename: string, data: BlobPart, mime = 'application/zip'): void {
  const url = URL.createObjectURL(new Blob([data], { type: mime }))
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  // 立即 revoke 会让部分 webview 下载落空，下一轮事件循环再释放
  setTimeout(() => URL.revokeObjectURL(url), 0)
}

/** 写二进制内容（base64） */
export async function writeVdfsBinary(
  path: string,
  b64: string,
  opts?: { create?: boolean }
): Promise<VdfsWriteResponse> {
  const resp = await callPlugin<VdfsWriteResponse>(VFDS_WRITE, {
    path: toWirePath(path),
    b64,
    create: opts?.create,
  })
  return { ...resp, path: toVdfsPath(resp?.path ?? path) }
}

/** 删除节点（目录需 recursive） */
export async function deleteVdfs(path: string, recursive = false): Promise<VdfsDeleteResponse> {
  const resp = await callPlugin<VdfsDeleteResponse>(VFDS_DELETE, {
    path: toWirePath(path),
    recursive,
  })
  return { ...resp, path: toVdfsPath(resp?.path ?? path) }
}

/**
 * 执行节点动作（如「测试连接」）。
 *
 * `action` 是 provider 自持的动词标识：本层只做地址翻译，**不解释语义**，
 * 也不认识任何具体动作——按钮由详情定义声明、结果由 provider 回答。
 */
export async function runVdfsAction(
  path: string,
  action: string,
  payload?: unknown
): Promise<VdfsActionResponse> {
  return callPlugin<VdfsActionResponse>(VFDS_ACTION, {
    path: toWirePath(path),
    action,
    ...(payload === undefined ? {} : { payload }),
  })
}

/** 新建目录 */
export async function mkdirVdfs(path: string): Promise<VdfsWriteResponse> {
  const resp = await callPlugin<VdfsWriteResponse>(VFDS_MKDIR, { path: toWirePath(path) })
  return { ...resp, path: toVdfsPath(resp?.path ?? path) }
}

/** 移动 / 重命名（同挂载点内） */
export async function moveVdfs(from: string, to: string): Promise<VdfsMoveResponse> {
  const resp = await callPlugin<VdfsMoveResponse>(VFDS_MOVE, {
    from: toWirePath(from),
    to: toWirePath(to),
  })
  return {
    from: toVdfsPath(resp?.from ?? from),
    to: toVdfsPath(resp?.to ?? to),
  }
}

/**
 * 订阅 / 取消订阅指定子树的数据变更（生命周期与视图严格绑定）。
 * 两个函数均吞错：无实时能力的 provider 由后端默认 no-op。
 */
export async function watchVdfs(path: string): Promise<void> {
  try {
    await callPlugin(VFDS_WATCH, { path: toWirePath(path) })
  } catch (err) {
    logger.debug('vdfs-service', `watchVdfs(${path}) failed:`, err)
  }
}

export async function unwatchVdfs(path: string): Promise<void> {
  try {
    await callPlugin(VFDS_UNWATCH, { path: toWirePath(path) })
  } catch (err) {
    logger.debug('vdfs-service', `unwatchVdfs(${path}) failed:`, err)
  }
}

function emptyNode(path: string): VdfsNode {
  return {
    path,
    name: path,
    title: path,
    kind: 'dir',
    status: 'unknown',
    access: 'l',
  }
}
