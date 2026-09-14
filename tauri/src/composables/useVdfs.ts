/**
 * useVdfs —— 三栏工作台的唯一数据逻辑（绑定一个 vdfs 数据地址）
 *
 * 不含任何资源类型知识：目录内容来自 `vdfs/list`、详情渲染器由节点的 `ext`
 * 决定（`registry/vdfsTypes`）。新增一种资源 = 后端实现一个 `VdfsProvider`，
 * 本文件零改动。
 *
 * ## 绑定模型（控件自包含，见 VdfsWorkbench）
 *
 * 宿主给一个**数据地址**（`addr`，如 `.vdfs` 或 `.vdfs/session/<id>`），本层
 * 完成全部数据装配，**不感知浏览器路由**——数据地址与浏览器地址是两个概念：
 *
 * - **左栏导航** = `addr` 的内容（其子目录清单，后端 order 排列）；
 * - **当前目录**（`cwd`）= 选中的左栏子目录（缺省第一个）；`addr` 无子目录时
 *   落在 `addr` 自身。中栏列表 = `cwd` 内容，点文件选中、点目录钻入
 *   （钻入由控件 emit `open`，宿主决定呈现方式，通常是 push 新地址页）；
 * - **选中节点**（`selectedNode`）：详情来源，按 `ext` 解析渲染器；
 * - **选中记忆**：同一数据地址的左栏选中项会被记住（往返 push / 返回后恢复）；
 * - **实时**：订阅总线 `vdfs` 频道，受影响的目录防抖刷新（非轮询）。
 *
 * ## UI 约定（S11）
 *
 * - **无面包屑**：路径即导航，层级靠左栏切目录 + 中栏点目录钻入 + 宿主返回键；
 * - **无「新建目录」按钮**：新建 = 新建一种**类型**（会话 / 模型 / …）；
 *   类型清单由当前目录节点声明（`new_types`），多类型时先选类型再命名；
 *   目录结构节点由后端 provider 自持，前端不暴露 mkdir 入口。
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
  listVdfs,
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
  newFileNameOf,
  parseVdfsValidation,
  vdfsJoin,
  vdfsParent,
  type VdfsChange,
  type VdfsFieldError,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'
import { dirIconOf, resolveVdfsRenderer, type VdfsRenderer } from '@/registry/vdfsTypes'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import type { NavRailItem } from '@/components/common/NavRail.vue'

/** 各数据地址的左栏选中记忆（模块级：往返 push / 返回后恢复原选中） */
const selectedMemo = new Map<string, string>()

export interface UseVdfsOptions {
  /** 绑定的数据地址（如 `.vdfs` 或 `.vdfs/session/<id>`）；变化 = 整体重载 */
  addr: Ref<string>
}

