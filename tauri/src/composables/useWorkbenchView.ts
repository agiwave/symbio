/**
 * useWorkbenchView —— 统一实体管理页（WorkbenchView）的唯一页面逻辑
 *
 * 整个 App 的实体管理 = 一套机制 + 一个页面（WorkbenchView）+ 一个本组合式。
 * 页面形态由「路由参数 + 后端注册表」自动配置，前端零逐特性开发：
 *
 * - leaf 模式（/entities/:types? 、 /settings）：
 *     类别 = providers 注册表解析 :types 参数（resolveActiveTypes）；
 *     list = 各类别 entities/list 并行；编辑器经 kind / kind:config_type 注册表分发，
 *     未注册走通用兜底（zip 面板 / JSON 表单 / 只读详情）。
 * - container 模式（/container/:kind/:id/entities 、 /agent/:agentId/entities）：
 *     类别 = ProviderInfo.container_kinds（后端下发标签/路径模板/内容模板/能力）；
 *     list = entities/list（payload.container 单请求全量分箱）；
 *     详情/新建 = 标准文本编辑器（entities/get 的 extra.content + entities/put 写回）。
 *
 * 两模式唯一差异点：
 * ① 类别来源（注册表两种形态）；② list/delete 是否携带 container 字段。
 * 状态机（清单/选中/新建/删除）全部委托 useWorkbench 唯一实现；
 * 协议同一套 `${prefix}/entities/*`。
 *
 * 另导出：
 * - detailDefinition：后端 entities/detail 下发的详情页定义（§3.2 解析链
 *   第二优先：注册 editor 缺席时由 DetailForm 渲染）；
 * - isManagerCreatable / buildMixedItems：纯函数（可单测）。
 */

import { computed, onBeforeUnmount, ref, shallowRef, watch, type ComputedRef, type Ref } from 'vue'
import {
  deleteEntity,
  getDetailDefinition,
  getEntity,
  getEntityStatus,
  listEntities,
  putContainerEntity,
  uploadEntityForm,
  uploadEntityZip,
} from '@/services/entities'
import { useWorkbench, type WorkbenchKindState } from '@/composables/useWorkbench'
import { subscribe } from '@/services/eventBus'
import { logger } from '@/utils/logger'
import {
  loadProviders,
  resolveActiveTypes,
  useEntityProviders,
} from '@/composables/useEntityProviders'
import { getEntityEditor, getEntityEditorFor, getEntityIcon } from '@/registry/entityTypes'
import type {
  ContainerKindInfo,
  DetailDefinition,
  ProviderInfo,
  EntityCapabilities,
  EntitySummary,
} from '@/schemas/entities'
import type { NavRailItem } from '@/components/common/NavRail.vue'

/** 页面模式：实体实体页 / 容器实体页（由路由参数决定，实例内恒定） */
export type WorkbenchPageMode = 'leaf' | 'container'

export interface WorkbenchViewOptions {
  mode: WorkbenchPageMode
  /** leaf 模式：路由 :types 参数（'all' | 逗号分隔 kind | 单 kind，缺省 all） */
  typesParam?: Ref<string | undefined>
  /** container 模式：容器所属 provider kind（如 'agent'） */
  containerKind?: string
  /** container 模式：容器条目 id 的响应式 getter */
  containerId?: () => string | undefined
}

