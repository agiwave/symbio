/**
 * VdfsWorkbench — 控件行为单测（happy-dom）
 *
 * 这台控件是三栏工作台的**唯一**实现，所以本文件测的是它的分工，不是某个类型：
 *
 * - **「新建」入口**：节点声明了可接受类型（`new_type`）时按钮可见，点一下**直接
 *   进入该类型的详情页**。没有「选方式」这一步——整包导入是详情页上的一条动作，
 *   不是第二种新建入口。它很容易只剩一个无参调用后被"顺手"加回一跳，
 *   所以值得钉住。
 * - **动作转发**：渲染器上抛的动作按**载荷形状**分流——带 `File` 的走
 *   `runPackAction`（如「导入整包」），其余走 `runAction`（如「导出」）。
 *   控件不认识任何具体动作名，所以这条分流必须由断言钉住，否则「导入」会
 *   悄悄退化成一条不带载荷的动作、或者前端又开始按动作名分支。
 * - **自动草稿 / 空态即引导 / 中栏收起 / 筛选**：都取「节点自述」为判据，
 *   新增一类资源前端零改动。
 * - **键盘**（`Ctrl+数字` 切类别、`↑↓` 移动焦点、`Home/End` 跳首尾）：绑定声明在控件里，
 *   判键与让路的机制在 `useHotkeys`（那里另有单测）。
 *
 * 环境说明：`useVdfs` 被整体替换为可控桩（它经 `services/vdfs` 出站），
 * 渲染器登记表也被挡掉（那会把整条会话渲染链连同传输层拉进来）。
 */

// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { computed, defineComponent, h, nextTick, ref, shallowRef, watch, type Ref } from 'vue'

const hoisted = vi.hoisted(() => {
  const fns = {
    startNew: vi.fn(),
    runAction: vi.fn(),
    runPackAction: vi.fn(),
    removeSelected: vi.fn(),
    loadMore: vi.fn(),
    loadTreeChildren: vi.fn(),
    refresh: vi.fn(),
    retryDetail: vi.fn(),
    markSaved: vi.fn(),
  }
  /** 每次 `startNew()` 实际开出的**类型**——断言的是"开了哪一类"，不只是"开了几次" */
  const newTypes: (string | null)[] = []
  return { stub: {} as Record<string, unknown>, fns, newTypes, rendererTitle: '', sessions: { setSessionSpace: vi.fn() } }
})

vi.mock('@/composables/useVdfs', () => ({ useVdfs: () => hoisted.stub }))
// 本控件只用到 store 的**一个**能力：声明「当前空间」（当前目录能新建会话时）。
// 用桩而不是真 store，本文件的用例就不必为一个与新建/转发无关的机制架 Pinia。
vi.mock('@/stores/sessions', () => ({ useSessionsStore: () => hoisted.sessions }))
// 真实的渲染器登记表会把整条会话渲染链（连同传输层）拉进来，故挡掉；
// 需要「详情渲染器」在场的用例改用下面登记的桩（见 RendererStub）。
vi.mock('@/registry/vdfsRenderers', () => ({}))

import VdfsWorkbench from '../VdfsWorkbench.vue'
import ConfirmDialog from '@/components/common/ConfirmDialog.vue'
import DetailShell from '../DetailShell.vue'
import { dispatchHotkeys } from '@/composables/useHotkeys'
import { registerVdfsRenderer, type VdfsRenderer } from '@/registry/vdfsTypes'
import type { DetailAction, VdfsItem, VdfsNewType, VdfsNode, VdfsTreeNode } from '@/schemas/vdfs'
import { NEW_RESOURCE_EVENT } from '@/utils/newResource'

/**
 * 详情渲染器桩：它只做两件事——
 *
 * - 把页面注入的**机制动作**渲染出来（证明「机制动作经控件注入所有渲染器」
 *   这条约定仍在：控件不自己渲染删除按钮）；
 * - 作为**动作上抛的源头**（本文件要测的就是控件怎么接这一抛）。
 *
 * 用 `$emit` 直接触发而不是渲染按钮，是为了让用例只依赖「上抛了什么」，
 * 不依赖渲染器内部长什么样——那是各渲染器自己的测试该管的事。
 */
const RendererStub = defineComponent({
  name: 'RendererStub',
  props: { mechanismActions: { type: Array, default: () => [] } },
  // 声明而非留给 fallthrough：`action` 是本桩要上抛的事件，不是要往下传的 attr
  emits: ['action', 'delete', 'dirty-change'],
  setup(props, { expose }) {
    expose({ currentTitle: () => hoisted.rendererTitle })
    return () => h(DetailShell, { actions: props.mechanismActions as DetailAction[] })
  },
})
// `session` 是本文件节点（`makeNode`）的渲染器键（ext → renderer 由 vdfsTypes 解析）
registerVdfsRenderer('session', RendererStub)

/** 表单桩：保留本地输入，用来验证保存失败只更新 error、不卸载或重置表单。 */
const FormRendererStub = defineComponent({
  name: 'FormRendererStub',
  props: { data: { type: Object, default: null }, error: { type: String, default: '' } },
  emits: ['save'],
  setup(props, { expose }) {
    expose({ markSaved: () => hoisted.fns.markSaved() })
    const name = ref('')
    watch(
      () => props.data,
      (data) => {
        name.value = data && typeof data === 'object' ? String((data as Record<string, unknown>).name ?? '') : ''
      },
      { immediate: true },
    )
    return () => h('div', [
      h('input', {
        class: 'renderer-name',
        value: name.value,
        onInput: (event: Event) => { name.value = (event.target as HTMLInputElement).value },
      }),
      props.error ? h('p', { class: 'renderer-error' }, props.error) : null,
    ])
  },
})
registerVdfsRenderer('form', FormRendererStub)

/** 正文桩：让 nodeText 在 DOM 里可直接观察，验证慢读时不会闪现旧正文。 */
const TextRendererStub = defineComponent({
  name: 'TextRendererStub',
  props: { data: { type: String, default: '' } },
  setup(props) {
    return () => h('pre', { class: 'renderer-text' }, props.data)
  },
})
registerVdfsRenderer('text', TextRendererStub)

/** 配置型：呈现 ext 是 skill、落成后节点 ext 是 form（详情是定义驱动表单） */
const SKILL: VdfsNewType = { ext: 'skill', title: '技能', node_ext: 'form' }

function makeNode(name: string): VdfsItem {
  return {
    path: `@vfs/session/${name}`,
    name,
    title: name,
    kind: 'session',
    status: 'active',
    access: 'rw',
    ext: 'session',
  }
}

/** `installStub` 装上的可写 ref（换目录的用例要在挂载后改它们） */
let stubRefs: {
  cwd: Ref<string>
  cwdNode: Ref<VdfsNode | null>
  cwdAddr: Ref<string>
  selectedNode: Ref<VdfsNode | null>
  items: Ref<VdfsItem[]>
  treeNodes: Ref<VdfsTreeNode[]>
  loading: Ref<boolean>
  saving: Ref<boolean>
  actionBusy: Ref<boolean>
  testResult: Ref<{ ok: boolean; message: string } | null>
  pinnedAtAddr: Ref<boolean>
  loadingDetail: Ref<boolean>
  detailLoadError: Ref<string>
  detailError: Ref<string>
  formData: Ref<Record<string, unknown> | null>
  nodeText: Ref<string>
} | null = null

