// @vitest-environment happy-dom
/**
 * useVdfs —— **VDFS 变更的消费单测**（路径过滤 + 重拉收敛）
 *
 * 锁定的是「错了也不报错、只是静默不收敛 / 静默重拉」的那类问题：
 *
 * 1. **影响判定**：变更路径落在当前目录自身 / **直接子项** / 祖先才重拉——收窄
 *    错了表现为「本页的东西变了却不更新」；放宽错了表现为「邻目录改一下，本页
 *    刷一次」，而「任意后代都算命中」更糟：会话转写逐帧变更在 `<sid>/message/<mid>`
 *    这一层，会让每一帧都挂一次防抖刷新，一次会话几十次白拉的 `vdfs/list`。
 * 2. **带正文的载荷就地落地、不重拉**：`delta` 追加、`content` 整条替换——两者
 *    都只改一个节点的**内容**，不改变任何节点的存在与顺序，列表也不内联正文。
 *    载荷自给自足（S27 收口：`path` 恒为被变更节点自身的地址），落到通用重拉上
 *    正好抵消载荷存在的意义。
 *    ⚠️ 这条豁免的前提是**该节点已经在列表里**——新建出来的那一项首帧就带正文，
 *    若一并豁免，新条目会**永远不出现在中栏**。两条用例分别钉住这两个方向
 *    （已知项零重拉 / 未知直接子项必重拉），它们是同一条规则的两半。
 * 3. **其余变更一律重拉收敛**：信封没有操作枚举，资源信号无载荷 ⇒ 回读 / 重拉。
 *    曾经 `appended` 有一条「就地拼接、不重拉」的快速路径，而它没有任何生产者
 *    ——现在**任何取值都不该被静默吞掉**。
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
  VDFS_EXT_MESSAGE,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'
// 回读理由是**词表**（独立模块，未被替身），断言按它取值——替身里不抄第二份
import { READBACK_REASON } from '@/services/readback'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')

/** 消息列表地址（被测的绑定地址） */
const MSG_DIR = `@vfs/session/abc/message`

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

