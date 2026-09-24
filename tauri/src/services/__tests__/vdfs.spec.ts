/**
 * vdfs 服务 —— 节点动作（`vdfs/action`）单测（node 环境）
 *
 * S10 新增的统一动作口：「测试连接」这类能力由 provider 自持，前端只转发标识。
 * 本单测锁定两件容易静默漂移的事：
 * 1. 地址与后端同一套口径（根锚点打头 = 系统资源，原样透传、零翻译）；
 * 2. 动作标识与载荷**原样**透传——前端不解释语义，也不擅自补字段。
 */

import { describe, expect, it, vi } from 'vitest'

vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn(), connectPlugin: vi.fn() }))
// 日志模块依赖浏览器环境，这里只测透传逻辑，故整体替身
vi.mock('@/utils/logger', () => ({ logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() } }))

import { callPlugin } from '@/services/plugin'
import { VDFS_ACTION, vdfsJoin } from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')
import { arrayBufferToBase64, base64ToBytes, listVdfs, readVdfs, runVdfsAction, statVdfs } from '../vdfs'
import { READBACK_REASON } from '../readback'
import { vdfsChangeInScope } from '../eventBus'
import { logger } from '@/utils/logger'

/** 最近一次插件调用（op + 载荷） */
function lastCall(): { op: string; payload: Record<string, unknown> } {
  const calls = vi.mocked(callPlugin).mock.calls
  const last = calls[calls.length - 1]
  return { op: last[0] as string, payload: (last[1] ?? {}) as Record<string, unknown> }
}

describe('runVdfsAction（vdfs/action）', () => {
  it('地址原样透传（前后端同口径），动作标识原样透传', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({
      action: 'test',
      ok: true,
      message: '校验通过',
    })
    const path = vdfsJoin(vdfsJoin('@vfs', 'model'), 'openai-gpt4o')
    const r = await runVdfsAction(path, 'test')

    expect(r.ok).toBe(true)
    expect(r.message).toBe('校验通过')
    expect(lastCall().op).toBe(VDFS_ACTION)
    expect(lastCall().payload).toEqual({
      path: '@vfs/model/openai-gpt4o',
      action: 'test',
    })
  })

  it('无载荷时不发送 payload 字段（后端按 Option 处理）', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({ action: 'test', ok: false, message: '失败' })
    await runVdfsAction(vdfsJoin(vdfsJoin('@vfs', 'mcp'), 'github'), 'test')

    expect(lastCall().payload).toEqual({ path: '@vfs/mcp/github', action: 'test' })
  })

  it('有载荷时原样透传（前端不解释其内容）', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({ action: 'test', ok: true, message: 'ok' })
    await runVdfsAction(vdfsJoin(vdfsJoin('@vfs', 'mcp'), 'github'), 'test', { verbose: true })

    expect(lastCall().payload).toEqual({
      path: '@vfs/mcp/github',
      action: 'test',
      payload: { verbose: true },
    })
  })
})

describe('base64 编解码（文件载荷的两个方向）', () => {
  it('arrayBufferToBase64 与标准 base64 一致（含分块边界）', () => {
    const bytes = new Uint8Array(0x8000 + 5) // 跨过 32KB 分块
    for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251
    expect(arrayBufferToBase64(bytes.buffer)).toBe(Buffer.from(bytes).toString('base64'))
  })

  it('base64ToBytes 与 arrayBufferToBase64 互逆（导出落地的字节必须原样）', () => {
    const bytes = new Uint8Array(0x8000 + 5)
    for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251
    const b64 = arrayBufferToBase64(bytes.buffer)
    expect(Array.from(base64ToBytes(b64))).toEqual(Array.from(bytes))
  })
})

/**
 * 变更订阅的作用域判定 —— VDFS 是唯一频道，「哪一类资源」全靠路径前缀分流。
 * 判错的后果是**静默漏事件**（该刷新的不刷新），故直接测谓词本身。
 */
