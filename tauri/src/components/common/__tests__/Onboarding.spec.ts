// @vitest-environment happy-dom
/**
 * 首启引导组件单测（happy-dom）
 *
 * 组件本身只是呈现，但有一条**静默失效**的风险值得钉住：两个动作按钮的导航目标。
 *
 * - 目标地址必须是**认出来的**类别目录（`store.modelDir` / `store.sessionDir`），
 *   而不是写死 `/vdfs/model` —— 段名由后端 provider 决定，写死会在段名变更时
 *   静默跳到一个空目录；
 * - 类别**认不出**时必须退回资源根，绝不把用户丢到一个不存在的地址上；
 * - 只有第 3 步（发第一条消息）之后才算「引导读完」——去配模型的路上还不是。
 *
 * 状态判据在 `stores/__tests__/onboarding.spec.ts` 覆盖，这里用桩把状态固定住。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'

const hoisted = vi.hoisted(() => ({
  push: vi.fn(),
  store: {} as Record<string, unknown>,
}))

vi.mock('vue-router', () => ({ useRouter: () => ({ push: hoisted.push }) }))
vi.mock('@/stores/onboarding', () => ({ useOnboardingStore: () => hoisted.store }))
vi.mock('@/services/systemLocation', () => ({
  currentLocation: { value: { kind: 'local', localPath: '/home/u/.symbio' } },
  locationError: { value: null },
  formatLocation: (loc: { localPath?: string }) => loc.localPath || '默认 (~/.symbio)',
}))

import Onboarding from '../Onboarding.vue'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

setVdfsRoot('@vfs')

const MODEL_DIR = '@vfs/model'
const SESSION_DIR = '@vfs/session'

function arrange(overrides: Record<string, unknown> = {}) {
  const close = vi.fn()
  const dismiss = vi.fn()
  hoisted.store = {
    open: true,
    systemReady: true,
    modelReady: false,
    sessionReady: true,
    hasPending: true,
    modelDir: MODEL_DIR,
    sessionDir: SESSION_DIR,
    close,
    dismiss,
    ...overrides,
  }
  return { close, dismiss }
}

function bench() {
  return mount(Onboarding)
}

beforeEach(() => {
  hoisted.push.mockClear()
})

describe('Onboarding 导航目标', () => {
  it('★ 去配模型 = 推「认出来的模型类别目录」，且**不**记为已读', async () => {
    const { close, dismiss } = arrange()
    const w = bench()

    await w.find('button.ob-btn:not(.primary):not(.ghost)').trigger('click')

    expect(hoisted.push, '地址必须由数据认出来（写死段名会在改名时静默跳空目录）').toHaveBeenCalledWith(
      '/vdfs/model',
    )
    expect(close).toHaveBeenCalled()
    expect(dismiss, '还没走到第 3 步，不能算读完').not.toHaveBeenCalled()
  })

  it('★ 发第一条消息 = 推会话目录 + 记为已读（这一步之后引导结束）', async () => {
    const { dismiss } = arrange()
    const w = bench()

    // 按文案找，不按下标：下标会随「模型那一步的按钮在不在」而变
    const go = w.findAll('button.ob-btn').find((b) => b.text().includes('开始第一段对话'))
    expect(go, '第 3 步必须有可执行入口').toBeTruthy()
    await go!.trigger('click')

    expect(hoisted.push).toHaveBeenCalledWith('/vdfs/session')
    expect(dismiss).toHaveBeenCalled()
  })

  it('类别认不出（dir 为 null）⇒ 退回资源根，而不是拼一个不存在的地址', async () => {
    const { close } = arrange({ modelDir: null })
    const w = bench()

    await w.find('button.ob-btn:not(.primary):not(.ghost)').trigger('click')

    expect(hoisted.push).toHaveBeenCalledWith('/vdfs')
    expect(close).toHaveBeenCalled()
  })
})

describe('Onboarding 步态呈现', () => {
  it('已配好模型 ⇒ 不给「去配置模型」，且说明改成已完成', () => {
    arrange({ modelReady: true, hasPending: false })
    const w = bench()

    // 第 2 步（模型）的说明——整体文案里第一段是「系统目录」那步
    expect(w.findAll('.ob-step')[1].text()).toContain('已经有可用的模型了')
    expect(w.text()).not.toContain('去配置模型')
    // hasPending=false ⇒ 收尾按钮是「开始使用」而不是「跳过引导」
    expect(w.find('button.ob-btn.primary').text()).toBe('开始使用')
  })

  it('未配好 ⇒ 收尾按钮是「跳过引导」', () => {
    arrange({ modelReady: false, hasPending: true })
    const w = bench()
    expect(w.find('button.ob-btn.primary').text()).toBe('跳过引导')
  })
})
