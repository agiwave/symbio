/**
 * useVdfs —— VDFS 通用资源页的唯一页面逻辑
 *
 * 不含任何资源类型知识（原实体机制的 `useWorkbenchView` 已随实体页下线）：
 * 挂载点来自后端注册、目录内容来自 `vdfs/list`、详情渲染器由节点的 `ext`
 * 决定（`registry/vdfsTypes`）。新增一种资源 = 后端实现一个 `VdfsProvider`，
 * 本文件零改动。
 *
 * ## 状态模型
 *
 * - **挂载点**（`mounts`）：虚拟根 `/` 的目录内容，同时是左栏导航；
 * - **当前目录**（`cwd`）：中栏列表的来源；点目录进入、点文件选中；
 * - **选中节点**（`selectedNode`）：详情来源，按 `ext` 解析渲染器；
 * - **实时**：订阅总线 `vdfs` 频道，本挂载点范围内防抖刷新（非轮询）。
 *
 * ## 校验错误的消费约定（要点五的消费端）
 *
 * `write` 失败时 provider 自持校验会返回**字段级**错误。服务层抛错，本层用
 * `parseVdfsValidation` 还原为 `fieldErrors`，供表单渲染器逐字段高亮。
 */

import { computed, onBeforeUnmount, ref, shallowRef, watch, type Ref } from 'vue'
import {
  arrayBufferToBase64,
  base64ToBytes,
  deleteVdfs,
  downloadBlob,
  fetchMounts,
  listVdfs,
  mkdirVdfs,
  moveVdfs,
  readVdfs,
  runVdfsAction,
  unwatchVdfs,
  watchVdfs,
  writeVdfs,
  writeVdfsBinary,
} from '@/services/vdfs'
import { subscribe } from '@/services/eventBus'
import {
  VFDS_EVENT_KIND,
  VFDS_ROOT,
  actionFileOf,
  isVdfsDir,
  mountNavVisible,
  newFileNameOf,
  parseVdfsValidation,
  vdfsBase,
  vdfsJoin,
  vdfsMountOf,
  vdfsParent,
  type VdfsChange,
  type VdfsFieldError,
  type VdfsMountInfo,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'
import { mountIconOf, resolveVdfsRenderer, type VdfsRenderer } from '@/registry/vdfsTypes'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import type { NavRailItem } from '@/components/common/NavRail.vue'

/** 面包屑一项 */
export interface VdfsCrumb {
  label: string
  path: string
}

export interface UseVdfsOptions {
  /** 路由 `:mount` 参数（可选）：决定初始挂载点 */
  mountParam?: Ref<string | undefined>
}

export function useVdfs(opts: UseVdfsOptions = {}) {
  const { showToast } = useToast()
  const mountParam = opts.mountParam

  // ==================== 挂载点（虚拟根） ====================
  const mounts = ref<VdfsMountInfo[]>([])

  /** 当前目录全路径；`/` = 虚拟根（挂载点清单） */
  const cwd = ref<string>(VFDS_ROOT)
  /** 当前挂载点（由 cwd 首段决定；虚拟根下为空串） */
  const activeMount = computed(() => vdfsMountOf(cwd.value))

  /**
   * 左栏导航 = 导航可见的挂载点（图标为纯 UI 映射）。
   *
   * 可见性由后端机制层声明（`nav_visible`），前端只按标记过滤——不做任何
   * 挂载名特判：隐藏的子树仍可经 `.vdfs/<挂载名>` 寻址访问。
   */
  const railItems = computed<NavRailItem[]>(() =>
    mounts.value.filter(mountNavVisible).map((m) => ({
      key: m.mount,
      label: m.label || m.mount,
      icon: mountIconOf(m.mount) ?? null,
      description: m.description,
      active: activeMount.value === m.mount,
    }))
  )

  /** 页标题：虚拟根 = 资源；挂载点根 = 挂载点标签；深层 = 末段名 */
  const title = computed(() => {
    if (cwd.value === VFDS_ROOT) return '资源'
    const m = mounts.value.find((x) => x.mount === activeMount.value)
    const root = m?.root || vdfsJoin(VFDS_ROOT, activeMount.value)
    if (cwd.value === root) return m?.label || activeMount.value
    return vdfsBase(cwd.value) || m?.label || '资源'
  })

  /** 面包屑（根 → … → 当前目录）；挂载点段用挂载点标签 */
  const breadcrumbs = computed<VdfsCrumb[]>(() => {
    if (cwd.value === VFDS_ROOT) return []
    const segs = cwd.value.slice(VFDS_ROOT.length + 1).split('/').filter(Boolean)
    const out: VdfsCrumb[] = [{ label: '资源', path: VFDS_ROOT }]
    let acc = VFDS_ROOT
    segs.forEach((s, i) => {
      acc = vdfsJoin(acc, s)
      const label = i === 0 ? mounts.value.find((m) => m.mount === s)?.label || s : s
      out.push({ label, path: acc })
    })
    return out
  })

  // ==================== 目录内容 ====================
  const items = ref<VdfsNode[]>([])
  const cwdNode = shallowRef<VdfsNode | null>(null)
  const loading = ref(false)
  const loadError = ref('')

  /** 当前目录是否可写（列表头「新建目录」可见性由访问位决定，前端不硬编码） */
  const canWriteHere = computed(() => {
    const node = cwdNode.value
    if (!node) {
      const m = mounts.value.find((x) => x.mount === activeMount.value)
      return Boolean(m?.access?.includes('w'))
    }
    return node.access.includes('w')
  })

  /** 加载当前目录 */
  async function refresh() {
    loading.value = true
    loadError.value = ''
    try {
      const resp = await listVdfs(cwd.value)
      items.value = resp.items
      cwdNode.value = resp.node
      // 选中项若已不在当前目录（被删/被移走），清理选中态
      const sel = selectedNode.value
      if (sel && !items.value.some((n) => n.path === sel.path)) clearSelection()
    } catch (err) {
      loadError.value = String(err)
      items.value = []
      logger.error('useVdfs', `加载目录失败 ${cwd.value}:`, err)
    } finally {
      loading.value = false
    }
  }

  /** 进入目录（或虚拟根） */
  async function enter(path: string) {
    if (path === cwd.value) return
    cwd.value = path
    clearSelection()
    await refresh()
  }

  /** 返回上一级 */
  async function goUp() {
    const parent = vdfsParent(cwd.value)
    if (parent !== cwd.value) await enter(parent)
  }

  /** 切到某挂载点（左栏点击） */
  async function switchMount(mount: string) {
    const m = mounts.value.find((x) => x.mount === mount)
    await enter(m?.root || vdfsJoin(VFDS_ROOT, mount))
  }

  /** 点列表项：目录 → 进入；文件 → 选中 */
  async function activate(node: VdfsNode) {
    if (isVdfsDir(node)) await enter(node.path)
    else await select(node)
  }

  // ==================== 选中项与详情 ====================
  const selectedNode = shallowRef<VdfsNode | null>(null)
  const selectedId = computed(() => selectedNode.value?.path ?? null)

  /** 详情渲染器（**前端选择详情页面的唯一入口**，按 ext 解析） */
  const renderer = computed<VdfsRenderer>(() => resolveVdfsRenderer(selectedNode.value))

  /** 文本类渲染器（text / json / markdown）的内容副本 */
  const nodeText = ref('')
  /** form 渲染器的数据（`vdfs/read` 文本按 JSON 解析） */
  const formData = ref<Record<string, unknown> | null>(null)
  const nodeBinary = ref(false)
  const loadingDetail = ref(false)

  const saving = ref(false)
  /** 动作执行中（与 saving 分开：动作不写数据，别让保存态跟着转） */
  const actionBusy = ref(false)
  /** 详情级错误（纯文本） */
  const detailError = ref('')
  /** 字段级校验错误（provider 自持校验的产物；机制不解释其含义） */
  const fieldErrors = ref<VdfsFieldError[]>([])

  /** 详情读取代次（防竞态：见 select 内说明） */
  let detailToken = 0

  function clearSelection() {
    detailToken++
    selectedNode.value = null
    nodeText.value = ''
    formData.value = null
    nodeBinary.value = false
    detailError.value = ''
    fieldErrors.value = []
  }

  /** 选中节点并按其渲染器加载数据 */
  async function select(node: VdfsNode | null) {
    if (!node) {
      clearSelection()
      return
    }
    selectedNode.value = node
    nodeText.value = ''
    formData.value = null
    nodeBinary.value = false
    detailError.value = ''
    fieldErrors.value = []

    const r = resolveVdfsRenderer(node)
    if (r !== 'form' && r !== 'text' && r !== 'json' && r !== 'markdown') return

    // 读取代次令牌：连点多项时，慢响应不得覆盖新选中项的数据
    const token = ++detailToken
    loadingDetail.value = true
    try {
      const content = await readVdfs(node.path)
      if (token !== detailToken) return
      if (!content) {
        detailError.value = '读取失败'
        return
      }
      nodeBinary.value = Boolean(content.binary)
      const text = content.text ?? ''
      if (r === 'form') {
        try {
          formData.value = text.trim() ? (JSON.parse(text) as Record<string, unknown>) : {}
        } catch (err) {
          detailError.value = `内容不是合法 JSON：${err}`
        }
      } else {
        nodeText.value = text
      }
    } finally {
      if (token === detailToken) loadingDetail.value = false
    }
  }

  // ==================== 写入 / 删除 / 新建 ====================
  /** 还原字段级校验错误；非本协议载荷按纯文本展示 */
  function captureError(err: unknown): string {
    const parsed = parseVdfsValidation(err)
    if (parsed) {
      fieldErrors.value = parsed.fields ?? []
      return parsed.message
    }
    fieldErrors.value = []
    return String(err)
  }

  /** 写入选中节点内容（form 传字段值对象，文本渲染器传文本） */
  async function write(payload: Record<string, unknown> | string): Promise<boolean> {
    const node = selectedNode.value
    if (!node) return false
    const text = typeof payload === 'string' ? payload : JSON.stringify(payload)
    saving.value = true
    detailError.value = ''
    fieldErrors.value = []
    try {
      await writeVdfs(node.path, text)
      showToast('success', `已保存「${node.title || node.name}」`)
      // 写入可能改变节点元数据（大小/状态）→ 就地重载
      await Promise.all([refresh(), select(node)])
      return true
    } catch (err) {
      detailError.value = captureError(err)
      showToast('error', `保存失败：${detailError.value}`)
      return false
    } finally {
      saving.value = false
    }
  }

  /** form 渲染器保存入口（DetailForm option 绑定语义：纯字段值） */
  async function saveFields(values: Record<string, unknown>): Promise<boolean> {
    return write(values)
  }

  /** 文本渲染器保存入口 */
  async function saveText(): Promise<boolean> {
    return write(nodeText.value)
  }

  /**
   * 执行**节点动作**（如「测试连接」「导出」）。
   *
   * 动作标识由详情定义声明、由 provider 解释，本层只负责把它送到
   * `vdfs/action` 并把结果（成功 / 失败 + 说明）呈现出来；**前端不认识
   * 任何具体动作**，新增动作无需改动这里。
   *
   * 唯一例外是**文件载荷**：结果里带 `filename` + `b64` 就下载它（见
   * `actionFileOf`）。这是形状判定而非动作判定——「导出」只是当前唯一
   * 按此形状回传数据的动作。
   */
  async function runAction(action: string): Promise<boolean> {
    const node = selectedNode.value
    if (!node) return false
    actionBusy.value = true
    detailError.value = ''
    try {
      const r = await runVdfsAction(node.path, action)
      if (r?.ok) {
        showToast('success', r.message || '执行成功')
        const file = actionFileOf(r.data)
        if (file) downloadBlob(file.filename, base64ToBytes(file.b64))
      } else {
        detailError.value = r?.message || '执行失败'
        showToast('error', detailError.value)
      }
      return r?.ok === true
    } catch (err) {
      detailError.value = captureError(err)
      showToast('error', `执行失败：${detailError.value}`)
      return false
    } finally {
      actionBusy.value = false
    }
  }

  /** 删除选中节点（目录递归） */
  async function removeSelected(): Promise<boolean> {
    const node = selectedNode.value
    if (!node) return false
    saving.value = true
    detailError.value = ''
    try {
      await deleteVdfs(node.path, isVdfsDir(node))
      showToast('success', `已删除「${node.title || node.name}」`)
      clearSelection()
      await refresh()
      return true
    } catch (err) {
      detailError.value = captureError(err)
      showToast('error', `删除失败：${detailError.value}`)
      return false
    } finally {
      saving.value = false
    }
  }

  /** 在当前目录下新建目录 */
  async function createDir(name: string): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) {
      showToast('error', '请填写目录名')
      return false
    }
    saving.value = true
    detailError.value = ''
    try {
      await mkdirVdfs(vdfsJoin(cwd.value, trimmed))
      showToast('success', `已新建「${trimmed}」`)
      await refresh()
      return true
    } catch (err) {
      detailError.value = captureError(err)
      showToast('error', `新建失败：${detailError.value}`)
      return false
    } finally {
      saving.value = false
    }
  }

  // ==================== 可接受的新建类型（§5） ====================
  //
  // 节点声明自己能新建哪些类型（`new_types`）；前端只负责「选类型 + 填名 + 组装
  // 地址 + 发写请求」，不认识任何具体类型——创建语义由 provider 自持。

  /**
   * 当前目录可接受的新建类型。
   *
   * 节点自身声明优先；**仅挂载点根**才回退到挂载点声明——挂载点的
   * `new_types` 描述的是「根下可建什么」，若泄漏到任意子目录，每个子目录
   * 都会长出与其语义无关的新建入口（如会话的「新建会话」出现在工作目录里）。
   */
  const creatableTypes = computed<VdfsNewType[]>(() => {
    const fromNode: VdfsNewType[] | undefined = cwdNode.value?.new_types
    if (fromNode && fromNode.length > 0) return fromNode
    const m = mounts.value.find((x) => x.mount === activeMount.value)
    if (!m || !m.root || cwd.value !== m.root) return []
    return m.new_types ?? []
  })

  /** 是否有可新建类型（添加按钮可见性；机制只认节点声明） */
  const canCreate = computed(() => creatableTypes.value.length > 0)

  /** 按类型新建：目标地址 = `<当前目录>/<名称>.<ext>`，一次 `write(create:true)` */
  async function createTyped(type: VdfsNewType, name: string): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) {
      showToast('error', '请填写名称')
      return false
    }
    const target = vdfsJoin(cwd.value, type.ext ? `${trimmed}.${type.ext}` : trimmed)
    saving.value = true
    detailError.value = ''
    fieldErrors.value = []
    try {
      await writeVdfs(target, '', { create: true })
      showToast('success', `已新建「${trimmed}」`)
      await refresh()
      return true
    } catch (err) {
      detailError.value = captureError(err)
      showToast('error', `新建失败：${detailError.value}`)
      return false
    } finally {
      saving.value = false
    }
  }

  /**
   * 按类型从**本地文件**新建（整包导入）：内容走二进制通道。
   *
   * 与 `createTyped` 同构——差别只是内容来源：`source = file` 的类型（如
   * `zip` 整包）不填名，目标名由**文件名**推导（主干 + 类型扩展名）。
   */
  async function createTypedFile(type: VdfsNewType, file: File): Promise<boolean> {
    const target = vdfsJoin(cwd.value, newFileNameOf(file.name, type.ext))
    saving.value = true
    detailError.value = ''
    fieldErrors.value = []
    try {
      const b64 = arrayBufferToBase64(await file.arrayBuffer())
      await writeVdfsBinary(target, b64, { create: true })
      showToast('success', `已导入「${file.name}」`)
      await refresh()
      return true
    } catch (err) {
      detailError.value = captureError(err)
      showToast('error', `导入失败：${detailError.value}`)
      return false
    } finally {
      saving.value = false
    }
  }

  /** 重命名选中节点（同挂载点内移动） */
  async function renameSelected(name: string): Promise<boolean> {
    const node = selectedNode.value
    const trimmed = name.trim()
    if (!node || !trimmed || trimmed === node.name) return false
    saving.value = true
    detailError.value = ''
    try {
      const to = vdfsJoin(vdfsParent(node.path), trimmed)
      await moveVdfs(node.path, to)
      showToast('success', `已重命名为「${trimmed}」`)
      clearSelection()
      await refresh()
      return true
    } catch (err) {
      detailError.value = captureError(err)
      showToast('error', `重命名失败：${detailError.value}`)
      return false
    } finally {
      saving.value = false
    }
  }

  // ==================== 实时（总线 vdfs 频道，非轮询） ====================
  // 归一化：有界 mount → 仅本挂载点范围内的变更触发刷新；
  // 变更可能落在当前目录之外（如子目录），一律防抖重拉当前目录收敛。
  let refreshTimer: ReturnType<typeof setTimeout> | null = null
  function scheduleRefresh(delay = 400) {
    if (refreshTimer) clearTimeout(refreshTimer)
    refreshTimer = setTimeout(() => {
      refreshTimer = null
      void refresh()
      const sel = selectedNode.value
      if (sel) void select(sel)
    }, delay)
  }

  /** 数据变更回调（挂载点过滤在 handler 内，订阅恒定一条） */
  function onChange(change: VdfsChange) {
    if (activeMount.value && change.mount !== activeMount.value) return
    scheduleRefresh()
  }

  const unsubBus = subscribe({ kind: VFDS_EVENT_KIND }, (busEvent) => {
    const change = busEvent.data?.data as VdfsChange | undefined
    if (!change || typeof change.mount !== 'string') return
    onChange(change)
  })

  // 订阅范围与当前目录严格绑定：切换目录时解除旧订阅、登记新订阅。
  // 后端对无实时能力的 provider 默认 no-op，因此这里无需按 provider 分流。
  let watched: string | null = null
  watch(
    cwd,
    (next, prev) => {
      // 虚拟根不在任何 provider 身上、无实时能力；跳过 watch/unwatch，否则后端报
      // 「虚拟根不是可操作节点」（见服务器日志 vdfs/watch / vdfs/unwatch 的 ERROR）。
      if (prev && prev !== VFDS_ROOT && prev !== next) void unwatchVdfs(prev)
      watched = next
      if (next !== VFDS_ROOT) void watchVdfs(next)
    },
    { immediate: true }
  )

  onBeforeUnmount(() => {
    unsubBus()
    if (refreshTimer) clearTimeout(refreshTimer)
    if (watched && watched !== VFDS_ROOT) void unwatchVdfs(watched)
  })

  // 路由 `:mount` 变化 → 进入对应挂载点（深链/前进后退）。
  // 首次进入由 boot() 统一落位，此处只处理后续变化。
  if (mountParam) {
    watch(mountParam, (m) => {
      const target = m ? vdfsJoin(VFDS_ROOT, m) : VFDS_ROOT
      if (target !== cwd.value) void enter(target)
    })
  }

  /** 首次进入：拉挂载点清单 + 按路由落位 + 加载当前目录 */
  async function boot() {
    mounts.value = await fetchMounts()
    const target = mountParam?.value ? vdfsJoin(VFDS_ROOT, mountParam.value) : VFDS_ROOT
    if (target !== cwd.value) cwd.value = target
    await refresh()
  }

  return {
    // 挂载点 / 导航
    mounts,
    railItems,
    activeMount,
    switchMount,
    // 目录
    cwd,
    title,
    breadcrumbs,
    cwdNode,
    items,
    loading,
    loadError,
    canWriteHere,
    refresh,
    enter,
    goUp,
    activate,
    // 选中 / 详情
    selectedNode,
    selectedId,
    select,
    renderer,
    nodeText,
    formData,
    nodeBinary,
    loadingDetail,
    saving,
    actionBusy,
    detailError,
    fieldErrors,
    clearSelection,
    // 操作
    saveFields,
    saveText,
    runAction,
    removeSelected,
    createDir,
    createTyped,
    createTypedFile,
    creatableTypes,
    canCreate,
    renameSelected,
    // 启动
    boot,
  }
}
