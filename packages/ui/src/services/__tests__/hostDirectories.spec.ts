import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const H = vi.hoisted(() => ({
  endpoint: 'http://server.test:8080',
  token: 'secret',
  initGatewayTransport: vi.fn(async () => {}),
  callPlugin: vi.fn(),
}))

vi.mock('../plugin', () => ({
  initGatewayTransport: H.initGatewayTransport,
  getOutboundEndpoint: () => H.endpoint,
  getOutboundToken: () => H.token,
  callPlugin: H.callPlugin,
}))

import { getHostRuntime, listHostDirectories } from '../hostDirectories'

beforeEach(() => {
  H.endpoint = 'http://server.test:8080'
  H.token = 'secret'
  H.initGatewayTransport.mockClear()
  H.callPlugin.mockClear()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('Server 宿主目录 API', () => {
  it('runtime 使用唯一 outbound endpoint/token，且不经插件路由', async () => {
    const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => ({
      ok: true,
      text: async () => JSON.stringify({
        homedir: '/srv/home/me',
        bootstrap_path: '/srv/home/me/.symbio_bootstrap',
        deployment: 'server',
        capabilities: { directory_browser: true },
      }),
    }))
    vi.stubGlobal('fetch', fetchMock)

    await expect(getHostRuntime()).resolves.toMatchObject({ homedir: '/srv/home/me', deployment: 'server' })
    expect(H.initGatewayTransport).toHaveBeenCalled()
    expect(fetchMock).toHaveBeenCalledWith('http://server.test:8080/api/v1/host/runtime', {
      headers: { Accept: 'application/json', Authorization: 'Bearer secret' },
      signal: expect.any(AbortSignal),
    })
    expect(H.callPlugin).not.toHaveBeenCalled()
  })

  it('目录路径来自当前 Server，并作为绝对路径 query 编码', async () => {
    const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => ({
      ok: true,
      text: async () => JSON.stringify({
        path: '/srv/projects',
        parent: '/srv',
        roots: ['/'],
        entries: [
          { name: 'app', path: '/srv/projects/app', kind: 'directory' },
          { name: 'README.md', path: '/srv/projects/README.md', kind: 'file' },
        ],
      }),
    }))
    vi.stubGlobal('fetch', fetchMock)

    const result = await listHostDirectories('/srv/projects')
    const url = String(fetchMock.mock.calls[0][0])
    expect(url).toBe('http://server.test:8080/api/v1/host/directories?path=%2Fsrv%2Fprojects')
    expect(result.entries).toEqual([
      { name: 'app', path: '/srv/projects/app', kind: 'directory' },
    ])
    expect(result.roots).toEqual([{ name: '/', path: '/' }])
    expect(H.callPlugin).not.toHaveBeenCalled()
  })

  it('token 为空时不发送 Authorization', async () => {
    H.token = ''
    const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => ({
      ok: true,
      text: async () => JSON.stringify({ path: '/', parent: null, roots: [], entries: [] }),
    }))
    vi.stubGlobal('fetch', fetchMock)

    await listHostDirectories('/')
    expect(fetchMock.mock.calls[0][1]).toMatchObject({ headers: { Accept: 'application/json' } })
    expect((fetchMock.mock.calls[0][1] as { headers: Record<string, string> }).headers.Authorization).toBeUndefined()
  })

  it('HTTP 错误正文可见', async () => {
    vi.stubGlobal('fetch', vi.fn(async (_url: string, _init?: RequestInit) => ({
      ok: false,
      status: 403,
      text: async () => JSON.stringify({ error: 'token invalid' }),
    })))
    await expect(listHostDirectories('/srv')).rejects.toThrow('token invalid')
  })
})