/** 可控的 `useVdfs` 桩：只填本文件关心的那几个键，其余给空值 */
function installStub(
  opts: {
    newType?: VdfsNewType | null
    selected?: VdfsNode | null
    items?: VdfsItem[]
    /** 当前目录地址（`@vfs` / `@vfs/session`…）；默认 `@vfs`（资源根） */
    cwd?: string
    /** 当前目录**节点**；`new_type.ext = session` 时控件会声明「当前空间」 */
    cwdNode?: VdfsNode | null
    /** 左栏类别（挂载层的目录名，按顺序）——键盘「切到第 N 个类别」的判据取自这里 */
    nav?: string[]
    /** 目录树形态；子层由节点按需加载，失败保留在节点内。 */
    treeMode?: boolean
    treeNodes?: VdfsTreeNode[]
    /** 是否还有更早的一页 */
    hasMore?: boolean
    /** 目录列表是否仍在加载 */
    loading?: boolean
    /** 深链到达态尚未由用户主动解除 */
    pinnedAtAddr?: boolean
    /** 详情读取态与两类独立错误 */
    loadingDetail?: boolean
    detailLoadError?: string
    detailError?: string
    /** 渲染器与已读数据（详情槽用例用） */
    renderer?: VdfsRenderer
    formData?: Record<string, unknown> | null
    nodeText?: string
  } = {}
) {
  const selectedNode = ref<VdfsNode | null>(opts.selected ?? null)
  const items = ref<VdfsItem[]>(opts.items ?? [])
  const treeNodes = shallowRef(opts.treeNodes ?? [])
  const loading = ref(opts.loading ?? false)
  const saving = ref(false)
  const actionBusy = ref(false)
  const testResult = ref<{ ok: boolean; message: string } | null>(null)
  const pinnedAtAddr = ref(opts.pinnedAtAddr ?? false)
  const loadingDetail = ref(opts.loadingDetail ?? false)
  const detailLoadError = ref(opts.detailLoadError ?? '')
  const detailError = ref(opts.detailError ?? '')
  const formData = ref<Record<string, unknown> | null>(opts.formData ?? null)
  const nodeText = ref(opts.nodeText ?? '')
  // 这两个**可写**：换目录的用例要在挂载后改它们（真实现里 `cwd` 立刻变、
  // `cwdNode` 要等 `refresh()` 回来，两者的时间差正是那条滞后 bug 的来源）
  const cwd = ref(opts.cwd ?? '@vfs')
  const cwdNode = shallowRef<VdfsNode | null>(opts.cwdNode ?? null)
  // `cwdNode` 是**哪个地址**的自述（真实现里由 list 回执同源登记）。缺省与 `cwd`
  // 一致 = 「自述已经跟上当前目录」；换目录的用例可以把两者拆开，复刻那一步滞后。
  const cwdAddr = ref(opts.cwd ?? '@vfs')
  stubRefs = {
    cwd, cwdNode, cwdAddr, selectedNode, items, treeNodes, loading, saving, actionBusy, testResult, pinnedAtAddr,
    loadingDetail, detailLoadError, detailError, formData, nodeText,
  }
  /** 筛选词：真实现是 ref + computed 投影；桩里复刻成同一个形状，
   *  好让「筛选框输入 → 列表收窄」这条链路在本文件也能被断言 */
  const filter = ref('')
  const filteredItems = computed(() => {
    const q = filter.value.trim().toLowerCase()
    if (!q) return items.value
    return items.value.filter(
      (n) => n.name.toLowerCase().includes(q) || (n.title ?? '').toLowerCase().includes(q)
    )
  })
  hoisted.stub = {
    // 左栏类别：与真实现同源（`useVdfs.navItems` = 挂载层的目录节点，顺序即后端顺序）
    navItems: computed(() => (opts.nav ?? []).map((name) => ({ key: name, label: name, icon: null, active: false }))),
    // 挂载层 vs 目录层：本文件的键盘用例走「在挂载层按数字切类别」，即就地 selectDir
    atMountLevel: computed(() => false),
    selectDir: vi.fn(),
    selectedName: ref<string | null>(null),
    pinnedAtAddr,
    cwd,
    // 当前目录节点：`new_type.ext = session` 时控件会声明「当前空间」
    // （会话挂载目录）；只有已加载完成且为空的目录才自动开草稿。
    // 默认给 null（不可新建的目录），本文件多数用例测的是新建入口与动作转发。
    cwdNode: computed<VdfsNode | null>(() => cwdNode.value),
    // 自述对应的地址（真实现里 `cwdAddr === cwd` 才说明自述已经跟上当前目录）
    cwdAddr: computed(() => cwdAddr.value),
    title: computed(() => '资源'),
    items,
    treeMode: ref(opts.treeMode ?? false),
    treeNodes,
    filteredItems,
    // 筛选入口是否在场：与真实现**同源** —— `cwdNode.search === true`（服务端
    // 声明，裁决在 list 响应里已做好）。前端不自判「条目够不够多」。
    searchable: computed(() => cwdNode.value?.search === true),
    filter,
    activeFilter: computed(() => filter.value.trim().toLowerCase()),
    filterEmpty: computed(
      () => Boolean(filter.value.trim()) && items.value.length > 0 && filteredItems.value.length === 0
    ),
    filterTruncated: computed(() => false),
    loading,
    loadError: ref(''),
    hasMore: ref(opts.hasMore ?? false),
    loadingMore: ref(false),
    select: vi.fn(),
    selectedNode,
    selectedId: computed(() => selectedNode.value?.path ?? null),
    renderer: computed(() => opts.renderer ?? 'form'),
    nodeText,
    formData,
    loadingDetail,
    detailLoadError,
    saving,
    actionBusy,
    testResult,
    clearTestResult: () => { testResult.value = null },
    detailError,
    fieldErrors: ref([]),
    // 机制动作（当前只有「删除」）由页面算好注入渲染器；可写节点才有它
    mechanismActions: computed<DetailAction[]>(() =>
      opts.selected ? [{ id: 'delete', label: '删除', style: 'secondary' }] : []
    ),
    mechanismBusyId: ref<string | null>(null),
    saveFields: vi.fn(),
    saveText: vi.fn(),
    // 可新建类型**来自当前目录节点的自述**（前端不做任何推导）——与真实现同源：
    // `useVdfs.creatableType` 先核 `cwdAddr === cwd`（自述与当前目录同源），
    // 再取 `cwdNode.new_type`。桩里必须用同一条来源，否则「换目录那一拍拿上一栏
    // 自述开草稿 / 顶着上一栏标题」这条回归会因为桩与真实现不同源而测不出来。
    creatableType: computed(() =>
      cwdAddr.value !== cwd.value
        ? undefined
        : (opts.newType ?? cwdNode.value?.new_type ?? undefined)
    ),
    canCreate: computed(() =>
      cwdAddr.value === cwd.value && Boolean(opts.newType ?? cwdNode.value?.new_type)
    ),
    draftSeq: ref(0),
    ...hoisted.fns,
  }
  // 记下每次「新建」开的是**哪一类**：只数次数会漏掉"开对了次数却开错了类型"
  hoisted.fns.startNew.mockImplementation(() => {
    hoisted.newTypes.push(
      (opts.newType ?? cwdNode.value?.new_type ?? null)?.ext ?? null
    )
  })
}

function bench() {
  return mount(VdfsWorkbench, { props: { addr: '@vfs' } })
}

/** header 上的「新建」按钮 */
function newButton(w: VueWrapper) {
  return w.findAll('button').find((b) => (b.attributes('title') ?? '').startsWith('新建'))
}

/** 从渲染器桩上抛一个动作（`file` 有值 = 该动作声明了 `pack`） */
async function emitAction(w: VueWrapper, id: string, file?: File, payload?: unknown) {
  w.findComponent({ name: 'RendererStub' }).vm.$emit('action', id, payload, file)
  await w.vm.$nextTick()
}

beforeEach(() => {
  Object.values(hoisted.fns).forEach((f) => f.mockClear())
  hoisted.sessions.setSessionSpace.mockClear()
  hoisted.newTypes.length = 0
  hoisted.rendererTitle = ''
  stubRefs = null
})

