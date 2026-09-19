/**
 * useVdfs —— 三栏工作台的唯一数据逻辑（绑定一个 vdfs 数据地址）
 *
 * 不含任何资源类型知识：目录内容来自 `vdfs/list`、详情渲染器由节点的 `ext`
 * 决定（`registry/vdfsTypes`）。新增一种资源 = 后端实现一个 `VdfsProvider`，
 * 本文件零改动。
 *
 * ## 绑定模型（控件自包含，见 VdfsWorkbench）
 *
 * 宿主给一个**数据地址**（`addr`，如 `<根>` 或 `<根>/session/<id>`），本层
 * 完成全部数据装配，**不感知浏览器路由**——数据地址与浏览器地址是两个概念：
 *
 * - **左栏导航** = `addr` 的内容（其子目录清单，后端 order 排列）；
 * - **当前目录**（`cwd`）= 选中的左栏子目录（缺省第一个）；`addr` 无子目录时
 *   落在 `addr` 自身。中栏列表 = `cwd` 内容，点文件选中、点目录钻入
 *   （钻入由控件 emit `open`，宿主决定呈现方式，通常是 push 新地址页）；
 * - **选中节点**（`selectedNode`）：详情来源，按 `ext` 解析渲染器；
 * - **选中记忆**：同一数据地址的左栏选中项会被记住（往返 push / 返回后恢复）；
 * - **实时**：订阅总线 `vdfs` 频道，受影响的目录防抖刷新（非轮询）。
 *   其中**追加型变更**（`appended`）就地拼接正文、不重拉——它是唯一一种能带
 *   增量载荷的变更，也是列表型数据流式输出的承载方式（见 `applyAppend`）。
 *
 * ## UI 约定（S11）
 *
 * - **无面包屑**：路径即导航，层级靠左栏切目录 + 中栏点目录钻入 + 宿主返回键；
 * - **无「新建目录」按钮**：新建 = 新建一种**类型**（会话 / 模型 / …）；
 *   类型清单由当前目录节点声明（`new_types`），多类型时先选类型，然后
 *   **直接进入该类型的详情页**（草稿态：无 id / 名字，与选中一项同一条通道，
 *   见 `startNew`）；目录结构节点由后端 provider 自持，前端不暴露 mkdir 入口。
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
  writeVdfs,
  writeVdfsBinary,
} from '@/services/vdfs'
import { subscribeVdfsChanged } from '@/services/eventBus'
import {
  VDFS_CHANGE_APPENDED,
  VDFS_STATUS_ACTIVE,
  actionFileOf,
  isVdfsDir,
  isVdfsDraft,
  isVdfsSystemAddr,
  newFileNameOf,
  parseVdfsValidation,
  vdfsAccessOf,
  vdfsJoin,
  vdfsParent,
  type DetailAction,
  type VdfsChange,
  type VdfsFieldError,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'
import { dirIconOf, resolveVdfsRenderer, type VdfsRenderer } from '@/registry/vdfsTypes'
import { useToast } from '@/composables/useToast'
import { useGenerationGuard } from '@/composables/useGenerationGuard'
import { logger } from '@/utils/logger'
import type { WorkbenchRailItem } from '@/components/common/Workbench.vue'

/** 各数据地址的左栏选中记忆（模块级：往返 push / 返回后恢复原选中） */
const selectedMemo = new Map<string, string>()

/**
 * 中栏一页的**名义**条数。
 *
 * 名义 = 后端可以返回更多（会话转写按「根」计量，一个 Turn 不拆），
 * 也可以返回更少（不认窗口参数的 provider 会返回全部——那种情况下
 * 「加载更早」一点就到底，见 `hasMore` 的说明）。
 */
const VDFS_PAGE_SIZE = 100

