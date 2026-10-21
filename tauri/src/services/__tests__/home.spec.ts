/**
 * home 服务 —— `home/*` 四条臂的**前端进程内驱动面**（node 环境）。
 *
 * ## 为什么这四条臂落在这里，而不在 e2e
 *
 * 四条臂全是 **home 插件的臂**（`home/set_workspace` · `home/get_workspace` 由 home 处理，
 * 前缀 = 目录名，没有第二条命名空间）。它们经 gateway **不可达**——`gateway/plugin.rs` 的入站 router 是
 * worker composite，而 home 是 composite 的**父**，`Composite::route` 对认不出的名字直接
 * `NotFound`、不向上转发（[RE-005](../../../../docs/006-ref/005-e2e覆盖矩阵.md "RE-005-e2e覆盖矩阵")
 * 的边界事实由 e2e `t69 ⑦` 钉住）。真实驱动面是 Tauri 的**进程内** `callPlugin`
 * （`services/home.ts` ⇒ `services/plugin.ts`），而那条链路只有真窗口里跑得起来——
 * 浏览器层 runner 的裁决见 RE-005「明确不做」（[T-075](../../../../docs/009-task/075-前端形态e2e补位.md "T-075-前端形态e2e补位")）。
 *
 * ## 本文件钉什么
 *
 * 断言**地址字面量**而不是「调用了某个函数」（与 `session.spec.ts` 同一条纪律）：
 * 地址拼错在真实环境里表现为「换了 homedir 却没生效」「工作区切了但下一次新建会话
 * 仍落到旧路径」——都是**静默退化**，肉眼很难发现。四条臂各一条 + 三处语义分支：
 *
 * | 臂 | 地址 | 副作用 |
 * |---|---|---|
 * | `getHomedirInfo` | `home/get_homedir` | 无（缺字段 ⇒ 退形状合法的空值，页面照常渲染） |
 * | `switchHomedir` | `home/reload` | 无（失败 ⇒ `null`，调用方自己分支） |
 * | `getWorkspacePath` | `home/get_workspace` | 记最近目录（占位值不记） |
 * | `setWorkspacePath` | `home/set_workspace` | 记最近目录 |
 *
 * 「读类吞错返兜底 / 写类直抛」的口径由 `services/fallback.ts` 定义，本文件只消费它；
 * 占位值 `~/projects` 与 `/projects` 结尾两条**不记**的分支同样是断言的一部分——
 * 记错了会让新建会话默认落到一个不存在的路径。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'

vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn(), setLastWorkdir: vi.fn() }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

import { callPlugin, setLastWorkdir } from '@/services/plugin'
import {
  getHomedirInfo,
  getWorkspacePath,
  setWorkspacePath,
  switchHomedir,
} from '../home'

const callPluginMock = vi.mocked(callPlugin)
const setLastWorkdirMock = vi.mocked(setLastWorkdir)

beforeEach(() => {
  vi.clearAllMocks()
})

describe('home 服务：home/* 两条臂', () => {
  it('getHomedirInfo 打 home/get_homedir，原样回传', async () => {
    const payload = { homedir: '/home/me/symbio', bootstrap_path: '/home/me/.symbio_bootstrap' }
    callPluginMock.mockResolvedValueOnce(payload)

    await expect(getHomedirInfo()).resolves.toEqual(payload)
    expect(callPluginMock).toHaveBeenCalledWith('home/get_homedir', {})
  })

  it('getHomedirInfo：后端未起 / 路由未注册 ⇒ 退形状合法的空值，不抛', async () => {
    callPluginMock.mockRejectedValueOnce(new Error('backend down'))

    await expect(getHomedirInfo()).resolves.toEqual({ homedir: '', bootstrap_path: '' })
  })

  it('getHomedirInfo：回包缺 homedir ⇒ 也退空值（不把半截响应当成切换成功）', async () => {
    callPluginMock.mockResolvedValueOnce({ bootstrap_path: '/x' } as never)

    await expect(getHomedirInfo()).resolves.toEqual({ homedir: '', bootstrap_path: '' })
  })

  it('switchHomedir 打 home/reload 且带 homedir；forceNative 经第四参透传', async () => {
    const reloaded = { homedir: '/new/home', workdir: '/w', status: 'ok' }
    callPluginMock.mockResolvedValueOnce(reloaded)

    await switchHomedir('/new/home', { forceNative: true })

    expect(callPluginMock).toHaveBeenCalledWith(
      'home/reload',
      { homedir: '/new/home' },
      undefined,
      { forceNative: true },
    )
  })

  it('switchHomedir：失败回 null（调用方据此决定要不要刷新数据）', async () => {
    callPluginMock.mockRejectedValueOnce(new Error('reload failed'))

    await expect(switchHomedir('/x')).resolves.toBeNull()
  })
})

describe('home 服务：workspace 两条臂', () => {
  const workspace = (workdir: string) => ({
    workdir,
    expanded_path: workdir,
    recent_workspaces: [workdir],
  })

  it('getWorkspacePath 打 home/get_workspace 并记最近目录', async () => {
    callPluginMock.mockResolvedValueOnce(workspace('/repo/app'))

    await getWorkspacePath()

    expect(callPluginMock).toHaveBeenCalledWith('home/get_workspace', {})
    expect(setLastWorkdirMock).toHaveBeenCalledWith('/repo/app')
  })

  it('getWorkspacePath：占位值不记（否则新建会话默认落到不存在的路径）', async () => {
    callPluginMock.mockResolvedValueOnce(workspace('~/projects'))

    await getWorkspacePath()

    expect(setLastWorkdirMock).not.toHaveBeenCalled()
  })

  it('getWorkspacePath：以 /projects 结尾的路径不记（同一占位值的另一形态）', async () => {
    callPluginMock.mockResolvedValueOnce(workspace('/home/me/projects'))

    await getWorkspacePath()

    expect(setLastWorkdirMock).not.toHaveBeenCalled()
  })

  it('setWorkspacePath 打 home/set_workspace 且带 path；成功即记最近目录', async () => {
    callPluginMock.mockResolvedValueOnce({ ...workspace('/repo/app'), status: 'ok' })

    await setWorkspacePath('/repo/app')

    expect(callPluginMock).toHaveBeenCalledWith('home/set_workspace', { path: '/repo/app' })
    expect(setLastWorkdirMock).toHaveBeenCalledWith('/repo/app')
  })

  it('setWorkspacePath：写类直抛——失败必须让调用方知道，不吞成「成功」假象', async () => {
    callPluginMock.mockRejectedValueOnce(new Error('bad path'))

    await expect(setWorkspacePath('/nope')).rejects.toThrow('bad path')
  })
})
