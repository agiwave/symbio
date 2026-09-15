// @vitest-environment happy-dom
/**
 * useVdfs —— **追加型变更**（`VDFS_CHANGE_APPENDED`）的消费单测
 *
 * 锁定 S18 的两件事，都是"错了也不报错、只悄悄少一段字"的类型：
 *
 * 1. `appended` **就地拼接、不重拉**——这是它与 `updated` 的分野；若误走
 *    `scheduleRefresh()`，流式期间会变成每帧一次全目录重拉（O(n²) 流量），
 *    而且重拉回来的快照还可能比本地旧。
 * 2. **刷新代际守卫**：在途 `read` 响应不得覆盖已应用的增量。缺了它，
 *    `append("def")` 之后到达的旧快照会把正文打回 `"abc"`，下一次 append
 *    再拼上去就成了 `"abcghi"` —— 静默损坏。
 */

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { defineComponent, nextTick, ref } from 'vue'
import { mount } from '@vue/test-utils'

const mocks = vi.hoisted(() => ({
  listVdfs: vi.fn(),
  readVdfs: vi.fn(),
  watchVdfs: vi.fn(),
  unwatchVdfs: vi.fn(),
  subscribe: vi.fn(),
}))

vi.mock('@/services/vdfs', () => ({
  listVdfs: mocks.listVdfs,
  readVdfs: mocks.readVdfs,
  watchVdfs: mocks.watchVdfs,
  unwatchVdfs: mocks.unwatchVdfs,
  // 以下未被本用例触达，仅为满足模块导入
  arrayBufferToBase64: vi.fn(),
  base64ToBytes: vi.fn(),
  deleteVdfs: vi.fn(),
  downloadBlob: vi.fn(),
  moveVdfs: vi.fn(),
  runVdfsAction: vi.fn(),
  writeVdfs: vi.fn(),
  writeVdfsBinary: vi.fn(),
}))
vi.mock('@/services/eventBus', () => ({ subscribe: mocks.subscribe }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

import { useVdfs } from '../useVdfs'
import {
  VDFS_CHANGE_APPENDED,
  VDFS_CHANGE_UPDATED,
  VDFS_EXT_MESSAGE,
  VDFS_ROOT,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'

/** 消息列表地址（被测的绑定地址） */
const MSG_DIR = `${VDFS_ROOT}/session/abc/消息`

/** 一个 `ext = message` 的列表项（只有 `r`，正文在内容里） */
function msgNode(id: string, title = id): VdfsNode {
  return {
    path: `${MSG_DIR}/${id}`,
    name: id,
    title,
    kind: 'file',
    status: 'streaming',
    access: 'r',
    ext: VDFS_EXT_MESSAGE,
    role: 'assistant',
    type: 'text',
    seq: 1,
  }
}

/** 手动可控的 Promise（模拟"在途"的 read 响应） */
function deferred<T>() {
  let resolve!: (v: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

/** 挂一个宿主组件，让 useVdfs 有组件实例（onBeforeUnmount / watch 需要） */
function mountHost(addr: string) {
  let api!: ReturnType<typeof useVdfs>
  const Host = defineComponent({
    setup() {
      api = useVdfs({ addr: ref(addr) })
      return () => null
    },
  })
  const wrapper = mount(Host)
  return { api, wrapper }
}

/** 取出 useVdfs 注册到总线上的变更回调（收的是总线信封，不是裸 VdfsChange） */
function busHandler(): (event: unknown) => void {
  const calls = mocks.subscribe.mock.calls
  const call = calls[calls.length - 1]
  expect(call, 'useVdfs 必须订阅 vdfs 变更频道').toBeTruthy()
  return call![1] as (event: unknown) => void
}

/** 按总线下发的真实形状投递一条 VDFS 变更 */
function emitChange(change: VdfsChange) {
  busHandler()({
    type: 'bus_event',
    data: { kind: 'vdfs', session_id: null, data: change },
  })
}

/** 让 refreshNav / refresh 的 promise 链跑完 */
async function settle() {
  for (let i = 0; i < 4; i += 1) await Promise.resolve()
  await nextTick()
}

beforeEach(() => {
  // reset（而非 clear）：`mockResolvedValueOnce` 的排队项也要清掉，否则用例之间互相污染
  vi.resetAllMocks()
  mocks.listVdfs.mockResolvedValue({
    path: MSG_DIR,
    node: { ...msgNode('__dir'), access: 'l', ext: undefined },
    items: [],
  })
  mocks.readVdfs.mockResolvedValue({ path: '', text: '', binary: false, size: 0 })
  mocks.watchVdfs.mockResolvedValue(undefined)
  mocks.unwatchVdfs.mockResolvedValue(undefined)
  mocks.subscribe.mockReturnValue(() => {})
})

describe('useVdfs 消费 appended（流式即列表项的追加）', () => {
  it('命中当前详情：就地拼接正文，且不触发任何重拉', async () => {
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()

    // 打开一条消息（正文来自 read）
    const node = msgNode('m1')
    mocks.readVdfs.mockResolvedValueOnce({
      path: node.path,
      text: '你好',
      binary: false,
      size: 6,
    })
    await api.select(node)
    await settle()
    expect(api.nodeText.value).toBe('你好')

    const listCalls = mocks.listVdfs.mock.calls.length

    // 追加两帧
    emitChange({ path: node.path, change: VDFS_CHANGE_APPENDED, delta: '，世界' })
    emitChange({ path: node.path, change: VDFS_CHANGE_APPENDED, delta: '！' })
    await settle()

    expect(api.nodeText.value).toBe('你好，世界！')
    expect(
      mocks.listVdfs.mock.calls.length,
      '追加不得触发重拉：它是增量语义，重拉就是 O(n²)',
    ).toBe(listCalls)
    expect(mocks.readVdfs.mock.calls.length, '追加不得触发重读').toBe(1)

    wrapper.unmount()
  })

  it('未命中当前详情：不拼接、也不重拉（尾部追加不影响列表结构与预览首行）', async () => {
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()

    const node = msgNode('m1')
    mocks.readVdfs.mockResolvedValueOnce({
      path: node.path,
      text: '你好',
      binary: false,
      size: 6,
    })
    await api.select(node)
    await settle()

    const listCalls = mocks.listVdfs.mock.calls.length
    emitChange({ path: `${MSG_DIR}/other`, change: VDFS_CHANGE_APPENDED, delta: '别处的增量' })
    await settle()

    expect(api.nodeText.value).toBe('你好')
    expect(mocks.listVdfs.mock.calls.length).toBe(listCalls)

    wrapper.unmount()
  })

  it('刷新代际守卫：在途 read 的旧快照不得覆盖已应用的增量', async () => {
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()

    const node = msgNode('m1')

    // 第一次读取：拿到基线 "abc"
    mocks.readVdfs.mockResolvedValueOnce({
      path: node.path,
      text: 'abc',
      binary: false,
      size: 3,
    })
    await api.select(node)
    await settle()
    expect(api.nodeText.value).toBe('abc')

    // 第二次读取挂起（模拟在途），期间追加落地
    const inflight = deferred<{ path: string; text: string; binary: boolean; size: number }>()
    mocks.readVdfs.mockReturnValueOnce(inflight.promise)
    const selecting = api.select(node)

    emitChange({ path: node.path, change: VDFS_CHANGE_APPENDED, delta: 'def' })
    expect(api.nodeText.value, '追加应立即可见').toBe('abcdef')

    // 旧快照此刻才到达：必须被丢弃
    inflight.resolve({ path: node.path, text: 'abc', binary: false, size: 3 })
    await selecting
    await settle()

    expect(api.nodeText.value, '旧快照覆盖了增量 → 后续 append 会拼出损坏文本').toBe('abcdef')

    // 再追加一帧：仍是正确拼接（而不是 "abc" + "ghi"）
    emitChange({ path: node.path, change: VDFS_CHANGE_APPENDED, delta: 'ghi' })
    expect(api.nodeText.value).toBe('abcdefghi')

    wrapper.unmount()
  })

  it('非追加变更仍走重拉（updated 是「请重读」）', async () => {
    vi.useFakeTimers()
    try {
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m1`, change: VDFS_CHANGE_UPDATED })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, 'updated 必须触发重拉收敛').toBeGreaterThan(
        listCalls,
      )
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })
})
