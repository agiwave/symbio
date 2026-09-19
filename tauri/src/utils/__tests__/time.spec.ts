import { describe, it, expect } from 'vitest'
import { formatDateTime, relativeTime } from '../time'

describe('relativeTime', () => {
  it('刚发生（不到一分钟）返回「刚刚」', () => {
    expect(relativeTime(Date.now())).toBe('刚刚')
  })

  it('分钟级返回 N 分钟前', () => {
    expect(relativeTime(Date.now() - 5 * 60 * 1000)).toBe('5 分钟前')
  })

  it('小时级返回 N 小时前', () => {
    expect(relativeTime(Date.now() - 3 * 3600 * 1000)).toBe('3 小时前')
  })

  it('天级返回 N 天前', () => {
    expect(relativeTime(Date.now() - 4 * 24 * 3600 * 1000)).toBe('4 天前')
  })

  it('接受秒级时间戳（内部按量级转毫秒）', () => {
    // 秒级「30 分钟前」与毫秒级同义——两种口径必须得到同一个结果
    const seconds = Math.floor((Date.now() - 30 * 60 * 1000) / 1000)
    expect(relativeTime(seconds)).toBe('30 分钟前')
  })

  it('未来时间戳返回空串（不显示「-N 分钟前」）', () => {
    expect(relativeTime(Date.now() + 60_000)).toBe('')
  })

  it('缺省 / 0 返回空串', () => {
    expect(relativeTime(undefined)).toBe('')
    expect(relativeTime(null)).toBe('')
    expect(relativeTime(0)).toBe('')
  })
})

describe('formatDateTime', () => {
  it('秒级与毫秒级时间戳得到同一个结果', () => {
    const ms = Date.UTC(2026, 8, 1, 12, 0, 0)
    expect(formatDateTime(ms / 1000)).toBe(formatDateTime(ms))
  })

  it('返回含 4 位年份的本地化串', () => {
    // 时区无关：只断言结构，不锁定具体格式
    expect(formatDateTime(Date.now())).toMatch(/\d{4}/)
  })

  it('缺省 / 0 返回空串', () => {
    expect(formatDateTime(undefined)).toBe('')
    expect(formatDateTime(0)).toBe('')
  })
})
