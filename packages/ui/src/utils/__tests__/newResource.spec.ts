// @vitest-environment happy-dom
import { describe, it, expect, vi } from 'vitest'
import { NEW_RESOURCE_EVENT, requestNewResource } from '../newResource'

describe('requestNewResource', () => {
  it('发出带目标地址的请求事件', () => {
    const handler = vi.fn()
    window.addEventListener(NEW_RESOURCE_EVENT, handler)
    try {
      requestNewResource('/vdfs/session')
      expect(handler).toHaveBeenCalledOnce()
      expect((handler.mock.calls[0][0] as CustomEvent<string>).detail).toBe('/vdfs/session')
    } finally {
      window.removeEventListener(NEW_RESOURCE_EVENT, handler)
    }
  })

  it('不填地址时 detail 为 null（CustomEvent 缺省值；交给当前页）', () => {
    const handler = vi.fn()
    window.addEventListener(NEW_RESOURCE_EVENT, handler)
    try {
      requestNewResource()
      expect((handler.mock.calls[0][0] as CustomEvent<unknown>).detail).toBeNull()
    } finally {
      window.removeEventListener(NEW_RESOURCE_EVENT, handler)
    }
  })
})
