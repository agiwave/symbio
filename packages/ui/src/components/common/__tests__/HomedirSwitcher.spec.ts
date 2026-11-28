/**
 * HomedirSwitcher — 系统目录切换对话框单测（happy-dom）
 *
 * 这是全 App 唯一能改「连到哪儿」的入口，也是**破坏性操作**：切换后所有活跃
 * chat 会话会被关闭、UI 数据整体重载。因此这里钉的是「不能误切」与「切失败要
 * 让人看见」两类回归：
 *
 * 1. **必经二次确认**：`handleSubmit` 不直接执行，只开确认框——少这一步就是
 *    手滑回车把整个后端换了。
 * 2. **校验在提交前**：本地空路径 / 远端非法地址都在**提交前**拦下并给出文案，
 *    且切换按钮在非法时禁用。
 * 3. **失败不关对话框**：切换失败要把错误留在框里（关了就只剩一个 toast，用户
 *    来不及读），同时 `busy` 期间**关不掉**（防止切换途中把对话框收了）。
 * 4. **先切出站再重载**：本地切换先保存 local 目标并重载传输，再调用
 *    `switchHomedir(path)`；不能拿 `forceNative` 提示越过已选 Server HTTP。
 * 5. **key 的归属**：地址里的 `?token=` 与密钥输入框是同一个东西的两个入口，
 *    输入框优先。
 */

// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'

const hoisted = vi.hoisted(() => ({
  getHomedirInfo: vi.fn(() =>
    Promise.resolve({ homedir: '/home/x/.symbio', bootstrap_path: '/home/x/.symbio/BOOTSTRAP.md' }),
  ),
  switchHomedir: vi.fn(() => Promise.resolve({})),
  reloadGatewayTransport: vi.fn(() => Promise.resolve()),
  loadSystemLocation: vi.fn(() => ({ kind: 'local' as const, localPath: '/saved' })),
  saveSystemLocation: vi.fn(),
  browser: false,
  pickPath: vi.fn(async (
    _kind?: string,
    _options?: { initialPath?: string; onBrowserPick?: (request: { kind: string; initialPath?: string }) => void },
  ) => '/ picked/dir' as string | null),
}))

vi.mock('@/services/home', () => ({
  getHomedirInfo: hoisted.getHomedirInfo,
  switchHomedir: hoisted.switchHomedir,
}))
vi.mock('@/services/plugin', () => ({ reloadGatewayTransport: hoisted.reloadGatewayTransport }))
vi.mock('@/services/nativePick', () => ({ pickPath: hoisted.pickPath }))
vi.mock('@/services/hostDirectories', () => ({
  getHostRuntime: vi.fn(async () => ({
    homedir: '/srv/home/me', bootstrap_path: '/srv/bootstrap', deployment: 'server',
    capabilities: { directory_browser: true },
  })),
  listHostDirectories: vi.fn(async () => ({ path: '/srv/home/me', parent: '/srv', roots: [], entries: [] })),
}))
vi.mock('@/utils/logger', () => ({
  logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}))
// 只桩掉持久化读写（会碰本地存储），解析与校验用真的——本用例要验的就是它们的判据
vi.mock('@/services/systemLocation', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/systemLocation')>()
  return {
    ...actual,
    loadSystemLocation: hoisted.loadSystemLocation,
    saveSystemLocation: hoisted.saveSystemLocation,
    isBrowserEnvironment: () => hoisted.browser,
  }
})

import HomedirSwitcher from '../HomedirSwitcher.vue'
import DirectoryPicker from '../DirectoryPicker.vue'
import { currentLocation } from '@/services/systemLocation'
import { useToast } from '@/composables/useToast'

const flush = () => new Promise((r) => setTimeout(r, 0))

async function openSwitcher(saved: Record<string, unknown>, manual = false): Promise<VueWrapper> {
  hoisted.loadSystemLocation.mockReturnValue({ kind: 'local', localPath: '', ...saved } as never)
  const w = mount(HomedirSwitcher, { props: { open: false } })
  await w.setProps({ open: true })
  await flush()
  if (manual) {
    await w.find('.manual-toggle').trigger('click')
    await flush()
  }
  return w
}

/** 浏览态用摘要卡片显示当前路径；手动态才有输入框 */
function currentPathText(w: VueWrapper): string {
  const card = w.find('.current-path')
  return card.exists() ? card.text() : (w.find('#sysdir-input').element as HTMLInputElement).value
}

