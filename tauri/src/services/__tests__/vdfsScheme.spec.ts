/**
 * vdfsScheme —— 会话地址方案的**运行期解析**
 *
 * 这三项以前是前端写死的常量（`VDFS_SESSION_DIR` / `VDFS_SEG_MESSAGES`），
 * 现在改成按数据认出来。本单测锁住「按什么认」：
 * - 挂载目录按 `new_type.ext = session` 认（provider 的自述，不是名字）；
 * - 转写段按 `kind = VDFS_KIND_MESSAGES` 认（稳定协议词，不是展示名）；
 * - 收件箱段按 `kind = VDFS_KIND_INBOX` 认（同款）。
 *
 * 之所以值得测：「认错」不报错，只会静默地把转写 / 发言写到错的地址上——
 * 那是灾难级且难查的失败。
 *
 * 另有一组锁**按挂载目录缓存**：会话挂载不止一份（根空间 / 各子智能体空间各
 * 一棵完整子树），「住在哪个空间」是地址的一部分，缓存成一份全局的就会把
 * 子空间的会话算到根空间头上。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))
const vdfs = vi.hoisted(() => ({ listVdfs: vi.fn(), statVdfs: vi.fn() }))
vi.mock('@/services/vdfs', () => ({ listVdfs: vdfs.listVdfs, statVdfs: vdfs.statVdfs }))

import {
  ensureSessionMountDir,
  ensureSessionScheme,
  ensureVdfsSessionScheme,
  isSessionMountDir,
  resetVdfsSessionScheme,
  sessionSpaceOf,
  vdfsSessionScheme,
} from '../vdfsScheme'
import {
  VDFS_EXT_SESSION,
  VDFS_KIND_INBOX,
  VDFS_KIND_MESSAGES,
  type VdfsNode,
} from '@/schemas/vdfs'
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

/** 根清单：会话挂载点靠 `new_type` 自述「可新建 session」 */
function rootListing() {
  return [
    node({ name: 'model', new_type: { ext: 'model', title: '模型' } }),
    node({ name: 'session', new_type: { ext: VDFS_EXT_SESSION, title: '会话' } }),
  ]
}

/** 会话清单：一个会话叶子 */
function sessionListing() {
  return [node({ name: 's1', kind: 'session', ext: VDFS_EXT_SESSION })]
}

/** 会话内部：两个集合段都靠 `kind` 认，与 `subsession` / `workdir` 并列 */
function sessionChildren() {
  return [
    node({ name: 'message', kind: VDFS_KIND_MESSAGES }),
    node({ name: 'inbox', kind: VDFS_KIND_INBOX }),
    node({ name: 'subsession', kind: 'dir' }),
    node({ name: 'workdir', kind: 'dir' }),
  ]
}

describe('ensureSessionMountDir：按 new_type 认挂载点', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
    vdfs.statVdfs.mockReset()
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

describe('ensureSessionScheme：按 kind 认两个集合段', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
    vdfs.statVdfs.mockReset()
  })

  it('取会话内部 kind = messages / inbox 的两个子目录（与子会话 / 工作目录区分开）', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: sessionListing() })
      .mockResolvedValueOnce({ path: '@vfs/session/s1', node: node({ name: 's1' }), items: sessionChildren() })

    const scheme = await ensureVdfsSessionScheme()

    expect(scheme).toEqual({
      mountDir: '@vfs/session',
      messagesSeg: 'message',
      inboxSeg: 'inbox',
    })
    // 两个段**一次列目录同时取**：它们住在同一个父下，分两次解析就是两次 IPC
    expect(vdfs.listVdfs).toHaveBeenCalledTimes(3)
  })

  it('展示名变了也认得出（这正是 kind 存在的理由）', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: sessionListing() })
      .mockResolvedValueOnce({
        path: '@vfs/session/s1',
        node: node({ name: 's1' }),
        // 段名换成别的（后端改文案），kind 不变
        items: [
          node({ name: 'transcript', kind: VDFS_KIND_MESSAGES }),
          node({ name: 'pending', kind: VDFS_KIND_INBOX }),
          node({ name: 'subsession' }),
        ],
      })

    await expect(ensureVdfsSessionScheme()).resolves.toEqual({
      mountDir: '@vfs/session',
      messagesSeg: 'transcript',
      inboxSeg: 'pending',
    })
  })

  it('零会话时抛错（集合段在会话内部，推导不出来）', async () => {
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

  it('有转写段但没有收件箱段时也抛错（半个方案不能用：发言会写到错的地址）', async () => {
    vdfs.listVdfs
      .mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
      .mockResolvedValueOnce({ path: '@vfs/session', node: node({ name: 'session' }), items: sessionListing() })
      .mockResolvedValueOnce({
        path: '@vfs/session/s1',
        node: node({ name: 's1' }),
        items: [node({ name: 'message', kind: VDFS_KIND_MESSAGES }), node({ name: 'workdir' })],
      })

    await expect(ensureVdfsSessionScheme()).rejects.toThrow(/没有 kind=inbox/)
  })
})

