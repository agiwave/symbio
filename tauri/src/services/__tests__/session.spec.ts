/**
 * session 服务 —— 会话清单映射单测（node 环境）
 *
 * S8 起 `listSessions()` 不再走 `entities/list`，而是读 `.vdfs/session` 的目录
 * 内容（`vdfs/list`）。本单测锁定「VdfsNode → SessionListItem」的映射口径：
 * 会话侧栏的标题 / 工作目录 / 运行中状态全部依赖它，一旦后端字段名或前端取值
 * 方式漂移，表现为**静默退化**（侧栏拿不到 workdir、停止按钮失效），很难肉眼发现。
 */

import { describe, expect, it, vi } from 'vitest'

// 服务层依赖 Tauri API 与本地存储，这里只测映射逻辑，故整体替身
vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn() }))
vi.mock('@/services/vdfs', () => ({ listVdfs: vi.fn() }))

import { listVdfs } from '@/services/vdfs'
import { VFDS_ROOT, vdfsJoin, type VdfsNode } from '@/schemas/vdfs'
import { listSessions } from '../session'

/** 构造一个会话节点（attributes 为 flatten 的场景字段） */
function sessionNode(over: Partial<VdfsNode> = {}): VdfsNode {
  return {
    path: vdfsJoin('.vdfs/session', 'abc'),
    name: 'abc',
    title: '会话标题',
    kind: 'session',
    status: 'active',
    access: 'rw',
    ext: 'session',
    updated_at: 1_700_000_000,
    ...over,
  }
}

function mockList(items: VdfsNode[]) {
  vi.mocked(listVdfs).mockResolvedValueOnce({
    path: vdfsJoin(VFDS_ROOT, 'session'),
    node: sessionNode({ name: 'session', title: '会话' }),
    items,
  })
}

describe('listSessions（.vdfs/session → SessionListItem）', () => {
  it('请求会话挂载点根，并映射全部字段', async () => {
    mockList([
      sessionNode({
        message_count: 12,
        metadata: { workdir: '/tmp/demo', title: 'T' },
        meta_tags: ['demo', '12 条'],
      }),
    ])

    const out = await listSessions()

    expect(vi.mocked(listVdfs)).toHaveBeenCalledWith(vdfsJoin(VFDS_ROOT, 'session'))
    expect(out).toHaveLength(1)
    expect(out[0]).toMatchObject({
      id: 'abc',
      name: '会话标题',
      message_count: 12,
      updated_at: 1_700_000_000,
      is_working: false,
      metadata: { workdir: '/tmp/demo', title: 'T' },
    })
  })

  it('status = working 判为运行中；缺省字段回落空态', async () => {
    mockList([sessionNode({ status: 'working', updated_at: undefined })])

    const [first] = await listSessions()

    // 运行中标记来自节点 status（机制口径：不另设 is_working 字段）
    expect(first.is_working).toBe(true)
    expect(first.updated_at).toBe(0)
    expect(first.message_count).toBe(0)
    expect(first.metadata).toEqual({})
  })

  it('空清单返回空数组（不抛错、不造占位项）', async () => {
    mockList([])
    await expect(listSessions()).resolves.toEqual([])
  })
})
