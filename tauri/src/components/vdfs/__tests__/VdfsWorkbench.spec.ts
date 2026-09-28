/**
 * VdfsWorkbench — **新建入口与动作转发**单测（happy-dom）
 *
 * 只测两件事：
 *
 * - **「新建」按钮**：节点声明了可接受类型（`new_type`）时可见，点一下**直接
 *   进入该类型的详情页**。没有「选方式」这一步——整包导入是详情页上的一条动作，
 *   不是第二种新建入口（见 ADR-029）。这条约定曾经由一整个提示态状态机承担，
 *   现在退化成一个无参调用，正因如此更值得钉住：它很容易被"顺手"加回去。
 * - **动作转发**：渲染器上抛的动作按**载荷形状**分流——带 `File` 的走
 *   `runPackAction`（如「导入整包」），其余走 `runAction`（如「导出」）。
 *   控件不认识任何具体动作名，所以这条分流必须由断言钉住，否则「导入」会
 *   悄悄退化成一条不带载荷的动作、或者前端又开始按动作名分支。
 *
 * 环境说明：`useVdfs` 被整体替换为可控桩（它经 `services/vdfs` 出站），
 * 渲染器登记表也被挡掉（那会把整条会话渲染链连同 Tauri 通道拉进来）。
 */

// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { computed, defineComponent, h, nextTick, ref, shallowRef, type Ref } from 'vue'

const hoisted = vi.hoisted(() => {
  const fns = { startNew: vi.fn(), runAction: vi.fn(), runPackAction: vi.fn() }
  /** 每次 `startNew()` 实际开出的**类型**——断言的是"开了哪一类"，不只是"开了几次" */
  const newTypes: (string | null)[] = []
  return { stub: {} as Record<string, unknown>, fns, newTypes, sessions: { setSessionSpace: vi.fn() } }
})

vi.mock('@/composables/useVdfs', () => ({ useVdfs: () => hoisted.stub }))
// 本控件只用到 store 的**一个**能力：声明「当前空间」（当前目录能新建会话时）。
// 用桩而不是真 store，本文件的用例就不必为一个与新建/转发无关的机制架 Pinia。
vi.mock('@/stores/sessions', () => ({ useSessionsStore: () => hoisted.sessions }))
// 真实的渲染器登记表会把整条会话渲染链（连同 Tauri 通道）拉进来，故挡掉；
// 需要「详情渲染器」在场的用例改用下面登记的桩（见 RendererStub）。
vi.mock('@/registry/vdfsRenderers', () => ({}))

import VdfsWorkbench from '../VdfsWorkbench.vue'
import DetailShell from '../DetailShell.vue'
import { registerVdfsRenderer } from '@/registry/vdfsTypes'
import type { DetailAction, VdfsNewType, VdfsNode } from '@/schemas/vdfs'

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
  emits: ['action'],
  setup(props) {
    return () =>
      h(DetailShell, { actions: props.mechanismActions as DetailAction[] })
  },
})
// `session` 是本文件节点（`makeNode`）的渲染器键（ext → renderer 由 vdfsTypes 解析）
registerVdfsRenderer('session', RendererStub)

/** 配置型：呈现 ext 是 skill、落成后节点 ext 是 form（详情是定义驱动表单） */
const SKILL: VdfsNewType = { ext: 'skill', title: '技能', node_ext: 'form' }