describe('按挂载目录缓存：子智能体空间是另一份方案', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
    vdfs.statVdfs.mockReset()
  })

  it('传挂载目录 = 解析那个空间那份；与默认那份互不覆盖', async () => {
    const SUB = '@vfs/agent/reviewer/session'
    // 子空间的会话内部：段名与根空间**可以不同**（都是展示名）
    vdfs.listVdfs
      .mockResolvedValueOnce({
        path: SUB,
        node: node({ name: 'session' }),
        items: [node({ name: 'sub-1', kind: 'session', ext: VDFS_EXT_SESSION })],
      })
      .mockResolvedValueOnce({
        path: `${SUB}/sub-1`,
        node: node({ name: 'sub-1' }),
        items: [
          node({ name: 'transcript', kind: VDFS_KIND_MESSAGES }),
          node({ name: 'pending', kind: VDFS_KIND_INBOX }),
        ],
      })

    const sub = await ensureSessionScheme(SUB)

    expect(sub).toEqual({ mountDir: SUB, messagesSeg: 'transcript', inboxSeg: 'pending' })
    // 只列了子空间那两处，没有去碰根清单
    expect(vdfs.listVdfs.mock.calls.map((c) => c[1])).toEqual([SUB, `${SUB}/sub-1`])
    // 子空间那份缓存住了：`vdfsSessionScheme(SUB)` 同步读得到
    expect(vdfsSessionScheme(SUB)).toEqual(sub)
    // 而**默认**那份还没解析过，仍不可用（不会被子空间那份顶替）
    expect(vdfsSessionScheme()).toBeNull()
  })
})

describe('地址拼接：后端两种口径都不能拼重', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
    vdfs.statVdfs.mockReset()
  })

  it('path 已是全路径时直接用（曾拼成 <根>/<根>/session ⇒ 读取转写 404）', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({
      path: '@vfs',
      node: node({ name: '' }),
      // 真实口径：根清单里挂载点的 path 就是展示全路径
      items: [
        node({ name: 'session', path: '@vfs/session', new_type: { ext: VDFS_EXT_SESSION, title: '会话' } }),
      ],
    })

    await expect(ensureSessionMountDir()).resolves.toBe('@vfs/session')
  })

  it('path 是段名时按父地址拼', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({
      path: '@vfs',
      node: node({ name: '' }),
      items: [
        node({ name: 'session', path: 'session', new_type: { ext: VDFS_EXT_SESSION, title: '会话' } }),
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
      inboxSeg: 'inbox',
    })
    // 列会话内部用的是会话自己的全路径，不是把它再挂到挂载目录下
    expect(vdfs.listVdfs.mock.calls[2][1]).toBe('@vfs/session/s1')
  })
})

describe('vdfsSessionScheme：同步读（事件回调用）', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
    vdfs.statVdfs.mockReset()
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
    expect(vdfsSessionScheme()).toEqual({
      mountDir: '@vfs/session',
      messagesSeg: 'message',
      inboxSeg: 'inbox',
    })
  })
})

/**
 * `isSessionMountDir` —— 判断「这个地址是不是会话挂载目录」。
 *
 * 它存在的理由是一类**地址形态混淆**：会话节点自己的 `ext` 也是 `session`
 * （所以都归会话渲染器），但只有挂载目录的 `new_type.ext` 是 `session`。
 * 把这个判据写错，前端就会拿 `<挂载目录>/<id>`（甚至 `<id>/subsession`）当空间，
 * 后续每次寻址都按 `<那个地址>/<id>/…` 拼，得到一串
 * 「会话内部不存在该路径」「没有 kind=messages 的子节点」。
 */
