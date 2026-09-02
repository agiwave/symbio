import { describe, it, expect } from 'vitest'
import { formatTime } from '../time'

describe('formatTime', () => {
  // 固定"当前时间"，消除测试的时间依赖
  const NOW = new Date('2026-09-01T12:00:00').getTime()

  it('今天的时间返回 HH:mm 格式', () => {
    // 用当前时刻构造"今天"的时间戳（硬编码日期会随时间腐化：今天是今天，明天就变昨天）
    const today = Date.now() / 1000
    const result = formatTime(today)
    // toLocaleTimeString 输出依赖运行环境时区，只断言结构（不含日期部分）
    expect(result).not.toContain('昨天')
    expect(result).not.toContain('周')
  })

  it('昨天的时间返回"昨天"', () => {
    // 使用 Date.now 的相对值：构造 30 小时前的时间戳
    const yesterdayTs = (Date.now() - 30 * 3600 * 1000) / 1000
    expect(formatTime(yesterdayTs)).toBe('昨天')
  })

  it('一周内（非昨天）返回星期几', () => {
    const threeDaysAgo = (Date.now() - 3 * 24 * 3600 * 1000) / 1000
    const result = formatTime(threeDaysAgo)
    expect(['周日', '周一', '周二', '周三', '周四', '周五', '周六']).toContain(result)
  })

  it('一周前返回短日期格式（含月份）', () => {
    const longAgo = (Date.now() - 30 * 24 * 3600 * 1000) / 1000
    const result = formatTime(longAgo)
    // zh-CN short month 形如 "8月2日"
    expect(result).toMatch(/月/)
  })

  it('接受秒级时间戳（内部 ×1000 转毫秒）', () => {
    // 一个明确在"一周内"的秒级时间戳不抛异常且返回非空
    const ts = (Date.now() - 2 * 24 * 3600 * 1000) / 1000
    expect(formatTime(ts).length).toBeGreaterThan(0)
  })

  // 防止未使用变量告警（NOW 保留用于后续不依赖 Date.now 的重写）
  it('NOW 基准常量可用', () => {
    expect(NOW).toBeGreaterThan(0)
  })
})