export function useVdfs(opts: UseVdfsOptions) {
  const { showToast } = useToast()
  const addr = opts.addr

  // ==================== 左栏导航（绑定地址的子目录清单） ====================
  /** `addr` 的内容（其子目录 = 左栏导航项） */
  const navDirs = ref<VdfsNode[]>([])

  /** 选中的左栏子目录名；null = `addr` 无子目录（中栏显示 `addr` 自身内容） */
  const selectedName = ref<string | null>(null)

  /** 当前目录全路径 = 绑定地址 + 选中的子目录 */
  const cwd = computed(() =>
    selectedName.value ? vdfsJoin(addr.value, selectedName.value) : addr.value
  )

  /** 左栏导航 = `addr` 的子目录（图标为纯 UI 映射），高亮 = 选中项 */
  const navItems = computed<NavRailItem[]>(() =>
    navDirs.value
      .filter(isVdfsDir)
      .map((n) => ({
        key: n.name,
        label: n.title || n.name,
        icon: dirIconOf(n.name) ?? null,
        description: n.description,
        active: n.name === selectedName.value,
      }))
  )

  /** 页标题 = 当前目录节点的标题（列表加载后即为当前目录的自述） */
  const title = computed(() => cwdNode.value?.title || cwdNode.value?.name || '资源')

  // ==================== 目录内容 ====================
  const items = ref<VdfsNode[]>([])
  const cwdNode = shallowRef<VdfsNode | null>(null)
  const loading = ref(false)
  const loadError = ref('')

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

  /**
   * 重拉左栏导航并落位选中：记忆优先（仍存在时），缺省第一个子目录。
   * 选中变化时内部已就地刷新当前目录（返回值告知调用方免重复刷）。
   */
  async function refreshNav(): Promise<boolean> {
    const resp = await listVdfs(addr.value)
    navDirs.value = resp.items
    const names = resp.items.filter(isVdfsDir).map((n) => n.name)
    const want = selectedMemo.get(addr.value)
    const next = want && names.includes(want) ? want : (names[0] ?? null)
    if (next === selectedName.value) return false
    selectedName.value = next
    if (next) selectedMemo.set(addr.value, next)
    clearSelection()
    void refresh()
    return true
  }

  /**
   * 整体重载（绑定地址变化 / 宿主要求）：落位左栏选中 + 刷新当前目录。
   * 选中落位变化时 refreshNav 已就地刷新，无需重复。
   */
  async function reload() {
    if (await refreshNav()) return
    await refresh()
  }

  /** 点左栏某项 = 就地切换当前目录（不产生新页面） */
  async function selectDir(name: string) {
    if (name === selectedName.value) return
    const n = navDirs.value.find((x) => x.name === name && isVdfsDir(x))
    if (!n) return
    selectedName.value = name
    selectedMemo.set(addr.value, name)
    clearSelection()
    await refresh()
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

  // ==================== 可接受的新建类型（§5） ====================
  //
  // 当前目录节点声明自己能新建哪些类型（`new_types`）；前端只负责「选类型 +
  // 填名 + 组装地址 + 发写请求」，不认识任何具体类型——创建语义由 provider 自持。
  // `.vdfs/session` 这类子目录节点由后端合成时携带其 new_types，因此无需任何
  // 「按目录名回退」的特判。

  /** 当前目录可接受的新建类型 */
  const creatableTypes = computed<VdfsNewType[]>(() => cwdNode.value?.new_types ?? [])

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

  /** 重命名选中节点（同一地址空间内移动） */
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
  // 变更影响当前目录（自身 / 祖先 / 子树内）才刷新；绑定地址一层的变化
  //（左栏子目录增删）顺带重拉导航。其余变更防抖重拉当前目录收敛。
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

  /** 是否影响某目录：变更路径是它自身 / 它的祖先 / 它子树内的成员 */
  function affects(dir: string, changePath: string): boolean {
    return changePath === dir || dir.startsWith(`${changePath}/`) || changePath.startsWith(`${dir}/`)
  }

  /** 数据变更回调（路径过滤在 handler 内，订阅恒定一条） */
  function onChange(change: VdfsChange) {
    if (affects(cwd.value, change.path)) scheduleRefresh()
    // 绑定地址一层（左栏子目录增删）→ 导航跟着变
    if (change.path === addr.value || vdfsParent(change.path) === addr.value) {
      void refreshNav()
    }
  }

  const unsubBus = subscribe({ kind: VFDS_EVENT_KIND }, (busEvent) => {
    const change = busEvent.data?.data as VdfsChange | undefined
    if (!change || typeof change.path !== 'string') return
    onChange(change)
  })

  // 订阅范围与当前目录严格绑定：切换目录时解除旧订阅、登记新订阅。
  // 后端对无实时能力的 provider 默认 no-op，因此这里无需按 provider 分流。
  let watched: string | null = null
  watch(
    cwd,
    (next, prev) => {
      // `.vdfs` 根不在任何 provider 身上、无实时能力；跳过 watch/unwatch，否则后端报
      // 「目录不是可操作节点」（见服务器日志 vdfs/watch / vdfs/unwatch 的 ERROR）。
      if (prev && prev !== VFDS_ROOT && prev !== next) void unwatchVdfs(prev)
      watched = next
      if (next !== VFDS_ROOT) void watchVdfs(next)
    },
    { immediate: true }
  )

  // 绑定地址变化 = 换了一个数据地址：整体重载（左栏、选中、当前目录）
  watch(addr, () => void reload(), { immediate: true })

  onBeforeUnmount(() => {
    unsubBus()
    if (refreshTimer) clearTimeout(refreshTimer)
    if (watched && watched !== VFDS_ROOT) void unwatchVdfs(watched)
  })

  return {
    // 左栏导航
    navItems,
    selectDir,
    selectedName,
    // 目录
    cwd,
    title,
    cwdNode,
    items,
    loading,
    loadError,
    refresh,
    reload,
    select,
    // 选中 / 详情
    selectedNode,
    selectedId,
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
    createTyped,
    createTypedFile,
    creatableTypes,
    canCreate,
    renameSelected,
  }
}
