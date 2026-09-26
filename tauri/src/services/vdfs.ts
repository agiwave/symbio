/**
 * VDFS 服务层 —— 统一资源与数据访问的前端入口
 *
 * 与后端对齐：symbio/src/symbio_core/vdfs/host.rs（分发）+ plugins/vdfs（协议入口）
 * 规范：docs/design/vdfs.md（机制）、docs/design/vdfs-frontend.md（前端页面规范）
 *
 * 设计：本层只做「地址 → 请求」的机械翻译，不含任何资源类型知识。
 * 全部资源共用同一组函数；新增一类资源无需改动本文件。
 *
 * 地址口径前后端同源：根锚点打头 = 系统资源，其余 = 磁盘文件
 * （后端 `UnifiedFs` 一处分流），本层不再做任何地址翻译。
 */

import { callPlugin, type PluginOptions } from './plugin'
import {
  VDFS_ACTION,
  VDFS_DELETE,
  VDFS_LIST,
  VDFS_READ,
  VDFS_ROOT_OP,
  VDFS_STAT,
  VDFS_UNWATCH,
  VDFS_WATCH,
  VDFS_WRITE,
  type VdfsActionResponse,
  type VdfsContent,
  type VdfsListResponse,
  type VdfsNode,
  type VdfsWriteResponse,
} from '../schemas/vdfs'
import { setVdfsRoot, vdfsRoot, vdfsRootResolved } from '../schemas/vdfsRoot'
import { logger } from '@/utils/logger'
import { withFallback } from './fallback'
// 回读理由是**词表**，不是本层的服务——从它的独立模块取，消费方也走同一处
// （见 `services/readback.ts` 的「为什么单独成一个模块」）。
import type { ReadbackReason } from './readback'

/** 有界列表的窗口参数：`limit` 条、游标 `before` 之后 */
export interface VdfsListOptions {
  limit?: number
  before?: string
}

/**
 * 三个回读动词共用的调用口——把**理由**放进 `metadata.origin`。
 *
 * 走的是 [`callPlugin`] 这一条既有出口（**不是第二套实现**）：超时、握手、错误
 * 口径全部复用，因此「带理由的调用」与「不带理由的调用」不可能行为分叉。
 *
 * 超时显式给出是因为它在 `callPlugin` 的形参里排在 options 之前——写死一个与
 * 缺省值相同的数，好过让调用方以为这里换了一套超时。
 */
const READBACK_TIMEOUT_MS = 30_000

function callReadback<TOutput>(
  reason: ReadbackReason,
  path: string,
  input: unknown
): Promise<TOutput> {
  return callPlugin<TOutput>(path, input, READBACK_TIMEOUT_MS, { origin: reason })
}

/**
 * **引导根锚点**（幂等）：调一次 `vdfs/root`，把回包里的根地址登记进
 * `schemas/vdfsRoot`。这是前端与「根叫什么」的唯一接触点——之后所有模块
 * 都从锚点读，后端改挂载名前端零改动。
 *
 * `main.ts` 在挂载前 `await` 本函数：路由换算（`vdfsAddress`）读锚点，
 * 必须先于首次导航就绪。失败（无 vdfs 插件 / IPC 断开）不抛错——锚点留空，
 * 地址代数退化为「无虚拟半」，页面仍可渲染物理半；错误已记日志可诊断。
 */
export async function ensureVdfsRoot(): Promise<void> {
  if (vdfsRootResolved()) return
  await withFallback(
    async () => {
      // `vdfs/root` 的定义就是「不给地址」：给了也不看，免得出现两套入参
      const resp = await callPlugin<VdfsListResponse>(VDFS_ROOT_OP, {})
      if (resp?.path) {
        setVdfsRoot(resp.path)
        logger.info('vdfs-service', 'root anchored:', resp.path)
      } else {
        logger.error('vdfs-service', 'vdfs/root 未返回根地址，虚拟半不可用')
      }
    },
    () => {},
    { tag: 'vdfs-service', what: 'vdfs/root failed，虚拟半不可用' }
  )
}

/**
 * 列目录。`path` 缺省 = 虚拟根目录（左栏导航的来源）。
 * 失败返回空目录（含最小节点），不抛错——列表页永远可渲染。
 *
 * `opts` 只在给了字段时才发出对应键：**不传参数时请求形状与从前一致**
 * （多一个 `limit: undefined` 也会被序列化成键，改变请求体形状）。
 */
