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
import { VDFS_ACTION, VDFS_WRITE, vdfsJoin } from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')
import { arrayBufferToBase64, base64ToBytes, runVdfsAction, writeVdfsBinary } from '../vdfs'
import { vdfsChangeInScope } from '../eventBus'

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

describe('整包导入（vdfs/write 的二进制通道）', () => {
  it('arrayBufferToBase64 与标准 base64 一致（含分块边界）', () => {
    const bytes = new Uint8Array(0x8000 + 5) // 跨过 32KB 分块
    for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251
    expect(arrayBufferToBase64(bytes.buffer)).toBe(Buffer.from(bytes).toString('base64'))
  })

  it('writeVdfsBinary 原样发送 b64（后端据 b64 判定二进制）', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({ path: '@vfs/skill/demo.zip', created: true })
    const r = await writeVdfsBinary(
      vdfsJoin(vdfsJoin('@vfs', 'skill'), 'demo.zip'),
      'UEsDBA==',
      { create: true }
    )

    expect(r.created).toBe(true)
    expect(lastCall().op).toBe(VDFS_WRITE)
    expect(lastCall().payload).toEqual({
      path: '@vfs/skill/demo.zip',
      b64: 'UEsDBA==',
      create: true,
    })
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
    expect(vdfsChangeInScope({ prefix: SESSIONS }, `${SESSIONS}/abc/消息/m1`)).toBe(true)
  })

  it('别人的路径不算命中（前缀必须整段匹配，不是字符串前缀）', () => {
    expect(vdfsChangeInScope({ prefix: SESSIONS }, vdfsJoin('@vfs', 'model/openai'))).toBe(false)
    expect(vdfsChangeInScope({ prefix: SESSIONS }, '@vfs/session-templates/x')).toBe(false)
  })

  it('directChildren 只放行直接子项（会话叶子），挡住更深的区段', () => {
    const scope = { prefix: SESSIONS, directChildren: true }
    expect(vdfsChangeInScope(scope, `${SESSIONS}/abc`)).toBe(true)
    expect(vdfsChangeInScope(scope, `${SESSIONS}/abc/消息/m1`)).toBe(false)
    // 前缀自身不是「子项」
    expect(vdfsChangeInScope(scope, SESSIONS)).toBe(false)
  })

  it('尾部斜杠不影响判定', () => {
    expect(vdfsChangeInScope({ prefix: `${SESSIONS}/` }, `${SESSIONS}/abc`)).toBe(true)
  })
})
