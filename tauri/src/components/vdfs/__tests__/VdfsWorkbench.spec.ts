/**
 * VdfsWorkbench — **提示态状态机**单测（happy-dom）
 *
 * 只测这一件事：详情槽里「选类型 / 选文件 / 重命名」三个瞬态的开合与路由。
 * 它们是本轮机制化的对象——此前是两个布尔（`creatingTyped` / `renaming`）加三个
 * 载荷 ref，互斥靠两个 `startXxx` 各自把对方复位来维持；现在是一个判别式
 * （`promptKind`）加三个载荷。互斥是**结构性的**，所以值得有断言钉住。
 *
 * 顺带钉住一条对外约定：类型清单**不再显示 `ext`**。`ext`（`session` / `form`）
 * 是渲染器键、机制细节——与「列表徽标只给目录、不给文件 ext」是同一条约定，
 * 靠断言挡住它被加回来。
 *
 * 环境说明：`useVdfs` 被整体替换为可控桩（它经 `services/vdfs` 出站），
 * 渲染器登记表也被挡掉（那会把整条会话渲染链连同 Tauri 通道拉进来）。
 */

// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { computed, defineComponent, h, ref } from 'vue'

const hoisted = vi.hoisted(() => {
  const fns = { startNew: vi.fn(), createTypedFile: vi.fn(), renameSelected: vi.fn() }
  return { stub: {} as Record<string, unknown>, fns }
})

vi.mock('@/composables/useVdfs', () => ({ useVdfs: () => hoisted.stub }))
// 真实的渲染器登记表会把整条会话渲染链（连同 Tauri 通道）拉进来，故挡掉；
// 需要「详情渲染器」在场的用例改用下面登记的桩（见 RendererStub）。
vi.mock('@/registry/vdfsRenderers', () => ({}))

import VdfsWorkbench from '../VdfsWorkbench.vue'
import DetailShell from '../DetailShell.vue'
import { registerVdfsRenderer } from '@/registry/vdfsTypes'
import type { DetailAction, VdfsNewType, VdfsNode } from '@/schemas/vdfs'

/**
 * 详情渲染器桩：**机制动作（重命名 / 删除）由页面注入渲染器**，渲染器只管转呈
 * （见 `useVdfs.mechanismActions` → `DetailShell`）。所以「点重命名」这条入口
 * 只能在渲染器挂载时才存在——桩据此把它接出来，好让「进入重命名提示」这条
 * 真实通道可被触发（而不是去调组件内部函数）。
 */
const RendererStub = defineComponent({
  name: 'RendererStub',
  props: { mechanismActions: { type: Array, default: () => [] } },
  emits: ['rename'],
  setup(props, { emit }) {
    return () =>
      h(DetailShell, {
        actions: props.mechanismActions as DetailAction[],
        onRun: () => emit('rename'),
      })
  },
})
// `session` 是本文件节点（`makeNode`）的渲染器键（ext → renderer 由 vdfsTypes 解析）
registerVdfsRenderer('session', RendererStub)

const SESSION: VdfsNewType = { ext: 'session', title: '会话' }
const MODEL: VdfsNewType = { ext: 'model', title: '模型' }
/** `source = file` 的类型：选文件即完成，不进详情页 */
const BUNDLE: VdfsNewType = { ext: 'zip', title: '整包', source: 'file' }

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
function installStub(opts: { types?: VdfsNewType[]; selected?: VdfsNode | null } = {}) {
  const types = opts.types ?? []
  const selectedNode = ref<VdfsNode | null>(opts.selected ?? null)
  hoisted.stub = {
    navItems: computed(() => []),
    selectDir: vi.fn(),
    selectedName: ref<string | null>(null),
    cwd: ref('@vfs'),
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
    // 机制动作（重命名 / 删除）由页面算好注入渲染器；可写节点才有重命名
    mechanismActions: computed<DetailAction[]>(() =>
      opts.selected ? [{ id: 'rename', label: '重命名', style: 'secondary' }] : []
    ),
    mechanismBusyId: ref<string | null>(null),
    saveFields: vi.fn(),
    saveText: vi.fn(),
    runAction: vi.fn(),
    removeSelected: vi.fn(),
    creatableTypes: computed(() => types),
    canCreate: computed(() => types.length > 0),
    draftSeq: ref(0),
    ...hoisted.fns,
  }
}

/** 按 `aria-label`（`VdfsActions` 对文字/图标按钮一律写入）或可见文案点按钮 */
async function click(w: VueWrapper, label: string) {
  const btn = w
    .findAll('button')
    .find((b) => b.attributes('aria-label') === label || b.text() === label)
  if (!btn) {
    throw new Error(`找不到按钮「${label}」；现有：${w.findAll('button').map((b) => b.text() || b.attributes('aria-label')).join(' | ')}`)
  }
  await btn.trigger('click')
}

/** 提示态动作行的可见文案（按渲染顺序） */
function actionLabels(w: VueWrapper): string[] {
  return w.findAll('.head-actions button').map((b) => b.attributes('aria-label') ?? '')
}

function bench() {
  return mount(VdfsWorkbench, { props: { addr: '@vfs' } })
}

