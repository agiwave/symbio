/**
 * 级联选项机制 —— 前端唯一实现（会话作用域）
 *
 * ## 定位
 *
 * 会话输入区下方的「选项行」完全由后端下发（选项宿主 = session 插件，
 * `worker/session/options/list`）。本组合式函数实现**机制**：
 *
 * - 拉取根选项 / 懒加载子项（`parent` 参数，与 VDFS 的目录寻址同构）；
 * - 通用分派 `dispatch`：把 `OptionAction` 变为真实调用——
 *   `payload` 深拷贝 → 原生取值原语（`pick`）→ 动态值写入点路径（`bind`）→
 *   `callPlugin(endpoint, payload)`；
 * - 会话状态落库的**本地镜射**：把写入的 `metadata` 补丁同步到会话 store
 *   （列表元数据 + 每会话 map 回填），使 `store.activeWorkdir` 等即时收敛。
 *
 * ## 零业务
 *
 * 本文件不含任何业务字段名（workdir / agent_id / risk_level…）：这些全部由
 * 后端在节点上声明，前端只搬运 `action.payload` 与 `action.bind`。
 *
 * ## 草稿态（新建会话前）
 *
 * 会话尚不存在时没有落库目标，`dispatch` 把本应写入 `metadata` 的补丁缓存到
 * `draftMetadata`，由新建流程在创建会话时一次性写入——选择项与真实会话完全同构。
 */

import { ref, watch, type Ref } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { callPlugin } from '@/services/plugin'
import { listOptions } from '@/services/options'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'
import type { OptionAction, OptionNode } from '@/schemas/options'

/** 机制原生取值原语（与后端 `OPTION_PICK_*` 对齐） */
const PICK_DIRECTORY = 'directory'
const PICK_FILE = 'file'

/** 分派结果：`applied` = 本次写入的动态值（原生取值 / 表单值），供调用方做本地回显 */
export interface OptionDispatchResult {
  applied?: unknown
  response?: unknown
}

export interface UseSessionOptions {
  /** 根选项（已按 order 排序） */
  nodes: Ref<OptionNode[]>
  loading: Ref<boolean>
  /** 草稿态已缓冲的 metadata 补丁（无会话时的选择暂存） */
  draftMetadata: Ref<Record<string, unknown>>
  /** 重新拉取根选项 */
  refresh: () => Promise<void>
  /** 懒加载某节点的子项（`sub` 内联为空时） */
  loadChildren: (parentId: string) => Promise<OptionNode[]>
  /** 通用分派（invoke / form 保存共用） */
  dispatch: (action: OptionAction, dynamic?: { value: unknown }) => Promise<OptionDispatchResult>
  /** 是否存在可落库的会话 */
  hasSession: () => boolean
}

/** JSON 深拷贝（选项载荷恒为 JSON 安全值） */
function cloneJson<T>(v: T): T {
  try {
    return JSON.parse(JSON.stringify(v ?? null)) as T
  } catch {
    return v
  }
}

/** 按点路径写入（父级缺失或非对象时自动补对象） */
function setByPath(root: Record<string, unknown>, path: string, value: unknown) {
  const parts = path.split('.').filter(Boolean)
  if (parts.length === 0) return
  let cur: Record<string, unknown> = root
  for (let i = 0; i < parts.length - 1; i++) {
    const k = parts[i]
    const next = cur[k]
    if (!next || typeof next !== 'object' || Array.isArray(next)) cur[k] = {}
    cur = cur[k] as Record<string, unknown>
  }
  cur[parts[parts.length - 1]] = value
}

/** 唤起原生选择对话框（机制级原语，不含业务语义） */
async function pickNative(kind: string): Promise<string | null> {
  const directory = kind === PICK_DIRECTORY
  if (!directory && kind !== PICK_FILE) return null
  const picked = await open({
    directory,
    multiple: false,
    title: directory ? '选择目录' : '选择文件',
  })
  if (typeof picked === 'string') return picked
  if (Array.isArray(picked) && typeof picked[0] === 'string') return picked[0]
  return null
}

/**
 * 会话作用域选项机制。
 *
 * @param sessionId 当前会话 id 的取值函数；返回空 = 草稿态（新建会话前）
 */
export function useSessionOptions(sessionId: () => string | undefined): UseSessionOptions {
  const store = useSessionsStore()
  const nodes = ref<OptionNode[]>([])
  const loading = ref(false)
  const draftMetadata = ref<Record<string, unknown>>({})

  const hasSession = () => Boolean(sessionId()?.trim())

  async function refresh() {
    loading.value = true
    try {
      const resp = await listOptions(sessionId())
      nodes.value = resp.nodes ?? []
    } catch (err) {
      logger.error('useSessionOptions', 'refresh 失败', err)
    } finally {
      loading.value = false
    }
  }

  async function loadChildren(parentId: string): Promise<OptionNode[]> {
    const sid = sessionId()
    if (!sid) return []
    try {
      const resp = await listOptions(sid, parentId)
      return resp.nodes ?? []
    } catch (err) {
      logger.error('useSessionOptions', 'loadChildren 失败', err)
      return []
    }
  }

  /** 从载荷中提取应落库的 metadata 补丁（`session/update` 的浅合并语义） */
  function metadataPatchOf(payload: unknown): Record<string, unknown> | null {
    if (!payload || typeof payload !== 'object') return null
    const meta = (payload as Record<string, unknown>).metadata
    if (meta && typeof meta === 'object' && !Array.isArray(meta)) {
      return meta as Record<string, unknown>
    }
    return null
  }

  async function dispatch(
    action: OptionAction,
    dynamic?: { value: unknown },
  ): Promise<OptionDispatchResult> {
    const payload = (cloneJson(action.payload) ?? {}) as unknown
    let applied: unknown

    // ① 原生取值原语：后端无法唤起原生对话框，前端取值后写入 bind 点路径
    if (action.pick) {
      const picked = await pickNative(action.pick)
      if (!picked) return {} // 用户取消
      applied = picked
      if (action.bind && payload && typeof payload === 'object') {
        setByPath(payload as Record<string, unknown>, action.bind, picked)
      }
    } else if (dynamic !== undefined && action.bind && payload && typeof payload === 'object') {
      // ② 表单 / 程序化动态值：写入 bind 点路径
      applied = dynamic.value
      setByPath(payload as Record<string, unknown>, action.bind, dynamic.value)
    }

    const patch = metadataPatchOf(payload)

    // ③ 草稿态：无落库目标 → 缓冲补丁，创建会话时一次写入
    if (!hasSession()) {
      if (patch) {
        draftMetadata.value = { ...draftMetadata.value, ...patch }
      }
      return { applied }
    }

    // ④ 正常分派：写后端 → 本地镜射 → 重拉选项（回显权威选中值）
    const sid = sessionId()!.trim()
    const response = await callPlugin(action.endpoint, payload, undefined, { session_id: sid })
    if (patch) store.applySessionMetadataPatch(sid, patch)
    await refresh()
    return { applied, response }
  }

  // 会话切换（含草稿态 → 真实会话）时重载根选项；草稿缓冲在进入真实会话后失效
  watch(
    () => sessionId(),
    (sid, prev) => {
      if (sid && !prev) draftMetadata.value = {}
      void refresh()
    },
    { immediate: true },
  )

  return {
    nodes,
    loading,
    draftMetadata,
    refresh,
    loadChildren,
    dispatch,
    hasSession,
  }
}