/** 让目录返回指定的条目（缺省空列表由 `beforeEach` 给） */
function listReturns(items: VdfsNode[]) {
  mocks.listVdfs.mockResolvedValue({
    path: MSG_DIR,
    node: { ...msgNode('__dir'), access: 'l', ext: undefined },
    items,
  })
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

describe('useVdfs 消费 VDFS 变更（信封没有操作枚举、非 delta 载荷 ⇒ 一律重拉收敛）', () => {
  it('影响当前目录的变更触发重拉（无载荷 = 「这个节点变了，请重读」）', async () => {
    vi.useFakeTimers()
    try {
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m1` })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, 'updated 必须触发重拉收敛').toBeGreaterThan(
        listCalls,
      )
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('创建与删除走**同一条**收敛路径（信封没有 created/deleted 之分：回读 NotFound 即删除）', async () => {
    vi.useFakeTimers()
    try {
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m9` })
      await vi.advanceTimersByTimeAsync(500)

      expect(
        mocks.listVdfs.mock.calls.length,
        '无载荷变更必须触发重拉（没有就地增删的快速路径）',
      ).toBeGreaterThan(listCalls)
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('不影响当前目录的变更**不**重拉（邻目录改一下，本页不该刷）', async () => {
    vi.useFakeTimers()
    try {
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      // 同一会话的**兄弟目录**：既不是 MSG_DIR 自身，也不在其子树内
      emitChange({ path: `@vfs/session/abc/其它/m1` })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, 'affects() 收窄失效会让全应用互相刷').toBe(
        listCalls,
      )
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('带 delta 的载荷帧**不**触发重拉（该项已在列表里 ⇒ 就地追加语义，防 O(n²) 刷新）', async () => {
    // 流式正文的每一帧都带 `data.delta`——若它们落到通用重拉上，等于给每一帧
    // 挂一次防抖刷新，正好抵消增量帧存在的意义。
    vi.useFakeTimers()
    try {
      listReturns([msgNode('m1')])
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m1`, data: { id: 'm1', delta: '片段' } })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, 'delta 帧不得触发目录重拉').toBe(listCalls)
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('带全量正文的载荷帧**不**触发重拉（该项已在列表里 ⇒ 载荷自给自足，就地整条替换）', async () => {
    // 首帧发的就是图里合并后的全量副本——它与 `delta` 一样只改一个节点的**内容**，
    // 不改变任何节点的存在与顺序，列表也不内联正文。让它落到通用重拉上，等于
    // 每收到一条消息的全量帧就白拉一次目录。
    vi.useFakeTimers()
    try {
      listReturns([msgNode('m1')])
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m1`, data: { id: 'm1', content: '全量' } })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, '全量帧不得触发目录重拉').toBe(listCalls)
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('当前目录里**新出现**的子项即使首帧就带正文也必须重拉（否则新条目永不出现）', async () => {
    // 这是上面两条豁免的边界，也是它们的代价所在：新建出来的一项，首帧就是带正文的
    // （写入即带内容 / 流式首帧即增量），而它**还不在列表里**。若一并按「带正文 ⇒
    // 不重拉」处理，中栏会一直缺这一条，直到某次无关的刷新顺手把它带出来。
    //
    // 判据是「列表里有没有这条路径」，与帧里带的是 `delta` 还是 `content` 无关
    // ——两个方向各钉一条，才是这条规则的完整形状。
    vi.useFakeTimers()
    try {
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m9`, data: { id: 'm9', delta: '新消息的第一段' } })
      await vi.advanceTimersByTimeAsync(500)

      expect(
        mocks.listVdfs.mock.calls.length,
        '本目录还不认识的直接子项 ⇒ 条目集合变了 ⇒ 必须重拉',
      ).toBeGreaterThan(listCalls)
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('新子项进入列表后，它的后续帧不再重拉（代价是「每新增一条一次」，不是「每帧一次」）', async () => {
    // 上一条若退化成「带正文的子项帧都重拉」，一次流式会话就是几十次白拉。这里钉住
    // 收敛：一旦该项出现在列表里，后续增量帧立刻回到零重拉。
    vi.useFakeTimers()
    try {
      listReturns([msgNode('m9')])
      const { wrapper } = mountHost(MSG_DIR)
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      emitChange({ path: `${MSG_DIR}/m9`, data: { id: 'm9', delta: '第二段' } })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, '已在列表里的项 ⇒ 增量帧零重拉').toBe(listCalls)
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('孙辈的变更**不**重拉当前目录（affects 只认自身 / 直接子项 / 祖先）', async () => {
    // 会话转写是在 `<sid>/message/<mid>` 这一层逐帧变更的，而浏览器常停在
    // `<根>/session` 或 `<根>`——「任意后代都算命中」会让每一条消息帧都挂一次
    // 防抖刷新，一次会话下来就是几十次白跑的 `vdfs/list`。
    vi.useFakeTimers()
    try {
      // 绑到会话清单目录，当前目录 = 它自身（mock 的 list 无子目录）
      const { wrapper } = mountHost('@vfs/session')
      await settle()
      const listCalls = mocks.listVdfs.mock.calls.length

      // 孙辈（三层之下）的**资源信号**：既不改变本目录的条目集合，也不在直接子项里
      emitChange({ path: `@vfs/session/abc/message/m1` })
      await vi.advanceTimersByTimeAsync(500)

      expect(mocks.listVdfs.mock.calls.length, '孙辈变更不得重拉本目录').toBe(listCalls)
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

  it('首屏只取最新一页（带 limit）', async () => {
    listReturns(page(0, 100))
    const { api, wrapper } = mountHost(MSG_DIR)
    await settle()

    expect(api.items.value).toHaveLength(100)
    expect(api.hasMore.value, '满页 ⇒ 可能还有更早的').toBe(true)
    // 目录刷新那一次带窗口参数；左栏导航那次不带（导航项本来就少）
    expect(mocks.listVdfs).toHaveBeenLastCalledWith(READBACK_REASON.VDFS_BROWSER, MSG_DIR, {
      limit: 100,
    })

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

    expect(mocks.listVdfs).toHaveBeenLastCalledWith(READBACK_REASON.VDFS_BROWSER, MSG_DIR, {
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
