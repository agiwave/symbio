/**
 * vdfs 服务 —— 节点动作（`vdfs/action`）单测（node 环境）
 *
 * S10 新增的统一动作口：「测试连接」这类能力由 provider 自持，前端只转发标识。
 * 本单测锁定两件容易静默漂移的事：
 * 1. 地址与其它操作同一套路径运算（前端 `.vdfs` 口径 → 线路 `/` 口径）；
 * 2. 动作标识与载荷**原样**透传——前端不解释语义，也不擅自补字段。
 */

import { describe, expect, it, vi } from 'vitest'

vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn() }))
// 日志模块依赖浏览器环境，这里只测翻译逻辑，故整体替身
vi.mock('@/utils/logger', () => ({ logger: { error: vi.fn(), warn: vi.fn() } }))

import { callPlugin } from '@/services/plugin'
import { VFDS_ACTION, VFDS_ROOT, VFDS_WRITE, vdfsJoin } from '@/schemas/vdfs'
import { arrayBufferToBase64, runVdfsAction, writeVdfsBinary } from '../vdfs'

/** 最近一次插件调用（op + 载荷） */
function lastCall(): { op: string; payload: Record<string, unknown> } {
  const calls = vi.mocked(callPlugin).mock.calls
  const last = calls[calls.length - 1]
  return { op: last[0] as string, payload: (last[1] ?? {}) as Record<string, unknown> }
}

describe('runVdfsAction（vdfs/action）', () => {
  it('按线路口径发送路径，动作标识原样透传', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({
      action: 'test',
      ok: true,
      message: '校验通过',
    })
    const path = vdfsJoin(vdfsJoin(VFDS_ROOT, 'model'), 'openai-gpt4o')
    const r = await runVdfsAction(path, 'test')

    expect(r.ok).toBe(true)
    expect(r.message).toBe('校验通过')
    expect(lastCall().op).toBe(VFDS_ACTION)
    expect(lastCall().payload).toEqual({
      path: '/model/openai-gpt4o',
      action: 'test',
    })
  })

  it('无载荷时不发送 payload 字段（后端按 Option 处理）', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({ action: 'test', ok: false, message: '失败' })
    await runVdfsAction(vdfsJoin(vdfsJoin(VFDS_ROOT, 'mcp'), 'github'), 'test')

    expect(lastCall().payload).toEqual({ path: '/mcp/github', action: 'test' })
  })

  it('有载荷时原样透传（前端不解释其内容）', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({ action: 'test', ok: true, message: 'ok' })
    await runVdfsAction(vdfsJoin(vdfsJoin(VFDS_ROOT, 'mcp'), 'github'), 'test', { verbose: true })

    expect(lastCall().payload).toEqual({
      path: '/mcp/github',
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

  it('writeVdfsBinary 按线路口径发送 b64（后端据 b64 判定二进制）', async () => {
    vi.mocked(callPlugin).mockResolvedValueOnce({ path: '/skill/demo.zip', created: true })
    const r = await writeVdfsBinary(
      vdfsJoin(vdfsJoin(VFDS_ROOT, 'skill'), 'demo.zip'),
      'UEsDBA==',
      { create: true }
    )

    expect(r.created).toBe(true)
    expect(lastCall().op).toBe(VFDS_WRITE)
    expect(lastCall().payload).toEqual({
      path: '/skill/demo.zip',
      b64: 'UEsDBA==',
      create: true,
    })
  })
})
