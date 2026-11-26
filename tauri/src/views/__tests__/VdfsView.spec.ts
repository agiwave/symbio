// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, reactive } from 'vue'

const hoisted = vi.hoisted(() => ({
  route: {
    params: { dir: 'model' },
    query: {} as Record<string, string>,
    fullPath: '/vdfs/model',
    path: '/vdfs/model',
    hash: '',
  },
  leaveGuard: vi.fn(),
  updateGuard: vi.fn(),
  workbenchConfirm: vi.fn(),
  push: vi.fn(),
  replace: vi.fn(),
  back: vi.fn(),
  refreshList: vi.fn(),
  showToast: vi.fn(),
}))

vi.mock('vue-router', () => ({
  useRoute: () => hoisted.route,
  useRouter: () => ({ push: hoisted.push, replace: hoisted.replace, back: hoisted.back }),
  onBeforeRouteLeave: (guard: () => unknown) => hoisted.leaveGuard.mockImplementation(guard),
  onBeforeRouteUpdate: (guard: () => unknown) => hoisted.updateGuard.mockImplementation(guard),
}))

vi.mock('@/components/vdfs/VdfsWorkbench.vue', () => ({
  default: defineComponent({
    name: 'VdfsWorkbench',
    setup(_props, { expose, slots }) {
      expose({ confirmRouteLeave: hoisted.workbenchConfirm })
      return () => h('div', [slots['rail-header']?.()])
    },
  }),
}))

vi.mock('@/components/common/HomedirEntry.vue', () => ({
  default: defineComponent({ name: 'HomedirEntry', setup: () => () => h('div') }),
}))

vi.mock('@/stores/sessions', () => ({
  useSessionsStore: () => ({ refreshList: hoisted.refreshList }),
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ showToast: hoisted.showToast }),
}))

import VdfsView, {
  UNKNOWN_ROUTE_FLAG,
  UNKNOWN_ROUTE_MESSAGE,
  UNKNOWN_ROUTE_QUERY,
} from '../VdfsView.vue'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

setVdfsRoot('@vfs')

let wrapper: VueWrapper | null = null

beforeEach(() => {
  window.history.replaceState({}, '', '/vdfs/model')
  hoisted.route = reactive({
    params: { dir: 'model' },
    query: {},
    fullPath: '/vdfs/model',
    path: '/vdfs/model',
    hash: '',
  })
  hoisted.leaveGuard.mockReset()
  hoisted.updateGuard.mockReset()
  hoisted.workbenchConfirm.mockReset()
  hoisted.workbenchConfirm.mockResolvedValue(true)
  hoisted.push.mockReset()
  hoisted.replace.mockReset()
  hoisted.back.mockReset()
  hoisted.refreshList.mockReset()
  hoisted.showToast.mockReset()
  wrapper = mount(VdfsView)
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('未知地址一次性提示', () => {
  it('正常首页无提示，也不做标记清理', () => {
    wrapper!.unmount()
    Object.assign(hoisted.route, {
      params: { dir: '' },
      query: {},
      fullPath: '/',
      path: '/',
      hash: '',
    })
    wrapper = mount(VdfsView)
    expect(hoisted.showToast).not.toHaveBeenCalled()
    expect(hoisted.replace).not.toHaveBeenCalled()
  })

  it('正常深链无提示', () => {
    wrapper!.unmount()
    Object.assign(hoisted.route, {
      params: { dir: 'session/abc' },
      query: {},
      fullPath: '/vdfs/session/abc',
      path: '/vdfs/session/abc',
      hash: '',
    })
    wrapper = mount(VdfsView)
    expect(hoisted.showToast).not.toHaveBeenCalled()
    expect(hoisted.replace).not.toHaveBeenCalled()
  })

  it('未知地址提示一次，保留其它 query/hash 并 replace 清理标记', async () => {
    hoisted.replace.mockImplementation(
      async (target: { path: string; query: Record<string, string>; hash?: string }) => {
        hoisted.route.path = target.path
        hoisted.route.query = target.query
        if (target.hash) hoisted.route.hash = target.hash
      },
    )
    hoisted.route.query = { [UNKNOWN_ROUTE_QUERY]: UNKNOWN_ROUTE_FLAG, select: 'abc' }
    hoisted.route.hash = '#detail'
    await wrapper!.vm.$nextTick()
    await flushPromises()

    expect(hoisted.showToast).toHaveBeenCalledTimes(1)
    expect(hoisted.showToast).toHaveBeenCalledWith('info', UNKNOWN_ROUTE_MESSAGE)
    expect(hoisted.replace).toHaveBeenCalledWith({
      path: '/vdfs/model',
      query: { select: 'abc' },
      hash: '#detail',
    })
    expect(hoisted.route.query[UNKNOWN_ROUTE_QUERY]).toBeUndefined()

    // 模拟清理后的 URL 刷新：重新挂载也不再提示。
    wrapper!.unmount()
    wrapper = mount(VdfsView)
    await flushPromises()
    expect(hoisted.showToast).toHaveBeenCalledTimes(1)
    expect(hoisted.replace).toHaveBeenCalledTimes(1)
  })
})

describe('VdfsView 未保存保护', () => {
  it('一级页提供可见命令入口并向外壳抛出打开请求', async () => {
    await wrapper!.find('.rail-command').trigger('click')
    expect(wrapper!.emitted('open-command-palette')).toHaveLength(1)
  })

  it('离开路由组件时调用工作台确认', async () => {
    hoisted.workbenchConfirm.mockResolvedValue(false)

    await expect(hoisted.leaveGuard()).resolves.toBe(false)
    expect(hoisted.workbenchConfirm).toHaveBeenCalledTimes(1)
  })

  it('同一 /vdfs/:dir 组件内 Back/Forward 也调用工作台确认', async () => {
    hoisted.workbenchConfirm.mockResolvedValue(false)

    await expect(hoisted.updateGuard({ path: '/vdfs/session', query: {} })).resolves.toBe(false)
    expect(hoisted.workbenchConfirm).toHaveBeenCalledTimes(1)
  })

  it('清理未知地址标记不打断未保存草稿', async () => {
    hoisted.route.query = { [UNKNOWN_ROUTE_QUERY]: UNKNOWN_ROUTE_FLAG }
    await wrapper!.vm.$nextTick()
    hoisted.workbenchConfirm.mockResolvedValue(false)

    expect(hoisted.updateGuard({ path: '/vdfs/model', query: {} })).toBe(true)
    expect(hoisted.workbenchConfirm).not.toHaveBeenCalled()
  })
})
