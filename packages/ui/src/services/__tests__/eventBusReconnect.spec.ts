/**
 * `connectEventBus` 的重连自愈 —— 「断开期间的帧」必须由本端补一次重读
 *
 * 锁一个静默失效：后端 `try_publish` 见 `is_closed` 会**摘除订阅**、帧被丢弃；
 * 而订阅已经不在表里，后端也没法补 resync（没人知道该通知谁）。这条路径的补丁只能
 * 在**本端重连成功**处打——否则断开期间漏掉的变更永久丢失，丢在正文中段的增量还会
 * 让「本地正文恒为真值前缀」这个前提失效，守卫只按前缀校验、不会纠正它。
 *
 * 两个触发点必须区分：**重连补、首连不补**——首连时各作用域还没有「本地视图可能
 * 落后」这回事，白重读一次只是浪费。第二例顺带锁住「一个重读处理器抛错不得吃掉
 * 后面的」：整份重读是唯一一层恢复机制，漏掉一半作用域即等于没恢复。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn(), connectPlugin: vi.fn() }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() }
}))

import { callPlugin, connectPlugin, type Connection } from '@/services/plugin'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')
import { _resetEventBusForTest, connectEventBus, subscribeVdfsChanged } from '../eventBus'

/**
 * 让 `connectPlugin` 返回一个**未连接**的桩。
 *
 * `isConnected` 取 false 才有第二次连接的入口：`connectEventBus` 对已连接的单例
 * 直接早返回，那样永远测不到重连分支。
 */
function stubDisconnectedConnect(): void {
  vi.mocked(connectPlugin).mockImplementation(
    async () =>
      ({
        connectionId: 'c-test',
        isConnected: false,
        close: vi.fn(async () => {})
      }) as unknown as Connection
  )
}

/** 等 fire-and-forget 的连接 / 登记链跑完 */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

beforeEach(() => {
  // 状态挂在 `globalThis` 上，会跨 spec 文件存活——不复位会读到别的文件留下的
  // `everConnected = true`，把首连误判成重连
  _resetEventBusForTest()
  vi.mocked(callPlugin).mockReset()
  vi.mocked(callPlugin).mockResolvedValue(undefined as never)
  vi.mocked(connectPlugin).mockReset()
  stubDisconnectedConnect()
})

describe('connectEventBus 的重连自愈', () => {
  it('首连不补重读，重连成功才补一次', async () => {
    let resyncs = 0
    const off = subscribeVdfsChanged(
      { prefix: '@vfs/session' },
      () => {},
      () => {
        resyncs++
      }
    )
    await settle()

    // 首连（`subscribe` 内部自动发起的那一次）：不补重读——各作用域此刻还没有
    // 「本地视图可能落后」这回事
    expect(resyncs).toBe(0)

    // 重连：断开期间后端已摘掉订阅，帧被静默丢弃且不补 resync
    await connectEventBus()
    await settle()
    expect(resyncs).toBe(1)

    off()
  })

  it('一个重读处理器抛错不阻断其余作用域', async () => {
    const order: string[] = []
    const offA = subscribeVdfsChanged(
      { prefix: '@vfs/session' },
      () => {},
      () => {
        order.push('a')
        throw new Error('重读失败')
      }
    )
    const offB = subscribeVdfsChanged(
      { prefix: '@vfs/session/s1' },
      () => {},
      () => {
        order.push('b')
      }
    )
    await settle()

    expect(order).toEqual([]) // 首连不补重读

    await connectEventBus() // 重连
    await settle()

    expect(order).toEqual(['a', 'b'])

    offA()
    offB()
  })
})
