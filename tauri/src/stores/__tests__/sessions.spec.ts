// @vitest-environment happy-dom
/**
 * sessions store —— VDFS 变更驱动的会话清单收敛
 *
 * 覆盖两条约定：
 *
 * 1. **只有一条变更频道**：会话清单的同步来自 `kind = 'vdfs'` 的
 *    `VdfsChangeEvent`，作用域按**展示地址前缀**分流（`.vdfs/session` 的直接子项
 *    = 会话叶子；转写列表项 / 子会话的变更不进侧栏）。
 * 2. **载荷是粗粒度的**（只有 path + change，不带快照）：
 *    - `updated`（含 busy/idle 运行态与标题变更）→ **重读该会话节点**（`vdfs/stat`）
 *      就地落定，零整表重拉；
 *    - `created` → 防抖重拉收敛排序与完整字段；
 *    - `deleted` → 本地即时移除。
 */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { flushPromises } from '@vue/test-utils'

/** store 在 setup 时订阅一次 vdfs 变更频道；此处捕获作用域与回调以便直接投递事件 */
const captured = vi.hoisted(() => ({
  scopes: [] as Array<{ prefix: string; directChildren?: boolean }>,
  handlers: [] as Array<(e: unknown) => void>,
}))

vi.mock('@/services/eventBus', () => ({
  subscribeVdfsChanged: (
    scope: { prefix: string; directChildren?: boolean },
    handler: (e: unknown) => void
  ) => {
    captured.scopes.push(scope)
    captured.handlers.push(handler)
    return () => {}
  },
  publishVdfsChangedLocal: vi.fn(),
}))

const sessionApi = vi.hoisted(() => ({ listSessions: vi.fn(async () => []) }))
vi.mock('@/services/session', () => ({
  listSessions: sessionApi.listSessions,
  clearSession: vi.fn(),
  createSessionId: () => 'generated-id',
  updateSession: vi.fn(),
  clearMessages: vi.fn(),
  deleteMessage: vi.fn(),
  updateMessage: vi.fn(),
}))

const vdfsApi = vi.hoisted(() => ({
  readVdfs: vi.fn(),
  statVdfs: vi.fn(),
  writeVdfs: vi.fn(),
  watchVdfs: vi.fn(async () => {}),
  unwatchVdfs: vi.fn(async () => {}),
}))
vi.mock('@/services/vdfs', () => ({
  readVdfs: vdfsApi.readVdfs,
  statVdfs: vdfsApi.statVdfs,
  writeVdfs: vdfsApi.writeVdfs,
  watchVdfs: vdfsApi.watchVdfs,
  unwatchVdfs: vdfsApi.unwatchVdfs,
}))
vi.mock('@/services/plugin', () => ({
  setLastWorkdir: vi.fn(),
  getLastWorkdir: () => '',
  callPlugin: vi.fn(),
}))

import { useSessionsStore } from '../sessions'
import { VDFS_SESSION_DIR, VDFS_ROOT, vdfsJoin } from '@/schemas/vdfs'

/** 投递一条变更（路径就是展示地址） */
function emit(change: Record<string, unknown>) {
  captured.handlers[0](change)
}

function sessionNode(over: Record<string, unknown> = {}) {
  return {
    path: `${VDFS_ROOT}/${VDFS_SESSION_DIR}/s1`,
    name: 's1',
    title: '解释一下 VDFS 的地址模型',
    kind: 'session',
    status: 'active',
    access: 'rw',
    ...over,
  }
}

