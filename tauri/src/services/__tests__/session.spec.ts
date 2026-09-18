/**
 * session 服务 —— 会话清单映射 + 会话级写入路由单测（node 环境）
 *
 * `listSessions()` 走 `vdfs/list`：读 `.vdfs/session` 的目录内容。
 * 本单测锁定「VdfsNode → SessionListItem」的映射口径：
 * 会话侧栏的标题 / 工作目录 / 运行中状态全部依赖它，一旦后端字段名或前端取值
 * 方式漂移，表现为**静默退化**（侧栏拿不到 workdir、停止按钮失效），很难肉眼发现。
 *
 * 另锁定两条**会话级写入**的落点：删除 → `vdfs/delete`，改 metadata → `vdfs/write`。
 * 这两条曾经各有专用路由（`session/clear` / `session/update`），已并入 VDFS——
 * 若哪天有人把路由改回去，这里会红。断言**地址**而不只是"调用了某个函数"：
 * 地址拼错（比如少了会话 id）在真实环境里表现为删错会话，是灾难级的。
 */

import { describe, expect, it, vi } from 'vitest'

// 服务层依赖 Tauri API 与本地存储，这里只测映射逻辑，故整体替身
vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn() }))
vi.mock('@/services/vdfs', () => ({
  listVdfs: vi.fn(),
  deleteVdfs: vi.fn(),
  writeVdfs: vi.fn(),
}))

import { deleteVdfs, listVdfs, writeVdfs } from '@/services/vdfs'
import {
  VDFS_ROOT,
  isWorkingStatus,
  vdfsJoin,
  vdfsSessionAddr,
  type VdfsNode,
} from '@/schemas/vdfs'
import { deleteSession, listSessions, updateSession } from '../session'

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
    path: vdfsJoin(VDFS_ROOT, 'session'),
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

    expect(vi.mocked(listVdfs)).toHaveBeenCalledWith(vdfsJoin(VDFS_ROOT, 'session'))
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
    mockList([sessionNode({ status: 'working', updated_at: undefined })])

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
    expect(vi.mocked(listVdfs)).toHaveBeenCalledWith(vdfsJoin(VDFS_ROOT, 'session'))

    mockList([])
    await listSessions(50)
    expect(vi.mocked(listVdfs)).toHaveBeenCalledWith(vdfsJoin(VDFS_ROOT, 'session'), {
      limit: 50,
    })
  })

  it('挂载根下与资源并列的配置文件不进清单（按 ext 判据，不按名字特判）', async () => {
    mockList([
      sessionNode({ name: 'abc' }),
      // 本插件的配置文件：真实文件名 `PLUGIN.yml`，ext = form（见 docs/design/vdfs.md §3.4）
      sessionNode({ name: 'PLUGIN.yml', title: '会话设置', ext: 'form' }),
    ])

    const out = await listSessions()

    expect(out).toHaveLength(1)
    expect(out[0].id).toBe('abc')
  })
})

describe('deleteSession（会话级删除 → vdfs/delete）', () => {
  it('删除会话走 VDFS 地址，而不是旧 session/clear 路由', async () => {
    vi.mocked(deleteVdfs).mockResolvedValueOnce({ path: vdfsSessionAddr('abc') })

    await deleteSession('abc')

    expect(vi.mocked(deleteVdfs)).toHaveBeenCalledWith(vdfsSessionAddr('abc'))
    // 地址必须**带会话 id**：少了 id 就是删整个会话挂载根（灾难级）
    expect(vdfsSessionAddr('abc')).toBe('.vdfs/session/abc')
  })

  it('删除失败向上抛（调用方据此回滚本地清单）', async () => {
    vi.mocked(deleteVdfs).mockRejectedValueOnce(new Error('NotFound'))
    await expect(deleteSession('gone')).rejects.toThrow('NotFound')
  })
})

describe('updateSession（会话 metadata → vdfs/write）', () => {
  it('metadata 写入走 VDFS 地址，载荷是 { metadata }', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      path: vdfsSessionAddr('abc'),
      created: false,
    })

    await updateSession('abc', { workdir: '/w' })

    // 用 lastCall：mock 的 calls 跨用例累积，取 [0] 会读到上一条用例的调用
    const [addr, body] = vi.mocked(writeVdfs).mock.lastCall!
    expect(addr).toBe(vdfsSessionAddr('abc'))
    // 载荷形状必须与后端 provider 的 `write` 约定一致（它只认 metadata / title）
    expect(JSON.parse(body as string)).toEqual({ metadata: { workdir: '/w' } })
  })

  it('title 与 metadata 同时给出时同帧写入（后端浅合并，两者互不覆盖）', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      path: vdfsSessionAddr('abc'),
      created: false,
    })

    await updateSession('abc', { workdir: '/w' }, '新标题')

    const body = JSON.parse(vi.mocked(writeVdfs).mock.lastCall![1] as string)
    expect(body).toEqual({ metadata: { workdir: '/w' }, title: '新标题' })
  })

  it('不给 title 时**不带**该键（否则后端会把标题清成空）', async () => {
    vi.mocked(writeVdfs).mockResolvedValueOnce({
      path: vdfsSessionAddr('abc'),
      created: false,
    })

    await updateSession('abc', { workdir: '/w' })

    const body = JSON.parse(vi.mocked(writeVdfs).mock.lastCall![1] as string)
    expect(body).not.toHaveProperty('title')
  })
})