beforeEach(() => {
  hoisted.getHomedirInfo.mockClear()
  hoisted.getHomedirInfo.mockResolvedValue({
    homedir: '/home/x/.symbio',
    bootstrap_path: '/home/x/.symbio/BOOTSTRAP.md',
  })
  hoisted.switchHomedir.mockClear()
  hoisted.switchHomedir.mockResolvedValue({})
  hoisted.reloadGatewayTransport.mockClear()
  hoisted.reloadGatewayTransport.mockResolvedValue(undefined)
  hoisted.saveSystemLocation.mockClear()
  hoisted.browser = false
  hoisted.pickPath.mockReset()
  hoisted.pickPath.mockResolvedValue('/ picked/dir')
  currentLocation.value = { kind: 'local', localPath: '' }
  const { toasts } = useToast()
  toasts.value.splice(0, toasts.value.length)
})

describe('HomedirSwitcher — 打开与预填', () => {
  it('未打开时不渲染', () => {
    expect(mount(HomedirSwitcher, { props: { open: false } }).find('.modal').exists()).toBe(false)
  })

  it('打开时读持久化并显示本地路径（只显示一次）', async () => {
    const w = await openSwitcher({ localPath: '/saved' })
    expect(currentPathText(w)).toBe('/saved')
    expect(w.findAll('.current-path')).toHaveLength(1)
  })

  it('路径为空时用当前数据位置兜底，且不再展示 bootstrap 等技术字段', async () => {
    const w = await openSwitcher({ localPath: '' })
    expect(hoisted.getHomedirInfo).toHaveBeenCalled()
    expect(currentPathText(w)).toBe('/home/x/.symbio')
    expect(w.text()).not.toContain('BOOTSTRAP')
    expect(w.text()).not.toContain('homedir')
  })

  it('读取 homedir 失败不炸（只 warn，仍可手动输入切换）', async () => {
    hoisted.getHomedirInfo.mockRejectedValueOnce(new Error('IPC 断了'))
    const w = await openSwitcher({ localPath: '' })
    expect(w.find('.modal').exists()).toBe(true)
    expect(w.find('.error').exists()).toBe(false)
  })

  it('远端：预填地址与密钥', async () => {
    const w = await openSwitcher({ kind: 'remote', remoteUrl: 'http://127.0.0.1:8080', remoteKey: 'k1' })
    expect((w.find('#sysdir-url').element as HTMLInputElement).value).toBe('http://127.0.0.1:8080')
    expect((w.find('#sysdir-key').element as HTMLInputElement).value).toBe('k1')
  })
})

describe('HomedirSwitcher — 提交前校验', () => {
  it('本地路径为空 → 切换按钮禁用', async () => {
    const w = await openSwitcher({ localPath: '' }, true)
    await w.find('#sysdir-input').setValue('')
    expect(w.find('.btn-primary').attributes('disabled')).toBeDefined()
  })

  it('本地路径非空 → 可提交', async () => {
    const w = await openSwitcher({ localPath: '/x' })
    expect(w.find('.btn-primary').attributes('disabled')).toBeUndefined()
  })

  it('远端地址非法 → 禁用；合法 → 可提交（含内嵌 token 也算合法）', async () => {
    const w = await openSwitcher({ kind: 'remote' })
    await w.find('#sysdir-url').setValue('not-a-url')
    expect(w.find('.btn-primary').attributes('disabled')).toBeDefined()
    await w.find('#sysdir-url').setValue('http://127.0.0.1:8080?token=KEY')
    expect(w.find('.btn-primary').attributes('disabled')).toBeUndefined()
  })

  it('host 判据：localhost / 含点 / IP 才可提交，裸主机名不行', async () => {
    const w = await openSwitcher({ kind: 'remote' })
    for (const ok of ['http://localhost:8080', 'http://gw.example:8080', 'http://127.0.0.1:8080']) {
      await w.find('#sysdir-url').setValue(ok)
      expect(w.find('.btn-primary').attributes('disabled')).toBeUndefined()
    }
    // 裸主机名没有点、不是 IP、也不是 localhost → 按设计判为非法
    await w.find('#sysdir-url').setValue('http://gw:8080')
    expect(w.find('.btn-primary').attributes('disabled')).toBeDefined()
  })

  it('本地空路径：按钮禁用（点不动），但**回车也走同一套校验**不绕过', async () => {
    const w = await openSwitcher({ localPath: '' }, true)
    await w.find('#sysdir-input').setValue('   ')
    // 按钮在非法时已禁用，点它不会触发提交；输入框的回车是另一条入口
    await w.find('#sysdir-input').trigger('keydown.enter')
    expect(w.find('.error').text()).toContain('请选择或填写一个文件夹')
    expect(w.find('.confirm-dialog').exists()).toBe(false)
    expect(hoisted.switchHomedir).not.toHaveBeenCalled()
  })

  it('远端非法地址：回车同样被拦下并给文案', async () => {
    const w = await openSwitcher({ kind: 'remote' })
    await w.find('#sysdir-url').setValue('ftp://x')
    await w.find('#sysdir-url').trigger('keydown.enter')
    expect(w.find('.error').text()).toContain('地址格式无效')
    expect(hoisted.saveSystemLocation).not.toHaveBeenCalled()
  })

  it('回车与按钮是同一入口：合法时回车也开确认框', async () => {
    const w = await openSwitcher({ localPath: '/x' }, true)
    await w.find('#sysdir-input').trigger('keydown.enter')
    expect(w.find('.confirm-dialog').exists()).toBe(true)
  })
})

