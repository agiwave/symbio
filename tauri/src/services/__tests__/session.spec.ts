/**
 * session 服务 —— 会话清单映射 + 全部写入落点单测（node 环境）
 *
 * `listSessions()` 走 `vdfs/list`：读 `<根>/session` 的目录内容。
 * 本单测锁定「VdfsNode → SessionListItem」的映射口径：
 * 会话侧栏的标题 / 工作目录 / 运行中状态全部依赖它，一旦后端字段名或前端取值
 * 方式漂移，表现为**静默退化**（侧栏拿不到 workdir、停止按钮失效），很难肉眼发现。
 *
 * 另锁定**全部五个写入落点**——它们曾经各有专用路由（`session/clear` /
 * `session/update` / `chat/update_message` / `chat/delete_message` /
 * `chat/clear_messages`），2026-09-18 起全部并入 VDFS（`session/update`
 * 于 2026-09-23 最后一条退役，会话与消息的 CRUD 至此全在 VDFS 上）：
 *
 * | 操作 | 落点 |
 * |---|---|
 * | 删除会话 | `vdfs/delete(<根>/session/<id>)` |
 * | 改 metadata / 标题 | `vdfs/write(<根>/session/<id>)` |
 * | 改写某条消息 | `vdfs/write(…/message/<mid>)` |
 * | 删该条及其后 | `vdfs/action(…/message/<mid>, "truncate")` |
 * | 清空历史 | `vdfs/action(…/message, "clear")` |
 *
 * 若哪天有人把路由改回去，这里会红。断言**地址**而不只是"调用了某个函数"：
 * 地址拼错（比如少了会话 id）在真实环境里表现为删错会话，是灾难级的。
 */

import { describe, expect, it, vi } from 'vitest'

/**
 * 协议夹具：挂载目录 + 转写段（与后端 provider 的现实取值一致）。
 *
 * `vi.hoisted` 是必须的：`vi.mock` 的工厂会被提升到 import 之前执行，
 * 直接引用模块顶层的 `const` 会撞 TDZ。
 */
const { SCHEME } = vi.hoisted(() => ({
  SCHEME: { mountDir: '@vfs/session', messagesSeg: 'message', inboxSeg: 'inbox' },
}))

// 服务层依赖 Tauri API 与本地存储，这里只测映射逻辑，故整体替身
vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn() }))
vi.mock('@/services/vdfs', () => ({
  listVdfs: vi.fn(),
  deleteVdfs: vi.fn(),
  writeVdfs: vi.fn(),
  runVdfsAction: vi.fn(),
}))
// 地址方案是**运行期数据**（列目录认出来）；单测不去列目录，直接注入协议夹具。
// 夹具本身是断言的一部分：它把「删除/写入落在哪个地址」钉成字面量。
// `ensureSessionScheme(mountDir?)` 省略参数 = 默认挂载目录那份（与
// `ensureSessionMountDir()` 同源），传参 = 那个空间那份——两种都返回夹具。
vi.mock('@/services/vdfsScheme', () => ({
  ensureSessionMountDir: vi.fn(async () => SCHEME.mountDir),
  ensureSessionScheme: vi.fn(async () => SCHEME),
  vdfsSessionScheme: vi.fn(() => SCHEME),
}))

import { deleteVdfs, listVdfs, runVdfsAction, writeVdfs } from '@/services/vdfs'
// 回读理由是**词表**（独立模块，未被替身），断言按它取值——替身里不抄第二份
import { READBACK_REASON } from '../readback'
import {
  VDFS_ACTION_CLEAR,
  VDFS_ACTION_TRUNCATE,
  isWorkingStatus,
  vdfsJoin,
  vdfsMessageAddr,
  vdfsMessagesAddr,
  vdfsSessionAddr,
  type VdfsItem,
} from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')
import {
  clearMessages,
  deleteMessage,
  deleteSession,
  listSessions,
  updateMessage,
  updateSession,
  type SessionMessage,
} from '../session'

