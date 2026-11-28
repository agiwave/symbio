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
import { afterEach, describe, expect, it } from 'vitest'
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

function mountNode(
  selectedId: string | null = null,
  extra: { tabStop?: boolean; focusPath?: string; level?: number; node?: TreeNode } = {},
) {
  return mount(VdfsTreeNode, {
    props: { node: extra.node ?? tree(), selectedId, ...extra },
    attachTo: document.body,
  })
}

afterEach(() => {
  document.body.replaceChildren()
})

describe('VdfsTreeNode：默认收拢', () => {
  it('挂载时子层不渲染（第一屏只有顶层结构）', () => {
    const w = mountNode()

    expect(w.text()).toContain('wd')
    expect(w.text(), '收起的目录里不渲染子项').not.toContain('a.txt')
    expect(w.get('[role="treeitem"]').attributes('aria-expanded')).toBe('false')
  })

  it('点目录才展开，再点收起', async () => {
    const w = mountNode()
    const row = w.get('[role="treeitem"]')

    await row.find('.tree-row').trigger('click')
    expect(w.text()).toContain('a.txt')
    // 逐层展开：下一层目录（`sub`）仍是收着的——「默认全展开」正是要去掉的那件事
    expect(w.text()).not.toContain('b.txt')
    expect(row.attributes('aria-expanded')).toBe('true')

    const subRow = w.findAll('[role="treeitem"]').find((r) => r.text() === 'sub')!
    await subRow.find('.tree-row').trigger('click')
    expect(w.text(), '展开下一层才看得到它的子项').toContain('b.txt')

    await row.find('.tree-row').trigger('click')
    expect(w.text()).not.toContain('a.txt')
  })

  it('选中项在这棵子树里 ⇒ 到达态就展开（否则选中项藏在收起的目录里）', () => {
    const w = mountNode('@vfs/wd/a.txt')

    expect(w.text()).toContain('a.txt')
    expect(w.get('[role="treeitem"]').attributes('aria-expanded')).toBe('true')
  })

  it('文件行没有展开箭头，点击上抛 activate（交给父组件选中）', async () => {
    const w = mountNode()
    await w.get('[role="treeitem"]').find('.tree-row').trigger('click') // 先展开

    const fileRow = w.findAll('[role="treeitem"]').find((r) => r.text() === 'a.txt')!
    expect(fileRow.attributes('aria-expanded')).toBeUndefined()
    await fileRow.find('.tree-row').trigger('click')

    expect(w.emitted('activate')?.[0]).toEqual([expect.objectContaining({ path: '@vfs/wd/a.txt' })])
  })

  it('treeitem 暴露层级与选中态，子层使用 group 语义', async () => {
    const w = mountNode('@vfs/wd/a.txt')
    const root = w.get('[role="treeitem"]')
    expect(root.attributes('aria-level')).toBe('1')
    expect(root.attributes('aria-selected')).toBe('false')
    expect(root.attributes('tabindex')).toBe('0')

    const children = w.get('[role="group"]').findAll('[role="treeitem"]')
    expect(children.map((row) => row.attributes('aria-level'))).toEqual(['2', '2'])
    expect(children[0].attributes('aria-selected')).toBe('true')
  })

  it('父级传入 focusPath 后，唯一 Tab 停靠点随 roving focus 下沉', async () => {
    const w = mountNode(null, { tabStop: false, focusPath: '@vfs/wd/a.txt' })
    const root = w.get('[role="treeitem"]')
    expect(root.attributes('tabindex')).toBe('-1')
    await root.find('.tree-row').trigger('click')

    const children = w.get('[role="group"]').findAll('[role="treeitem"]')
    expect(children.map((row) => row.attributes('tabindex'))).toEqual(['0', '-1'])
  })

  it('ArrowRight 展开目录，再按一次进入第一个子项', async () => {
    const w = mountNode()
    const root = w.get('[role="treeitem"]')
    ;(root.element as HTMLElement).focus()

    await root.trigger('keydown', { key: 'ArrowRight' })
    expect(root.attributes('aria-expanded')).toBe('true')
    expect(document.activeElement).toBe(root.element)

    await root.trigger('keydown', { key: 'ArrowRight' })
    const firstChild = w.get('[role="group"]').find('[role="treeitem"]')
    expect(document.activeElement).toBe(firstChild.element)
  })

  it('ArrowLeft 从叶子回到父目录，再按一次折叠父目录', async () => {
    const w = mountNode('@vfs/wd/a.txt')
    const root = w.get('[role="treeitem"]')
    const child = w.get('[role="group"]').find('[role="treeitem"]')
    ;(child.element as HTMLElement).focus()

    await child.trigger('keydown', { key: 'ArrowLeft' })
    expect(document.activeElement).toBe(root.element)
    expect(root.attributes('aria-expanded')).toBe('true')

    await root.trigger('keydown', { key: 'ArrowLeft' })
    expect(root.attributes('aria-expanded')).toBe('false')
    expect(w.find('[role="group"]').exists()).toBe(false)
  })

  it('Enter / Space 仍等价于点击：目录展开，文件上抛激活', async () => {
    const w = mountNode()
    const root = w.get('[role="treeitem"]')
    await root.trigger('keydown', { key: 'Enter' })
    expect(root.attributes('aria-expanded')).toBe('true')

    const fileRow = w.findAll('[role="treeitem"]').find((row) => row.text() === 'a.txt')!
    await fileRow.trigger('keydown', { key: 'Enter' })
    await fileRow.trigger('keydown', { key: ' ' })
    expect(w.emitted('activate')).toHaveLength(2)
  })
})