describe('VdfsWorkbench 新建入口', () => {
  it('目录声明了可新建类型 ⇒ 按钮可见，标题带上类型名', () => {
    installStub({ newType: SKILL })
    const w = bench()

    const btn = newButton(w)
    expect(btn, '声明了 new_type 就该有新建按钮').toBeTruthy()
    expect(btn?.attributes('title')).toBe('新建 技能')
  })

  it('动作忙态禁用新建入口，不能留在途保存旁再开一张草稿', async () => {
    installStub({ newType: SKILL })
    const w = bench()
    hoisted.fns.startNew.mockClear()
    const btn = newButton(w)!
    expect(btn.element.disabled).toBe(false)

    stubRefs!.actionBusy.value = true
    await nextTick()

    expect(btn.element.disabled).toBe(true)
    await btn.trigger('click')
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()
  })

  it('未声明可新建类型 ⇒ 没有按钮（机制只认节点声明，不做任何回退）', () => {
    installStub({ newType: null })
    expect(newButton(bench())).toBeUndefined()
  })

  it('★ 换类别的那一拍不得顶着**上一类**的标题（MCP → Skill 串类回归）', async () => {
    // 用户报的现象：左栏 Mcp 项与 Skill 项都显示「新建Mcp」，Skill 的功能出不来。
    // 根因：`cwdNode` 是异步 list 回执带来的滞后自述。从 MCP 切到 Skill 时
    // `cwd` 立刻变、自述还停在 MCP，若照直取 `cwdNode.new_type`，Skill 页那一拍
    // 顶着「新建 MCP」，点下去开的也是 MCP 的草稿。
    const mcpType: VdfsNewType = { ext: 'mcp', title: 'MCP' }
    const skillType: VdfsNewType = { ext: 'skill', title: '技能' }
    installStub({
      cwd: '@vfs/mcp',
      cwdNode: {
        path: '@vfs/mcp', name: 'mcp', title: 'MCP',
        kind: 'dir', status: '', access: 'l', new_type: mcpType,
      },
    })
    const w = bench()
    expect(newButton(w)?.attributes('title'), 'MCP 页显示自己的标题').toBe('新建 MCP')

    // 切到 Skill：地址先变，自述还是 MCP 的（cwdAddr 仍停在 @vfs/mcp）
    stubRefs!.cwd.value = '@vfs/skill'
    await nextTick()
    expect(
      newButton(w),
      '自述没跟上之前按钮该先缺席，而不是拿 MCP 的标题冒充 Skill',
    ).toBeUndefined()

    // 自述回来（cwdAddr 与 cwd 同源，且换成 Skill 的类型）⇒ 标题与类型都切过来
    stubRefs!.cwdAddr.value = '@vfs/skill'
    stubRefs!.cwdNode.value = {
      path: '@vfs/skill', name: 'skill', title: '技能',
      kind: 'dir', status: '', access: 'l', new_type: skillType,
    }
    await nextTick()
    expect(newButton(w)?.attributes('title'), 'Skill 页显示自己的标题').toBe('新建 技能')
  })

  it('点「新建」**直接进详情页**：一次无参调用，没有"选方式"这一步', async () => {
    installStub({ newType: SKILL })
    const w = bench()
    // 挂载时「可新建的目录」会先自动备好一张草稿（另一条机制，见下文 describe）。
    // 这一条只钉按钮这一跳，故把那一次清掉，断言才不会把两条机制混成一个数。
    hoisted.fns.startNew.mockClear()
    await newButton(w)!.trigger('click')

    // 无参：类型由控件自己从当前目录节点读（`startNew` 内部读 `creatableType`），
    // 于是「新建」只有一个入口形态，调用方无从表达"选哪个入口"
    expect(hoisted.fns.startNew).toHaveBeenCalledWith()
    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
    // 没有第二跳：不存在"进了某个提示态、还要再选一次"的中间界面
    expect(hoisted.fns.runAction).not.toHaveBeenCalled()
    expect(hoisted.fns.runPackAction).not.toHaveBeenCalled()
  })
})

describe('VdfsWorkbench 动作转发：按**载荷形状**分流', () => {
  it('不带文件的上抛 ⇒ runAction（如「导出」「测试连接」）', async () => {
    installStub({ selected: makeNode('s1') })
    const w = bench()
    await emitAction(w, 'export')

    expect(hoisted.fns.runAction).toHaveBeenCalledWith('export')
    expect(hoisted.fns.runPackAction).not.toHaveBeenCalled()
  })

  it('连接测试回执持续显示且不遮住表单，字段变化后作废', async () => {
    installStub({ renderer: 'session', selected: { path: '@vfs/model/p', name: 'p', title: 'P', kind: 'model', status: 'active', access: 'rw', ext: 'session' } })
    const w = bench()
    stubRefs!.testResult.value = { ok: true, message: '连接测试通过：gpt-4o · openai_chat · 18 ms' }
    await nextTick()
    expect(w.find('.vdfs-action-result').text()).toContain('连接测试通过')
    expect(w.findComponent({ name: 'RendererStub' }).exists()).toBe(true)

    w.findComponent({ name: 'RendererStub' }).vm.$emit('dirty-change', true)
    await nextTick()
    expect(w.find('.vdfs-action-result').exists()).toBe(false)
    w.unmount()
  })

  it('动作参数原样交给 runAction，普通文件动作仍走 pack 通道', async () => {
    installStub({ selected: makeNode('s1') })
    const w = bench()
    const draft = { model: 'draft-model' }
    await emitAction(w, 'test', undefined, draft)
    expect(hoisted.fns.runAction).toHaveBeenCalledWith('test', draft)
  })

  it('带 File 的上抛 ⇒ runPackAction（如「导入整包」）', async () => {
    installStub({ selected: makeNode('s1') })
    const w = bench()
    const file = new File(['PK'], 'demo.zip', { type: 'application/zip' })
    await emitAction(w, 'import', file)

    // 动作名与文件原样转交：控件不认识「导入」，只认「这个动作的载荷是文件」
    expect(hoisted.fns.runPackAction).toHaveBeenCalledWith('import', file, undefined)
    expect(hoisted.fns.runAction).not.toHaveBeenCalled()
  })
})

