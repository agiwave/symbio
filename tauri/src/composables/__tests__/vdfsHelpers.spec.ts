/**
 * useVdfs 纯 helper 的单测 —— 直接从 `useVdfs.ts` 抽出的五个无状态判据。
 *
 * 这些函数原本只能借 mount composable 间接覆盖；抽出后在这里把判据的边界
 * （尤其是「静默忽略脏数据」与「孙辈不算命中」）逐条钉死，防止后续重构把
 * 语义悄悄放宽/收窄。
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  affects,
  draftNodeOf,
  findTreeNode,
  focusNameOf,
  nodeMatchesFilter,
} from '../vdfsHelpers'
import { VDFS_STATUS_ACTIVE, type VdfsItem, type VdfsTreeNode } from '@/schemas/vdfs'
import { resetVdfsRoot, setVdfsRoot } from '@/schemas/vdfsRoot'

function item(path: string, extra: Partial<VdfsItem> = {}): VdfsItem {
  return {
    path,
    name: path.split('/').pop()!,
    title: '',
    kind: 'file',
    status: VDFS_STATUS_ACTIVE,
    access: 'r',
    ...extra,
  }
}

describe('focusNameOf —— 路由选中项归一化', () => {
  it('接受一个普通末段名，并去掉首尾空白', () => {
    expect(focusNameOf('  session-1  ')).toBe('session-1')
  })

  it.each([null, '', '   ', '.', '..', 'a/b', 'a\\b'])('拒绝越界形状 %j', (raw) => {
    expect(focusNameOf(raw)).toBeNull()
  })
})

describe('nodeMatchesFilter —— 列表筛选判据', () => {
  const base = item('@vfs/session/s1', {
    name: 'S1',
    title: '第一次对话',
    description: '关于架构的讨论',
    meta_tags: ['重要', 'review'],
  })

  // 约定：`q` 由调用方（`useVdfs` 的 activeFilter）预先 lowerCase，本函数只lowerCase节点侧
  it.each([
    ['名字', 's1'],
    ['标题', '对话'],
    ['描述', '架构'],
    ['标签', 'review'],
  ])('命中%s', (_label, q) => {
    expect(nodeMatchesFilter(base, q)).toBe(true)
  })

  it('无命中返回 false', () => {
    expect(nodeMatchesFilter(base, '不存在的词')).toBe(false)
  })

  it('非数组的 meta_tags 被静默忽略', () => {
    const dirty = item('@vfs/x', { name: 'x', meta_tags: 'oops' })
    expect(() => nodeMatchesFilter(dirty, 'oops')).not.toThrow()
    expect(nodeMatchesFilter(dirty, 'oops')).toBe(false)
  })

  it('数组里混入的非字符串标签被忽略', () => {
    const dirty = item('@vfs/x', { name: 'x', meta_tags: [123, {}] })
    expect(() => nodeMatchesFilter(dirty, '123')).not.toThrow()
    expect(nodeMatchesFilter(dirty, '123')).toBe(false)
  })

  it('混合标签数组里合法的字符串仍参与命中，非字符串被忽略', () => {
    const dirty = item('@vfs/x', { name: 'x', meta_tags: [123, 'review'] })
    expect(nodeMatchesFilter(dirty, 'review')).toBe(true)
  })
})

describe('findTreeNode —— 按完整地址在树中查找', () => {
  const tree: VdfsTreeNode[] = [
    {
      item: item('@vfs/session'),
      children: [
        {
          item: item('@vfs/session/s1'),
          children: [{ item: item('@vfs/session/s1/message/m1'), children: [] }],
        },
      ],
    },
  ]

  it('命中顶层 / 嵌套 / 深层节点', () => {
    expect(findTreeNode(tree, '@vfs/session')?.item.path).toBe('@vfs/session')
    expect(findTreeNode(tree, '@vfs/session/s1')?.item.path).toBe('@vfs/session/s1')
    expect(findTreeNode(tree, '@vfs/session/s1/message/m1')?.item.path).toBe(
      '@vfs/session/s1/message/m1'
    )
  })

  it('找不到返回 null（空树 / 不存在的路径）', () => {
    expect(findTreeNode([], '@vfs/session')).toBeNull()
    expect(findTreeNode(tree, '@vfs/model')).toBeNull()
  })
})

describe('affects —— 变更是否影响目录的条目集合', () => {
  beforeEach(() => setVdfsRoot('@vfs'))
  afterEach(() => resetVdfsRoot())

  const dir = '@vfs/session'

  it.each([
    ['目录自身', '@vfs/session'],
    ['直接子项', '@vfs/session/s1'],
    ['祖先', '@vfs'],
  ])('命中%s', (_label, changePath) => {
    expect(affects(dir, changePath)).toBe(true)
  })

  it.each([
    ['孙辈（会话转写的逐帧变更）', '@vfs/session/s1/message/m1'],
    ['兄弟目录', '@vfs/model'],
  ])('不命中%s', (_label, changePath) => {
    expect(affects(dir, changePath)).toBe(false)
  })
})

describe('draftNodeOf —— 新建草稿节点', () => {
  const type = {
    ext: 'model',
    node_ext: 'form',
    title: '模型',
    description: '在详情页里填好，保存时一次写入',
    schema: { fields: ['name'] },
  }

  it('草稿无地址 / 无名字，渲染器键取 node_ext', () => {
    const draft = draftNodeOf(type)
    expect(draft.path).toBe('')
    expect(draft.name).toBe('')
    expect(draft.title).toBe('')
    expect(draft.ext).toBe('form')
    expect(draft.status).toBe(VDFS_STATUS_ACTIVE)
    expect(draft.access).toBe('w')
  })

  it('node_ext 缺省时渲染器键回落到 ext', () => {
    expect(draftNodeOf({ ext: 'session', title: '会话' }).ext).toBe('session')
  })

  it('呈现描述 / 说明 / 图标键随类型下发', () => {
    const draft = draftNodeOf(type)
    expect(draft.schema).toBe(type.schema)
    expect(draft.description).toBe(type.description)
    expect(draft.config_type).toBe('model')
  })
})