export async function listVdfs(
  reason: ReadbackReason,
  path = vdfsRoot(),
  opts?: VdfsListOptions
): Promise<VdfsListResponse> {
  const payload: Record<string, unknown> = { path }
  if (opts?.limit !== undefined) payload.limit = opts.limit
  if (opts?.before) payload.before = opts.before
  // 空目录要带上 `path`，故兜底是 thunk（惰性）而非常量
  const empty = (): VdfsListResponse => ({ path, node: emptyNode(path), items: [] })
  return withFallback(
    async () => (await callReadback<VdfsListResponse>(reason, VDFS_LIST, payload)) ?? empty(),
    empty,
    { tag: 'vdfs-service', what: `listVdfs(${path}) failed` }
  )
}

/** 读元数据；失败返回 null。`reason` 见 [`READBACK_REASON`]（必填，进路由留痕）。 */
export function statVdfs(reason: ReadbackReason, path: string): Promise<VdfsNode | null> {
  // 节点不存在是**预期内**的失败（stat 的常规用法就是先探一下），故降为 debug
  return withFallback(() => callReadback<VdfsNode>(reason, VDFS_STAT, { path }), () => null, {
    tag: 'vdfs-service',
    what: `statVdfs(${path}) failed`,
    level: 'debug',
  })
}

/** 读内容；失败返回 null。`reason` 见 [`READBACK_REASON`]（必填，进路由留痕）。 */
export function readVdfs(reason: ReadbackReason, path: string): Promise<VdfsContent | null> {
  return withFallback(() => callReadback<VdfsContent>(reason, VDFS_READ, { path }), () => null, {
    tag: 'vdfs-service',
    what: `readVdfs(${path}) failed`,
  })
}

/**
 * 写内容（创建或覆盖）。
 *
 * 校验失败时**抛错**（错误带字段级信息，用 parseVdfsValidation 还原），
 * 调用方据此逐字段提示——这是 provider 自持校验的消费端约定。
 *
 * `opts.ctx` 是**调用上下文**（`workdir` / `session_id` 等）。多数写入不需要它
 * （目标地址已经说明了一切），但会话收件箱的写入需要：后端把它作为
 * `start_turn` 回退链的第一档工作目录（`vdfs_host_ctx(ctx).get(WORKDIR)`），
 * 丢了它就只能靠会话 metadata——而「会话还没绑过 workdir」正是新建会话那一刻。
 */
export async function writeVdfs(
  path: string,
  text: string,
  opts?: { create?: boolean; etag?: string; ctx?: PluginOptions }
): Promise<VdfsWriteResponse> {
  return callPlugin<VdfsWriteResponse>(
    VDFS_WRITE,
    {
      path,
      text,
      create: opts?.create,
      etag: opts?.etag,
    },
    undefined,
    opts?.ctx
  )
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

/**
 * 删除节点（目录需 recursive）。
 *
 * **无回执载荷**：删哪儿是调用方自己说的，回执里没有信息量（与 watch / unwatch
 * 同形：成功即成功）。
 */
export async function deleteVdfs(path: string, recursive = false): Promise<void> {
  await callPlugin(VDFS_DELETE, { path, recursive })
}

/**
 * 执行节点动作（如「测试连接」）。
 *
 * `action` 是 provider 自持的动词标识：本层只做地址传递，**不解释语义**，
 * 也不认识任何具体动作——按钮由详情定义声明、结果由 provider 回答。
 *
 * `ctx` 是调用上下文，与 [`writeVdfs`] 同款（少数动作需要它，如会话恢复需要
 * 把 `mode` / `risk_level` 之外的运行上下文带给编排）。
 */
export async function runVdfsAction(
  path: string,
  action: string,
  payload?: unknown,
  ctx?: PluginOptions
): Promise<VdfsActionResponse> {
  return callPlugin<VdfsActionResponse>(
    VDFS_ACTION,
    {
      path,
      action,
      ...(payload === undefined ? {} : { payload }),
    },
    undefined,
    ctx
  )
}

/**
 * 订阅 / 取消订阅指定子树的数据变更（生命周期与视图严格绑定）。
 * 两个函数均吞错：无实时能力的 provider 由后端默认 no-op。
 */
export async function watchVdfs(path: string): Promise<void> {
  await withFallback(() => callPlugin(VDFS_WATCH, { path }), () => {}, {
    tag: 'vdfs-service',
    what: `watchVdfs(${path}) failed`,
    level: 'debug',
  })
}

export async function unwatchVdfs(path: string): Promise<void> {
  await withFallback(() => callPlugin(VDFS_UNWATCH, { path }), () => {}, {
    tag: 'vdfs-service',
    what: `unwatchVdfs(${path}) failed`,
    level: 'debug',
  })
}

/** 列目录失败时的兜底节点：纯自述，**不带地址**（地址由 `VdfsListResponse.path` 承载） */
function emptyNode(path: string): VdfsNode {
  return {
    name: path,
    title: path,
    kind: 'dir',
    status: 'unknown',
    access: 'l',
  }
}
