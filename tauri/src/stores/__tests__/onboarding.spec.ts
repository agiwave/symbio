// @vitest-environment happy-dom
/**
 * 首启引导的**判据与持久化**单测
 *
 * 它要钉的是一条「不报错但会烦人」的行为：**只有该做的事还没做时才自动弹**。
 *
 * - 已配好模型的用户不该被一张「请去配置」的空话卡片拦在门口 —— 判据是**数据**
 *   （`<根>/model` 里有没有条目），不是「读没读过引导」；
 * - 用户关掉 / 跳过一次之后不该再弹 —— 这条才是「读没读过」负责的，且要落盘；
 * - 任何一步的读取失败都不得把引导整体炸掉（它是锦上添花的面板，不该成为故障面）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const hoisted = vi.hoisted(() => ({
  listVdfs: vi.fn(),
  findMountDirByNewTypeExt: vi.fn(),
}))

vi.mock('@/services/vdfs', () => ({ listVdfs: hoisted.listVdfs }))
vi.mock('@/services/vdfsScheme', () => ({
  findMountDirByNewTypeExt: hoisted.findMountDirByNewTypeExt,
}))
// 连接状态是模块级 ref；引导只读它，故给一个可写对象即可
vi.mock('@/services/systemLocation', () => ({ locationError: { value: null } }))
vi.mock('@/utils/logger', () => ({
  logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}))

import { useOnboardingStore } from '../onboarding'
import { locationError } from '@/services/systemLocation'

const MODEL_DIR = '@vfs/model'
const SESSION_DIR = '@vfs/session'

/** 默认：模型类别认得出、会话类别认得出、模型目录有 1 个条目（= 已配好） */
function arrange(opts: { models?: number; modelDir?: string | null } = {}) {
  const models = opts.models ?? 1
  const modelDir = opts.modelDir === undefined ? MODEL_DIR : opts.modelDir
  hoisted.findMountDirByNewTypeExt.mockImplementation(async (ext: string) =>
    ext === 'model' ? modelDir : SESSION_DIR,
  )
  hoisted.listVdfs.mockResolvedValue({
    path: modelDir ?? '',
    node: { path: modelDir ?? '', name: 'model', title: '模型', kind: 'dir', status: '', access: 'l' },
    items: Array.from({ length: models }, (_, i) => ({
      path: `${MODEL_DIR}/m${i}`,
      name: `m${i}`,
      title: `模型${i}`,
      kind: 'model',
      status: 'active',
      access: 'rw',
      ext: 'form',
    })),
  })
}

beforeEach(() => {
  vi.resetAllMocks()
  localStorage.clear()
  setActivePinia(createPinia())
  ;(locationError as { value: string | null }).value = null
})

describe('onboarding 自动弹出（判据是数据，不是「读过没」）', () => {
  it('还没有模型 ⇒ 弹', async () => {
    arrange({ models: 0 })
    const s = useOnboardingStore()
    await s.maybeOpen()
    expect(s.open).toBe(true)
    expect(s.modelReady).toBe(false)
  })

  it('★ 已经有模型 ⇒ **不弹**（已配好的用户不该被「请去配置」拦下）', async () => {
    arrange({ models: 2 })
    const s = useOnboardingStore()
    await s.maybeOpen()
    expect(s.modelReady).toBe(true)
    expect(s.open).toBe(false)
  })

  it('用户跳过过一次 ⇒ 之后一律不自动弹（且落盘）', async () => {
    arrange({ models: 0 })
    const s = useOnboardingStore()
    await s.maybeOpen()
    s.dismiss()

    // 新的一次启动（同一份盘上状态）
    setActivePinia(createPinia())
    const next = useOnboardingStore()
    await next.maybeOpen()

    expect(next.dismissed).toBe(true)
    expect(next.open).toBe(false)
  })
})

describe('onboarding 三步判据', () => {
  it('系统目录：连接异常 ⇒ 不就绪', async () => {
    arrange({ models: 0 })
    ;(locationError as { value: string | null }).value = '远端不可达'
    const s = useOnboardingStore()
    await s.refresh()
    expect(s.systemReady).toBe(false)
  })

  it('会话类别：认不出挂载点 ⇒ 第三步不就绪（引导据此不把用户丢到空地址）', async () => {
    arrange({ models: 0 })
    hoisted.findMountDirByNewTypeExt.mockImplementation(async (ext: string) =>
      ext === 'model' ? MODEL_DIR : null,
    )
    const s = useOnboardingStore()
    await s.refresh()
    expect(s.sessionDir).toBeNull()
    expect(s.sessionReady).toBe(false)
  })

  it('某一步读取失败不炸整体：只把那一步判为未就绪，会话那步照常判定', async () => {
    hoisted.findMountDirByNewTypeExt.mockImplementation(async (ext: string) => {
      if (ext === 'model') throw new Error('IPC 断了')
      return SESSION_DIR
    })
    const s = useOnboardingStore()
    await s.refresh()
    expect(s.modelReady).toBe(false)
    expect(s.sessionReady, '一步失败不得阻断其余判定').toBe(true)
  })
})

describe('onboarding 手动重开', () => {
  it('openManually 不看「已读过」——「关于」里的入口随时可用', async () => {
    arrange({ models: 1 })
    const s = useOnboardingStore()
    s.dismiss()
    s.openManually()
    expect(s.open).toBe(true)
  })

  it('close 只是关掉面板，不记为已读（下次启动仍按数据判据决定）', async () => {
    arrange({ models: 0 })
    const s = useOnboardingStore()
    await s.maybeOpen()
    s.close()
    expect(s.dismissed).toBe(false)
    expect(s.open).toBe(false)
  })
})
