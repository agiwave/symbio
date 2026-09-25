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
import { computed, defineComponent, h, ref } from 'vue'

const hoisted = vi.hoisted(() => {
  const fns = { startNew: vi.fn(), runAction: vi.fn(), runPackAction: vi.fn() }
  return { stub: {} as Record<string, unknown>, fns, sessions: { setSessionSpace: vi.fn() } }
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

/** 可控的 `useVdfs` 桩：只填本文件关心的那几个键，其余给空值 */
function installStub(opts: { newType?: VdfsNewType | null; selected?: VdfsNode | null } = {}) {
  const selectedNode = ref<VdfsNode | null>(opts.selected ?? null)
  hoisted.stub = {
    navItems: computed(() => []),
    selectDir: vi.fn(),
    selectedName: ref<string | null>(null),
    cwd: ref('@vfs'),
    // 当前目录节点：`new_type.ext = session` 时控件会声明「当前空间」
    // （会话挂载目录），故桩里给它一个「不可新建」的目录节点——本文件测的是
    // 新建入口与动作转发，空间声明不参与。
    cwdNode: computed<VdfsNode | null>(() => null),
    title: computed(() => '资源'),
    items: ref<VdfsNode[]>([]),
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
    // 可新建类型直接来自当前目录节点的自述（前端不做任何推导）
    creatableType: computed(() => opts.newType ?? undefined),
    canCreate: computed(() => Boolean(opts.newType)),
    draftSeq: ref(0),
    ...hoisted.fns,
  }
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
