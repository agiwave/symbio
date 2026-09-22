// @vitest-environment happy-dom
/**
 * useVdfs —— **VDFS 变更的消费单测**（路径过滤 + 重拉收敛）
 *
 * 锁定的是「错了也不报错、只是静默不收敛 / 静默重拉」的那类问题：
 *
 * 1. **影响判定**：变更路径落在当前目录自身 / 祖先 / 子树内才重拉——收窄错了
 *    表现为「邻目录改一下，本页刷一次」；放宽错了表现为「本页的东西变了却不更新」。
 * 2. **词汇表是闭集且没有特殊分支**：变更只有 `created` / `updated` / `deleted`
 *    三个取值、且**不带载荷**（见 `schemas/vdfs.VdfsChange`），因此一律走同一条
 *    重拉路径。曾经 `appended` 有一条「就地拼接、不重拉」的快速路径（防 O(n²)
 *    流量），而它没有任何生产者——现在**任何取值都不该被静默吞掉**。
 *
 * 一个已删除的场景留作记录：从前还有「刷新代际守卫」（在途 `read` 的旧快照不得
 * 覆盖已应用的增量）。它随 `delta` 一起消失——没有本地增量，就没有可被覆盖的东西。
 */

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { defineComponent, nextTick, ref } from 'vue'
import { mount } from '@vue/test-utils'

const mocks = vi.hoisted(() => ({
  listVdfs: vi.fn(),
  readVdfs: vi.fn(),
  writeVdfs: vi.fn(),
  subscribeVdfsChanged: vi.fn(),
}))