export function useWorkbenchView(opts: WorkbenchViewOptions) {
  const { mode, containerKind = '', containerId } = opts
  const typesParam = opts.typesParam
  const isContainer = mode === 'container'
  const { providers, getProvider, labelOf, capabilitiesOf } = useEntityProviders()

  function requireContainerId(): string {
    const id = containerId?.()
    if (!id) throw new Error('容器 id 缺失')
    return id
  }

  // ============ 差异点①：类别来源（后端注册表两种形态） ============
  /** leaf：providers 注册表解析 :types 参数 */
  const activeTypes: ComputedRef<ProviderInfo[]> = computed(() =>
    isContainer ? [] : resolveActiveTypes(providers.value, typesParam?.value)
  )
  /** container：ProviderInfo.container_kinds（标签/路径模板/内容模板/能力后端下发） */
  const containerKinds = computed<ContainerKindInfo[]>(() =>
    isContainer ? (getProvider(containerKind)?.container_kinds ?? []) : []
  )
  const isMulti = computed(() => !isContainer && activeTypes.value.length > 1)

  // ============ 差异点②：list/delete 是否携带 container ============
  const wb = useWorkbench({
    categories: () =>
      isContainer
        ? containerKinds.value.map((k) => ({
            kind: k.kind,
            label: k.label,
            capabilities: k.capabilities,
          }))
        : activeTypes.value.map((p) => ({
            kind: p.kind,
            label: p.label,
            capabilities: p.capabilities,
          })),
    listItems: isContainer ? undefined : (kind) => listEntities(kind),
    listAll: isContainer
      ? async () => {
          const id = containerId?.()
          if (!id) return { items: [] }
          const resp = await listEntities(containerKind, { container: id })
          return { items: resp.items, capabilities: resp.capabilities }
        }
      : undefined,
    deleteItem: (kind, id) =>
      isContainer
        ? deleteEntity(containerKind, id, requireContainerId())
        : deleteEntity(kind, id),
    autoSelect: !isContainer,
    logTag: 'WorkbenchView',
  })

  // ============ 核心委托（唯一状态机，统一导出名） ============
  const typeStates = wb.kindStates
  const items = wb.itemsAll
  const selected = wb.selected
  const selectedId = wb.selectedId
  const creating = wb.creating
  const saving = wb.saving
  const loading = wb.loading
  const deletingId = wb.deletingId
  const showToast = wb.showToast

  /** 选中（复合键 `${kind}:${id}`；null = 清除） */
  function select(key: string | null) {
    wb.selectKey(key)
  }

  /** 加载清单（先幂等确保注册表已拉取——容器页可能先于 MainLayout 挂载） */
  async function loadAll() {
    loading.value = true
    try {
      await loadProviders()
      await wb.loadAll()
      // 机制规则：单类型页 + 清单为空 + 该类型可创建 → 自动进入新建态
      // （如 session 首启显示「新建会话」引导，而非空白详情）
      if (
        !isContainer &&
        !creating.value &&
        activeTypes.value.length === 1 &&
        allItemsEmpty.value &&
        canCreate.value
      ) {
        beginCreate(activeTypes.value[0]!.kind)
      }
    } finally {
      loading.value = false
    }
  }
  const refreshKind = wb.refreshKind

  // ============ 展示派生（entity） ============
  const totalCount = computed(() =>
    Object.values(typeStates.value).reduce((n, s) => n + s.items.length, 0)
  )
  const allItemsEmpty = computed(() =>
    activeTypes.value.every((d) => (typeStates.value[d.kind]?.items.length ?? 0) === 0)
  )

  /** 列表简洁模式：活动类型中任一开启 compact_list 即生效（仅图标 + 标题） */
  const isCompact = computed(() => activeTypes.value.some((p) => p.compact_list))

  /** 页标题：多类型 = "实体"，单类型/容器 = 类别标签 */
  const title = computed(() => {
    if (isContainer) return activeKindMeta.value?.label ?? ''
    return isMulti.value ? '实体' : kindLabel(activeTypes.value[0]?.kind ?? '')
  })

  const emptyHint = computed(() => {
    if (isContainer) return ''
    if (isMulti.value) return canCreate.value ? '点击右上角「新建」创建实体' : ''
    const kind = activeTypes.value[0]?.kind
    if (!kind) return ''
    const c = capsOf(kind)
    if (c.zip_upload) return '点击右上角「新建」上传 ZIP（文件名即实体目录名）'
    if (creatableByEditor(kind)) return '点击右上角「新建」开始'
    if (isManagerCreatable(getProvider(kind) ?? makeReadonly(kind))) {
      return '点击右上角「新建」填写表单创建'
    }
    return ''
  })

  /** 某 kind 的展示标签（container 用后端 container_kinds 标签，leaf 用注册表标签） */
  function kindLabel(kind: string): string {
    if (isContainer) return containerKinds.value.find((k) => k.kind === kind)?.label ?? kind
    return labelOf(kind, kind)
  }

  /** 某 kind 的能力（未加载/未知 → 只读空态） */
  function capsOf(kind: string): EntityCapabilities {
    return capabilitiesOf(kind)
  }

  /** 某 kind 的专属编辑表单（新建用；未注册 null → 通用兜底） */
  function createEditor(kind: string | null) {
    return kind ? getEntityEditor(kind) ?? null : null
  }
  /** 选中项的专属编辑表单：项级 config_type 优先，回退 kind；未注册 null → 通用兜底 */
  const selectedEditor = computed(() =>
    selected.value ? getEntityEditorFor(selected.value.item) ?? null : null
  )

  /**
   * 类型可创建 = 两种通道之一：
   * ① manager 通道（isManagerCreatable：zip 上传 / manifest 表单）；
   * ② editor 引导通道（capabilities.independent_form 且该 kind 注册了 kind 级
   *    editor——新建按钮渲染该 editor 的引导态，如 session 的「新建会话」）。
   */
  function creatableByEditor(kind: string): boolean {
    if (isManagerCreatable(getProvider(kind) ?? makeReadonly(kind))) return false
    const caps = capabilitiesOf(kind)
    return Boolean(caps.independent_form && !caps.read_only && getEntityEditor(kind))
  }

  /** 当前活动类型中可创建的类型（leaf 多类型新建面板用） */
  const creatableInActive = computed(() =>
    activeTypes.value.filter((p) => isManagerCreatable(p) || creatableByEditor(p.kind))
  )
  /**
   * 列表头「新建」可见性 —— 能力驱动（§3.4，前端不得硬编码）：
   * - 容器模式：子类别声明了 path_hint（路径模板）才可创建；path_hint 空 =
   *   系统管理型（如子会话），由机制派生，无新建入口；
   * - 实体模式：creatableInActive 非空（manager 通道 supports_upload+mutable，
   *   或 editor 引导通道 independent_form+kind 级 editor）。
   *   setting（mutable=false、分区清单固定）两通道皆否 → 不显示。
   */
  const canCreate = computed(() =>
    isContainer ? Boolean(activeKindMeta.value?.path_hint) : creatableInActive.value.length > 0
  )
  /**
   * 列表头「刷新」可见性 —— 能力驱动（§3.4）：实体模式要求所有活动类型均
   * 声明 refreshable（任一类型清单由生命周期事件通道自持同步（session）或
   * 固定（setting）即不显示手动刷新）；容器模式看当前子类别能力（树/条目
   * 清单保留手动刷新）。
   */
  const canRefresh = computed(() => {
    if (isContainer) return activeKindMeta.value?.capabilities.refreshable ?? true
    return activeTypes.value.length > 0 && activeTypes.value.every((p) => p.capabilities.refreshable)
  })
  const canDeleteSelected = computed(() => {
    const sel = selected.value
    if (!sel) return false
    if (isContainer) return mutableKind(sel.kind)
    const caps = capabilitiesOf(sel.kind)
    return Boolean(caps.mutable && !caps.read_only)
  })

  /** 容器子类别的可写判定（子类别无 supports_upload 概念，写经 entities/put） */
  function mutableKind(kind: string): boolean {
    const caps = isContainer
      ? containerKinds.value.find((k) => k.kind === kind)?.capabilities
      : capabilitiesOf(kind)
    return Boolean(caps?.mutable && !caps?.read_only)
  }

  /** 兜底只读 provider（未知类型） */
  function makeReadonly(kind: string): ProviderInfo {
    return {
      kind,
      provider_name: kind,
      prefix: kind,
      capabilities: { zip_upload: false, independent_form: false, realtime_status: false, refreshable: true, mutable: false, test_connection: false, read_only: true },
      order: 999,
      label: kind,
      supports_upload: false,
    }
  }

  // ============ 新建（统一入口，按模式/能力自动分流） ============
  const createKind = ref<string | null>(null)
  const activeFormKind = computed<string | null>(() =>
    creating.value ? createKind.value : (selected.value?.kind ?? null)
  )

  /** 新建入口：容器 → 名称+内容编辑器；leaf 多类型 → 类型选择面板；单类型 → 直达 */
  function onNew() {
    uploadError.value = null
    manifestError.value = null
    if (isContainer) {
      startCreate()
      return
    }
    if (isMulti.value) {
      startTypeChoice()
      return
    }
    const kind = activeTypes.value[0]?.kind
    if (kind) beginCreate(kind)
  }

  /** leaf：绑定某类型并进入创建态 */
  function beginCreate(kind: string) {
    createKind.value = kind
    if (!creating.value) wb.enterCreate()
  }
  /** leaf：进入创建态但不绑定类型 → 渲染类型选择面板 */
  function startTypeChoice() {
    createKind.value = null
    if (!creating.value) wb.enterCreate()
  }
  function cancelCreate(opts?: { autoSelect?: boolean }) {
    createKind.value = null
    editorError.value = ''
    wb.cancelCreate(opts)
  }

  // ==================== 详情页定义（definition-driven detail） ====================
  // 解析顺序：注册专属 editor → 本定义（DetailForm 渲染）→ 通用兜底。
  // 新建态（id 空）与选中态共用一个请求令牌防竞态。
  //
  // 切换目标时**立即失效旧定义**（置 null）：否则 DetailForm 会以旧定义
  // 重挂载并用旧 load_path 拉取错误数据源（如设置分区切换后拉到上一个
  // 分区的 config），新定义到达仅更新 prop、不会重新加载——表现为
  // 「除首个分区外全部空白」。定义生命周期与选中目标严格同步。
  const detailDefinition = shallowRef<DetailDefinition | null>(null)
  let detailToken = 0
  watch(
    () => {
      if (isContainer) return ''
      if (creating.value && createKind.value) return `create:${createKind.value}`
      const sel = selected.value
      return sel ? `sel:${sel.item.kind}:${sel.item.id}` : ''
    },
    async (key) => {
      if (!key) {
        detailDefinition.value = null
        return
      }
      detailDefinition.value = null
      const [scope, kind, ...rest] = key.split(':')
      const id = scope === 'create' ? '' : rest.join(':')
      const token = ++detailToken
      const def = await getDetailDefinition(kind, id)
      if (token === detailToken) detailDefinition.value = def
    },
    { immediate: true }
  )

  // ============ leaf：zip 上传 / 表单 / JSON 兜底保存 ============
  const zipUploading = ref(false)
  const uploadError = ref<string | null>(null)
  const manifestError = ref<string | null>(null)
  const draftName = ref('')
  const draftManifest = ref('{}')

  async function saveZip(file: File) {
    const kind = createKind.value
    if (!kind) return
    const name = file.name.replace(/\.zip$/i, '')
    if (!name) {
      uploadError.value = '请提供以 .zip 结尾的文件'
      return
    }
    zipUploading.value = true
    uploadError.value = null
    try {
      const buf = await file.arrayBuffer()
      const resp = await uploadEntityZip(kind, name, buf)
      showToast('success', `已上传 ${kindLabel(kind)}「${resp.id || name}」`)
      cancelCreate()
      await refreshKind(kind)
      select(`${kind}:${resp.id || name}`)
    } catch (err) {
      uploadError.value = `上传失败: ${err}`
      showToast('error', `上传失败: ${err}`)
    } finally {
      zipUploading.value = false
    }
  }

  async function saveForm(payload: {
    id: string
    manifest: Record<string, unknown>
    skipValidation?: boolean
  }) {
    const kind = activeFormKind.value
    if (!kind) return
    saving.value = true
    try {
      const manifest = { ...payload.manifest }
      if (payload.skipValidation) manifest.skip_validation = true
      const resp = await uploadEntityForm(kind, payload.id, manifest)
      showToast('success', `已保存 ${kindLabel(kind)}「${payload.id}」`)
      cancelCreate()
      await refreshKind(kind)
      select(`${kind}:${resp.id || payload.id}`)
    } catch (err) {
      showToast('error', `保存失败: ${err}`)
    } finally {
      saving.value = false
    }
  }

  async function saveManifest() {
    const kind = activeFormKind.value
    if (!kind) return
    const name = draftName.value.trim()
    if (!name) {
      manifestError.value = '请填写名称'
      showToast('error', '请填写名称')
      return
    }
    let parsed: Record<string, unknown>
    try {
      parsed = JSON.parse(draftManifest.value || '{}')
    } catch (err) {
      manifestError.value = `JSON 解析失败: ${err}`
      showToast('error', 'JSON 格式错误')
      return
    }
    manifestError.value = null
    saving.value = true
    try {
      const resp = await uploadEntityForm(kind, name, { ...parsed, id: name })
      showToast('success', `已保存 ${kindLabel(kind)}「${name}」`)
      cancelCreate()
      await refreshKind(kind)
      select(`${kind}:${resp.id || name}`)
    } catch (err) {
      manifestError.value = `保存失败: ${err}`
      showToast('error', `保存失败: ${err}`)
    } finally {
      saving.value = false
    }
  }

  /** 设为默认（model 专属 editor 的 set-default 事件 → 统一 upload 通道） */
  async function saveDefault() {
    const sel = selected.value
    if (!sel) return
    const config = (sel.item.config ?? {}) as Record<string, unknown>
    saving.value = true
    try {
      await uploadEntityForm(sel.kind, sel.item.id, {
        ...config,
        is_default: true,
        skip_validation: true,
      })
      showToast('success', `已将「${sel.item.name || sel.item.id}」设为默认`)
      await refreshKind(sel.kind)
    } catch (err) {
      showToast('error', `设置默认失败: ${err}`)
    } finally {
      saving.value = false
    }
  }

  /** 连接测试（capabilities.test_connection 类型） */
  async function testConnection() {
    const sel = selected.value
    if (!sel) return
    wb.testing.value = true
    try {
      const resp = await getEntityStatus(sel.kind, sel.item.id)
      if (!resp) {
        showToast('error', '该后端暂不支持连接测试')
        return
      }
      sel.item.status = resp.status
      sel.item.status_detail = resp.status_detail ?? undefined
      showToast(resp.status === 'connected' ? 'success' : 'error', resp.status_detail || resp.status)
    } catch (err) {
      showToast('error', `测试失败: ${err}`)
    } finally {
      wb.testing.value = false
    }
  }

  // 通用 JSON 兜底：选中时预填编辑 JSON（independent_form 且无专属 editor）
  watch(
    () => selected.value,
    (sel) => {
      if (!sel || isContainer) return
      const caps = capsOf(sel.kind)
      const hasEditor = Boolean(selectedEditor.value)
      if (caps.independent_form && !caps.zip_upload && !hasEditor) {
        draftName.value = sel.item.name || sel.item.id || ''
        const body: Record<string, unknown> = {}
        for (const [k, v] of Object.entries(sel.item)) {
          if (['kind', 'provider', 'name', 'id', 'description', 'summary', 'updated_at', 'status', 'status_detail'].includes(k)) {
            continue
          }
          if (v !== null && v !== undefined) body[k] = v
        }
        draftManifest.value = JSON.stringify(body, null, 2)
        manifestError.value = null
      }
    }
  )

  // ============ container：身份信息 / 类别切换 / 文本编辑 ============
  /** 容器条目名（meta 展示；统一协议 entities/get，容器外详情） */
  const containerName = ref('')
  if (isContainer) {
    watch(
      () => containerId?.(),
      async (id) => {
        if (!id) return
        try {
          await loadProviders()
          const item = await getEntity(containerKind, id)
          containerName.value = item?.name || id
        } catch (err) {
          logger.warn('useWorkbenchView', '容器条目名获取失败:', err)
          containerName.value = id
        }
      },
      { immediate: true }
    )
  }

  /** 活动子类别（缺省取后端声明的第一个） */
  const activeKind = ref('')
  watch(
    containerKinds,
    (kinds) => {
      if (!kinds.some((k) => k.kind === activeKind.value)) {
        activeKind.value = kinds[0]?.kind ?? ''
      }
    },
    { immediate: true }
  )
  const activeKindMeta = computed(
    () => containerKinds.value.find((k) => k.kind === activeKind.value) ?? null
  )

  function switchKind(next: string) {
    activeKind.value = next
    wb.selectKey(null)
    fetchedEntry.value = null
    if (wb.creating.value) wb.cancelCreate()
    editorError.value = ''
  }

  /** 类别计数（含 0 值类别，供导航角标） */
  const counts = computed<Record<string, number>>(() => {
    const out: Record<string, number> = {}
    for (const k of containerKinds.value) out[k.kind] = out[k.kind] ?? 0
    for (const e of wb.itemsAll.value) out[e.kind] = (out[e.kind] ?? 0) + 1
    return out
  })

  /** 侧边栏数据：container 模式 = container_kinds → NavRail 项；entity = undefined（由应用外壳承担） */
  const railItems = computed<NavRailItem[] | undefined>(() => {
    if (!isContainer) return undefined
    return containerKinds.value.map((k) => ({
      key: k.kind,
      label: k.label,
      icon: getEntityIcon(k.kind) ?? undefined,
      count: counts.value[k.kind] || undefined,
      active: activeKind.value === k.kind,
    }))
  })

  const kindEntries = computed(() => wb.itemsOf(activeKind.value))

  // === 文本编辑（entities/get 的 extra.content + entities/put 写回） ===
  // 机制分流：子实体带 extra.content（文件型）→ 文本编辑器；
  // 不带（系统管理型，如子会话）→ 只读详情面板。
  // 选中子条目：类别分箱（列表形态）优先；树视图等自管懒加载数据的子类别
  // 不进分箱，以最近一次 entities/get 读取结果兜底（选中态严格比对，无悬挂）
  const fetchedEntry = ref<EntitySummary | null>(null)
  const selectedEntry = computed(() => {
    const found = kindEntries.value.find((e) => wb.isSelected(e.kind, e.id))
    if (found) return found
    const f = fetchedEntry.value
    return f && wb.isSelected(f.kind, f.id) ? f : null
  })
  const content = ref('')
  const originalContent = ref('')
  const loadingContent = ref(false)
  const entryHasContent = ref(false)
  const editorError = ref('')
  const dirty = computed(() => content.value !== originalContent.value)

  /** 选中容器子条目并加载内容（id = 容器内相对路径） */
  async function selectEntry(id: string) {
    if (!isContainer) return
    const cid = containerId?.()
    if (!cid) return
    wb.selectKey(`${activeKind.value}:${id}`)
    editorError.value = ''
    loadingContent.value = true
    try {
      const item = await getEntity(containerKind, id, cid)
      if (item) fetchedEntry.value = item
      const c = item?.content
      if (typeof c === 'string') {
        content.value = c
        originalContent.value = c
        entryHasContent.value = true
      } else {
        // 非 content 型子实体：无编辑语义，详情走只读面板
        content.value = ''
        originalContent.value = ''
        entryHasContent.value = false
      }
    } catch (err) {
      editorError.value = `读取失败: ${err}`
      content.value = ''
      originalContent.value = ''
      entryHasContent.value = false
    } finally {
      loadingContent.value = false
    }
  }

  async function reloadSelected() {
    if (selectedEntry.value) await selectEntry(selectedEntry.value.id)
  }

  async function saveEntry() {
    const cid = containerId?.()
    const entry = selectedEntry.value
    if (!cid || !entry) return
    saving.value = true
    editorError.value = ''
    try {
      await putContainerEntity(containerKind, entry.id, content.value, cid)
      originalContent.value = content.value
      showToast('success', `已保存「${entry.id}」`)
      await wb.refreshKind(entry.kind)
    } catch (err) {
      editorError.value = `保存失败: ${err}`
      showToast('error', `保存失败: ${err}`)
    } finally {
      saving.value = false
    }
  }

  // === 实时刷新（§3.3）：容器 `data` 粗粒度事件 → 防抖同步选中子实体内容 ===
  // 仅在选中内容未被用户编辑时跟随（编辑中不覆盖输入）。
  // 订阅为 kind 级（sessionId 不入过滤器——避免触发会话回放缓冲的
  // drain 副作用），目标容器在 handler 内过滤。
  let detailReloadTimer: ReturnType<typeof setTimeout> | null = null
  const unsubDetailData = subscribe(
    { kind: containerKind },
    (busEvent) => {
      const inner = busEvent.data?.data as { type?: string } | undefined
      if (inner?.type !== 'data') return
      if (busEvent.data.session_id !== containerId?.()) return
      if (detailReloadTimer) clearTimeout(detailReloadTimer)
      detailReloadTimer = setTimeout(() => {
        detailReloadTimer = null
        if (selectedEntry.value && entryHasContent.value && !dirty.value) {
          void selectEntry(selectedEntry.value.id)
        }
      }, 800)
    }
  )
  onBeforeUnmount(() => {
    unsubDetailData()
    if (detailReloadTimer) clearTimeout(detailReloadTimer)
  })

  // === 新建（路径模板 / 内容模板均来自后端 container_kinds 声明） ===
  const newName = ref('')
  const newContent = ref('')

  /** 名字归一化：去空白；路径模板以 .md 结尾时容错去掉误带的后缀 */
  function normalizeName(raw: string): string {
    const n = raw.trim()
    if (activeKindMeta.value?.path_hint?.endsWith('.md')) return n.replace(/\.md$/i, '')
    return n
  }

  function buildPath(name: string): string {
    const hint = activeKindMeta.value?.path_hint
    return hint ? hint.replace('<name>', name) : name
  }

  const createPathPreview = computed(() => {
    const hint = activeKindMeta.value?.path_hint ?? '<path>'
    const name = normalizeName(newName.value)
    return name ? hint.replace('<name>', name) : hint
  })

  function validateName(): string | null {
    const name = normalizeName(newName.value)
    if (!name) return '请填写名称'
    if (!/^[\w\u4e00-\u9fa5][\w\u4e00-\u9fa5.-]*$/.test(name)) {
      return '名称仅允许中文 / 字母 / 数字 / 下划线 / 点 / 连字符'
    }
    const path = buildPath(name)
    if (wb.itemsAll.value.some((e: EntitySummary) => e.id === path)) {
      return `已存在同名实体：${path}`
    }
    return null
  }

  function startCreate() {
    wb.enterCreate()
    editorError.value = ''
    newName.value = ''
    newContent.value = activeKindMeta.value?.default_content ?? ''
  }

  async function saveCreate() {
    const cid = containerId?.()
    if (!cid) return
    const invalid = validateName()
    if (invalid) {
      editorError.value = invalid
      return
    }
    const path = buildPath(normalizeName(newName.value))
    saving.value = true
    editorError.value = ''
    try {
      await putContainerEntity(containerKind, path, newContent.value, cid)
      showToast('success', `已创建「${path}」`)
      wb.cancelCreate()
      await wb.loadAll()
      await selectEntry(path)
    } catch (err) {
      editorError.value = `创建失败: ${err}`
      showToast('error', `创建失败: ${err}`)
    } finally {
      saving.value = false
    }
  }

  // ============ 删除 / 优先级徽标 ============
  /** 删除（确认/状态/toast/刷新/清选中全部在核心状态机） */
  function remove(kind: string, id: string) {
    return wb.remove(kind, id, kindLabel(kind))
  }

  /** extra.priority 下发的优先级徽标文本（无优先级返回空串） */
  function priorityLabel(e: EntitySummary): string {
    const p = e.priority
    return typeof p === 'number' ? `优先级 ${p}` : ''
  }

  return {
    // 模式与侧边栏
    mode,
    isContainer,
    railItems,
    title,
    // 类别
    activeTypes,
    isMulti,
    isCompact,
    containerKinds,
    activeKind,
    activeKindMeta,
    switchKind,
    counts,
    // 清单
    typeStates,
    items,
    kindEntries,
    totalCount,
    allItemsEmpty,
    emptyHint,
    loading,
    loadAll,
    refreshKind,
    // 选中
    selected,
    selectedId,
    select,
    // 新建（统一入口 + 分流状态）与列表头动作开关（能力驱动，§3.4）
    creating,
    creatableInActive,
    canCreate,
    canRefresh,
    // 视图模板用的别名（语义：列表头按钮是否渲染）
    showCreate: canCreate,
    showRefresh: canRefresh,
    onNew,
    createKind,
    activeFormKind,
    beginCreate,
    startTypeChoice,
    startCreate,
    cancelCreate,
    newName,
    newContent,
    createPathPreview,
    saveCreate,
    // entity 保存流
    saving,
    zipUploading,
    uploadError,
    manifestError,
    draftName,
    draftManifest,
    saveZip,
    saveForm,
    saveManifest,
    saveDefault,
    // leaf 详情派生
    capsOf,
    kindLabel,
    createEditor,
    selectedEditor,
    detailDefinition,
    canDeleteSelected,
    // container 文本编辑
    containerName,
    containerIdRef: computed(() => containerId?.() ?? ''),
    selectedEntry,
    content,
    dirty,
    loadingContent,
    entryHasContent,
    editorError,
    selectEntry,
    reloadSelected,
    saveEntry,
    priorityLabel,
    // 删除 / 测试
    deletingId,
    remove,
    testConnection,
    testing: wb.testing,
    showToast,
  }
}

/** 是否可在实体管理器内创建/删除（supports_upload 为"协议实现"维度，与 mutable 解耦） */
export function isManagerCreatable(p: {
  supports_upload?: boolean
  capabilities?: EntityCapabilities
}): boolean {
  return Boolean(p.supports_upload && p.capabilities?.mutable && !p.capabilities.read_only)
}

/**
 * 混合平排列表纯函数（供单测）：把所有活动类型的所有实体展平为一张列表。
 *
 * **排序原则：完全尊重服务器返回顺序，前端不做 name 排序。**
 * - 类型顺序 = activeTypes 顺序（由注册表 order / 路由参数决定）；
 * - 每类型内 = 该类型 `entities/list` 返回的原序（后端决定展示次序，如设置分区清单）。
 */
export function buildMixedItems(
  activeTypes: readonly ProviderInfo[],
  typeStates: Record<string, WorkbenchKindState>
): EntitySummary[] {
  const flat: EntitySummary[] = []
  for (const d of activeTypes) {
    for (const it of typeStates[d.kind]?.items ?? []) {
      flat.push(it)
    }
  }
  return flat
}

/** 供适配层做 props 类型标注 */
export type { Ref }