describe('HomedirSwitcher — 二次确认与执行', () => {
  it('合法提交只开确认框，不立刻切换', async () => {
    const w = await openSwitcher({ localPath: '/x' })
    await w.find('.btn-primary').trigger('click')
    expect(w.findComponent({ name: 'ConfirmDialog' }).props('visible')).toBe(true)
    expect(hoisted.switchHomedir).not.toHaveBeenCalled()
  })

  it('确认后：先持久化并重载出站，再切换 homedir、emit reloaded、关闭', async () => {
    const w = await openSwitcher({ localPath: '/new/home' })
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()
    expect(hoisted.saveSystemLocation).toHaveBeenCalledWith({
      kind: 'local',
      localPath: '/new/home',
    })
    // 旧目标可能是远端；必须先让 outbound 指向 local/native，home/reload 才不会发往旧 Server。
    expect(hoisted.switchHomedir).toHaveBeenCalledWith('/new/home')
    expect(hoisted.reloadGatewayTransport.mock.invocationCallOrder[0])
      .toBeLessThan(hoisted.switchHomedir.mock.invocationCallOrder[0])
    expect(w.emitted('reloaded')).toHaveLength(1)
    expect(w.emitted('update:open')).toEqual([[false]])
  })

  it('embedded 从远端切回 local：重载传输完成后才允许调用 home/reload', async () => {
    let releaseReload: (() => void) | undefined
    hoisted.loadSystemLocation.mockReturnValue({
      kind: 'remote', remoteUrl: 'http://old-server:8080', remoteKey: 'k',
    } as never)
    hoisted.reloadGatewayTransport.mockImplementationOnce(() => new Promise<void>((resolve) => {
      releaseReload = resolve
    }))
    const w = mount(HomedirSwitcher, { props: { open: false } })
    await w.setProps({ open: true })
    await flush()
    await w.findAll('.seg-btn').find((button) => button.text() === '本机目录')!.trigger('click')
    await w.find('.manual-toggle').trigger('click')
    await w.find('#sysdir-input').setValue('/local/home')
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()

    expect(hoisted.saveSystemLocation).toHaveBeenCalledWith({ kind: 'local', localPath: '/local/home' })
    expect(hoisted.switchHomedir).not.toHaveBeenCalled()
    releaseReload?.()
    await flush()
    expect(hoisted.switchHomedir).toHaveBeenCalledWith('/local/home')
    expect(hoisted.switchHomedir.mock.invocationCallOrder[0])
      .toBeGreaterThan(hoisted.reloadGatewayTransport.mock.invocationCallOrder[0])
  })

  it('browser local 先切到同源 Server，再经 HTTP home/reload（不 force native）', async () => {
    hoisted.browser = true
    const w = await openSwitcher({ localPath: '/srv/new/home' })
    expect(w.text()).not.toContain('当前 Server')
    expect(w.text()).not.toContain('homedir')

    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()

    expect(hoisted.switchHomedir).toHaveBeenCalledWith('/srv/new/home')
    expect(hoisted.reloadGatewayTransport).toHaveBeenCalled()
    expect(hoisted.reloadGatewayTransport.mock.invocationCallOrder[0])
      .toBeLessThan(hoisted.switchHomedir.mock.invocationCallOrder[0])
  })

  it('远端：密钥输入框优先于地址里的 token', async () => {
    const w = await openSwitcher({ kind: 'remote' })
    await w.find('#sysdir-url').setValue('http://127.0.0.1:8080?token=from-url')
    await w.find('#sysdir-key').setValue('from-input')
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()
    expect(hoisted.saveSystemLocation).toHaveBeenCalledWith(
      expect.objectContaining({ remoteUrl: 'http://127.0.0.1:8080', remoteKey: 'from-input' }),
    )
  })

  it('远端：embedded 不可达也保留 HTTP，不提示回退本地', async () => {
    const w = await openSwitcher({ kind: 'remote' })
    await w.find('#sysdir-url').setValue('http://127.0.0.1:8080')
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()

    expect(hoisted.reloadGatewayTransport).toHaveBeenCalledTimes(1)
    const { toasts } = useToast()
    const toast = toasts.value[toasts.value.length - 1]
    expect(toast?.text).toContain('不会自动切回本机')
    expect(toast?.text).not.toContain('自动回退本地')
  })

  it('远端：密钥留空则取地址里的 token', async () => {
    const w = await openSwitcher({ kind: 'remote' })
    await w.find('#sysdir-url').setValue('http://127.0.0.1:8080?token=from-url')
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()
    expect(hoisted.saveSystemLocation).toHaveBeenCalledWith(
      expect.objectContaining({ remoteKey: 'from-url' }),
    )
  })
})

