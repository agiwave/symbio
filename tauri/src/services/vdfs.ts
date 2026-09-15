/**
 * VDFS 服务层 —— 统一资源与数据访问的前端入口
 *
 * 与后端对齐：symbio/src/symbio_core/vdfs/host.rs（分发）+ plugins/vdfs（协议入口）
 * 规范：docs/design/vdfs.md（机制）、docs/design/vdfs-frontend.md（前端页面规范）
 *
 * 设计：本层只做「地址 → 请求」的机械翻译，不含任何资源类型知识。
 * 全部资源共用同一组函数；新增一类资源无需改动本文件。
 *
 * 地址口径前后端同源：`.vdfs` 打头 = 系统资源，其余 = 磁盘文件
 * （后端 `UnifiedFs` 一处分流），本层不再做任何地址翻译。
 */

import { callPlugin } from './plugin'
import {
  VDFS_ACTION,
  VDFS_DELETE,
  VDFS_LIST,
  VDFS_MKDIR,
  VDFS_MOVE,
  VDFS_READ,
  VDFS_STAT,
  VDFS_TREE,
  VDFS_UNWATCH,
  VDFS_WATCH,
  VDFS_WRITE,
  VDFS_ROOT,
  type VdfsActionResponse,
  type VdfsContent,
  type VdfsDeleteResponse,
  type VdfsListResponse,
  type VdfsMoveResponse,
  type VdfsNode,
  type VdfsTreeResponse,
  type VdfsWriteResponse,
} from '../schemas/vdfs'
import { logger } from '@/utils/logger'

/**
 * 列目录。`path` 缺省 = `.vdfs` 根目录（左栏导航的来源）。
 * 失败返回空目录（含最小节点），不抛错——列表页永远可渲染。
 */
export async function listVdfs(path = VDFS_ROOT): Promise<VdfsListResponse> {
  try {
    const resp = await callPlugin<VdfsListResponse>(VDFS_LIST, { path })
    if (!resp) return { path, node: emptyNode(path), items: [] }
    return resp
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
    const resp = await callPlugin<VdfsTreeResponse>(VDFS_TREE, {
      path,
      depth: opts?.depth,
      limit: opts?.limit,
    })
    if (!resp) return { path, nodes: [], truncated: false }
    return resp
  } catch (err) {
    logger.error('vdfs-service', `treeVdfs(${path}) failed:`, err)
    return { path, nodes: [], truncated: false }
  }
}

/** 读元数据；失败返回 null */
export async function statVdfs(path: string): Promise<VdfsNode | null> {
  try {
    return await callPlugin<VdfsNode>(VDFS_STAT, { path })
  } catch (err) {
    logger.debug('vdfs-service', `statVdfs(${path}) failed:`, err)
    return null
  }
}

/** 读内容；失败返回 null */
export async function readVdfs(path: string): Promise<VdfsContent | null> {
  try {
    return await callPlugin<VdfsContent>(VDFS_READ, { path })
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
  return callPlugin<VdfsWriteResponse>(VDFS_WRITE, {
    path,
    text,
    create: opts?.create,
    etag: opts?.etag,
  })
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
  return callPlugin<VdfsWriteResponse>(VDFS_WRITE, {
    path,
    b64,
    create: opts?.create,
  })
}

/** 删除节点（目录需 recursive） */
export async function deleteVdfs(path: string, recursive = false): Promise<VdfsDeleteResponse> {
  return callPlugin<VdfsDeleteResponse>(VDFS_DELETE, { path, recursive })
}

/**
 * 执行节点动作（如「测试连接」）。
 *
 * `action` 是 provider 自持的动词标识：本层只做地址传递，**不解释语义**，
 * 也不认识任何具体动作——按钮由详情定义声明、结果由 provider 回答。
 */
export async function runVdfsAction(
  path: string,
  action: string,
  payload?: unknown
): Promise<VdfsActionResponse> {
  return callPlugin<VdfsActionResponse>(VDFS_ACTION, {
    path,
    action,
    ...(payload === undefined ? {} : { payload }),
  })
}

/** 新建目录 */
export async function mkdirVdfs(path: string): Promise<VdfsWriteResponse> {
  return callPlugin<VdfsWriteResponse>(VDFS_MKDIR, { path })
}

/** 移动 / 重命名（同一地址空间内） */
export async function moveVdfs(from: string, to: string): Promise<VdfsMoveResponse> {
  return callPlugin<VdfsMoveResponse>(VDFS_MOVE, { from, to })
}

/**
 * 订阅 / 取消订阅指定子树的数据变更（生命周期与视图严格绑定）。
 * 两个函数均吞错：无实时能力的 provider 由后端默认 no-op。
 */
export async function watchVdfs(path: string): Promise<void> {
  try {
    await callPlugin(VDFS_WATCH, { path })
  } catch (err) {
    logger.debug('vdfs-service', `watchVdfs(${path}) failed:`, err)
  }
}

export async function unwatchVdfs(path: string): Promise<void> {
  try {
    await callPlugin(VDFS_UNWATCH, { path })
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
