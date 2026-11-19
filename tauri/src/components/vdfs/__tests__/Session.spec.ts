// @vitest-environment happy-dom
/**
 * Session —— 选中会话时的**地址重落**（子会话必须能被打开）
 *
 * 会话在前端的身份是「空间 + id」。子会话
 * （`<挂载目录>/<父会话>/subsession/<子>`）的 `ext` 也是 `session`，但地址比普通
 * 会话多嵌两段：原样交给 store，`parseVdfsSessionAddr` 会把 `<父会话>/subsession`
 * 当成空间，后续清单 / 转写 / 收件箱 / 工作目录全部按这个假空间拼，得到一串
 * 「会话内部不存在该路径」「该路径不可写入」——用户看到的就是**子会话窗口打不开、
 * 没有内容**。
 *
 * 而它在后端是独立会话（凭自身 id 直取），与父会话同处一个空间。因此这里只做
 * 一件事：把地址重新落到 `<挂载目录>/<子>`，并把节点自述一起交给 store（面板要
 * 的 workdir / 标题 / 运行态都在清单项上，而它不在清单里）。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises, shallowMount } from '@vue/test-utils'

const mocks = vi.hoisted(() => ({
  selectSession: vi.fn(async () => {}),
  sessionSpaceOf: vi.fn(),
}))

vi.mock('@/stores/sessions', () => ({
  useSessionsStore: () => ({ selectSession: mocks.selectSession }),
}))
vi.mock('@/services/vdfsScheme', () => ({ sessionSpaceOf: mocks.sessionSpaceOf }))

import Session from '../Session.vue'
import type { VdfsItem } from '@/schemas/vdfs'

const MOUNT = '@vfs/session'

function sessionItem(path: string, over: Partial<VdfsItem> = {}): VdfsItem {
  return {
    path,
    name: path.split('/').pop() ?? path,
    title: '一段会话',
    kind: 'session',
    status: 'active',
    access: 'rw',
    ext: 'session',
    ...over,
  }
}

function mountWith(node: VdfsItem) {
  return shallowMount(Session, { props: { node } })
}

describe('Session：选中会话时的地址重落', () => {
  beforeEach(() => {
    mocks.selectSession.mockClear()
    mocks.sessionSpaceOf.mockReset()
  })

  it('普通会话：父目录就是空间，地址逐字节不变', async () => {
    mocks.sessionSpaceOf.mockResolvedValue(MOUNT)

    const node = sessionItem(`${MOUNT}/s1`)
    mountWith(node)
    await flushPromises()

    expect(mocks.selectSession).toHaveBeenCalledWith(`${MOUNT}/s1`, node)
  })

  it('子会话：重新落到 `<挂载目录>/<子>`（不把 `subsession` 当成空间）', async () => {
    mocks.sessionSpaceOf.mockResolvedValue(MOUNT)

    const node = sessionItem(`${MOUNT}/p1/subsession/s2`)
    mountWith(node)
    await flushPromises()

    // 决定性的一条：id 仍是 s2，空间仍是 `@vfs/session`
    expect(mocks.selectSession).toHaveBeenCalledWith(`${MOUNT}/s2`, node)
  })

  it('子智能体空间里的子会话：落在那个空间，而不是根空间', async () => {
    const agentMount = '@vfs/agent/a1/session'
    mocks.sessionSpaceOf.mockResolvedValue(agentMount)

    const node = sessionItem(`${agentMount}/p1/subsession/s2`)
    mountWith(node)
    await flushPromises()

    expect(mocks.selectSession).toHaveBeenCalledWith(`${agentMount}/s2`, node)
  })

  it('空间认不出来（自述读不到）⇒ 原样交给 store，不否决', async () => {
    mocks.sessionSpaceOf.mockResolvedValue(null)

    const node = sessionItem(`${MOUNT}/s1`)
    mountWith(node)
    await flushPromises()

    expect(mocks.selectSession).toHaveBeenCalledWith(`${MOUNT}/s1`, node)
  })

  it('连点两项：先发起的回执不得覆盖后选中的那一项', async () => {
    let release!: (v: string) => void
    mocks.sessionSpaceOf
      .mockImplementationOnce(() => new Promise<string>((r) => (release = r)))
      .mockResolvedValueOnce(MOUNT)

    const first = sessionItem(`${MOUNT}/s1`)
    const wrapper = mountWith(first)
    // 换选中（第二次解析立刻回来）
    await wrapper.setProps({ node: sessionItem(`${MOUNT}/s2`) })
    await flushPromises()
    release(MOUNT)
    await flushPromises()

    // 只有后选中那一项被交给 store
    expect(mocks.selectSession).toHaveBeenCalledTimes(1)
    expect(mocks.selectSession).toHaveBeenCalledWith(`${MOUNT}/s2`, expect.objectContaining({ path: `${MOUNT}/s2` }))
  })
})