describe('HomedirSwitcher — 失败与忙态', () => {
  it('切换失败：恢复原目标与传输，错误留在框里，不报告成功，也不关闭对话框', async () => {
    hoisted.switchHomedir.mockRejectedValueOnce(new Error('reload 失败'))
    const w = await openSwitcher({ localPath: '/x' })
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()
    expect(w.find('.error').text()).toContain('切换失败')
    expect(hoisted.saveSystemLocation).toHaveBeenNthCalledWith(1, {
      kind: 'local', localPath: '/x',
    })
    expect(hoisted.saveSystemLocation).toHaveBeenNthCalledWith(2, {
      kind: 'local', localPath: '/x',
    })
    expect(hoisted.reloadGatewayTransport).toHaveBeenCalledTimes(2)
    expect(w.emitted('reloaded')).toBeUndefined()
    expect(w.emitted('update:open')).toBeUndefined()
  })

  it('browser local 重载失败：恢复原连接目标与传输，不报告成功', async () => {
    hoisted.browser = true
    hoisted.switchHomedir.mockRejectedValueOnce(new Error('Server reload 失败'))
    const w = await openSwitcher({ localPath: '/srv/old' }, true)
    await w.find('#sysdir-input').setValue('/srv/new')
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()

    expect(hoisted.saveSystemLocation).toHaveBeenNthCalledWith(1, {
      kind: 'local', localPath: '/srv/new',
    })
    expect(hoisted.saveSystemLocation).toHaveBeenNthCalledWith(2, {
      kind: 'local', localPath: '/srv/old',
    })
    expect(hoisted.reloadGatewayTransport).toHaveBeenCalledTimes(2)
    expect(w.find('.error').text()).toContain('Server reload 失败')
    expect(w.emitted('reloaded')).toBeUndefined()
    expect(w.emitted('update:open')).toBeUndefined()
  })

  it('忙态期间关不掉（防止切换途中收起对话框）', async () => {
    let release: (v: unknown) => void = () => {}
    hoisted.switchHomedir.mockReturnValueOnce(
      new Promise((resolve) => {
        release = resolve as (v: unknown) => void
      }),
    )
    const w = await openSwitcher({ localPath: '/x' })
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()
    expect(w.find('.btn-primary').text()).toBe('切换中…')
    await w.find('.close-btn').trigger('click')
    expect(w.emitted('update:open')).toBeUndefined()
    release({})
  })

  it('闲时点遮罩可关闭', async () => {
    const w = await openSwitcher({ localPath: '/x' })
    await w.find('.modal-mask').trigger('click')
    expect(w.emitted('update:open')).toEqual([[false]])
  })
})

describe('HomedirSwitcher — 浏览目标目录', () => {
  it('browser Browse 打开 Server 目录组件，选择后填入 homedir', async () => {
    hoisted.browser = true
    hoisted.pickPath.mockImplementationOnce(async (kind, options) => {
      options?.onBrowserPick?.({ kind: kind ?? 'directory' })
      return null
    })
    const w = await openSwitcher({ localPath: '/srv/old' })
    await w.find('.primary-pick').trigger('click')
    await flush()

    const picker = w.findComponent(DirectoryPicker)
    expect(picker.exists()).toBe(true)
    expect(picker.props('initialPath')).toBe('/srv/old')
    picker.vm.$emit('select', '/srv/new')
    await flush()
    expect(currentPathText(w)).toBe('/srv/new')
    expect(hoisted.pickPath).toHaveBeenCalledWith('directory', expect.objectContaining({
      onBrowserPick: expect.any(Function),
    }))
  })

  it('选中后填入路径', async () => {
    const w = await openSwitcher({ localPath: '' })
    await w.find('.primary-pick').trigger('click')
    await flush()
    expect(hoisted.pickPath).toHaveBeenCalled()
    expect(currentPathText(w)).toBe('/ picked/dir')
  })

  it('对话框抛错不炸（只记日志）', async () => {
    hoisted.pickPath.mockRejectedValueOnce(new Error('用户取消'))
    const w = await openSwitcher({ localPath: '' })
    await w.find('.primary-pick').trigger('click')
    await flush()
    expect(w.find('.modal').exists()).toBe(true)
  })

  it('用户未选（返回 null）时不覆盖已填路径', async () => {
    hoisted.pickPath.mockResolvedValueOnce(null as unknown as string)
    const w = await openSwitcher({ localPath: '/keep' })
    await w.find('.primary-pick').trigger('click')
    await flush()
    expect(currentPathText(w)).toBe('/keep')
  })
})