describe('isSessionMountDir：按父目录自述分辨挂载目录与会话叶子', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
    vdfs.statVdfs.mockReset()
  })

  it('声明 new_type.ext = session 的目录 = 挂载目录', async () => {
    vdfs.statVdfs.mockResolvedValue(
      node({ name: 'session', new_type: { ext: VDFS_EXT_SESSION, title: '会话' } }),
    )

    await expect(isSessionMountDir('@vfs/session')).resolves.toBe(true)
    expect(vdfs.statVdfs).toHaveBeenCalledWith(READBACK_REASON.BOOTSTRAP, '@vfs/session')
  })

  it('会话节点（ext = session 但不是目录）与子会话清单都不是挂载目录', async () => {
    // 会话叶子：ext 撞了，但没有 new_type（new_type 是「这里能新建什么」的声明）
    vdfs.statVdfs.mockResolvedValueOnce(node({ name: 's1', kind: 'session', ext: VDFS_EXT_SESSION }))
    await expect(isSessionMountDir('@vfs/session')).resolves.toBe(false)

    // 子会话清单：kind 自成一类，同样没有 new_type
    vdfs.statVdfs.mockResolvedValueOnce(node({ name: 'subsession', kind: 'subsession' }))
    await expect(isSessionMountDir('@vfs/session/s1/subsession')).resolves.toBe(false)
  })

  it('自述读不到时返回 null（不否决）—— 一次 IPC 抖动不该把正常会话判成不可用', async () => {
    vdfs.statVdfs.mockResolvedValueOnce(null)
    await expect(isSessionMountDir('@vfs/session')).resolves.toBeNull()
  })

  it('已解析过的挂载不必再 stat（根空间那份 / 各子空间那份都在缓存里）', async () => {
    vdfs.listVdfs.mockResolvedValueOnce({ path: '@vfs', node: node({ name: '' }), items: rootListing() })
    await ensureSessionMountDir()

    await expect(isSessionMountDir('@vfs/session')).resolves.toBe(true)
    expect(vdfs.statVdfs).not.toHaveBeenCalled()
  })
})

/**
 * `sessionSpaceOf` —— 一个会话条目**属于哪个空间**。
 *
 * 这是「子会话打不开」的修复点：子会话的地址是
 * `<挂载目录>/<父会话>/subsession/<子>`，比普通会话多嵌两段。若拿父目录当空间
 * （`parseVdfsSessionAddr` 的机械切分就是这样），后续每一次寻址都按一个不存在的
 * 空间拼，得到「会话内部不存在该路径」「没有 kind=messages 的子节点」。
 *
 * 判据必须只靠**节点自述**（`new_type` 只有挂载目录才有）——前端不认识
 * `subsession` 这个段名，也不该认识。
 */
describe('sessionSpaceOf：从条目向上认它所属的会话空间', () => {
  beforeEach(() => {
    resetVdfsSessionScheme()
    vdfs.listVdfs.mockReset()
    vdfs.statVdfs.mockReset()
  })

  it('普通会话：父目录就是挂载目录（一次 stat 即可）', async () => {
    vdfs.statVdfs.mockResolvedValueOnce(
      node({ name: 'session', new_type: { ext: VDFS_EXT_SESSION, title: '会话' } }),
    )

    await expect(sessionSpaceOf('@vfs/session/s1')).resolves.toBe('@vfs/session')
    expect(vdfs.statVdfs).toHaveBeenCalledTimes(1)
    expect(vdfs.statVdfs).toHaveBeenCalledWith(READBACK_REASON.BOOTSTRAP, '@vfs/session')
  })

  it('子会话：越过 `subsession` 与父会话两层，落到挂载目录', async () => {
    // 第一跳：子会话清单目录（没有 new_type ⇒ 不是空间）
    vdfs.statVdfs.mockResolvedValueOnce(node({ name: 'subsession', kind: 'subsession' }))
    // 第二跳：父会话（ext = session 但**没有** new_type，仍不是空间）
    vdfs.statVdfs.mockResolvedValueOnce(
      node({ name: 'p1', kind: 'session', ext: VDFS_EXT_SESSION }),
    )
    // 第三跳：挂载目录
    vdfs.statVdfs.mockResolvedValueOnce(
      node({ name: 'session', new_type: { ext: VDFS_EXT_SESSION, title: '会话' } }),
    )

    await expect(sessionSpaceOf('@vfs/session/p1/subsession/s2')).resolves.toBe('@vfs/session')
  })

  it('自述读不到 ⇒ null（调用方不得据此否决，见 isSessionMountDir 的约定）', async () => {
    vdfs.statVdfs.mockResolvedValueOnce(null)
    await expect(sessionSpaceOf('@vfs/session/s1')).resolves.toBeNull()
  })

  it('一路到根都不是空间 ⇒ null（不无限向上）', async () => {
    vdfs.statVdfs.mockResolvedValue(node({ name: 'x' }))
    await expect(sessionSpaceOf('@vfs/a/b/c')).resolves.toBeNull()
  })
})