describe('sessions store — VDFS 变更的清单收敛', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    captured.scopes.length = 0
    captured.handlers.length = 0
    sessionApi.listSessions.mockClear()
    vdfsApi.statVdfs.mockReset()
    vdfsApi.writeVdfs.mockReset()
    vdfsApi.readVdfs.mockResolvedValue({ text: '{"messages":[]}' })
  })

  it('新建会话 = 一次 vdfs/write 到会话挂载根；id 由后端生成，前端不编造', async () => {
    // 后端返回的是**树内相对路径**（容器把挂载目录名补在前面），前端只取末段
    vdfsApi.writeVdfs.mockResolvedValue({ path: 'session/s9', created: true })
    const store = useSessionsStore()

    const id = await store.createSession({ workdir: 'D:/work' })

    expect(id).toBe('s9')
    expect(vdfsApi.writeVdfs).toHaveBeenCalledTimes(1)
    const [path, body, opts] = vdfsApi.writeVdfs.mock.calls[0] as [string, string, { create: boolean }]
    expect(path).toBe(vdfsJoin(VDFS_ROOT, VDFS_SESSION_DIR))
    expect(opts).toEqual({ create: true })
    // 目标名不由前端给：会话名字来自首条消息（display_title），此处只带 metadata
    expect(JSON.parse(body)).toEqual({ metadata: expect.objectContaining({ workdir: 'D:/work' }) })
    expect(store.list[0]?.id).toBe('s9')
    expect(store.activeId).toBe('s9')
  })

  it('后端没给出新地址时报错，而不是插一条 id 为空的会话', async () => {
    vdfsApi.writeVdfs.mockResolvedValue({ path: '', created: true })
    const store = useSessionsStore()
    await expect(store.createSession()).rejects.toThrow()
    expect(store.list).toHaveLength(0)
  })

  it('store 在 setup 时恰好订阅一次，作用域 = 会话叶子的直接子项', () => {
    useSessionsStore()
    expect(captured.handlers).toHaveLength(1)
    expect(captured.scopes[0]).toEqual({
      prefix: vdfsJoin(VDFS_ROOT, VDFS_SESSION_DIR),
      directChildren: true,
    })
  })

  it('updated → 重读该会话节点拿新 status，就地落角标且零整表重拉', async () => {
    const store = useSessionsStore()
    store.titles['s1'] = '新对话'
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: { title: '新对话' } } as never)
    vdfsApi.statVdfs.mockResolvedValueOnce(sessionNode({ status: 'working' }))

    emit({ path: '.vdfs/session/s1', change: 'updated' })
    await flushPromises()

    // 唯一取值位置就是这次 stat（地址即展示地址）
    expect(vdfsApi.statVdfs).toHaveBeenCalledWith('.vdfs/session/s1')
    expect(store.list[0].status).toBe('working')
    expect(store.isSessionWorking('s1')).toBe(true)
    // 标题也随节点自述就地更新，不因为一次变更把整张清单重拉一遍
    expect(store.titles['s1']).toBe('解释一下 VDFS 的地址模型')
    expect(store.list[0].metadata?.title).toBe('解释一下 VDFS 的地址模型')
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('updated 但重读失败 → 不改动任何状态（也不凭空重拉）', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'working', metadata: { title: 'T' } } as never)
    vdfsApi.statVdfs.mockResolvedValueOnce(null)

    emit({ path: '.vdfs/session/s1', change: 'updated' })
    await flushPromises()

    expect(store.list[0].status).toBe('working')
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('created → 防抖重拉清单（收敛排序与完整字段）', async () => {
    vi.useFakeTimers()
    try {
      const store = useSessionsStore()
      emit({ path: '.vdfs/session/newsid', change: 'created' })
      expect(sessionApi.listSessions).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(800)
      expect(sessionApi.listSessions).toHaveBeenCalledTimes(1)
      expect(store.list).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })

  it('deleted → 本地即时移除，不等重拉', () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, metadata: {} } as never)

    emit({ path: '.vdfs/session/s1', change: 'deleted' })

    expect(store.list).toHaveLength(0)
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('appended（转写增量）与会话清单无关：既不重读也不重拉', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, metadata: {} } as never)

    emit({ path: '.vdfs/session/s1/消息/m1', change: 'appended', delta: '半句' })
    await flushPromises()

    expect(vdfsApi.statVdfs).not.toHaveBeenCalled()
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
    expect(store.list).toHaveLength(1)
  })
})