function makeNode(name: string): VdfsNode {
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

/** `installStub` 装上的两个可写 ref（换目录的用例要在挂载后改它们） */
let stubRefs: { cwd: Ref<string>; cwdNode: Ref<VdfsNode | null> } | null = null

/** 可控的 `useVdfs` 桩：只填本文件关心的那几个键，其余给空值 */
function installStub(
  opts: {
    newType?: VdfsNewType | null
    selected?: VdfsNode | null
    items?: VdfsNode[]
    /** 当前目录地址（`@vfs` / `@vfs/session`…）；默认 `@vfs`（资源根） */
    cwd?: string
    /** 当前目录**节点**；`new_type.ext = session` 时控件会声明「当前空间」 */
    cwdNode?: VdfsNode | null
  } = {}
) {
  const selectedNode = ref<VdfsNode | null>(opts.selected ?? null)
  const items = ref<VdfsNode[]>(opts.items ?? [])
  // 这两个**可写**：换目录的用例要在挂载后改它们（真实现里 `cwd` 立刻变、
  // `cwdNode` 要等 `refresh()` 回来，两者的时间差正是那条滞后 bug 的来源）
  const cwd = ref(opts.cwd ?? '@vfs')
  const cwdNode = shallowRef<VdfsNode | null>(opts.cwdNode ?? null)
  stubRefs = { cwd, cwdNode }
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
    navItems: computed(() => []),
    selectDir: vi.fn(),
    selectedName: ref<string | null>(null),
    cwd,
    // 当前目录节点：`new_type.ext = session` 时控件会声明「当前空间」
    // （会话挂载目录）并自动开一份新会话草稿（见控件内的 autoDraft）。
    // 默认给 null（不可新建的目录），本文件多数用例测的是新建入口与动作转发。
    cwdNode: computed<VdfsNode | null>(() => cwdNode.value),
    title: computed(() => '资源'),
    items,
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
    loading: ref(false),
    loadError: ref(''),
    hasMore: ref(false),
    loadingMore: ref(false),
    refresh: vi.fn(),
    loadMore: vi.fn(),
    select: vi.fn(),
    selectedNode,
    selectedId: computed(() => selectedNode.value?.path ?? null),
    renderer: computed(() => 'form'),
    nodeText: ref(''),
    formData: ref(null),
    loadingDetail: ref(false),
    saving: ref(false),
    actionBusy: ref(false),
    detailError: ref(''),
    fieldErrors: ref([]),
    // 机制动作（当前只有「删除」）由页面算好注入渲染器；可写节点才有它
    mechanismActions: computed<DetailAction[]>(() =>
      opts.selected ? [{ id: 'delete', label: '删除', style: 'secondary' }] : []
    ),
    mechanismBusyId: ref<string | null>(null),
    saveFields: vi.fn(),
    saveText: vi.fn(),
    removeSelected: vi.fn(),
    // 可新建类型**来自当前目录节点的自述**（前端不做任何推导）——与真实现同源：
    // `useVdfs.creatableType = cwdNode.new_type`。桩里必须用同一个来源，
    // 否则「进入会话空间自动开草稿」那条链会因为桩与真实现不同源而测不出来。
    creatableType: computed(() => opts.newType ?? cwdNode.value?.new_type ?? undefined),
    canCreate: computed(() => Boolean(opts.newType ?? cwdNode.value?.new_type)),
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
async function emitAction(w: VueWrapper, id: string, file?: File) {
  w.findComponent({ name: 'RendererStub' }).vm.$emit('action', id, file)
  await w.vm.$nextTick()
}

beforeEach(() => {
  Object.values(hoisted.fns).forEach((f) => f.mockClear())
  hoisted.newTypes.length = 0
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

  it('未声明可新建类型 ⇒ 没有按钮（机制只认节点声明，不做任何回退）', () => {
    installStub({ newType: null })
    expect(newButton(bench())).toBeUndefined()
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

  it('带 File 的上抛 ⇒ runPackAction（如「导入整包」）', async () => {
    installStub({ selected: makeNode('s1') })
    const w = bench()
    const file = new File(['PK'], 'demo.zip', { type: 'application/zip' })
    await emitAction(w, 'import', file)

    // 动作名与文件原样转交：控件不认识「导入」，只认「这个动作的载荷是文件」
    expect(hoisted.fns.runPackAction).toHaveBeenCalledWith('import', file)
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
 * 进入一个**可新建**的目录时自动备好一张草稿。
 *
 * 它是「列表为空 ⇒ 直接显示详情（新建）、中栏收起」这条机制的**上游**：收栏判据
 * 在容器里（`common/Workbench.vue` 的 `showList`），但容器自己不开草稿，草稿由
 * 这里开。所以这一段不对，MCP / 技能页就永远收不了栏——那正是用户报的现象。
 *
 * ⚠️ 判据**必须与类型无关**。曾经它写的是 `creatableType.ext === 'session'`，
 * 于是只有会话页有草稿、其余类型一律没有：一份写死在前端的类型名单，后端每加
 * 一类资源都要回来补。下面的第 2、3 条是这条教训的验收——把那个 `ext` 判断加
 * 回去，它们会红，那正是它们存在的理由。
 *
 * 正确的判据只有一条：**目录自己声明了可新建什么**（`cwdNode.new_type`）。
 * 其余两条边界与类型无关，缺一条就会帮倒忙（都会静默走样：不报错，只是草稿
 * 出现在不该出现的地方 / 覆盖用户的选择 / 删了又"复活"）：
 *
 * 1. 只在**用户还没选中任何东西**时做——覆盖深链进来的选中项等于抢用户的选择；
 * 2. **每个目录只自动开一次**——否则清空选中 / 删掉刚选中的项之后草稿会复活；
 * 3. **目录没声明 `new_type`** ⇒ 不可新建 ⇒ 不开（由后端声明，不由类型名决定）。
 */
describe('VdfsWorkbench 进入可新建目录时自动备好草稿', () => {
  const SESSION_TYPE: VdfsNewType = { ext: 'session', title: '会话' }
  /** 会话挂载目录节点：`new_type.ext = session` ⇒ 控件据此认定「当前空间」 */
  const mountNode: VdfsNode = {
    path: '@vfs/session',
    name: 'session',
    title: '会话',
    kind: 'dir',
    status: 'active',
    access: 'rw',
    ext: 'dir',
    new_type: SESSION_TYPE,
  }

  it('落在会话目录且未选中任何项 ⇒ 自动进入新建态', () => {
    installStub({ cwd: '@vfs/session', cwdNode: mountNode })
    bench()

    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
  })

  it('★ 模型 / MCP / 技能目录**同样**自动开（曾写死成只认 session）', () => {
    installStub({
      cwd: '@vfs/mcp',
      cwdNode: { ...mountNode, path: '@vfs/mcp', name: 'mcp', new_type: { ext: 'mcp', title: 'MCP' } },
    })
    bench()

    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)
  })

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

  it('★ 换目录的那一拍不得拿**上一栏**的目录自述开草稿（否则上一类的详情页会开到这一栏）', async () => {
    // 用户报的现象：模型页显示智能体详情、会话页显示模型详情——**一步滞后**。
    // 成因：换目录时地址（`cwd`）立刻变，而目录自述（`cwdNode`）要等列表回来才更新，
    // 中间那一拍 `creatableType` 仍是上一栏的类型。故监听源必须是目录自述。
    const agent: VdfsNode = {
      ...mountNode,
      path: '@vfs/agent',
      name: 'agent',
      new_type: { ext: 'agent', title: '智能体' },
    }
    const model: VdfsNode = {
      ...mountNode,
      path: '@vfs/model',
      name: 'model',
      new_type: { ext: 'model', title: '模型' },
    }
    installStub({ cwd: '@vfs/agent', cwdNode: agent })
    bench()
    expect(hoisted.newTypes).toEqual(['agent'])

    // 切到模型：地址先变，自述还没回来 —— 这一拍里绝不能开（开了就是 agent 的详情）
    stubRefs!.cwd.value = '@vfs/model'
    await nextTick()
    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(1)

    // 自述回来 ⇒ 开的是**模型**的草稿
    stubRefs!.cwdNode.value = model
    await nextTick()
    expect(hoisted.fns.startNew).toHaveBeenCalledTimes(2)
    expect(hoisted.newTypes).toEqual(['agent', 'model'])
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