/** 构造一个会话**条目**（`path` + 节点自述；attributes 为 flatten 的场景字段） */
function sessionItem(over: Partial<VdfsItem> = {}): VdfsItem {
  return {
    path: vdfsJoin('@vfs/session', 'abc'),
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

function mockList(items: VdfsItem[]) {
  vi.mocked(listVdfs).mockResolvedValueOnce({
    path: vdfsJoin('@vfs', 'session'),
    node: sessionItem({ name: 'session', title: '会话' }),
    items,
  })
}

describe('listSessions（<根>/session → SessionListItem）', () => {
  it('请求会话挂载点根，并映射全部字段', async () => {
    mockList([
      sessionItem({
        message_count: 12,
        metadata: { workdir: '/tmp/demo', title: 'T' },
        meta_tags: ['demo', '12 条'],
      }),
    ])

    const out = await listSessions()

    expect(vi.mocked(listVdfs)).toHaveBeenCalledWith(
      READBACK_REASON.LIST_REFRESH,
      vdfsJoin('@vfs', 'session')
    )
    expect(out).toHaveLength(1)
    expect(out[0]).toMatchObject({
      id: 'abc',
      name: '会话标题',
      message_count: 12,
      updated_at: 1_700_000_000,
      status: 'active',
      metadata: { workdir: '/tmp/demo', title: 'T' },
    })
  })

  it('节点 status 原样透传（不压缩成布尔）；缺省字段回落空态', async () => {
    mockList([sessionItem({ status: 'working', updated_at: undefined })])

    const [first] = await listSessions()

    // 运行态就是节点 status 的一个取值，边界上不折算成 is_working
    expect(first.status).toBe('working')
    expect(isWorkingStatus(first.status)).toBe(true)
    expect(first.updated_at).toBe(0)
    expect(first.message_count).toBe(0)
    expect(first.metadata).toEqual({})
  })

  it('空清单返回空数组（不抛错、不造占位项）', async () => {
    mockList([])
    await expect(listSessions()).resolves.toEqual([])
  })

  // 有界列表是**可选能力**：不传参时请求形状必须与从前逐字节一致。
  // 曾经 `listSessions()` 默认就带 `{ limit: 100 }`，既有断言（上面那条
  // toHaveBeenCalledWith(path)）因此失败——这条把这个契约钉死。
  it('只有显式传 limit 才带窗口参数；不传时请求形状不变', async () => {
    mockList([])
    await listSessions()
    expect(vi.mocked(listVdfs)).toHaveBeenCalledWith(
      READBACK_REASON.LIST_REFRESH,
      vdfsJoin('@vfs', 'session')
    )

    mockList([])
    await listSessions(50)
    expect(vi.mocked(listVdfs)).toHaveBeenCalledWith(
      READBACK_REASON.LIST_REFRESH,
      vdfsJoin('@vfs', 'session'),
      { limit: 50 }
    )
  })

  it('挂载根下与资源并列的配置文件不进清单（按 ext 判据，不按名字特判）', async () => {
    mockList([
      sessionItem({ name: 'abc' }),
      // 本插件的配置文件：真实文件名 `PLUGIN.yml`，ext = form（见 docs/design/vdfs.md §3.4）
      sessionItem({ name: 'PLUGIN.yml', title: '会话设置', ext: 'form' }),
    ])

    const out = await listSessions()

    expect(out).toHaveLength(1)
    expect(out[0].id).toBe('abc')
  })
})

describe('deleteSession（会话级删除 → vdfs/delete）', () => {
  it('删除会话走 VDFS 地址，而不是旧 session/clear 路由', async () => {
    // 删除**无回执载荷**（删哪儿是调用方自己说的），故只解一个 void
    vi.mocked(deleteVdfs).mockResolvedValueOnce(undefined)

    await deleteSession('abc')

    expect(vi.mocked(deleteVdfs)).toHaveBeenCalledWith(vdfsSessionAddr(SCHEME.mountDir, 'abc'))
    // 地址必须**带会话 id**：少了 id 就是删整个会话挂载根（灾难级）
    expect(vdfsSessionAddr(SCHEME.mountDir, 'abc')).toBe('@vfs/session/abc')
  })

  it('删除失败向上抛（调用方据此回滚本地清单）', async () => {
    vi.mocked(deleteVdfs).mockRejectedValueOnce(new Error('NotFound'))
    await expect(deleteSession('gone')).rejects.toThrow('NotFound')
  })
})

describe('updateSession（会话 metadata → vdfs/write）', () => {
  it('metadata 写入走 VDFS 地址，载荷是 { metadata }', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      created: false,
    })

    await updateSession('abc', { workdir: '/w' })

    // 用 lastCall：mock 的 calls 跨用例累积，取 [0] 会读到上一条用例的调用
    const [addr, body] = vi.mocked(writeVdfs).mock.lastCall!
    expect(addr).toBe(vdfsSessionAddr(SCHEME.mountDir, 'abc'))
    // 载荷形状必须与后端 provider 的 `write` 约定一致（它只认 metadata / title）
    expect(JSON.parse(body as string)).toEqual({ metadata: { workdir: '/w' } })
  })

  it('title 与 metadata 同时给出时同帧写入（后端浅合并，两者互不覆盖）', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      created: false,
    })

    await updateSession('abc', { workdir: '/w' }, '新标题')

    const body = JSON.parse(vi.mocked(writeVdfs).mock.lastCall![1] as string)
    expect(body).toEqual({ metadata: { workdir: '/w' }, title: '新标题' })
  })

  it('不给 title 时**不带**该键（否则后端会把标题清成空）', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      created: false,
    })

    await updateSession('abc', { workdir: '/w' })

    const body = JSON.parse(vi.mocked(writeVdfs).mock.lastCall![1] as string)
    expect(body).not.toHaveProperty('title')
  })
})