describe('vdfsChangeInScope（按展示地址前缀分流）', () => {
  const SESSIONS = vdfsJoin('@vfs', 'session')

  it('前缀本身与子树内的变更都算命中', () => {
    expect(vdfsChangeInScope({ prefix: SESSIONS }, SESSIONS)).toBe(true)
    expect(vdfsChangeInScope({ prefix: SESSIONS }, `${SESSIONS}/abc`)).toBe(true)
    expect(vdfsChangeInScope({ prefix: SESSIONS }, `${SESSIONS}/abc/message/m1`)).toBe(true)
  })

  it('别人的路径不算命中（前缀必须整段匹配，不是字符串前缀）', () => {
    expect(vdfsChangeInScope({ prefix: SESSIONS }, vdfsJoin('@vfs', 'model/openai'))).toBe(false)
    expect(vdfsChangeInScope({ prefix: SESSIONS }, '@vfs/session-templates/x')).toBe(false)
  })

  it('directChildren 只放行直接子项（会话叶子），挡住更深的区段', () => {
    const scope = { prefix: SESSIONS, directChildren: true }
    expect(vdfsChangeInScope(scope, `${SESSIONS}/abc`)).toBe(true)
    expect(vdfsChangeInScope(scope, `${SESSIONS}/abc/message/m1`)).toBe(false)
    // 前缀自身不是「子项」
    expect(vdfsChangeInScope(scope, SESSIONS)).toBe(false)
  })

  it('尾部斜杠不影响判定', () => {
    expect(vdfsChangeInScope({ prefix: `${SESSIONS}/` }, `${SESSIONS}/abc`)).toBe(true)
  })
})

/**
 * 读 / 列入口的**失败口径**：吞错返兜底（`services/fallback.ts` 的 `withFallback`）。
 *
 * 这一组是在把三处手写 `try/catch → logger → 兜底` 收进原语之后补的——原语化是
 * 行为保持的重构，但**没有测试就证明不了"保持"**：兜底值（空目录 vs `null`）、
 * 日志级别（error vs debug）都是可陈述的口径，必须逐条钉住。
 */
describe('listVdfs / statVdfs / readVdfs 的失败口径', () => {
  const mocked = vi.mocked(callPlugin)
  const logError = vi.mocked(logger.error)
  const logDebug = vi.mocked(logger.debug)

  it('listVdfs 失败 → 形状合法的空目录（页面照常渲染），日志 error', async () => {
    mocked.mockReset()
    logError.mockReset()
    mocked.mockRejectedValueOnce(new Error('ipc down'))
    const r = await listVdfs(READBACK_REASON.VDFS_BROWSER, vdfsJoin('@vfs', 'session'))
    // 关键：不是 null，而是**同型**的空列表——调用方无需分支
    expect(r).toEqual({
      path: vdfsJoin('@vfs', 'session'),
      node: expect.objectContaining({ path: vdfsJoin('@vfs', 'session') }),
      items: [],
    })
    expect(logError).toHaveBeenCalled()
  })

  it('listVdfs 后端返回 falsy → 同样给空目录（不是把 undefined 透出去）', async () => {
    mocked.mockReset()
    mocked.mockResolvedValueOnce(null)
    const r = await listVdfs(READBACK_REASON.VDFS_BROWSER, vdfsJoin('@vfs', 'session'))
    expect(r.items).toEqual([])
    expect(r.path).toBe(vdfsJoin('@vfs', 'session'))
  })

  it('listVdfs 成功 → 原样返回（不掺兜底）', async () => {
    mocked.mockReset()
    const ok = { path: '/p', node: { path: '/p' }, items: [{ path: '/p/a' }] }
    mocked.mockResolvedValueOnce(ok)
    await expect(listVdfs(READBACK_REASON.VDFS_BROWSER, '/p')).resolves.toBe(ok)
  })

  it('statVdfs 失败 → null，且降为 **debug**（节点不存在是预期内的失败）', async () => {
    mocked.mockReset()
    logDebug.mockReset()
    logError.mockReset()
    mocked.mockRejectedValueOnce(new Error('not found'))
    await expect(statVdfs(READBACK_REASON.VDFS_BROWSER, '/nope')).resolves.toBeNull()
    expect(logDebug).toHaveBeenCalled()
    // 预期内的失败不得记 error——否则日志失去信噪比，盖住真正的故障
    expect(logError).not.toHaveBeenCalled()
  })

  it('readVdfs 失败 → null，日志 error（读不到正文不是预期内的事）', async () => {
    mocked.mockReset()
    logError.mockReset()
    mocked.mockRejectedValueOnce(new Error('boom'))
    await expect(readVdfs(READBACK_REASON.VDFS_BROWSER, '/p')).resolves.toBeNull()
    expect(logError).toHaveBeenCalled()
  })
})
