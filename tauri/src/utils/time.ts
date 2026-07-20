/**
 * 格式化时间戳为可读字符串
 */
export function formatTime(timestamp: number): string {
  const date = new Date(timestamp * 1000)
  const now = new Date()
  const diff = now.getTime() - date.getTime()
  
  // 今天
  if (diff < 86400000 && date.getDate() === now.getDate()) {
    return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  }
  // 昨天
  if (diff < 172800000) {
    return '昨天'
  }
  // 一周内
  if (diff < 604800000) {
    const days = ['周日', '周一', '周二', '周三', '周四', '周五', '周六']
    return days[date.getDay()]
  }
  // 其他
  return date.toLocaleDateString('zh-CN', { month: 'short', day: 'numeric' })
}