/** header 上的「新建」按钮（title 随类型数变化） */
async function clickNew(w: VueWrapper) {
  const btn = w.findAll('button').find((b) => (b.attributes('title') ?? '').startsWith('新建'))
  if (!btn) throw new Error('找不到「新建」按钮')
  await btn.trigger('click')
}

beforeEach(() => {
  Object.values(hoisted.fns).forEach((f) => f.mockClear())
})

describe('VdfsWorkbench 提示态：选类型', () => {
  it('多于一种类型时点「新建」进提示态：类型清单**就是**动作行，且不显示 ext', async () => {
    installStub({ types: [SESSION, MODEL] })
    const w = bench()
    await clickNew(w)

    // 类型清单与「取消」同处一行——没有另一套列表渲染
    expect(actionLabels(w)).toEqual(['会话', '模型', '取消'])
    // `ext` 是渲染器键、机制细节，不得摆给用户看
    expect(w.text()).not.toContain('session')
    expect(w.text()).not.toContain('model')
  })

  it('选非 file 类型 ⇒ 收起提示并进入该类型的详情页（startNew）', async () => {
    installStub({ types: [SESSION, MODEL] })
    const w = bench()
    await clickNew(w)
    await click(w, '会话')

    expect(hoisted.fns.startNew).toHaveBeenCalledWith(SESSION)
    // 提示态已收起：动作行不复存在（详情槽交回渲染器）
    expect(w.find('.head-actions').exists()).toBe(false)
  })

  it('恰好一种类型 ⇒ 点「新建」跳过类型选择，直接落到那一条路', async () => {
    installStub({ types: [SESSION] })
    const w = bench()
    await clickNew(w)

    expect(actionLabels(w)).toEqual([]) // 没有类型动作行
    expect(hoisted.fns.startNew).toHaveBeenCalledWith(SESSION)
  })
})

describe('VdfsWorkbench 提示态：从本地文件新建', () => {
  it('选中 file 类型 ⇒ 进文件提示态：导入（未选文件时禁用）/ 上一步 / 取消', async () => {
    installStub({ types: [SESSION, BUNDLE] })
    const w = bench()
    await clickNew(w)
    await click(w, '整包')

    expect(actionLabels(w)).toEqual(['导入', '上一步', '取消'])
    const importBtn = w.findAll('.head-actions button').find((b) => b.attributes('aria-label') === '导入')
    expect(importBtn?.attributes('disabled'), '未选文件时导入不可点').toBeDefined()
  })

  it('恰好一种类型且是 file 类型 ⇒ 点「新建」直达文件提示态（无类型选择这一步）', async () => {
    installStub({ types: [BUNDLE] })
    const w = bench()
    await clickNew(w)

    expect(actionLabels(w)).toEqual(['导入', '取消']) // 单类型故无「上一步」
  })

  it('「上一步」回到类型选择；「取消」收起提示', async () => {
    installStub({ types: [SESSION, BUNDLE] })
    const w = bench()
    await clickNew(w)
    await click(w, '整包')
    expect(actionLabels(w)).toEqual(['导入', '上一步', '取消'])

    await click(w, '上一步')
    expect(actionLabels(w)).toEqual(['会话', '整包', '取消'])

    await click(w, '取消')
    expect(w.find('.head-actions').exists()).toBe(false)
  })
})

describe('VdfsWorkbench 提示态：重命名', () => {
  it('选中项可写 ⇒ 重命名提示预填原名，确定即调 renameSelected', async () => {
    installStub({ selected: makeNode('s1'), types: [SESSION] })
    hoisted.fns.renameSelected.mockResolvedValue(true)
    const w = bench()

    await click(w, '重命名')
    const input = w.find('input.prompt-input')
    expect((input.element as HTMLInputElement).value).toBe('s1')

    await input.setValue('s1-改')
    await click(w, '确定')
    expect(hoisted.fns.renameSelected).toHaveBeenCalledWith('s1-改')
    // 提示已收起（详情槽交回渲染器）：重命名输入框不复存在
    expect(w.find('input.prompt-input').exists()).toBe(false)
  })
})

describe('VdfsWorkbench 提示态：互斥', () => {
  it('重命名提示开着时点「新建」⇒ 换成类型清单，不叠两个提示', async () => {
    installStub({ selected: makeNode('s1'), types: [SESSION, MODEL] })
    const w = bench()

    await click(w, '重命名')
    expect(actionLabels(w)).toEqual(['确定', '取消'])

    // 「新建」入口恒在（header），故这是一条**可达**的两个提示互换路径：
    // 判别式只可能停在其中一个，界面不会同时出现两套提示外壳。
    await clickNew(w)
    expect(actionLabels(w)).toEqual(['会话', '模型', '取消'])
  })

  it('提示态占用详情槽时，详情渲染器不挂载（同槽只有一个主人）', async () => {
    installStub({ selected: makeNode('s1'), types: [SESSION] })
    const w = bench()

    // 未开提示：详情渲染器在场（重命名入口可见）
    expect(w.find('button[aria-label="重命名"]').exists()).toBe(true)

    await click(w, '重命名')
    // 开提示：渲染器让位——它的动作（含重命名）随之消失
    expect(w.find('button[aria-label="重命名"]').exists()).toBe(false)
  })
})