describe('VdfsTreeNode：子层按需加载', () => {
  function lazyTree(): TreeNode {
    return { item: dir('@vfs/wd', 'wd'), children: [], loaded: false }
  }

  it('首次展开请求直接子项；加载后收起再展开不重复请求', async () => {
    const lazy = lazyTree()
    const w = mountNode(null, { node: lazy })
    const root = w.get('[role="treeitem"]')

    await root.find('.tree-row').trigger('click')
    expect(root.attributes('aria-expanded')).toBe('true')
    expect(w.emitted('load-children')).toEqual([['@vfs/wd']])

    await w.setProps({
      node: {
        ...lazy,
        loading: true,
      },
    })
    expect(root.attributes('aria-busy')).toBe('true')
    expect(w.get('.tree-loading').text()).toContain('加载中')

    await w.setProps({
      node: {
        ...lazy,
        loaded: true,
        children: [{ item: file('@vfs/wd/a.txt', 'a.txt'), children: [] }],
      },
    })
    expect(w.text()).toContain('a.txt')
    expect(w.find('.tree-loading').exists()).toBe(false)

    await root.find('.tree-row').trigger('click')
    expect(w.text()).not.toContain('a.txt')
    await root.find('.tree-row').trigger('click')
    expect(w.text()).toContain('a.txt')
    expect(w.emitted('load-children')).toHaveLength(1)
  })

  it('失败保留目录与错误，点击再次展开可重试', async () => {
    const failed: TreeNode = { ...lazyTree(), loadError: '加载失败：连接中断' }
    const w = mountNode(null, { node: failed })
    const root = w.get('[role="treeitem"]')

    await root.find('.tree-row').trigger('click')
    expect(w.get('.tree-load-error').text()).toContain('连接中断')
    expect(w.emitted('load-children')).toEqual([['@vfs/wd']])

    await root.find('.tree-row').trigger('click')
    expect(w.emitted('load-children')).toEqual([['@vfs/wd'], ['@vfs/wd']])
  })

  it('右键先发起加载，子层就绪后才把焦点移入首项', async () => {
    const lazy = lazyTree()
    const w = mountNode(null, { node: lazy })
    const root = w.get('[role="treeitem"]')
    ;(root.element as HTMLElement).focus()

    await root.trigger('keydown', { key: 'ArrowRight' })
    expect(w.emitted('load-children')).toEqual([['@vfs/wd']])
    expect(document.activeElement).toBe(root.element)

    await w.setProps({
      node: {
        ...lazy,
        loaded: true,
        children: [{ item: file('@vfs/wd/a.txt', 'a.txt'), children: [] }],
      },
    })
    const child = w.get('[role="group"]').find('[role="treeitem"]')
    expect(document.activeElement).toBe(child.element)
  })
})
