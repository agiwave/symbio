// @vitest-environment happy-dom
/**
 * VdfsTreeNode —— 中栏目录树的**展开态默认值**
 *
 * 工作目录动辄成百上千项：默认全展开时第一屏几乎全是深层文件，顶层结构反而看不见，
 * 而树存在的理由正是「先看结构再往下走」。因此默认**收拢**——点开才展开。
 *
 * 唯一的例外是「当前选中项就在这棵子树里」（重新挂载 / 换目录回来时的到达态）：
 * 那时把沿途展开，否则选中的文件藏在收起的目录里，用户会以为选择丢了。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import VdfsTreeNode from '../VdfsTreeNode.vue'
import type { VdfsItem, VdfsTreeNode as TreeNode } from '@/schemas/vdfs'

function dir(path: string, name: string): VdfsItem {
  return { path, name, title: name, kind: 'dir', status: 'active', access: 'lt' }
}
function file(path: string, name: string): VdfsItem {
  return { path, name, title: name, kind: 'file', status: 'active', access: 'rw', ext: 'text' }
}

/** `root/` ├─ `a.txt` └─ `sub/` └─ `b.txt` */
function tree(): TreeNode {
  const sub: TreeNode = {
    item: dir('@vfs/wd/sub', 'sub'),
    children: [{ item: file('@vfs/wd/sub/b.txt', 'b.txt'), children: [] }],
  }
  return {
    item: dir('@vfs/wd', 'wd'),
    children: [{ item: file('@vfs/wd/a.txt', 'a.txt'), children: [] }, sub],
  }
}

function mountNode(selectedId: string | null = null) {
  return mount(VdfsTreeNode, { props: { node: tree(), selectedId } })
}

describe('VdfsTreeNode：默认收拢', () => {
  it('挂载时子层不渲染（第一屏只有顶层结构）', () => {
    const w = mountNode()

    expect(w.text()).toContain('wd')
    expect(w.text(), '收起的目录里不渲染子项').not.toContain('a.txt')
    expect(w.get('[role="button"]').attributes('aria-expanded')).toBe('false')
  })

  it('点目录才展开，再点收起', async () => {
    const w = mountNode()
    const row = w.get('[role="button"]')

    await row.trigger('click')
    expect(w.text()).toContain('a.txt')
    // 逐层展开：下一层目录（`sub`）仍是收着的——「默认全展开」正是要去掉的那件事
    expect(w.text()).not.toContain('b.txt')
    expect(row.attributes('aria-expanded')).toBe('true')

    const subRow = w.findAll('[role="button"]').find((r) => r.text() === 'sub')!
    await subRow.trigger('click')
    expect(w.text(), '展开下一层才看得到它的子项').toContain('b.txt')

    await row.trigger('click')
    expect(w.text()).not.toContain('a.txt')
  })

  it('选中项在这棵子树里 ⇒ 到达态就展开（否则选中项藏在收起的目录里）', () => {
    const w = mountNode('@vfs/wd/a.txt')

    expect(w.text()).toContain('a.txt')
    expect(w.get('[role="button"]').attributes('aria-expanded')).toBe('true')
  })

  it('文件行没有展开箭头，点击上抛 activate（交给父组件选中）', async () => {
    const w = mountNode()
    await w.get('[role="button"]').trigger('click') // 先展开

    const fileRow = w.findAll('[role="button"]').find((r) => r.text() === 'a.txt')!
    expect(fileRow.attributes('aria-expanded')).toBeUndefined()
    await fileRow.trigger('click')

    expect(w.emitted('activate')?.[0]).toEqual([expect.objectContaining({ path: '@vfs/wd/a.txt' })])
  })
})
