/**
 * vdfsScheme —— 会话地址方案的**运行期解析**
 *
 * 这两项以前是前端写死的常量（`VDFS_SESSION_DIR` / `VDFS_SEG_MESSAGES`），
 * 现在改成按数据认出来。本单测锁住「按什么认」：
 * - 挂载目录按 `new_types` 里含 `ext = session` 认（provider 的自述，不是名字）；
 * - 转写段按 `kind = VDFS_KIND_MESSAGES` 认（稳定协议词，不是展示名）。
 *
 * 之所以值得测：「认错」不报错，只会静默地把转写写到错的地址上——
 * 那是灾难级且难查的失败。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))
const vdfs = vi.hoisted(() => ({ listVdfs: vi.fn() }))
vi.mock('@/services/vdfs', () => ({ listVdfs: vdfs.listVdfs }))

import {
  ensureSessionMountDir,
  ensureVdfsSessionScheme,
  resetVdfsSessionScheme,
  vdfsSessionScheme,
} from '../vdfsScheme'
import { VDFS_EXT_SESSION, VDFS_KIND_MESSAGES, type VdfsNode } from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'
// 回读理由是**词表**（独立模块，未被替身），断言按它取值——替身里不抄第二份
import { READBACK_REASON } from '../readback'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')

function node(over: Partial<VdfsNode> & { name: string }): VdfsNode {
  return {
    path: over.name,
    title: over.name,
    kind: 'dir',
    status: 'active',
    access: 'l',
    ...over,
  }
}

/** 根清单：会话挂载点靠 `new_types` 自述「可新建 session」 */
function rootListing() {
  return [
    node({ name: 'model', new_types: [{ ext: 'model', title: '模型' }] }),
    node({ name: 'session', new_types: [{ ext: VDFS_EXT_SESSION, title: '会话' }] }),
  ]
}

/** 会话清单：一个会话叶子 */
function sessionListing() {
  return [node({ name: 's1', kind: 'session', ext: VDFS_EXT_SESSION })]
}

/** 会话内部：转写列表靠 `kind` 认，与 `subsession` / `workdir` 并列 */
function sessionChildren() {
  return [
    node({ name: 'message', kind: VDFS_KIND_MESSAGES }),
    node({ name: 'subsession', kind: 'dir' }),
    node({ name: 'workdir', kind: 'dir' }),
  ]
}

describe('ensureSessionMountDir：按 new_types 认挂载点', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
  })

  it('取「声明可新建 ext=session」的那个子节点，而不是按名字', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })

    const mountDir = await ensureSessionMountDir()

    // 决定性的一点：名字若被改（比如注册成 conversations），这里照样认得出
    expect(mountDir).toBe('@vfs/session')
    // 只给理由、不给路径 = 走 listVdfs 的缺省参数（根锚点 vdfsRoot() = '@vfs'）
    expect(vdfs.listVdfs).toHaveBeenCalledWith(READBACK_REASON.BOOTSTRAP)
  })

  it('幂等：第二次不再列目录（带缓存）', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })

    await ensureSessionMountDir()
    await ensureSessionMountDir()

    expect(vdfs.listVdfs).toHaveBeenCalledTimes(1)
  })

  it('认不到挂载点时抛错（宁可报错，也不要订到拼错的地址上）', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({
      path: '@vfs',
      node: node({ name: '' }),
      items: [node({ name: 'model' })],
    })

    await expect(ensureSessionMountDir()).rejects.toThrow(/会话挂载点未找到/)
  })
})

describe('ensureVdfsSessionScheme：按 kind 认转写段', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
  })

  it('取会话内部 kind = messages 的那个子目录（与子会话 / 工作目录区分开）', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: sessionListing() })
      .mockResolvedValueOnce({ path: '@vfs/session/s1', node: node({ name: 's1' }), items: sessionChildren() })

    const scheme = await ensureVdfsSessionScheme()

    expect(scheme).toEqual({ mountDir: '@vfs/session', messagesSeg: 'message' })
  })

  it('展示名变了也认得出（这正是 kind 存在的理由）', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: sessionListing() })
      .mockResolvedValueOnce({
        path: '@vfs/session/s1',
        node: node({ name: 's1' }),
        // 段名换成别的（后端改文案），kind 不变
        items: [node({ name: 'transcript', kind: VDFS_KIND_MESSAGES }), node({ name: 'subsession' })],
      })

    await expect(ensureVdfsSessionScheme()).resolves.toEqual({
      mountDir: '@vfs/session',
      messagesSeg: 'transcript',
    })
  })

  it('零会话时抛错（转写段在会话内部，推导不出来）', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: [] })

    await expect(ensureVdfsSessionScheme()).rejects.toThrow(/没有任何会话可供推导/)
  })

  it('会话内部没有 kind=messages 的子目录时抛错', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: sessionListing() })
      .mockResolvedValueOnce({
        path: '@vfs/session/s1',
        node: node({ name: 's1' }),
        items: [node({ name: 'subsession' })],
      })

    await expect(ensureVdfsSessionScheme()).rejects.toThrow(/没有 kind=messages/)
  })
})

describe('地址拼接：后端两种口径都不能拼重', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
  })

  it('path 已是全路径时直接用（曾拼成 <根>/<根>/session ⇒ 读取转写 404）', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({
      path: '@vfs',
      node: node({ name: '' }),
      // 真实口径：根清单里挂载点的 path 就是展示全路径
      items: [
        node({ name: 'session', path: '@vfs/session', new_types: [{ ext: VDFS_EXT_SESSION, title: '会话' }] }),
      ],
    })

    await expect(ensureSessionMountDir()).resolves.toBe('@vfs/session')
  })

  it('path 是段名时按父地址拼', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({
      path: '@vfs',
      node: node({ name: '' }),
      items: [
        node({ name: 'session', path: 'session', new_types: [{ ext: VDFS_EXT_SESSION, title: '会话' }] }),
      ],
    })

    await expect(ensureSessionMountDir()).resolves.toBe('@vfs/session')
  })

  it('会话节点的 path 是全路径时不再拼挂载目录', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({
        path: '@vfs/session',
        node: node({ name: 'session' }),
        items: [node({ name: 's1', path: '@vfs/session/s1', kind: 'session', ext: VDFS_EXT_SESSION })],
      })
      .mockResolvedValueOnce({ path: '@vfs/session/s1', node: node({ name: 's1' }), items: sessionChildren() })

    await expect(ensureVdfsSessionScheme()).resolves.toEqual({
      mountDir: '@vfs/session',
      messagesSeg: 'message',
    })
    // 列会话内部用的是会话自己的全路径，不是把它再挂到挂载目录下
    expect(vdfs.listVdfs.mock.calls[2][1]).toBe('@vfs/session/s1')
  })
})

describe('vdfsSessionScheme：同步读（事件回调用）', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
  })

  it('未解析时返回 null —— 引导窗口内事件不路由，而不是拿错的段名去拼', () => {
    expect(vdfsSessionScheme()).toBeNull()
  })

  it('两者都就绪后才返回方案', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: sessionListing() })
      .mockResolvedValueOnce({ path: '@vfs/session/s1', node: node({ name: 's1' }), items: sessionChildren() })

    // 只解析挂载目录时，完整方案仍不可用（转写段还不知道）
    await ensureSessionMountDir()
    expect(vdfsSessionScheme()).toBeNull()

    await ensureVdfsSessionScheme()
    expect(vdfsSessionScheme()).toEqual({ mountDir: '@vfs/session', messagesSeg: 'message' })
  })
})
