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
 * 4. **控制面强制 native**：本地切换走 `switchHomedir(path, { forceNative: true })`
 *    ——即便当前已指向远端 / 远端不可达，也必须能切回来。
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
  openDialog: vi.fn(() => Promise.resolve('/ picked/dir')),
}))

vi.mock('@/services/home', () => ({
  getHomedirInfo: hoisted.getHomedirInfo,
  switchHomedir: hoisted.switchHomedir,
}))
vi.mock('@/services/plugin', () => ({ reloadGatewayTransport: hoisted.reloadGatewayTransport }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: hoisted.openDialog }))
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
  }
})

import HomedirSwitcher from '../HomedirSwitcher.vue'
import { currentLocation } from '@/services/systemLocation'
import { useToast } from '@/composables/useToast'

const flush = () => new Promise((r) => setTimeout(r, 0))

async function openSwitcher(saved: Record<string, unknown>): Promise<VueWrapper> {
  hoisted.loadSystemLocation.mockReturnValue({ kind: 'local', localPath: '', ...saved } as never)
  const w = mount(HomedirSwitcher, { props: { open: false } })
  await w.setProps({ open: true })
  await flush()
  return w
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
  hoisted.openDialog.mockClear()
  hoisted.openDialog.mockResolvedValue('/ picked/dir')
  currentLocation.value = { kind: 'local', localPath: '' }
  const { toasts } = useToast()
  toasts.value.splice(0, toasts.value.length)
})

describe('HomedirSwitcher — 打开与预填', () => {
  it('未打开时不渲染', () => {
    expect(mount(HomedirSwitcher, { props: { open: false } }).find('.modal').exists()).toBe(false)
  })

  it('打开时读持久化并预填本地路径', async () => {
    const w = await openSwitcher({ localPath: '/saved' })
    expect((w.find('#sysdir-input').element as HTMLInputElement).value).toBe('/saved')
  })

  it('本地模式顺带拉 bootstrap 位置；路径为空时用真实 homedir 兜底', async () => {
    const w = await openSwitcher({ localPath: '' })
    expect(hoisted.getHomedirInfo).toHaveBeenCalled()
    expect((w.find('#sysdir-input').element as HTMLInputElement).value).toBe('/home/x/.symbio')
    expect(w.text()).toContain('/home/x/.symbio/BOOTSTRAP.md')
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
    const w = await openSwitcher({ localPath: '' })
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
    const w = await openSwitcher({ localPath: '' })
    await w.find('#sysdir-input').setValue('   ')
    // 按钮在非法时已禁用，点它不会触发提交；输入框的回车是另一条入口
    await w.find('#sysdir-input').trigger('keydown.enter')
    expect(w.find('.error').text()).toContain('请输入目标 homedir 路径')
    expect(w.find('.confirm-overlay').exists()).toBe(false)
    expect(hoisted.switchHomedir).not.toHaveBeenCalled()
  })

  it('远端非法地址：回车同样被拦下并给文案', async () => {
    const w = await openSwitcher({ kind: 'remote' })
    await w.find('#sysdir-url').setValue('ftp://x')
    await w.find('#sysdir-url').trigger('keydown.enter')
    expect(w.find('.error').text()).toContain('远端地址格式无效')
    expect(hoisted.saveSystemLocation).not.toHaveBeenCalled()
  })

  it('回车与按钮是同一入口：合法时回车也开确认框', async () => {
    const w = await openSwitcher({ localPath: '/x' })
    await w.find('#sysdir-input').trigger('keydown.enter')
    expect(w.find('.confirm-overlay').exists()).toBe(true)
  })
})

describe('HomedirSwitcher — 二次确认与执行', () => {
  it('合法提交只开确认框，不立刻切换', async () => {
    const w = await openSwitcher({ localPath: '/x' })
    await w.find('.btn-primary').trigger('click')
    expect(w.findComponent({ name: 'ConfirmDialog' }).props('visible')).toBe(true)
    expect(hoisted.switchHomedir).not.toHaveBeenCalled()
  })

  it('确认后：持久化 + 强制 native 切换 + 重载传输 + emit reloaded + 关闭', async () => {
    const w = await openSwitcher({ localPath: '/new/home' })
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()
    expect(hoisted.saveSystemLocation).toHaveBeenCalledWith({
      kind: 'local',
      localPath: '/new/home',
    })
    // 控制面必须强制命中本机后端，否则远端不可达时就切不回来了
    expect(hoisted.switchHomedir).toHaveBeenCalledWith('/new/home', { forceNative: true })
    expect(hoisted.reloadGatewayTransport).toHaveBeenCalled()
    expect(w.emitted('reloaded')).toHaveLength(1)
    expect(w.emitted('update:open')).toEqual([[false]])
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
  it('切换失败：错误留在框里，且**不关闭**对话框', async () => {
    hoisted.switchHomedir.mockRejectedValueOnce(new Error('reload 失败'))
    const w = await openSwitcher({ localPath: '/x' })
    await w.find('.btn-primary').trigger('click')
    await w.findComponent({ name: 'ConfirmDialog' }).vm.$emit('confirm')
    await flush()
    expect(w.find('.error').text()).toContain('切换失败')
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

describe('HomedirSwitcher — 浏览本地目录', () => {
  it('选中后填入路径', async () => {
    const w = await openSwitcher({ localPath: '' })
    await w.findAll('.btn').find((b) => b.text() === '浏览…')!.trigger('click')
    await flush()
    expect(hoisted.openDialog).toHaveBeenCalled()
    expect((w.find('#sysdir-input').element as HTMLInputElement).value).toBe('/ picked/dir')
  })

  it('对话框抛错不炸（只记日志）', async () => {
    hoisted.openDialog.mockRejectedValueOnce(new Error('用户取消'))
    const w = await openSwitcher({ localPath: '' })
    await w.findAll('.btn').find((b) => b.text() === '浏览…')!.trigger('click')
    await flush()
    expect(w.find('.modal').exists()).toBe(true)
  })

  it('用户未选（返回 null）时不覆盖已填路径', async () => {
    hoisted.openDialog.mockResolvedValueOnce(null as unknown as string)
    const w = await openSwitcher({ localPath: '/keep' })
    await w.findAll('.btn').find((b) => b.text() === '浏览…')!.trigger('click')
    await flush()
    expect((w.find('#sysdir-input').element as HTMLInputElement).value).toBe('/keep')
  })
})
