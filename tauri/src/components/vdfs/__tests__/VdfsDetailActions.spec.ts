/**
 * 三个渲染器的**动作装配**护栏（happy-dom）
 *
 * P2/P3 把「机制动作」从各渲染器自算改成页面单点计算、`DetailShell` 统一合并
 * （`mergeDetailActions`）。合并规则本身由 `schemas/__tests__/vdfs-form.spec.ts`
 * 覆盖；这里锁的是**每个渲染器声明的自有动作**与**注入动作的落位**。
 *
 * 为什么值得单独一测：规则收敛之后，最容易出的两种错都是「点下去才发现不对」——
 * 某个渲染器**又把机制动作自己拼了一遍**（同页两个删除按钮），或者**把注入项吞掉**
 * （删除整段消失）。两侧都断言，两个方向就都关上了。
 */
// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import VdfsTextDetail from '../VdfsTextDetail.vue'
import VdfsReadonlyDetail from '../VdfsReadonlyDetail.vue'
import VdfsSessionDetail from '../VdfsSessionDetail.vue'
import type { DetailAction, VdfsItem } from '@/schemas/vdfs'

/** 一个可写资源节点 */
function node(partial: Partial<VdfsItem> = {}): VdfsItem {
  return {
    path: '@vfs/model/p1',
    name: 'p1',
    title: 'P1',
    kind: 'model',
    status: 'active',
    access: 'rw',
    ...partial,
  }
}

/** 页面注入的机制动作（任何已落盘可写节点都有：删除） */
const MECHANISM: DetailAction[] = [
  { id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' },
]

/** 动作区按钮的 tooltip 序列（图标按钮的 tooltip 就是动作名） */
function titles(w: ReturnType<typeof mount>): (string | undefined)[] {
  return w.findAll('.ea-btn').map((b) => b.attributes('title'))
}

describe('VdfsTextDetail 动作装配', () => {
  function mountText(mechanism: DetailAction[] = MECHANISM) {
    return mount(VdfsTextDetail, {
      props: { node: node(), data: '# hi', mechanismActions: mechanism },
      // 编辑器与本测无关：只关心动作区
      global: { stubs: { CodeEditor: true } },
    })
  }

  it('自有动作（保存 / 还原）在前，机制动作经 divider 在后', () => {
    const w = mountText()
    expect(titles(w)).toEqual(['保存', '还原', '删除'])
    expect(w.find('.ea-divider').exists()).toBe(true)
  })

  it('机制动作 delete 原样上抛给页面（渲染器不自行处理）', async () => {
    const w = mountText()
    await w.find('button[title="删除"]').trigger('click')
    expect(w.emitted('delete')).toHaveLength(1)
    // 渲染器没有 save 被误触（自有动作未被机制动作带跑）
    expect(w.emitted('save')).toBeUndefined()
  })

  it('页面未注入机制动作时只剩自有动作（只读 / 草稿 / 系统地址的形态）', () => {
    const w = mountText([])
    expect(titles(w)).toEqual(['保存', '还原'])
    expect(w.find('.ea-divider').exists()).toBe(false)
  })
})

describe('VdfsReadonlyDetail 动作装配', () => {
  it('无自有动作：动作区完全由机制注入构成，且不插 divider', () => {
    const w = mount(VdfsReadonlyDetail, {
      props: { node: node(), mechanismActions: MECHANISM },
    })
    expect(titles(w)).toEqual(['删除'])
    expect(w.find('.ea-divider').exists()).toBe(false)
  })

  it('机制动作 delete 原样上抛给页面', async () => {
    const w = mount(VdfsReadonlyDetail, {
      props: { node: node(), mechanismActions: MECHANISM },
    })
    await w.find('button[title="删除"]').trigger('click')
    expect(w.emitted('delete')).toHaveLength(1)
  })

  it('未注入机制动作时动作区为空（外部只读文件：无事可做）', () => {
    const w = mount(VdfsReadonlyDetail, { props: { node: node() } })
    expect(w.findAll('.ea-btn')).toHaveLength(0)
  })
})

describe('VdfsSessionDetail 动作装配', () => {
  /** Session 的子图很重（聊天工作区）：用桩只保留 props 通道 */
  const SessionStub = {
    name: 'Session',
    props: ['node', 'actions', 'mechanismActions', 'mechanismBusy', 'saving'],
    template: '<div class="session-stub" />',
  }

  function mountSession(target: VdfsItem, mechanism: DetailAction[] = MECHANISM) {
    return mount(VdfsSessionDetail, {
      props: { node: target, mechanismActions: mechanism, mechanismBusy: 'delete' },
      global: { stubs: { Session: SessionStub } },
    })
  }

  it('自有动作只有「浏览内部」；草稿（新建态）没有内部可浏览，故不给', () => {
    const w = mountSession(node({ kind: 'session', ext: 'session' }))
    const actions = w.findComponent(SessionStub).props('actions') as DetailAction[]
    expect(actions.map((a) => a.id)).toEqual(['open-container'])
    expect(actions[0].payload).toEqual({ kind: 'session' })

    // 草稿没有地址 ⇒ 没有「同名目录」可进入
    const draft = mountSession(node({ path: '', name: '', kind: 'session', ext: 'session' }))
    expect(draft.findComponent(SessionStub).props('actions')).toEqual([])
  })

  it('机制动作与忙态原样下传给 Session（渲染器不自算、不改写）', () => {
    const w = mountSession(node({ kind: 'session', ext: 'session' }))
    const s = w.findComponent(SessionStub)
    expect(s.props('mechanismActions')).toEqual(MECHANISM)
    expect(s.props('mechanismBusy')).toBe('delete')
  })
})
