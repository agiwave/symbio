// @vitest-environment happy-dom
/**
 * sessions store —— 实体事件驱动的标题收敛
 *
 * 覆盖「标题变更不因发起者不同而走不同链路」这一约定：后端在
 * `handlers::invoke_update`（手动改名）与 `orchestrator::ensure_auto_title`
 * （首条消息后自动命名）**两处**都会发 `updated` + `display_title`，
 * store 必须就地消费它，而不是等下一次整表重拉。
 */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'

/** store 在 setup 时订阅一次实体总线；此处捕获回调以便直接投递事件 */
const captured = vi.hoisted(() => ({ handlers: [] as Array<(e: unknown) => void> }))

vi.mock('@/services/eventBus', () => ({
  subscribeEntityChanged: (_kind: string, handler: (e: unknown) => void) => {
    captured.handlers.push(handler)
    return () => {}
  },
  publishEntityChangedLocal: vi.fn(),
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

vi.mock('@/services/vdfs', () => ({ readVdfs: vi.fn(async () => ({ text: '{"messages":[]}' })) }))
vi.mock('@/services/plugin', () => ({
  setLastWorkdir: vi.fn(),
  getLastWorkdir: () => '',
  callPlugin: vi.fn(),
}))

import { useSessionsStore } from '../sessions'

describe('sessions store — 实体事件的标题收敛', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    captured.handlers.length = 0
    sessionApi.listSessions.mockClear()
  })

  it('store 在 setup 时恰好订阅一次实体总线', () => {
    useSessionsStore()
    expect(captured.handlers).toHaveLength(1)
  })

  it('updated 携带 display_title → 就地更新 titles 与清单项，且零重拉', () => {
    const store = useSessionsStore()
    // 模拟清单里已有该会话（新建时的默认标题）
    store.titles['s1'] = '新对话'
    store.list.push({ id: 's1', metadata: { title: '新对话' } } as never)

    captured.handlers[0]({ change: 'updated', id: 's1', title: '解释一下 VDFS 的地址模型' })

    expect(store.titles['s1']).toBe('解释一下 VDFS 的地址模型')
    expect(store.list[0].metadata?.title).toBe('解释一下 VDFS 的地址模型')
    // 关键：不因为一次标题变更就把整张清单重拉一遍
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('updated 未携带标题 → 不改动任何标题', () => {
    const store = useSessionsStore()
    store.titles['s1'] = '新对话'
    store.list.push({ id: 's1', metadata: { title: '新对话' } } as never)

    captured.handlers[0]({ change: 'updated', id: 's1', title: null })

    expect(store.titles['s1']).toBe('新对话')
    expect(store.list[0].metadata?.title).toBe('新对话')
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('updated 指向清单外的会话 → 只写 titles，不凭空插入清单项', () => {
    const store = useSessionsStore()
    captured.handlers[0]({ change: 'updated', id: 'unknown', title: '别的会话' })

    expect(store.titles['unknown']).toBe('别的会话')
    expect(store.list).toHaveLength(0)
  })
})
