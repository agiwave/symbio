import { beforeEach, describe, expect, it, vi } from 'vitest'

const { callPlugin } = vi.hoisted(() => ({ callPlugin: vi.fn() }))
vi.mock('@/services/plugin', () => ({ callPlugin }))

import { ROUTE_SANDBOX_GRANT } from '@/constants/routes.gen'
import { grantSandboxToolForever } from '../sandboxGrant'

describe('sandbox/grant 永久授权服务', () => {
  beforeEach(() => {
    callPlugin.mockReset()
  })

  it('使用审批卡 profile_id 调用生成的 sandbox/grant 路由，并返回后端档案与清单', async () => {
    const response = {
      profile_id: 'strict',
      grants: { shell: 'high', vdfs_delete: 'medium' },
    }
    callPlugin.mockResolvedValueOnce(response)

    await expect(grantSandboxToolForever('s1', 'strict', 'shell', 'high')).resolves.toEqual(response)

    expect(callPlugin).toHaveBeenCalledWith(
      ROUTE_SANDBOX_GRANT,
      { profile_id: 'strict', tool: 'shell', risk: 'high' },
      30_000,
      { session_id: 's1', origin: 'sandbox-grant' },
    )
    expect(ROUTE_SANDBOX_GRANT).toBe('sandbox/grant')
  })

  it.each(['', '   '])('空 profile_id（%j）在发请求前失败，不回落 default', async (profileId) => {
    await expect(grantSandboxToolForever('s1', profileId, 'shell', 'high')).rejects.toThrow(
      '沙箱档案绑定',
    )
    expect(callPlugin).not.toHaveBeenCalled()
  })
})