describe('VdfsWorkbench 详情槽', () => {
  it('机制动作由页面算好、经控件注入渲染器（控件不自己渲染删除）', () => {
    installStub({ selected: makeNode('s1') })
    const w = bench()

    expect(w.findComponent({ name: 'RendererStub' }).props('mechanismActions')).toEqual([
      { id: 'delete', label: '删除', style: 'secondary' },
    ])
  })

  it('★ 慢读时骨架优先，真实表单与旧字段值都不渲染', () => {
    const node: VdfsItem = { ...makeNode('m1'), kind: 'model', ext: 'form' }
    installStub({ selected: node, formData: { name: '旧值' }, loadingDetail: true })
    const w = bench()

    expect(w.find('.vdfs-detail-skeleton').exists()).toBe(true)
    expect(w.findComponent({ name: 'FormRendererStub' }).exists()).toBe(false)
    expect(w.find('.renderer-name').exists()).toBe(false)
    expect(w.text()).not.toContain('旧值')
  })

  it('★ 慢读时旧 nodeText 也不得先渲染成可编辑正文', async () => {
    const node: VdfsItem = { ...makeNode('a.md'), kind: 'file', ext: 'text' }
    installStub({ selected: node, renderer: 'text', nodeText: '旧正文', loadingDetail: true })
    const w = bench()
    expect(w.find('.renderer-text').exists()).toBe(false)

    stubRefs!.loadingDetail.value = false
    await nextTick()
    expect(w.find('.renderer-text').text()).toBe('旧正文')
  })

  it('读取失败持久显示明确错误与「重试详情」，不挂载未就绪表单', async () => {
    const node: VdfsItem = { ...makeNode('m1'), kind: 'model', ext: 'form' }
    installStub({
      selected: node,
      formData: { name: '旧值' },
      detailLoadError: '读取失败：连接中断',
    })
    const w = bench()

    expect(w.find('.vdfs-detail-error').attributes('role')).toBe('alert')
    expect(w.find('.vdfs-detail-error').text()).toContain('读取失败：连接中断')
    expect(w.findComponent({ name: 'FormRendererStub' }).exists()).toBe(false)
    await w.find('.vdfs-detail-error button').trigger('click')
    expect(hoisted.fns.retryDetail).toHaveBeenCalledTimes(1)
  })

  it('表单普通保存只写字段，带动作参数的保存改走 vdfs/action', async () => {
    const node: VdfsItem = { ...makeNode('m1'), kind: 'model', ext: 'form' }
    installStub({ selected: node, formData: { name: '服务端值' } })
    const saveFields = vi.mocked(hoisted.stub.saveFields as ReturnType<typeof vi.fn>)
    saveFields.mockResolvedValue(true)
    hoisted.fns.runAction.mockResolvedValue(true)
    const w = bench()
    const form = w.findComponent({ name: 'FormRendererStub' })

    form.vm.$emit('save', { name: '普通保存' })
    expect(saveFields).toHaveBeenCalledWith({ name: '普通保存' })
    expect(hoisted.fns.runAction).not.toHaveBeenCalled()

    form.vm.$emit('save', { name: '离线保存' }, {
      fields: { name: '动作声明值' },
      skip_validation: true,
    })
    expect(saveFields).toHaveBeenCalledTimes(1)
    expect(hoisted.fns.runAction).toHaveBeenCalledWith('save', {
      fields: { name: '离线保存' },
      skip_validation: true,
    })
    await nextTick()
  })

  it('旧保存回执不得把新选中的表单标记为已保存', async () => {
    const first: VdfsItem = { ...makeNode('m1'), kind: 'model', ext: 'form' }
    const second: VdfsItem = { ...makeNode('m2'), kind: 'model', ext: 'form' }
    installStub({ selected: first, formData: { name: 'first' } })
    let resolveSave!: (ok: boolean) => void
    hoisted.fns.runAction.mockReturnValue(new Promise<boolean>((resolve) => {
      resolveSave = resolve
    }))
    const w = bench()

    w.findComponent({ name: 'FormRendererStub' }).vm.$emit(
      'save',
      { name: 'saved' },
      { skip_validation: true },
    )
    stubRefs!.selectedNode.value = second
    await nextTick()
    resolveSave(true)
    await Promise.resolve()
    await nextTick()

    expect(hoisted.fns.markSaved).not.toHaveBeenCalled()
  })

  it('★ 保存失败仍保留当前表单与用户输入，只展示操作错误', async () => {
    const node: VdfsItem = { ...makeNode('m1'), kind: 'model', ext: 'form' }
    installStub({ selected: node, formData: { name: '服务端值' } })
    const w = bench()
    const input = w.find('.renderer-name')
    await input.setValue('用户输入')

    stubRefs!.detailError.value = '保存失败：写入被拒绝'
    await nextTick()

    expect(w.findComponent({ name: 'FormRendererStub' }).exists()).toBe(true)
    expect(w.find<HTMLInputElement>('.renderer-name').element.value).toBe('用户输入')
    expect(w.find('.renderer-error').text()).toContain('写入被拒绝')
    expect(w.find('.vdfs-detail-error').exists()).toBe(false)
    expect(w.find('.vdfs-detail-skeleton').exists()).toBe(false)
  })

  it('删除确认冻结目标与最新标题，后续选中变化不改变删除对象', async () => {
    const target = { ...makeNode('s1'), title: '旧标题' }
    installStub({ selected: target })
    hoisted.rendererTitle = '重命名后的标题'
    const w = bench()
    const renderer = w.findComponent({ name: 'RendererStub' })

    renderer.vm.$emit('delete')
    await nextTick()
    expect(w.text()).toContain('将删除「重命名后的标题」')

    stubRefs!.selectedNode.value = makeNode('s2')
    await nextTick()
    expect(w.text()).toContain('将删除「重命名后的标题」')

    const dialog = w.findAllComponents(ConfirmDialog)[0]
    dialog.vm.$emit('confirm')
    expect(hoisted.fns.removeSelected).toHaveBeenCalledWith(
      expect.objectContaining({ path: target.path, title: '重命名后的标题' }),
    )
  })

  it('路由离开确认在取消后保留 dirty，确认后放行', async () => {
    installStub({ selected: makeNode('s1') })
    const w = bench()
    const renderer = w.findComponent({ name: 'RendererStub' })
    renderer.vm.$emit('dirty-change', true)
    await nextTick()

    let settled = false
    const first = w.vm.confirmRouteLeave().then((allowed) => {
      settled = true
      return allowed
    })
    await nextTick()
    expect(settled).toBe(false)
    const leaveDialog = () =>
      w.findAllComponents(ConfirmDialog).find((d) => d.props('title') === '放弃未保存的修改？')!
    leaveDialog().vm.$emit('cancel')
    await expect(first).resolves.toBe(false)

    const second = w.vm.confirmRouteLeave()
    leaveDialog().vm.$emit('confirm')
    await expect(second).resolves.toBe(true)
    await expect(w.vm.confirmRouteLeave()).resolves.toBe(true)
  })

  it('有未保存修改时阻止刷新与硬离页，确认放弃后放行', async () => {
    installStub({ selected: makeNode('s1') })
    const w = bench()
    w.findComponent({ name: 'RendererStub' }).vm.$emit('dirty-change', true)
    await nextTick()

    const dirtyUnload = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(dirtyUnload)
    expect(dirtyUnload.defaultPrevented).toBe(true)

    const pending = w.vm.confirmRouteLeave()
    w.findAllComponents(ConfirmDialog).find((d) => d.props('title') === '放弃未保存的修改？')!.vm.$emit('confirm')
    await pending
    const cleanUnload = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(cleanUnload)
    expect(cleanUnload.defaultPrevented).toBe(false)
  })
})

/**
 * 列表筛选。
 *
 * **入口可见性由服务端声明**：`cwdNode.search` 来自 list 响应——条目够不够多、
 * 值不值得检索，这个裁决在后端统一做（阈值见 plugins/vdfs/host.rs），控件只认
 * 声明：未表态（search 缺席，如 stat 上的节点）与明确 false 一律不给。这条边界
 * 值得钉住，它很容易被"顺手"改回 `items.length > 0`——那等于前端又自己长出一条
 * 裁决，而且一个筛不出东西的搜索框会让用户以为"可以搜库"：筛选是**纯客户端
 * 投影**（只筛已加载条目，不发请求、不动分页游标），不是全库内容检索
 * （`vdfs/search` 的结果不保证落在当前目录里，混进中栏会破坏「点目录即钻入」
 * 的空间心智）。
 *
 * 同时钉住三种空态的区分：目录为空 / 筛掉了 / 加载失败——它们的下一步动作
 * 完全不同（新建 / 清筛选 / 重试），说成一句就是把用户晾在原地。
 */
