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
 * - **实时**：订阅总线 `vdfs` 频道，受影响的目录防抖刷新（非轮询）。信封没有
 *   操作枚举，语义全在载荷的字段上：
 *   - 带**正文**的（`delta` 增量 / `content` 整条替换）只改一个节点的内容，
 *     就地落地、**不重拉目录**——命中当前详情才动手（见 `applyAppend` /
 *     `applyReplace`）。转写列表的消息增量由 `stores/sessionTranscriptSync`
 *     消费；本页只顺带消费「正打开的详情恰是一条流式消息」的场景。
 *   - 其余（资源信号 / 状态帧）重拉收敛，但只在变更**真的影响本目录的条目集合**
 *     时才拉（见 `affects`：自身 / 直接子项 / 祖先）。
 *
 * ## UI 约定（S11）
 *
 * - **无面包屑**：路径即导航，层级靠左栏切目录 + 中栏点目录钻入 + 宿主返回键；
 * - **无「新建目录」按钮**：新建 = 新建一种**类型**（会话 / 模型 / …）；
 *   类型由当前目录节点声明（`new_type`，至多一个），点添加即**直接进入该类型的
 *   详情页**（草稿态：无 id / 名字，与选中一项同一条通道，见 `startNew`）。
 *   详情页上可能另有一条「导入整包」动作（由 `DetailAction.pack` 声明），
 *   它取一个本地文件作为动作载荷——**不是第二种新建入口**，只是同一页上的动作；
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
  readVdfs,
  runVdfsAction,
  writeVdfs,
} from '@/services/vdfs'
import { READBACK_REASON } from '@/services/readback'
import { subscribeVdfsChanged } from '@/services/eventBus'
import {
  VDFS_STATUS_ACTIVE,
  actionFileOf,
  isVdfsDir,
  isVdfsDraft,
  parseVdfsValidation,
  vdfsAccessOf,
  vdfsJoin,
  vdfsParent,
  type DetailAction,
  type VdfsChange,
  type VdfsFieldError,
  type VdfsItem,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'
import {
  dirIconOf,
  isTextualRenderer,
  rendererReadsNodeText,
  resolveVdfsRenderer,
  type VdfsRenderer,
} from '@/registry/vdfsTypes'
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
  /** `addr` 的内容（其子目录 = 左栏导航项）；条目 = 地址 + 节点（`VdfsItem`） */
  const navDirs = ref<VdfsItem[]>([])

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
  /** 中栏条目 = 地址 + 节点；`cwdNode` 是**目录自身的自述**（纯节点，不带地址） */
  const items = ref<VdfsItem[]>([])
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
      const resp = await listVdfs(READBACK_REASON.VDFS_BROWSER, cwd.value, {
        limit: VDFS_PAGE_SIZE,
      })
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
      const resp = await listVdfs(READBACK_REASON.VDFS_BROWSER, cwd.value, {
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
    const resp = await listVdfs(READBACK_REASON.VDFS_BROWSER, addr.value)
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
  /**
   * 当前选中项 = **条目**（地址 + 节点）或**草稿**。
   *
   * 草稿（新建态）还没有落盘，因此没有地址——`path` 是空串，这正是
   * `isVdfsDraft` 的判据（见 `schemas/vdfs.isVdfsDraft`）。
   */
  const selectedNode = shallowRef<VdfsItem | null>(null)
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
   * 场景：节点正文流式追加（`updated` + `delta` 就地拼接，**不发重读**），而此前
   * 发出的一次 `read` 响应稍后到达——它带的是**旧快照**，会把刚拼上去的字抹掉，
   * 随后的追加再拼上去就得到损坏文本。失败是**静默的**：不报错、不崩溃，只少一段字。
   *
   * 记法：读取前记下该路径「已应用过几次追加」，响应回来若这个数变了，说明读取
   * 期间有增量落地（本地内容比响应新），**丢弃该响应**。丢弃是安全的：节点增删
   * 另有 `created` / `deleted` 事件兜底。
   *
   * 与详情读取的代次**必须是两个实例**：两者作废时机不同（前者随选中变化，
   * 后者随增量落地），共用一个代次会互相误伤。转写列表那份同款守卫在
   * `stores/sessionTranscriptSync.ts`（同一规则、不同视图）。
   */
  const appendGuard = useGenerationGuard()

  // 详情渲染器里「正文即文本缓冲」的那些（追加可安全拼接）——判定在
  // `registry/vdfsTypes::isTextualRenderer`（那里也是 `VdfsRenderer` 的定义处）。
  // 先前这里手写了一份 `new Set([...])`，与另外两处各写一份、且其中一处其实
  // 不是同一个集合（见该函数的注释）。

  /**
   * 就地应用一条**带增量的**变更；返回是否命中**当前打开的详情**。
   *
   * `delta` 有 ⇒ 尾部追加。命中即拼接，且**不触发刷新**——载荷存在就是为了
   * 让消费端零回读：它只说「尾部多了这些字」，列表项的结构与预览首行都不受
   * 尾部追加影响，为它重拉整目录是纯粹的浪费（流式正文逐帧到达，那会变成
   * O(n²) 的 `vdfs/list` 流量）。
   */
  function applyAppend(change: VdfsChange): boolean {
    // S27：载荷在 `data` 上——消息帧是 `{id, delta}` 窄载荷；无载荷或载荷里
    // 没有 `delta`（全量帧 / 节点视图）都不是「尾部追加」。
    const data = change.data as { delta?: unknown } | null | undefined
    const delta = typeof data?.delta === 'string' ? data.delta : null
    const node = selectedNode.value
    if (!delta || !node || node.path !== change.path) return false
    if (!isTextualRenderer(renderer.value)) return false
    nodeText.value += delta
    appendGuard.advance(node.path)
    return true
  }

  /**
   * 就地应用一条**带全量正文的**变更；返回是否命中**当前打开的详情**。
   *
   * `content` 有 ⇒ 整条替换（后端首帧发的就是图里合并后的全量副本）。与
   * `applyAppend` 同一条道理：它只改一个节点的**内容**，不改变任何节点的存在
   * 与顺序，列表也不内联正文——所以同样不该重拉目录。
   *
   * 只有**纯文本缓冲**能就地替换；`form` 渲染器要从正文 parse 出字段值（那是
   * `select` 的活），故返回 `false` 由调用方走一次详情重读。
   */
  function applyReplace(change: VdfsChange): boolean {
    const data = change.data as { content?: unknown } | null | undefined
    const content = data?.content
    const node = selectedNode.value
    if (typeof content !== 'string' || !node || node.path !== change.path) return false
    if (!isTextualRenderer(renderer.value)) return false
    nodeText.value = content
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
  async function select(node: VdfsItem | null) {
    if (!node) {
      clearSelection()
      return
    }
    // 同一个节点的**重读**（刷新收敛）不清空正文缓冲：清空会让详情区先白一下
    // 再填回来，而这次重读拿到的往往就是同一份内容。换到别的节点才清。
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
    if (!rendererReadsNodeText(r))
      return

    // 读取代次令牌：连点多项时，慢响应不得覆盖新选中项的数据
    const token = detailGuard.advance()
    // 追加代际快照：读取期间若有增量落地，本地内容比这次响应新 → 丢弃响应
    const gen = appendGuard.revision(node.path)
    loadingDetail.value = true
    try {
      const content = await readVdfs(READBACK_REASON.VDFS_BROWSER, node.path)
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
  // 「删除」不是某个资源的私有动作，而是**任何已落盘且可写的节点**都能做的默认
  // 动作。它由本层算一次，经控件注入所有详情渲染器；渲染器只声明自己特有的动作
  // （save / reset / test / open-container…）。
  //
  // 此前这组动作有 4 份实现（form / session / text / readonly 各算一遍，形态还
  // 有两种），而后端 `detail_definition.actions` 又对同一概念各声明一次——同一件
  // 事三个来源。收敛到此处后，**存在与否只有一个来源**；渲染器若要用更贴切的
  // 文案重新声明同名动作，由渲染器声明的那一份胜出（见 DetailShell.mergeActions）。
  //
  // 判据只用**访问位**：可写位 `w` ⇒ 可删除。
  //
  // **曾经还有「重命名」**（= 同一地址空间内移动），随 `vdfs/move` 整条下线了：
  // 移动不是 VDFS 的核心原语（跨子树时它是 copy+delete，见 `VdfsRequest` 的
  // 「没有 `Move`」一节），当前外层也没有提供。要加回来时它属于**访问层的组合
  // 操作**，不是渲染器动作——别再从这里塞一个必然报错的按钮。
  //
  // 草稿（新建态）不给删除：还没落盘的东西无从删除。

  /** 页面对当前节点的默认动作集（渲染器自有的动作不在此列） */
  const mechanismActions = computed<DetailAction[]>(() => {
    const node = selectedNode.value
    if (!node || isVdfsDraft(node) || !vdfsAccessOf(node).write) return []
    return [{ id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' }]
  })

  /**
   * 正在执行的机制动作 id（当前只有 `'delete'` / null）。
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
   * 写完落到 provider 在响应里给出的**新名字**上（`VdfsWriteResponse.name`；
   * 只有匿名写才有它）——新建的落点是刚建出来的那一项，而不是那张已经失效的草稿。
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
        // 新建：选中 provider 建出来的那一项。回执只给**名字**（匿名写由 provider
        // 生成），地址由「当前目录 + 名字」构成——但中栏刚重载过，按名字找回即可
        // （名字是目录内的路径段，唯一）。
        const created = items.value.find((n) => n.name === resp.name)
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
   * 执行**节点动作**（如「测试连接」「导入整包」「导出」）。
   *
   * 动作标识由详情定义声明、由 provider 解释，本层只负责把它送到
   * `vdfs/action` 并把结果（成功 / 失败 + 说明）呈现出来；**前端不认识
   * 任何具体动作**，新增动作无需改动这里。
   *
   * 唯一例外是**文件载荷**，且两个方向都是形状判定而非动作判定：
   * - 结果里带 `filename` + `b64` → 下载它（见 `actionFileOf`，如「导出」）；
   * - 动作声明了 `pack` → 载荷是一个本地文件（见 `runPackAction`，如「导入」）。
   *
   * 目标地址与 `write` 同一条规则：**草稿（新建态）打在当前目录上**——使用方
   * 只说「在这个目录里做这件事」，名字 / 落点由 provider 决定。
   */
  async function runAction(action: string, payload?: unknown): Promise<boolean> {
    const node = selectedNode.value
    if (!node) return false
    const target = isVdfsDraft(node) ? cwd.value : node.path
    actionBusy.value = true
    detailError.value = ''
    try {
      const r = await runVdfsAction(target, action, payload)
      if (r?.ok) {
        showToast('success', r.message || '执行成功')
        const file = actionFileOf(r.data)
        if (file) downloadBlob(file.filename, base64ToBytes(file.b64))
      } else {
        detailError.value = r?.message || '执行失败'
        showToast('error', detailError.value)
      }
      // 动作可能改变条目集合（导入建出一条、清空删掉一片）→ 重拉收敛
      await refresh()
      // 草稿是**瞬态**：动作在草稿上成功 = 它已经落盘（如「导入整包」建出一条），
      // 草稿页的使命到此结束——继续留着它，等于显示一个并不存在的资源的详情。
      if (r?.ok && isVdfsDraft(node)) clearSelection()
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
  // 当前目录节点声明自己能新建**哪一类**东西（`new_type`，至多一个）；前端只
  // 负责「进入详情页」，不认识任何具体类型——创建语义由 provider 自持。
  // `<根>/session` 这类子目录节点由后端合成时携带其 `new_type`，因此无需任何
  // 「按目录名回退」的特判。
  //
  // **新建 = 选中一张草稿节点**，与「选中一项」走同一条详情通道：同一个
  // `ext` → 同一个渲染器 → 同一个保存入口。名字不由前端先问：它是 provider
  // 的私有知识，保存时由后端生成（写目录自身，见 `write`）。
  //
  // 「导入整包」不在这里：它是**详情页上的一条动作**（由定义声明、经
  // `DetailAction.pack` 表明载荷是一个本地文件），由 `runPackAction` 执行。
  // 曾经它被表达成类型的第二种「入口形态」，于是前端要先问「选哪种方式」，
  // 还得为类型补上 `source` / `import` 两个与呈现无关的字段（见 ADR-029）。

  /** 当前目录可接受的新建类型（至多一个；`undefined` = 该目录不可新建） */
  const creatableType = computed<VdfsNewType | undefined>(() => cwdNode.value?.new_type)

  /** 是否可新建（添加按钮可见性；机制只认节点声明） */
  const canCreate = computed(() => Boolean(creatableType.value))

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
  function draftNodeOf(type: VdfsNewType): VdfsItem {
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

  /**
   * 草稿代际：每进一次新建态 +1。
   *
   * 详情组件的 `:key` 用节点路径——草稿没有路径，连续新建时 key 会撞在一起，
   * 组件不重挂载就会**留着上一份草稿的内容**。这个自增号就是草稿的临时身份。
   */
  const draftSeq = ref(0)

  /** 进入新建态：选中一张该类型的草稿节点（详情区按 `ext` 渲染） */
  function startNew() {
    const type = creatableType.value
    if (!type) return
    draftSeq.value += 1
    void select(draftNodeOf(type))
  }

  /**
   * 把一个**本地文件**作为动作载荷执行（`DetailAction.pack` 声明的动作）。
   *
   * 后端唤不起原生对话框、也拿不到用户刚选的文件，故「取文件」这一步只能由前端
   * 做。载荷形状 `{filename, b64}` 与「导出」的结果同形（`actionFileOf` 认的那
   * 一个）——两者对称，因此前端**不必认识「导入」这个动作**：它只认
   * 「这个动作的载荷是一个文件」这个**形状**。
   *
   * 目标地址由 `runAction` 决定（草稿态 = 当前目录，与 `write` 同一条规则）。
   */
  async function runPackAction(action: string, file: File): Promise<boolean> {
    let b64: string
    try {
      b64 = arrayBufferToBase64(await file.arrayBuffer())
    } catch (err) {
      detailError.value = `读取文件失败：${captureError(err)}`
      showToast('error', detailError.value)
      return false
    }
    return runAction(action, { filename: file.name, b64 })
  }

  // ==================== 实时（总线 vdfs 频道，非轮询） ====================
  // 变更影响当前目录**的条目集合**才刷新；绑定地址一层的变化（左栏子目录增删）
  // 顺带重拉导航。其余变更防抖重拉当前目录收敛。
  //
  // **带正文的变更是唯一的例外**：`delta` 就地拼接、`content` 就地替换，两者都
  // **不重拉**（见 `applyAppend` / `applyReplace`）。它们不改变任何节点的存在与
  // 顺序，也不改变列表项（列表不内联正文）——让它们走下面的通用分支，等于给流式
  // 每一帧都挂一次防抖刷新（一次会话下来就是几十次白跑的 `vdfs/list`），正好抵消
  // 载荷存在的意义。
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
  function affects(dir: string, changePath: string): boolean {
    return changePath === dir || vdfsParent(changePath) === dir || dir.startsWith(`${changePath}/`)
  }

  /**
   * 带正文的变更里，若命中的是一条**本目录还不认识的直接子项**，说明条目集合真的
   * 变了 ⇒ 重拉。
   *
   * 带正文的帧一律不重拉，是为流式逐帧省流量（见 `applyAppend` 的说明）；但那条
   * 豁免默认了一个前提——**被改的节点已经在列表里**。新建出来的那一项不满足这个
   * 前提：它的首帧就是带正文的（写入即带内容 / 流式首帧即增量），此时不重拉会让
   * 新条目**永远不出现在中栏**，要等下一次无关的刷新才补上。
   *
   * 判据只用「列表里有没有这条路径」，**不问帧里带的是 `delta` 还是 `content`**
   * ——帧形状是协议的实现细节，「条目集合有没有变」才是本层要知道的事。
   *
   * 代价被两件事夹住：① 只对**直接子项**生效（孙辈的流式帧照旧不打扰本目录）；
   * ② 一旦该项进入列表就不再命中（同一条消息的后续帧不会再拉）。合起来是
   * 「每新增一条最多多拉一次」，而不是「每帧一次」。
   */
  function refreshIfUnknownChild(changePath: string) {
    if (vdfsParent(changePath) !== cwd.value) return
    if (items.value.some((n) => n.path === changePath)) return
    scheduleRefresh()
  }

  /** 数据变更回调（路径过滤在 handler 内，订阅恒定一条） */
  function onChange(change: VdfsChange) {
    const data = change.data as { delta?: unknown; content?: unknown } | null | undefined

    // ① 带增量的变更：命中当前详情就就地拼接，未命中就什么也不做。
    if (typeof data?.delta === 'string') {
      applyAppend(change)
      refreshIfUnknownChild(change.path)
      return
    }

    // ② 带全量正文的变更（首帧发的就是合并后的全量副本）：命中当前详情就就地
    //    替换，同样不重拉目录。命中当前详情但渲染器不吃纯文本（`form` 要从正文
    //    parse 字段值）⇒ 只重读这一条，仍不重拉目录。
    if (data != null && typeof data === 'object' && data.content != null) {
      const sel = selectedNode.value
      if (!applyReplace(change) && sel && sel.path === change.path) void select(sel)
      refreshIfUnknownChild(change.path)
      return
    }

    // ③ 其余变更（资源信号 / 只有状态的消息帧）：只有真的影响本目录的条目集合
    //    才重拉。
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
      releaseChanges = subscribeVdfsChanged({ prefix: addr.value }, onChange, () =>
        // 重同步：后端通道曾满，本端可能漏了增删改。整份重载本地址（含左栏与当前目录）
        // ——本页是通用资源浏览器，没有比「按绑定地址重读」更权威的收敛方式。
        void reload()
      )
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
    runPackAction,
    removeSelected,
    startNew,
    draftSeq,
    creatableType,
    canCreate,
  }
}