describe('updateMessage（改写某条消息 → vdfs/write）', () => {
  it('写**单条消息**地址，载荷是消息 JSON', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      created: false,
    })

    const patch: SessionMessage = { id: 'm1', content: '改过的' }
    await updateMessage('abc', patch)

    const [addr, body] = vi.mocked(writeVdfs).mock.lastCall!
    expect(addr).toBe(vdfsMessageAddr(SCHEME, 'abc', 'm1'))
    // 地址必须**指到那一条**：少一层就落到列表上（后端会拒），再少一层就是
    // 会话 metadata（会静默写错地方——那才是最坏的结果）
    expect(vdfsMessageAddr(SCHEME, 'abc', 'm1')).toBe('@vfs/session/abc/message/m1')
    expect(JSON.parse(body as string)).toEqual({ id: 'm1', content: '改过的' })
  })

  it('地址取自 message.id —— 载荷与地址必须指同一条', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      created: false,
    })

    await updateMessage('abc', { id: 'm7' })

    expect(vi.mocked(writeVdfs).mock.lastCall![0]).toBe(vdfsMessageAddr(SCHEME, 'abc', 'm7'))
  })
})

describe('deleteMessage（删该条及其后 → action("truncate")）', () => {
  it('地址是**单条消息**，动作是 truncate（不是 delete）', async () => {
    vi.mocked(runVdfsAction).mockResolvedValueOnce({
      action: VDFS_ACTION_TRUNCATE,
      ok: true,
      message: '已截断 2 条',
      data: ['m2', 'm3'],
    })

    await deleteMessage('abc', 'm2')

    expect(vi.mocked(runVdfsAction)).toHaveBeenCalledWith(
      vdfsMessageAddr(SCHEME, 'abc', 'm2'),
      VDFS_ACTION_TRUNCATE
    )
    expect(vdfsMessageAddr(SCHEME, 'abc', 'm2')).toBe('@vfs/session/abc/message/m2')
  })

  it('回执原样透传：store 靠它做幂等对齐', async () => {
    vi.mocked(runVdfsAction).mockResolvedValueOnce({
      action: VDFS_ACTION_TRUNCATE,
      ok: true,
      message: '已截断 2 条',
      data: ['m2', 'm3'],
    })

    await expect(deleteMessage('abc', 'm2')).resolves.toEqual({
      deleted_ids: ['m2', 'm3'],
    })
  })

  it('data 里的非字符串被滤掉（不把脏数据当消息 id 用）', async () => {
    vi.mocked(runVdfsAction).mockResolvedValueOnce({
      action: VDFS_ACTION_TRUNCATE,
      ok: true,
      message: 'ok',
      data: [1, null, 'm3', { id: 'm4' }],
    })

    await expect(deleteMessage('abc', 'm1')).resolves.toEqual({ deleted_ids: ['m3'] })
  })

  it('data 缺失 ⇒ 空列表（目标不存在 = 什么都没删）', async () => {
    vi.mocked(runVdfsAction).mockResolvedValueOnce({
      action: VDFS_ACTION_TRUNCATE,
      ok: true,
      message: '目标消息不存在，未做任何修改',
    })

    await expect(deleteMessage('abc', 'gone')).resolves.toEqual({ deleted_ids: [] })
  })
})

describe('clearMessages（清空历史 → action("clear")）', () => {
  it('地址是**消息列表目录**（不是单条、也不是会话本体），动作是 clear', async () => {
    vi.mocked(runVdfsAction).mockResolvedValueOnce({
      action: VDFS_ACTION_CLEAR,
      ok: true,
      message: '已清空会话历史',
    })

    await clearMessages('abc')

    expect(vi.mocked(runVdfsAction)).toHaveBeenCalledWith(
      vdfsMessagesAddr(SCHEME, 'abc'),
      VDFS_ACTION_CLEAR
    )
    // 少一层就清到会话本体（元数据 / 标题一并没了），多一层就不是列表
    expect(vdfsMessagesAddr(SCHEME, 'abc')).toBe('@vfs/session/abc/message')
  })

  it('失败向上抛（调用方据此不做本地清空）', async () => {
    vi.mocked(runVdfsAction).mockRejectedValueOnce(new Error('Forbidden'))
    await expect(clearMessages('abc')).rejects.toThrow('Forbidden')
  })
})