describe('VdfsWorkbench 列表筛选', () => {
  const list = () => [makeNode('alpha'), makeNode('beta'), makeNode('gamma')]

  /** 当前目录节点：`search` 是 list 响应里的裁决（缺席 = 服务端未表态） */
  function dirNode(search?: boolean): VdfsNode {
    const node: VdfsNode = {
      path: '@vfs/session',
      name: 'session',
      title: '会话',
      kind: 'dir',
      status: 'active',
      access: 'rw',
      ext: 'dir',
    }
    if (search !== undefined) node.search = search
    return node
  }

  it('服务端声明 search: true ⇒ 出现筛选框；输入即收窄列表（大小写不敏感）', async () => {
    installStub({ items: list(), cwdNode: dirNode(true) })
    const w = bench()
    const input = w.find('.vdfs-filter-input')
    expect(input.exists()).toBe(true)
    expect(w.findAll('.vdfs-card')).toHaveLength(3)

    await input.setValue('BETA')
    expect(w.findAll('.vdfs-card')).toHaveLength(1)
    expect(w.find('.vdfs-card').text()).toContain('beta')
  })

  it('★ 服务端未表态（search 缺席）⇒ 不给筛选框（没有声明的功能不自己长出来）', () => {
    installStub({ items: list(), cwdNode: dirNode() })
    expect(bench().find('.vdfs-filter-input').exists()).toBe(false)
  })

  it('★ 有内容但服务端明确不启用（false）⇒ 仍不给筛选框（不是「有东西就可筛」）', () => {
    installStub({ items: list(), cwdNode: dirNode(false) })
    expect(bench().find('.vdfs-filter-input').exists()).toBe(false)
  })

  it('★ 空目录但服务端声明启用 ⇒ 筛选框仍在（前端不二次裁决条目数）', () => {
    installStub({ items: [], cwdNode: dirNode(true) })
    expect(bench().find('.vdfs-filter-input').exists()).toBe(true)
  })

  it('筛掉全部结果 ⇒ 空态说明是"没有匹配"并给出清除入口（而非「此目录为空」）', async () => {
    installStub({ items: list(), cwdNode: dirNode(true) })
    const w = bench()
    await w.find('.vdfs-filter-input').setValue('zzz')

    expect(w.find('.empty-state').text()).toContain('没有匹配')
    expect(w.find('.empty-state').text()).not.toContain('此目录为空')

    await w.find('.empty-action').trigger('click')
    expect(w.findAll('.vdfs-card')).toHaveLength(3)
  })

  it('筛掉全部结果且还有更早页 ⇒ 仍保留“加载更早”入口', async () => {
    installStub({ items: list(), cwdNode: dirNode(true), hasMore: true })
    const w = bench()
    await w.find('.vdfs-filter-input').setValue('zzz')

    expect(w.find('.empty-state').text()).toContain('没有匹配')
    expect(w.find('.list-more').exists()).toBe(true)
    expect(w.find('.vdfs-list .list-more').exists()).toBe(false)
    await w.find('.more-btn').trigger('click')
    expect(hoisted.fns.loadMore).toHaveBeenCalledTimes(1)
  })

  it('目录子层读取失败 ⇒ 就地显示错误，点击节点重试', async () => {
    const dir: VdfsItem = { ...makeNode('docs'), kind: 'dir', access: 'lt', ext: 'dir' }
    installStub({
      treeMode: true,
      treeNodes: [{ item: dir, children: [], loaded: false, loadError: '加载失败：连接中断' }],
    })
    const w = bench()

    await w.get('.tree-row').trigger('click')
    expect(w.get('.tree-load-error').text()).toContain('连接中断')
    expect(w.find('.tree-error').exists()).toBe(false)
    expect(w.find('.empty-state').exists()).toBe(false)
    expect(hoisted.fns.loadTreeChildren).toHaveBeenCalledWith(dir.path)
  })

  it('懒加载目录显示局部加载态，不伪装成空目录', async () => {
    const dir: VdfsItem = { ...makeNode('docs'), kind: 'dir', access: 'lt', ext: 'dir' }
    installStub({
      treeMode: true,
      treeNodes: [{ item: dir, children: [], loaded: false, loading: true }],
    })
    const w = bench()

    await w.get('.tree-row').trigger('click')
    expect(w.get('.tree-loading').text()).toContain('加载中')
    expect(w.get('[role="treeitem"]').attributes('aria-busy')).toBe('true')
    expect(w.find('.empty-state').exists()).toBe(false)
  })

  it('树模式也保留当前层续页入口，不再提示整树截断', async () => {
    installStub({
      treeMode: true,
      treeNodes: [{ item: makeNode('a'), children: [] }],
      hasMore: true,
    })
    const w = bench()

    expect(w.find('.list-more').exists()).toBe(true)
    expect(w.text()).not.toContain('目录过大，仅显示部分层级')
    await w.find('.more-btn').trigger('click')
    expect(hoisted.fns.loadMore).toHaveBeenCalledTimes(1)
  })

  it('Esc 与 × 都能清除筛选', async () => {
    installStub({ items: list(), cwdNode: dirNode(true) })
    const w = bench()
    await w.find('.vdfs-filter-input').setValue('alpha')
    expect(w.findAll('.vdfs-card')).toHaveLength(1)

    await w.find('.vdfs-filter-input').trigger('keydown', { key: 'Escape' })
    expect(w.findAll('.vdfs-card')).toHaveLength(3)

    await w.find('.vdfs-filter-input').setValue('beta')
    await w.find('.filter-clear').trigger('click')
    expect(w.findAll('.vdfs-card')).toHaveLength(3)
  })
})

/**
 * **空目录**自动备好草稿；已有条目的目录默认列表 + 空详情。
 *
 * 判据仍必须与类型无关：目录自述声明 `new_type`，且本次 list 已加载完成、条目确实
 * 为空。自述与 items 同源（`cwdAddr === cwd`），加载中 / 读失败不提前开；深链到达态
 * 不抢；每个目录只开一次；schema 驱动的选择器页只由用户明确新建。
 */
describe('VdfsWorkbench 空目录自动草稿与明确新建', () => {
  const SESSION_TYPE: VdfsNewType = { ext: 'session', title: '会话' }
  /** 会话挂载目录节点：`new_type.ext = session` ⇒ 控件据此认定「当前空间」 */
  const mountNode: VdfsNode = {
    path: '@vfs/session',
    name: 'session',
    title: '会话',
    kind: 'dir',
    status: 'active',
    // 挂载根的访问位是 `l`（后端 `VdfsNode::dir("", "会话", VdfsAccess::LIST)`），
    // 新建走「写目录自身」，不靠 `w` 位。
    access: 'l',
    ext: 'dir',
    new_type: SESSION_TYPE,
  }

  it('落在会话目录且未选中任何项 ⇒ 自动进入新建态', () => {
    installStub({ cwd: '@vfs/session', cwdNode: mountNode })
    bench()

    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
  })

  it('★ 空的可新建目录不区分类型（模型 / MCP / 技能同样自动开）', () => {
    installStub({
      cwd: '@vfs/mcp',
      cwdNode: { ...mountNode, path: '@vfs/mcp', name: 'mcp', new_type: { ext: 'mcp', title: 'MCP' } },
    })
    bench()

    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
  })

  it.each(['model', 'mcp', 'skill', 'sandbox'])(
    '★ %s 目录已有条目 ⇒ 默认列表 + 空详情，不自动抢草稿',
    (name) => {
      installStub({
        cwd: `@vfs/${name}`,
        cwdNode: {
          ...mountNode,
          path: `@vfs/${name}`,
          name,
          new_type: { ext: name, title: name, node_ext: 'form' },
        },
        items: [makeNode('existing')],
      })
      const w = bench()

      expect(hoisted.fns.startNew).not.toHaveBeenCalled()
      expect(w.findAll('.vdfs-card')).toHaveLength(1)
      expect(w.find('.workbench-list').exists()).toBe(true)
      expect(w.findComponent({ name: 'FormRendererStub' }).exists()).toBe(false)
    },
  )

  it('★ 后端新下发的类型（前端没见过的名字）⇒ 照样自动开（不靠类型名单）', () => {
    installStub({
      cwd: '@vfs/widget',
      cwdNode: {
        ...mountNode,
        path: '@vfs/widget',
        name: 'widget',
        // 一个前端代码里搜不到的类型名：只要后端声明可新建，就该给草稿
        new_type: { ext: 'widget', title: '挂件', node_ext: 'form' },
      },
    })
    bench()

    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
  })

  it('★ 换目录时自述与 items 必须同源，错位的那一拍不得开草稿', async () => {
    const agent: VdfsNode = {
      ...mountNode,
      path: '@vfs/subagent',
      name: 'subagent',
      new_type: { ext: 'subagent', title: '子智能体' },
    }
    const model: VdfsNode = {
      ...mountNode,
      path: '@vfs/model',
      name: 'model',
      new_type: { ext: 'model', title: '模型' },
    }
    installStub({ cwd: '@vfs/subagent', cwdNode: agent, items: [makeNode('existing')] })
    bench()
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()

    // 地址先变，自述与条目仍属于上一栏。
    stubRefs!.cwd.value = '@vfs/model'
    await nextTick()
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()

    // 新自述先到，但来源地址尚未对齐，仍不能认。
    stubRefs!.cwdNode.value = model
    await nextTick()
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()

    // 同源登记后的空列表才是一份可自动开草稿的完整回执。
    stubRefs!.items.value = []
    stubRefs!.cwdAddr.value = '@vfs/model'
    await nextTick()
    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
    expect(hoisted.newTypes).toEqual(['model'])
  })

  it('列表加载中不得提前把空数组当成结果自动开草稿', async () => {
    installStub({ cwd: '@vfs/session', cwdNode: mountNode, items: [], loading: true })
    bench()
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()

    stubRefs!.loading.value = false
    await nextTick()
    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
  })

  it('深链到达态不得被自动草稿抢走（包括 list 后仍在 stat 的窗口）', () => {
    installStub({ cwd: '@vfs/session', cwdNode: mountNode, items: [], pinnedAtAddr: true })
    bench()
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()
  })

  it('已有条目时，Ctrl+N / 命令面板的明确新建请求仍直接开草稿', async () => {
    const cwd = '@vfs/explicit-new'
    installStub({
      cwd,
      cwdNode: { ...mountNode, path: cwd, name: 'explicit-new' },
      items: [makeNode('existing')],
    })
    const w = bench()
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()

    window.dispatchEvent(new CustomEvent(NEW_RESOURCE_EVENT, { detail: cwd }))
    await nextTick()
    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
    w.unmount()
  })

  it('明确新建早于列表回执时排队，目录就绪后只开一次', async () => {
    const cwd = '@vfs/explicit-new-loading'
    installStub({
      cwd,
      cwdNode: { ...mountNode, path: cwd, name: 'explicit-new-loading' },
      items: [makeNode('existing')],
      loading: true,
    })
    const w = bench()

    window.dispatchEvent(new CustomEvent(NEW_RESOURCE_EVENT, { detail: cwd }))
    await nextTick()
    expect(hoisted.fns.startNew).not.toHaveBeenCalled()

    stubRefs!.loading.value = false
    await nextTick()
    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
    w.unmount()
  })

  it('已有选中项 ⇒ 不覆盖用户的选择', () => {
    installStub({ cwd: '@vfs/session', cwdNode: mountNode, selected: makeNode('existing') })
    bench()

    expect(hoisted.fns.startNew).not.toHaveBeenCalled()
  })

  it('目录不可新建（没有 new_type）⇒ 不自动开', () => {
    installStub({ cwd: '@vfs/session', cwdNode: { ...mountNode, new_type: undefined } })
    bench()

    expect(hoisted.fns.startNew).not.toHaveBeenCalled()
  })

  it('★ 选择器页（market 渲染器）⇒ 进目录不开草稿（先看列表）', () => {
    installStub({
      cwd: '@vfs/plugin_manager',
      cwdNode: {
        ...mountNode,
        path: '@vfs/plugin_manager',
        name: 'plugin_manager',
        title: '插件管理',
        // 市场是「添加插件」页面：可以点新建打开，但进目录不自动开
        new_type: { ext: 'market', node_ext: 'market', title: '添加插件' },
      },
    })
    bench()

    expect(hoisted.fns.startNew).not.toHaveBeenCalled()
  })
})

