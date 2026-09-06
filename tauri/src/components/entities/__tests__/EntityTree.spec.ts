/**
 * EntityTree — 树视图机制单测（happy-dom）
 *
 * tree 机制 =「层级（parent）+ 懒加载（parent 请求参数）+ 选择」：
 * - 根层经 entities/list（parent 缺省）加载；
 * - 展开可展开节点懒加载下一层（每层只请求一次）；
 * - 节点点击上抛 select(id)，详情流由容器页机制承接。
 */
// @vitest-environment happy-dom
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'

const { listEntitiesMock, subscribeMock } = vi.hoisted(() => ({
  listEntitiesMock: vi.fn(),
  subscribeMock: vi.fn(() => () => {}),
}))
vi.mock('@/services/entities', () => ({
  listEntities: listEntitiesMock,
  watchEntity: vi.fn(),
  unwatchEntity: vi.fn(),
}))
vi.mock('@/services/eventBus', () => ({
  subscribe: subscribeMock,
}))

import EntityTree from '../EntityTree.vue'
import type { EntitySummary } from '@/schemas/entities'

function node(id: string, parent: string | undefined, expandable: boolean): EntitySummary {
  return {
    kind: 'dir',
    id,
    name: id.split('/').pop() ?? id,
    status: 'active',
    parent,
    expandable,
  }
}

const ROOT = [
  node('src', undefined, true),
  node('README.md', undefined, false),
]

describe('EntityTree：层级 + 懒加载 + 选择', () => {
  beforeEach(() => {
    listEntitiesMock.mockReset()
    listEntitiesMock.mockImplementation(async (_kind: string, opts?: { parent?: string }) => {
      if (!opts?.parent) return { kind: 'dir', items: ROOT, capabilities: {} as never }
      if (opts.parent === 'src')
        return {
          kind: 'dir',
          items: [node('src/lib.rs', 'src', false)],
          capabilities: {} as never,
        }
      return { kind: 'dir', items: [], capabilities: {} as never }
    })
  })

  it('根层加载后按 parent 派生层级', async () => {
    const w = mount(EntityTree, {
      props: { containerKind: 'session', containerId: 's1', subKind: 'dir' },
    })
    await flushPromises()
    expect(listEntitiesMock).toHaveBeenCalledWith('session', {
      container: 's1',
      subKind: 'dir',
      parent: undefined,
    })
    const rows = w.findAll('.node-row')
    expect(rows.map((r) => r.text().replace('▸', '').trim())).toEqual(['src', 'README.md'])
  })

  it('展开可展开节点懒加载下一层（每层仅请求一次）', async () => {
    const w = mount(EntityTree, {
      props: { containerKind: 'session', containerId: 's1', subKind: 'dir' },
    })
    await flushPromises()

    await w.find('.node-row').trigger('click')
    await flushPromises()
    // 子层级出现，缩进渲染
    const texts = w.findAll('.node-row').map((r) => r.text().replace('▸', '').trim())
    expect(texts).toEqual(['src', 'lib.rs', 'README.md'])

    // 收起再展开：不再发起第二次请求
    await w.find('.node-row').trigger('click')
    await w.find('.node-row').trigger('click')
    await flushPromises()
    expect(listEntitiesMock).toHaveBeenCalledTimes(2) // 根层 + src 层
  })

  it('叶子节点（expandable = false）不可展开，点击仅上抛选择', async () => {
    const w = mount(EntityTree, {
      props: { containerKind: 'session', containerId: 's1', subKind: 'dir' },
    })
    await flushPromises()
    await w.findAll('.node-row')[1].trigger('click')
    await flushPromises()
    expect(w.emitted('select')![0]).toEqual(['README.md'])
    expect(listEntitiesMock).toHaveBeenCalledTimes(1) // 无下层请求
  })
})