export interface UseVdfsOptions {
  /** 绑定的数据地址（如 `<根>` 或 `<根>/session/<id>`）；变化 = 整体重载 */
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
  const navItems = computed<WorkbenchRailItem[]>(() =>
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

  /**
   * 是否还有更早的条目。
   *
   * 它是**启发值**：后端返回满页 ⇒ 可能还有；点「加载更早」拿到 0 条新项即判到底。
   * 之所以不要求后端回 `has_more`：那要每个 provider 都多算一次，而「多翻一次空页」
   * 的代价只是一次请求——**宁可多问一次，也不要为了精确给机制加字段**。
   */
  const hasMore = ref(false)
  const loadingMore = ref(false)

  /** 加载当前目录（**有界**：默认只取最新一页） */
  async function refresh() {
    loading.value = true
    loadError.value = ''
    try {
      const resp = await listVdfs(cwd.value, { limit: VDFS_PAGE_SIZE })
      items.value = resp.items
      cwdNode.value = resp.node
      hasMore.value = resp.items.length >= VDFS_PAGE_SIZE
      // 选中项若已不在当前目录（被删/被移走），清理选中态。
      // ⚠️ 有界列表下**不能**仅凭「不在这页里」就判它没了——它可能只是落在更早的
      // 一页（深链直开旧会话就是这种情形）。只有确信已拿全时才清理。
      // ⚠️ **草稿不受此判**：它本来就不在清单里（还没落盘），路径为空。
      const sel = selectedNode.value
      if (
        sel &&
        !isVdfsDraft(sel) &&
        !hasMore.value &&
        !items.value.some((n) => n.path === sel.path)
      ) {
        clearSelection()
      }
    } catch (err) {
      loadError.value = String(err)
      items.value = []
      hasMore.value = false
      logger.error('useVdfs', `加载目录失败 ${cwd.value}:`, err)
    } finally {
      loading.value = false
    }
  }

  /**
   * 加载更早的一页（追加到列表尾部）。
   *
   * 游标 = 当前最后一项的 `name`（后端按它定位「这一条之后」）。追加时按 `path`
   * 去重：不认窗口参数的 provider 每次都返回同一整页，去重后**新项为 0**，
   * 于是 `hasMore` 收敛为 false —— 一次空转换来的正确性，比引入 `has_more`
   * 协议字段便宜。
   */
  async function loadMore() {
    if (loadingMore.value || !hasMore.value) return
    const last = items.value[items.value.length - 1]
    if (!last) return
    loadingMore.value = true
    try {
      const resp = await listVdfs(cwd.value, {
        limit: VDFS_PAGE_SIZE,
        before: last.name,
      })
      const known = new Set(items.value.map((n) => n.path))
      const fresh = resp.items.filter((n) => !known.has(n.path))
      items.value = items.value.concat(fresh)
      hasMore.value = fresh.length > 0 && resp.items.length >= VDFS_PAGE_SIZE
    } catch (err) {
      logger.error('useVdfs', `加载更早失败 ${cwd.value}:`, err)
      hasMore.value = false
    } finally {
      loadingMore.value = false
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

  /**
   * 详情读取的**代次守卫**（机制唯一实现，见 `useGenerationGuard`）。
   *
   * 连点多项时，慢响应不得覆盖新选中项的数据；清空选中同样要使在途响应作废
   * （`clearSelection` advance 一次即可）。
   */
  const detailGuard = useGenerationGuard()

  /**
   * 追加的**代际守卫**（键控：键 = 节点路径）——防「在途重读覆盖已应用的增量」。
   *
   * 场景：节点正文流式追加（`appended` 就地拼接，**不发重读**），而此前发出的
   * 一次 `read` 响应稍后到达——它带的是**旧快照**，会把刚拼上去的字抹掉，随后
   * 的追加再拼上去就得到损坏文本。失败是**静默的**：不报错、不崩溃，只少一段字。
   *
   * 记法：读取前记下该路径「已应用过几次追加」，响应回来若这个数变了，说明读取
   * 期间有增量落地（本地内容比响应新），**丢弃该响应**。丢弃是安全的：节点增删
   * 另有 `created` / `deleted` 事件兜底。
   *
   * 与详情读取的代次**必须是两个实例**：两者作废时机不同（前者随选中变化，
   * 后者随增量落地），共用一个代次会互相误伤。
   */
  const appendGuard = useGenerationGuard()

  /** 详情渲染器里「正文即文本缓冲」的那些（追加可安全拼接）；表单/二进制不在其列 */
  const TEXTUAL_RENDERERS = new Set(['text', 'markdown', 'json', 'message'])

  /**
   * 就地应用一条追加型变更；返回是否命中**当前打开的详情**。
   *
   * 命中即拼接，且**不触发刷新**——这正是 `appended` 与 `updated` 的分野：
   * 前者说「尾部多了这些字」，后者说「这个节点变了，请重读」。
   * 未命中当前详情时什么也不做：列表项的结构（`created` / `deleted`）与
   * 预览首行都不受尾部追加影响，为它重拉整目录是纯粹的浪费。
   */
  function applyAppend(change: VdfsChange): boolean {
    const delta = change.delta
    const node = selectedNode.value
    if (!delta || !node || node.path !== change.path) return false
    if (!TEXTUAL_RENDERERS.has(renderer.value)) return false
    nodeText.value += delta
    appendGuard.advance(node.path)
    return true
  }

  function clearSelection() {
    detailGuard.advance()
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
    // 同一个节点的**重读**（刷新收敛）不清空正文缓冲：清空会与在途增量打架——
    // 见 `appendGuard`，被丢弃的旧响应必须留下「本地已更新的那一份」可看，
    // 而不是留下一片空白。换到别的节点才清。
    const sameNode = selectedNode.value?.path === node.path
    selectedNode.value = node
    if (!sameNode) {
      nodeText.value = ''
      formData.value = null
      nodeBinary.value = false
    }
    detailError.value = ''
    fieldErrors.value = []

    // 草稿（新建态）**没有内容可读**：它还没落盘，路径是空的——去读它只会落到
    // 目录地址上。详情渲染器自己知道怎么呈现「还没有的东西」（表单给默认值、
    // 会话给新建引导），机制只需保证不替它读一个不存在的节点。
    if (isVdfsDraft(node)) return

    const r = resolveVdfsRenderer(node)
    if (r !== 'form' && r !== 'text' && r !== 'json' && r !== 'markdown' && r !== 'message')
      return

    // 读取代次令牌：连点多项时，慢响应不得覆盖新选中项的数据
    const token = detailGuard.advance()
    // 追加代际快照：读取期间若有增量落地，本地内容比这次响应新 → 丢弃响应
    const gen = appendGuard.revision(node.path)
    loadingDetail.value = true
    try {
      const content = await readVdfs(node.path)
      if (detailGuard.revision() !== token) return
      if (appendGuard.revision(node.path) !== gen) return
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
      // 用**自己取到的那个代次**比对，不重新 revision()：中途自增会让加载态
      // 永远复位不了（见 useGenerationGuard 的「边界」）
      if (detailGuard.revision() === token) loadingDetail.value = false
    }
  }

  // ==================== 机制动作（单一定义点） ====================
  //
  // 「删除」与「重命名」不是某个资源的私有动作，而是**任何已落盘且可写的节点**
  // 都能做的默认动作。它们由本层算一次，经控件注入所有详情渲染器；渲染器只声明
  // 自己特有的动作（save / reset / test / open-container…）。
  //
  // 此前这组动作有 4 份实现（form / session / text / readonly 各算一遍，形态还
  // 有两种），而后端 `detail_definition.actions` 又对同一概念各声明一次——同一件
  // 事三个来源。收敛到此处后，**存在与否只有一个来源**；渲染器若要用更贴切的
  // 文案重新声明同名动作，由渲染器声明的那一份胜出（见 DetailShell.mergeActions）。
  //
  // 判据只用**访问位**与**地址空间**，不涉及任何资源类型：
  // - 可写位 `w` ⇒ 可删除；
  // - 重命名 = 在同一地址空间内移动。只有物理半边（工作目录文件）由文件系统
  //   provider 承载 `move`；虚拟半边（`<根>` 系统资源）的各插件 provider 一律
  //   没有实现它，给入口只会换来一个必然报错的按钮——故只对物理地址给。
  //
  // 草稿（新建态）两者都不给：还没落盘的东西既无从删除，也无从改名。

  /** 页面对当前节点的默认动作集（渲染器自有的动作不在此列） */
  const mechanismActions = computed<DetailAction[]>(() => {
    const node = selectedNode.value
    if (!node || isVdfsDraft(node) || !vdfsAccessOf(node).write) return []
    const out: DetailAction[] = []
    if (!isVdfsSystemAddr(node.path)) {
      out.push({ id: 'rename', label: '重命名', style: 'secondary' })
    }
    out.push({ id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' })
    return out
  })

  /**
   * 正在执行的机制动作 id（`'delete'` / `'rename'` / null）。
   *
   * 只驱动按钮的进行中文案（`busy_label`）与禁用态，不参与任何判据——「有写操作
   * 在途」那个更宽的概念是 `saving`，它还要锁住列表上的新建 / 刷新按钮。
   */
  const mechanismBusyId = ref<string | null>(null)

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

  /**
   * 写入选中节点内容（form 传字段值对象，文本渲染器传文本）。
   *
   * **草稿（新建态）的写入目标 = 当前目录自身**：使用方没有给名字，名字由
   * provider 生成（`id 归 provider`）——这正是「新建」在机制上的形态，与
   * 「保存一项已有资源」是同一个动作（同一条 `vdfs/write`），差别只有
   * 目标地址与 `create` 意图（目录自身没有可覆盖的目标，必须带 create）。
   *
   * 写完落到 provider 在响应里给出的**新地址**上：新建的落点是刚建出来的
   * 那一项，而不是那张已经失效的草稿。
   */
  async function write(payload: Record<string, unknown> | string): Promise<boolean> {
    const node = selectedNode.value
    if (!node) return false
    const draft = isVdfsDraft(node)
    const text = typeof payload === 'string' ? payload : JSON.stringify(payload)
    saving.value = true
    detailError.value = ''
    fieldErrors.value = []
    try {
      const resp = await writeVdfs(draft ? cwd.value : node.path, text, draft ? { create: true } : undefined)
      showToast('success', draft ? '已新建' : `已保存「${node.title || node.name}」`)
      // 写入可能改变节点元数据（大小/状态）→ 就地重载
      await refresh()
      if (!draft) {
        await select(node)
      } else {
        // 新建：选中 provider 建出来的那一项（响应 `path` 是展示口径，与清单同源）
        const created = items.value.find((n) => n.path === resp.path)
        if (created) await select(created)
      }
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
    mechanismBusyId.value = 'delete'
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
      mechanismBusyId.value = null
    }
  }

  // ==================== 可接受的新建类型（§5） ====================
  //
  // 当前目录节点声明自己能新建哪些类型（`new_types`）；前端只负责「选类型 +
  // 进入详情页」，不认识任何具体类型——创建语义由 provider 自持。
  // `<根>/session` 这类子目录节点由后端合成时携带其 new_types，因此无需任何
  // 「按目录名回退」的特判。
  //
  // **新建 = 选中一张草稿节点**，与「选中一项」走同一条详情通道：同一个
  // `ext` → 同一个渲染器 → 同一个保存入口。名字不由前端先问：它是 provider
  // 的私有知识，保存时由后端生成（写目录自身，见 `write`）。

  /** 当前目录可接受的新建类型 */
  const creatableTypes = computed<VdfsNewType[]>(() => cwdNode.value?.new_types ?? [])

  /** 是否有可新建类型（添加按钮可见性；机制只认节点声明） */
  const canCreate = computed(() => creatableTypes.value.length > 0)

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
  function draftNodeOf(type: VdfsNewType): VdfsNode {
    return {
      path: '',
      name: '',
      title: '',
      kind: type.ext,
      status: VDFS_STATUS_ACTIVE,
      access: 'w',
      ext: type.node_ext || type.ext,
      // 呈现描述随类型下发：草稿与落成后**同一张详情**（用户第 1 点）
      schema: type.schema,
      // 图标键读 `config_type`（与清单项同源）——纯 UI 映射，草稿照给
      config_type: type.ext,
    }
  }

  /**
   * 草稿代际：每进一次新建态 +1。
   *
   * 详情组件的 `:key` 用节点路径——草稿没有路径，连续新建时 key 会撞在一起，
   * 组件不重挂载就会**留着上一份草稿的内容**。这个自增号就是草稿的临时身份。
   */
  const draftSeq = ref(0)

  /** 进入新建态：选中一张该类型的草稿节点（详情区按 `ext` 渲染） */
  function startNew(type: VdfsNewType) {
    draftSeq.value += 1
    void select(draftNodeOf(type))
  }

  /**
   * 按类型从**本地文件**新建（整包导入）：内容走二进制通道。
   *
   * 这是「新建」里唯一**不进入详情页**的形态——因为内容（字节）在打开详情页
   * 之前就已经齐了，没有「边看边填」的过程：`source = file` 的类型（如 `zip`
   * 整包）目标名由**文件名**推导（主干 + 类型扩展名），一次写完即完成。
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
    mechanismBusyId.value = 'rename'
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
      mechanismBusyId.value = null
    }
  }

  // ==================== 实时（总线 vdfs 频道，非轮询） ====================
  // 变更影响当前目录（自身 / 祖先 / 子树内）才刷新；绑定地址一层的变化
  //（左栏子目录增删）顺带重拉导航。其余变更防抖重拉当前目录收敛。
  //
  // **追加型变更是唯一的例外**：它就地拼接、不重拉（见 `applyAppend`）。
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
    // 追加型变更**到此为止**：命中当前详情就就地拼接，未命中就什么也不做。
    // 它不改变任何节点的存在与顺序，也不改变预览首行——因此既不该重拉目录，
    // 也不该重拉左栏导航。让它走下面的通用分支，等于给流式每一帧都挂一次
    // 防抖刷新（O(n²) 流量），正好抵消 `appended` 存在的意义。
    if (change.change === VDFS_CHANGE_APPENDED) {
      applyAppend(change)
      return
    }
    if (affects(cwd.value, change.path)) scheduleRefresh()
    // 绑定地址一层（左栏子目录增删）→ 导航跟着变
    if (change.path === addr.value || vdfsParent(change.path) === addr.value) {
      void refreshNav()
    }
  }

  // 订阅范围 = **绑定地址**（含整棵子树）。三件事由 `subscribeVdfsChanged` 一次办妥：
  // 后端 `vdfs/watch` 登记（引用计数 + 串行链，最后一个订阅者撤走才摘除）、
  // 总线 `vdfs` 频道、以及**前端本地通道**（`publishVdfsChangedLocal`——会话 store
  // 的乐观写靠它通知其它页面；从前用裸 `subscribe` 收不到这条通道）。
  //
  // 订「绑定地址」而不是「当前目录」：左栏子目录的增删要能刷新导航，而那类变更
  // 落在 `addr` 上、不在 `cwd` 子树里；`cwd` 只是 `addr` 的子目录，本就在其子树内，
  // 后端按前缀投递子树变更，因此订 `addr` 覆盖得住。代价是邻目录的变更也会送到
  // handler，由 `affects` 收窄。
  //
  // 「虚拟根不在任何 provider 身上、不该登记 watch」这条知识归 `subscribeVdfsChanged`
  // 所有（它内部跳过根），本层不再复述。
  let releaseChanges: (() => void) | null = null

  // 绑定地址变化 = 换了一个数据地址：换订阅 + 整体重载（左栏、选中、当前目录）
  watch(
    addr,
    () => {
      releaseChanges?.()
      releaseChanges = subscribeVdfsChanged({ prefix: addr.value }, onChange)
      void reload()
    },
    { immediate: true }
  )

  onBeforeUnmount(() => {
    releaseChanges?.()
    if (refreshTimer) clearTimeout(refreshTimer)
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
    hasMore,
    loadingMore,
    refresh,
    loadMore,
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
    // 机制动作（单一定义点，经控件注入所有渲染器）
    mechanismActions,
    mechanismBusyId,
    // 操作
    saveFields,
    saveText,
    runAction,
    removeSelected,
    startNew,
    draftSeq,
    createTypedFile,
    creatableTypes,
    canCreate,
    renameSelected,
  }
}