/**
 * 「列表为空 ⇒ 直接显示详情（新建）」的**端到端**连着测。
 *
 * 容器单测（`common/__tests__/Workbench.spec.ts`）钉的是判据本身；这里要钉的是
 * **控件喂给容器的三个事实对不对**——判据再对，喂错了照样不对：
 *
 * - `hasListContent` 取 `filteredItems.length`（筛选后为空也算空）；
 * - `hasDetail` 取「有选中项且有渲染器」；
 * - `hasDraft` 取 `isVdfsDraft(selectedNode)`。
 *
 * 最后一条最容易写错成"选中项是不是某种资源类型"——草稿的判据是**没有 path**，
 * 与类型无关（模型 / 技能 / 会话的草稿都该收起中栏）。
 */
describe('VdfsWorkbench 列表为空时收起中栏（喂给容器的三个事实）', () => {
  /** 草稿节点：**没有 path** 就是草稿（`isVdfsDraft` 的定义） */
  const draft = (): VdfsNode =>
    ({ name: '', title: '', kind: 'session', status: 'active', access: 'rw', ext: 'session' }) as VdfsNode

  it('空列表 + 草稿详情 + 可新建 ⇒ 中栏不渲染（用户的原始诉求）', () => {
    installStub({ newType: { ext: 'session', title: '会话' }, selected: draft(), items: [] })
    const w = bench()

    expect(w.find('.workbench-list').exists()).toBe(false)
  })

  it('空列表但右栏是**已有条目** ⇒ 中栏保留（那句空态解释必须留着）', () => {
    installStub({ newType: { ext: 'session', title: '会话' }, selected: makeNode('existing'), items: [] })
    const w = bench()

    expect(w.find('.workbench-list').exists()).toBe(true)
  })

  it('列表有内容 ⇒ 无论右栏是什么都不收起', () => {
    installStub({
      newType: { ext: 'session', title: '会话' },
      selected: draft(),
      items: [makeNode('a'), makeNode('b')],
    })
    const w = bench()

    expect(w.find('.workbench-list').exists()).toBe(true)
  })

  it('草稿判据是"没有 path"：配置型草稿（非会话）同样收起中栏', () => {
    // 技能草稿：ext 仍是 session（桩的渲染器键），但关键是它**没有 path**
    installStub({ newType: SKILL, selected: draft(), items: [] })
    const w = bench()

    expect(w.find('.workbench-list').exists()).toBe(false)
  })
})

/**
 * 空态即引导：可新建的空目录必须给一个**可执行**的出口。
 *
 * 中栏会有可见空态的主要场合是「刚把最后一项删掉 / 清空了选中」——此时自动草稿
 * 已经开过一次（`autoDraftDone`），不会再开，用户看到的就只有那句「此目录为空」。
 * 从前它只提示「点击右上角「新建」添加」，等于把用户推去找一个图标。
 *
 * ⚠️ 文案里的类型名必须来自**类型自述的 `title`**（后端下发），不是前端按类型
 * 分支写死的字面量——后者会让「新增一类资源前端零开发」在这条路径上失效。
 */
describe('VdfsWorkbench 空态即引导', () => {
  it('★ 可新建的空目录 ⇒ 给「新建第一个 ⟨类型标题⟩」按钮，点它就开草稿', async () => {
    installStub({ newType: { ext: 'session', title: '会话' }, selected: null, items: [] })
    const w = bench()

    const cta = w.find('.empty-action.primary')
    expect(cta.exists(), '空目录不能只给说明、不给出口').toBe(true)
    expect(cta.text(), '类型名取自类型自述（后端下发）').toBe('新建第一个会话')

    const before = hoisted.fns.startNew.mock.calls.length
    await cta.trigger('click')
    expect(hoisted.fns.startNew.mock.calls.length).toBe(before + 1)
  })

  it('不可新建的空目录 ⇒ 只说「该目录由系统管理」，不给按钮（不给假的出口）', () => {
    installStub({ newType: null, selected: null, items: [], cwdNode: null })
    const w = bench()

    expect(w.find('.empty-action.primary').exists()).toBe(false)
    expect(w.text()).toContain('该目录由系统管理')
  })
})

/**
 * 键盘：五条绑定**声明在本控件**（机制与守卫在 `useHotkeys`，那里另有单测）。
 *
 * 本文件钉的是控件这一层的分工：
 *
 * - `Ctrl+数字` 切的是**左栏顺序**的第 N 个类别，且数字超出类别数时**不吞键**
 *   ——`Ctrl+9` 是浏览器自己的「跳到最后一个标签页」，被一条空转的死键吃掉
 *   比没有这条键更糟；
 * - ↑↓ 只**移动焦点**、不改变选中：浏览与决定是两件事（`Enter` 才开详情）；
 * - Home/End 跳到首尾，并把唯一的 Tab 停靠点同步过去；
 * - ↑↓ 的 `when` 判据是「焦点在这份列表里」，不是「本页是 VDFS 页」——
 *   焦点在详情栏或聊天输入框时抢走方向键会直接破坏滚动与输入。
 *
 * ⚠️ 必须 `attachTo: document.body`：焦点判据读的是 `document.activeElement`，
 * 而脱离文档的元素拿不到焦点（VTU 默认挂在游离节点上，那样这些用例全测不到）。
 * ⚠️ 必须 `unmount`：登记表是模块级单例，注销走 `onScopeDispose`。
 */
