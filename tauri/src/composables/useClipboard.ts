/**
 * useClipboard — 复制文本到剪贴板（唯一实现）
 *
 * 优先用异步 Clipboard API；webview / 非安全上下文里它可能不存在或被拒，
 * 退回 textarea + execCommand('copy')。两条路都失败时返回 false，由调用方
 * 明确告知用户「复制失败」，而不是让「复制」按钮点了没反应。
 */
export async function copyText(text: string): Promise<boolean> {
  if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // 落到下面的兜底
    }
  }
  if (typeof document === 'undefined') return false
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.setAttribute('readonly', '')
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(ta)
    return ok
  } catch {
    return false
  }
}