vi.mock('@/services/vdfs', () => ({
  listVdfs: mocks.listVdfs,
  readVdfs: mocks.readVdfs,
  writeVdfs: mocks.writeVdfs,
  // 以下未被本用例触达，仅为满足模块导入
  arrayBufferToBase64: vi.fn(),
  base64ToBytes: vi.fn(),
  deleteVdfs: vi.fn(),
  downloadBlob: vi.fn(),
  moveVdfs: vi.fn(),
  runVdfsAction: vi.fn(),
  writeVdfsBinary: vi.fn(),
}))
// 订阅入口整体替身：`subscribeVdfsChanged` 已把「总线频道 + 后端 watch 登记 +
// 本地通道」三件事收在一处（它自己由 services/__tests__/eventBusWatch.spec.ts 覆盖），
// 本用例只关心「收到变更后怎么处理」，因此直接拿到裸 VdfsChange 回调。
vi.mock('@/services/eventBus', () => ({ subscribeVdfsChanged: mocks.subscribeVdfsChanged }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

import { useVdfs } from '../useVdfs'
import {
  VDFS_CHANGE_CREATED,
  VDFS_CHANGE_DELETED,
  VDFS_CHANGE_UPDATED,
  VDFS_EXT_MESSAGE,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')

/** 消息列表地址（被测的绑定地址） */
const MSG_DIR = `@vfs/session/abc/消息`

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

/** 取出 useVdfs 交给订阅入口的变更回调（订阅入口收裸 VdfsChange，总线信封的解包在它内部） */
function changeHandler(): (change: VdfsChange) => void {
  const calls = mocks.subscribeVdfsChanged.mock.calls
  const call = calls[calls.length - 1]
  expect(call, 'useVdfs 必须订阅 vdfs 变更').toBeTruthy()
  return call![1] as (change: VdfsChange) => void
}

/** 投递一条 VDFS 变更 */
function emitChange(change: VdfsChange) {
  changeHandler()(change)
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
  mocks.subscribeVdfsChanged.mockReturnValue(() => {})
})

describe('useVdfs 消费 VDFS 变更（词汇闭集、无载荷 ⇒ 一律重拉收敛）', () => {
  it('影响当前目录的变更触发重拉（updated = 「这个节点变了，请重读」）', async () => {
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

  it('created 与 deleted 走**同一条**收敛路径（三个取值没有分叉）', async () => {
    for (const change of [VDFS_CHANGE_CREATED, VDFS_CHANGE_DELETED]) {
      vi.resetAllMocks()
      mocks.listVdfs.mockResolvedValue({
        path: MSG_DIR,
        node: { ...msgNode('__dir'), access: 'l', ext: undefined },
        items: [],
      })
      mocks.readVdfs.mockResolvedValue({ path: '', text: '', binary: false, size: 0 })
      mocks.subscribeVdfsChanged.mockReturnValue(() => {})

      vi.useFakeTimers()
      try {
        const { wrapper } = mountHost(MSG_DIR)
        await settle()
        const listCalls = mocks.listVdfs.mock.calls.length

        emitChange({ path: `${MSG_DIR}/m9`, change })
        await vi.advanceTimersByTimeAsync(500)

        expect(
          mocks.listVdfs.mock.calls.length,
          `${change} 必须触发重拉（没有就地增删的快速路径）`,
        ).toBeGreaterThan(listCalls)
        wrapper.unmount()
      } finally {
        vi.useRealTimers()
      }
    }
  })

  it('不影响当前目录的变更**不**重拉（邻目录改一下，本页不该刷）', async () => {
    vi.useFakeTimers()
    try {
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      // 同一会话的**兄弟目录**：既不是 MSG_DIR 自身，也不在其子树内
      emitChange({ path: `@vfs/session/abc/其它/m1`, change: VDFS_CHANGE_UPDATED })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, 'affects() 收窄失效会让全应用互相刷').toBe(
        listCalls,
      )
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('已废除的取值不再有特殊分支：投一条 `appended` 也走通用重拉（不再被静默吞掉）', async () => {
    // 这条是**负向契约**，锁住本次收窄：`appended` 曾有一条「到此为止、就地拼接、
    // 不重拉」的提前返回。若有人把它加回词汇表却不给生产者，这里会提醒他
    // 「消费端要么处理它，要么它根本不该存在」——而不是像从前那样静默吞掉。
    vi.useFakeTimers()
    try {
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m1`, change: 'appended' })
      await vi.advanceTimersByTimeAsync(500)

      expect(
        mocks.listVdfs.mock.calls.length,
        '未知取值不得被静默吞掉：它必须落到通用重拉上',
      ).toBeGreaterThan(listCalls)
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('useVdfs 有界列表（中栏只取最新一页 + 加载更早）', () => {
  /** N 个列表项（name 形如 m<i>，便于断言游标） */
  function page(from: number, n: number): VdfsNode[] {
    return Array.from({ length: n }, (_, i) => msgNode('m' + (from + i)))
  }

  function listReturns(items: VdfsNode[]) {
    mocks.listVdfs.mockResolvedValue({
      path: MSG_DIR,
      node: { ...msgNode('__dir'), access: 'l', ext: undefined },
      items,
    })
  }

  it('首屏只取最新一页（带 limit）', async () => {
    listReturns(page(0, 100))
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()

    expect(api.items.value).toHaveLength(100)
    expect(api.hasMore.value, '满页 ⇒ 可能还有更早的').toBe(true)
    // 目录刷新那一次带窗口参数；左栏导航那次不带（导航项本来就少）
    expect(mocks.listVdfs).toHaveBeenLastCalledWith(MSG_DIR, { limit: 100 })

    wrapper.unmount()
  })

  it('不满一页 ⇒ 没有更早的（不会多问一次）', async () => {
    listReturns(page(0, 7))
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()

    expect(api.items.value).toHaveLength(7)
    expect(api.hasMore.value).toBe(false)

    wrapper.unmount()
  })

  it('加载更早：以最后一项为游标追加，按 path 去重', async () => {
    listReturns(page(0, 100))
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()

    listReturns(page(100, 3))
    await api.loadMore()
    await settle()

    expect(mocks.listVdfs).toHaveBeenLastCalledWith(MSG_DIR, {
      limit: 100,
      before: 'm99',
    })
    expect(api.items.value).toHaveLength(103)
    expect(api.items.value[102].name).toBe('m102')
    expect(api.hasMore.value, '续页不满 ⇒ 到底了').toBe(false)

    wrapper.unmount()
  })

  it('后端不认窗口参数（每页都返回同一整页）⇒ 去重后判到底，不产生重复项', async () => {
    const full = page(0, 100)
    listReturns(full)
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()
    expect(api.hasMore.value).toBe(true)

    // 同一整页再回来一次：新项为 0
    listReturns(full)
    await api.loadMore()
    await settle()

    expect(api.items.value, '不得重复追加').toHaveLength(100)
    expect(api.hasMore.value, '没有新项即到底').toBe(false)

    wrapper.unmount()
  })
})

describe('useVdfs 新建 = 选中一张草稿节点（与「选中一项」同一条详情通道）', () => {
  const MODEL_DIR = `@vfs/model`

  /** 目录声明的「可新建类型」：`ext` 是呈现扩展名，`node_ext` 才是渲染器键 */
  const MODEL_NEW_TYPE = {
    ext: 'model',
    title: '模型',
    node_ext: 'form',
    schema: { sections: [{ title: null, collapsed: false, fields: [] }] },
  }

  function dirNode() {
    return {
      path: MODEL_DIR,
      name: 'model',
      title: '模型',
      kind: 'dir',
      status: 'active',
      access: 'l',
      new_types: [MODEL_NEW_TYPE],
    }
  }

  it('草稿节点没有 id / 名字，但渲染器与 schema 就是该类型落成后的那一套', async () => {
    mocks.listVdfs.mockResolvedValue({ path: MODEL_DIR, node: dirNode(), items: [] })
    const { api, wrapper } = mountHost(MODEL_DIR)
    await settle()
    expect(api.canCreate.value).toBe(true)

    api.startNew(api.creatableTypes.value[0]!)
    await settle()

    const n = api.selectedNode.value!
    expect(n.path, '草稿还没落盘 ⇒ 没有地址').toBe('')
    expect(n.name, '草稿还没有名字').toBe('')
    // 关键：渲染器键取 `node_ext`（form），不是呈现扩展名（model）——
    // 否则会落到 fallback 兜底，而不是与「选中一项」同一个详情页
    expect(n.ext).toBe('form')
    expect(n.schema).toEqual(MODEL_NEW_TYPE.schema)
    expect(api.renderer.value).toBe('form')
    expect(mocks.readVdfs, '草稿没有内容可读，不该去读一个不存在的节点').not.toHaveBeenCalled()

    wrapper.unmount()
  })

  it('草稿保存：写到**目录自身**并带 create 意图（名字由后端生成）', async () => {
    mocks.listVdfs.mockResolvedValue({ path: MODEL_DIR, node: dirNode(), items: [] })
    const { api, wrapper } = mountHost(MODEL_DIR)
    await settle()
    api.startNew(api.creatableTypes.value[0]!)
    await settle()

    mocks.writeVdfs.mockResolvedValue({ path: `${MODEL_DIR}/model-a1b2c3d4`, created: true })
    await api.saveFields({ name: '我的模型' })

    const [path, text, opts] = mocks.writeVdfs.mock.calls[0] as [
      string,
      string,
      { create: boolean },
    ]
    expect(path, '草稿的目标是它所在的那个目录（使用方不说叫什么）').toBe(MODEL_DIR)
    expect(opts).toEqual({ create: true })
    expect(JSON.parse(text), '填好的字段必须原样送出').toEqual({ name: '我的模型' })

    wrapper.unmount()
  })

  it('连续两次新建：草稿代际自增，详情组件据此重挂载（不残留上一份草稿）', async () => {
    mocks.listVdfs.mockResolvedValue({ path: MODEL_DIR, node: dirNode(), items: [] })
    const { api, wrapper } = mountHost(MODEL_DIR)
    await settle()

    api.startNew(api.creatableTypes.value[0]!)
    const first = api.draftSeq.value
    api.startNew(api.creatableTypes.value[0]!)
    expect(api.draftSeq.value).toBe(first + 1)

    wrapper.unmount()
  })
})