describe('VdfsWorkbench 键盘与动态反馈', () => {
  let w: VueWrapper | null = null

  function mountAttached(opts: Parameters<typeof installStub>[0] = {}) {
    installStub(opts)
    w = mount(VdfsWorkbench, { props: { addr: '@vfs' }, attachTo: document.body })
    return w
  }

  /** 打一次键并交给派发（真实页面里那一刀是 MainLayout 挂在 window 上的监听器） */
  function pressKey(over: KeyboardEventInit) {
    const evt = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over })
    document.body.dispatchEvent(evt)
    return { consumed: dispatchHotkeys(evt), prevented: evt.defaultPrevented }
  }

  const cards = () => Array.from(document.querySelectorAll<HTMLElement>('.vdfs-card'))

  afterEach(() => {
    w?.unmount()
    w = null
    vi.useRealTimers()
    document.body.replaceChildren()
  })

  it('Ctrl+数字 ⇒ 按左栏顺序切到第 N 个类别', () => {
    const mounted = mountAttached({ nav: ['session', 'model', 'skill'] })
    expect(mounted.findAll('.nav-btn').map((b) => b.attributes('aria-label'))).toEqual([
      'session',
      'model',
      'skill',
    ])

    expect(pressKey({ key: '2', ctrlKey: true })).toEqual({ consumed: true, prevented: true })
    expect(hoisted.stub.selectDir).toHaveBeenCalledWith('model')
  })

  it('★ 数字超出类别数 ⇒ 不吞键（那个组合还给浏览器，不是变成一条什么都不做的死键）', () => {
    mountAttached({ nav: ['session', 'model', 'skill'] })

    expect(pressKey({ key: '9', ctrlKey: true })).toEqual({ consumed: false, prevented: false })
    expect(hoisted.stub.selectDir).not.toHaveBeenCalled()
  })

  it('长列表只有一个 Tab 停靠点，↑↓ 同步移动 roving tabindex', async () => {
    mountAttached({ items: [makeNode('a'), makeNode('b'), makeNode('c')] })
    expect(cards().map((card) => card.tabIndex)).toEqual([0, -1, -1])

    const list = document.querySelector<HTMLElement>('.vdfs-list')!
    Object.defineProperty(list, 'clientHeight', { configurable: true, value: 100 })
    Object.defineProperty(list, 'scrollHeight', { configurable: true, value: 1000 })
    list.dispatchEvent(new Event('scroll'))
    expect(list.querySelector<HTMLElement>('[role="scrollbar"]')?.tabIndex).toBe(-1)

    cards()[0].focus()

    expect(pressKey({ key: 'ArrowDown' }).consumed).toBe(true)
    await nextTick()
    expect(document.activeElement).toBe(cards()[1])
    expect(cards().map((card) => card.tabIndex)).toEqual([-1, 0, -1])
    expect(hoisted.stub.select).not.toHaveBeenCalled()
  })

  it('深链选中项成为唯一的初始 Tab 停靠点', async () => {
    mountAttached({ items: [makeNode('a'), makeNode('b'), makeNode('c')], selected: makeNode('b') })
    await nextTick()
    expect(cards().map((card) => card.tabIndex)).toEqual([-1, 0, -1])
  })

  it('Home / End 跳到首尾并同步 Tab 停靠点', async () => {
    mountAttached({ items: [makeNode('a'), makeNode('b'), makeNode('c')] })
    cards()[1].focus()

    expect(pressKey({ key: 'End' }).consumed).toBe(true)
    await nextTick()
    expect(document.activeElement).toBe(cards()[2])
    expect(cards().map((card) => card.tabIndex)).toEqual([-1, -1, 0])

    expect(pressKey({ key: 'Home' }).consumed).toBe(true)
    await nextTick()
    expect(document.activeElement).toBe(cards()[0])
    expect(cards().map((card) => card.tabIndex)).toEqual([0, -1, -1])
  })

  it('筛选移除当前浏览项后，Tab 停靠点回落到可见首项', async () => {
    const mounted = mountAttached({
      items: [makeNode('alpha'), makeNode('beta'), makeNode('gamma')],
      selected: makeNode('beta'),
      cwdNode: { path: '@vfs/session', name: 'session', title: '会话', kind: 'dir', status: '', access: 'l', search: true },
    })
    cards()[1].focus()
    await mounted.find('.vdfs-filter-input').setValue('gamma')
    await nextTick()

    expect(cards()).toHaveLength(1)
    expect(cards()[0].tabIndex).toBe(0)
  })

  it('深链选中项不在当前页时，可见首项仍可 Tab 到达', () => {
    mountAttached({ items: [makeNode('gamma')], selected: makeNode('beta') })
    expect(cards()).toHaveLength(1)
    expect(cards()[0].tabIndex).toBe(0)
  })

  it('↑↓ 只移动焦点、不改选中', () => {
    mountAttached({ items: [makeNode('a'), makeNode('b'), makeNode('c')] })
    cards()[0].focus()

    expect(pressKey({ key: 'ArrowDown' }).consumed).toBe(true)
    expect(document.activeElement).toBe(cards()[1])

    expect(pressKey({ key: 'ArrowUp' }).consumed).toBe(true)
    expect(document.activeElement).toBe(cards()[0])

    expect(hoisted.stub.select).not.toHaveBeenCalled()
  })

  it('到顶/到底停住：不绕回（长列表里绕回会把人甩到看不见的位置）', () => {
    mountAttached({ items: [makeNode('a'), makeNode('b')] })
    cards()[1].focus()

    expect(pressKey({ key: 'ArrowDown' }).consumed).toBe(true)
    expect(document.activeElement).toBe(cards()[1])
  })

  it('树模式使用 tree/treeitem 语义，↑↓ 在可见 .tree-node 之间移动焦点', async () => {
    mountAttached({
      treeMode: true,
      treeNodes: [
        { item: makeNode('a'), children: [] },
        { item: makeNode('b'), children: [] },
      ],
    })
    const list = document.querySelector<HTMLElement>('.vdfs-list')!
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.tree-node'))
    expect(list.getAttribute('role')).toBe('tree')
    expect(list.getAttribute('aria-label')).toBe('资源目录树')
    expect(rows.map((row) => row.getAttribute('role'))).toEqual(['treeitem', 'treeitem'])
    rows[0].focus()

    expect(pressKey({ key: 'ArrowDown' }).consumed).toBe(true)
    await nextTick()
    expect(document.activeElement).toBe(rows[1])

    expect(pressKey({ key: 'ArrowUp' }).consumed).toBe(true)
    expect(document.activeElement).toBe(rows[0])
  })

  it('树深链选中后代文件时，祖先展开且只有该文件是 Tab 停靠点', async () => {
    const selected = makeNode('nested/b.txt')
    mountAttached({
      treeMode: true,
      selected,
      treeNodes: [{
        item: {
          path: '@vfs/session/nested', name: 'nested', title: 'nested',
          kind: 'dir', status: '', access: 'lt', ext: 'dir',
        },
        children: [
          { item: makeNode('nested/a.txt'), children: [] },
          { item: selected, children: [] },
        ],
      }],
    })
    await nextTick()

    const rows = Array.from(document.querySelectorAll<HTMLElement>('.tree-node'))
    expect(rows).toHaveLength(3)
    expect(rows[0].getAttribute('aria-expanded')).toBe('true')
    expect(rows.map((row) => row.tabIndex)).toEqual([-1, -1, 0])
    expect(rows[2].getAttribute('aria-selected')).toBe('true')

    rows[2].focus()
    expect(pressKey({ key: 'ArrowUp' }).consumed).toBe(true)
    await nextTick()
    expect(rows.map((row) => row.tabIndex)).toEqual([-1, 0, -1])
    expect(rows[2].getAttribute('aria-selected')).toBe('true')
  })

  it('树刷新更新扁平 items 时不清除深层浏览焦点', async () => {
    const selected = makeNode('nested/b.txt')
    mountAttached({
      treeMode: true,
      selected,
      treeNodes: [{
        item: {
          path: '@vfs/session/nested', name: 'nested', title: 'nested',
          kind: 'dir', status: '', access: 'lt', ext: 'dir',
        },
        children: [
          { item: makeNode('nested/a.txt'), children: [] },
          { item: selected, children: [] },
        ],
      }],
    })
    await nextTick()
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.tree-node'))
    rows[1].focus()
    await nextTick()
    expect(rows.map((row) => row.tabIndex)).toEqual([-1, 0, -1])

    stubRefs!.items.value = [makeNode('nested/a.txt'), selected]
    await nextTick()

    expect(document.activeElement).toBe(rows[1])
    expect(rows.map((row) => row.tabIndex)).toEqual([-1, 0, -1])
    expect(rows[2].getAttribute('aria-selected')).toBe('true')
  })

  it('扁平列表保持 listbox/option 语义', () => {
    mountAttached({ items: [makeNode('a'), makeNode('b')] })
    const list = document.querySelector<HTMLElement>('.vdfs-list')!
    expect(list.getAttribute('role')).toBe('listbox')
    expect(list.getAttribute('aria-label')).toBe('资源列表')
    expect(Array.from(list.querySelectorAll('[role="option"]'))).toHaveLength(2)
  })

  it('★ 焦点不在列表里 ⇒ ↑↓ 还给页面（详情栏与聊天输入的滚动不被抢走）', () => {
    const mounted = mountAttached({ items: [makeNode('a'), makeNode('b')] })
    const outside = document.createElement('input')
    document.body.appendChild(outside)
    outside.focus()

    expect(pressKey({ key: 'ArrowDown' })).toEqual({ consumed: false, prevented: false })
    expect(document.activeElement).toBe(outside)
    expect(mounted.exists()).toBe(true)
  })

  it('刷新中同时给出禁用态、旋转反馈与读屏状态', async () => {
    mountAttached({ items: [makeNode('a')], loading: true })
    const refresh = document.querySelector<HTMLButtonElement>('.refresh-btn')!
    expect(refresh.disabled).toBe(true)
    expect(refresh.classList.contains('is-loading')).toBe(true)
    expect(refresh.getAttribute('aria-label')).toBe('正在刷新')
    expect(document.querySelector('[role="status"]')?.textContent).toContain('正在刷新资源列表')

    stubRefs!.loading.value = false
    await nextTick()
    expect(refresh.disabled).toBe(false)
    expect(refresh.classList.contains('is-loading')).toBe(false)
    expect(refresh.getAttribute('aria-label')).toBe('刷新')
    expect(document.querySelector('[role="status"]')?.textContent).toBe('')
  })

  it('刷新按钮进入忙态后仍把焦点还给触发者', async () => {
    installStub({ items: [makeNode('a')] })
    let finish!: () => void
    hoisted.fns.refresh.mockImplementationOnce(() => new Promise<void>((resolve) => {
      finish = resolve
    }))
    const w = mountAttached({ items: [makeNode('a')] })
    const refresh = document.querySelector<HTMLButtonElement>('.refresh-btn')!
    refresh.focus()
    void w.find('.refresh-btn').trigger('click')

    stubRefs!.loading.value = true
    await nextTick()
    expect(refresh.disabled).toBe(true)
    document.body.tabIndex = -1
    document.body.focus()
    document.body.removeAttribute('tabindex')
    expect(document.activeElement).toBe(document.body)

    stubRefs!.loading.value = false
    finish()
    await vi.waitFor(() => expect(document.activeElement).toBe(refresh))
    expect(refresh.disabled).toBe(false)
  })

  it('相对时间随分钟时钟自动老化，无需刷新页面', async () => {
    vi.useFakeTimers()
    const now = new Date(2026, 10, 26, 12, 0, 0).getTime()
    vi.setSystemTime(now)
    mountAttached({ items: [{ ...makeNode('a'), updated_at: now / 1000 }] })
    expect(document.querySelector('.vdfs-card')?.textContent).toContain('刚刚')

    vi.advanceTimersByTime(60_000)
    await nextTick()
    expect(document.querySelector('.vdfs-card')?.textContent).toContain('1 分钟前')
  })
})

/**
 * 「当前空间」声明 —— 只有**会话挂载目录**才能被声明成会话空间。
 *
 * 这条判据错了不会报错，只会把清单 / 新建 / 转写订阅 / 收件箱全部导到一个不是
 * 挂载目录的地址上，然后控制台刷出一串
 * 「会话内部不存在该路径」「没有 kind=messages 的子节点」「该路径不可写入」。
 * 三个条件缺一不可，每条都对应一次真实事故：
 *
 * 1. `new_type.ext = session` —— 判据与 `services/vdfsScheme` 认出根挂载同源；
 * 2. 必须是**目录**（访问位含 `l`）—— 会话节点自己的 ext 也是 `session`，
 *    但它是叶子（`rw`），选中一个会话就把 `<挂载目录>/<id>` 当成了空间；
 * 3. 自述与当前目录**同源**（`cwdAddr === cwd`）—— 换目录时 `cwdNode` 一步滞后，
 *    进入会话后点开 `message` / `subsession`，它们的地址就被上一栏的自述
 *    （`ext = session`）认成了空间。
 */
describe('VdfsWorkbench 当前空间声明（会话挂载目录）', () => {
  const SESSION_TYPE: VdfsNewType = { ext: 'session', title: '会话' }
  const mountNode: VdfsNode = {
    path: '@vfs/session',
    name: 'session',
    title: '会话',
    kind: 'dir',
    status: 'active',
    access: 'l',
    ext: 'dir',
    new_type: SESSION_TYPE,
  }

  it('会话挂载目录（可下钻 + 声明可新建 session）⇒ 声明为当前空间', () => {
    installStub({ cwd: '@vfs/session', cwdNode: mountNode })

    bench()

    expect(hoisted.sessions.setSessionSpace).toHaveBeenCalledWith('@vfs/session')
  })

  it('★ 会话节点自身（ext 也是 session，但它是叶子）不声明空间', () => {
    // 会话叶子：`rw`（不可下钻），ext 与会话挂载目录撞名
    const sessionLeaf: VdfsNode = {
      path: '@vfs/session/s1',
      name: 's1',
      title: 's1',
      kind: 'session',
      status: 'active',
      access: 'rw',
      ext: 'session',
      new_type: SESSION_TYPE,
    }
    installStub({ cwd: '@vfs/session/s1', cwdNode: sessionLeaf })

    bench()

    expect(hoisted.sessions.setSessionSpace).not.toHaveBeenCalled()
  })

  it('★ 换目录后自述还没回来（cwd 变了、cwdAddr 没变）⇒ 不拿上一栏的自述声明空间', () => {
    installStub({ cwd: '@vfs/session', cwdNode: mountNode })
    bench()
    hoisted.sessions.setSessionSpace.mockClear()

    // 用户点开会话内部的 `message` 目录：地址立刻变了，而 `cwdNode` 还是上一栏的
    stubRefs!.cwd.value = '@vfs/session/s1/message'
    // `cwdAddr` 仍停在 '@vfs/session'，正是那一拍的状态
    return nextTick().then(() => {
      expect(hoisted.sessions.setSessionSpace).not.toHaveBeenCalled()
    })
  })

  it('★ 只有目录声明 session 但不可下钻 ⇒ 不声明（目录性优先于类型自述）', () => {
    installStub({ cwd: '@vfs/x', cwdNode: { ...mountNode, path: '@vfs/x', name: 'x', access: 'rw' } })

    bench()

    expect(hoisted.sessions.setSessionSpace).not.toHaveBeenCalled()
  })
})
