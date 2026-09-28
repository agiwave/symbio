// @vitest-environment happy-dom
/**
 * 冷启动落点单测
 *
 * 落点**恒为会话目录**（不是资源根）。钉住三条，它们**错了都不报错**，只会让
 * 用户每次启动被拽去别处：
 *
 * 1. 落到**会话目录**（由后端挂载点解析，不硬编码段名）；
 * 2. 挂载点解析失败 ⇒ 降级 `/vdfs`（**不阻断启动**）；
 * 3. **不还原旧的位置记忆**——「回上次地址」整条已下线（见下）。
 *
 * 另一条同样静默的约定：守卫只在**首页**介入（深链 `/vdfs/...` 原样放行），
 * 且用 `replace` 落点（否则返回键会在 `/` 上死循环）。
 *
 * ## 「回上次地址」已下线（2026-09-28）
 *
 * 它先后露出两个**不可能在本机制内修好**的毛病，故整体去掉而不是继续加判据：
 *
 * 1. 打包后的 webview 里 `location.pathname` 不是 `/`，守卫的 `to.path !== '/'`
 *    判据每次提前 return ⇒ 记忆从未被读过（现象：每次都进会话）；
 * 2. 修好判据后它**反而"正确地做错事"**：退出某智能体再重启，又被送回该智能体
 *    ——那正是上次停留的地方，还原成功了，只是用户要的不是这个。
 *
 * 判「该不该回上次」需要一个稳定的**中性位置**概念，而 VDFS 地址是分形的
 * （子空间与根空间挂载名完全同名，实测 `/vdfs/session/<id>` 与
 * `/vdfs/agent/<id>` 结构相同），纯地址形状分不出两者。
 *
 * ⚠️ 因此下面**必须**有一条用例断言「盘上残留旧记忆也不还原」：老用户升级后
 * localStorage 里仍有 `symbio.nav`，那是本文件唯一能证明功能真关掉了的地方。
 *
 * ## 之所以值得单独测
 *
 * `coldStartPath` 是 **async**，而 vue-router 的 `redirect` 不接受 Promise——
 * 写法稍一变（改回 `redirect: () => coldStartPath()`）就直接 `TS2322`，但
 * **「守卫忘记 return」这类错误类型系统看不见**，运行时表现为「停在空白页」。
 * 故用真 router 跑真实导航。
 *
 * ## 为什么每个用例自带超时
 *
 * 用例要拿一份**全新**的 router（守卫挂在实例上，复用会串状态），于是每个 `it`
 * 都 `vi.resetModules()` + 重新 `import('../index')`——这会把整棵路由 / 视图依赖
 * 图重新实例化一遍，单个用例实测 2.4s。默认 5s 超时**贴着这个量级**：全量并发跑
 * （50 个文件抢 CPU）时就会偶发超时。超时值因此按**这个用例在满负载下的耗时**给，
 * 而不是按它空跑时的耗时给——否则就是一条随机变红的测试，最后一定被人加
 * `retry` 了事。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

/** 单独跑 2.4s；全量并发下要留足余量（见文件头） */
const ROUTER_RELOAD_TIMEOUT = 30_000

const hoisted = vi.hoisted(() => ({
  ensureSessionMountDir: vi.fn(async () => '/symbio-root/session'),
  loggerInfo: vi.fn(),
}))

vi.mock('@/services/vdfsScheme', () => ({
  ensureSessionMountDir: hoisted.ensureSessionMountDir,
}))
vi.mock('@/utils/logger', () => ({
  logger: { info: hoisted.loggerInfo, warn: vi.fn(), error: vi.fn() },
}))

/** 每次取一份**全新**的 router（守卫挂在实例上，复用会串状态） */
async function freshRouter() {
  vi.resetModules()
  const mod = await import('../index')
  return mod.default
}

beforeEach(() => {
  setActivePinia(createPinia())
  localStorage.clear()
  hoisted.ensureSessionMountDir.mockReset()
  hoisted.ensureSessionMountDir.mockResolvedValue('/symbio-root/session')
  hoisted.loggerInfo.mockClear()
})

describe('冷启动落点', () => {
  it('落到会话目录（由后端挂载点解析，不硬编码段名）', async () => {
    const router = await freshRouter()
    await router.push('/')
    await router.isReady()

    expect(router.currentRoute.value.path).toBe('/vdfs/session')
    expect(hoisted.ensureSessionMountDir).toHaveBeenCalled()
  }, ROUTER_RELOAD_TIMEOUT)

  /**
   * ★ 「回上次地址」已下线的**验收用例**。
   *
   * 老用户升级后 localStorage 里仍留着旧版本写下的 `symbio.nav`。这条断言
   * 「即使它在，也一律走会话目录」——是本文件唯一能证明那级真被去掉的地方。
   * 若有人日后把记忆接回来，这条会红，那正是它该做的。
   */
  it('★ 盘上残留旧位置记忆 ⇒ 一律不还原，仍落会话目录', async () => {
    localStorage.setItem('symbio.nav', JSON.stringify({ lastPath: '/vdfs/session/abc123' }))
    const router = await freshRouter()
    await router.push('/')
    await router.isReady()

    expect(router.currentRoute.value.path).toBe('/vdfs/session')
    // 仍走解析（证明记忆分支真的没了，而不是"恰好解析出同一个值"）
    expect(hoisted.ensureSessionMountDir).toHaveBeenCalled()
  }, ROUTER_RELOAD_TIMEOUT)

  it('挂载点解析失败 ⇒ 降级 /vdfs 且不阻断启动', async () => {
    hoisted.ensureSessionMountDir.mockRejectedValue(new Error('根清单为空'))
    const router = await freshRouter()
    await router.push('/')
    await router.isReady()

    expect(router.currentRoute.value.path).toBe('/vdfs')
    expect(hoisted.loggerInfo).toHaveBeenCalled()
  }, ROUTER_RELOAD_TIMEOUT)

  it('深链 /vdfs/... 不被冷启动落点改写', async () => {
    const router = await freshRouter()
    await router.push('/vdfs/model')
    await router.isReady()

    expect(router.currentRoute.value.path).toBe('/vdfs/model')
    expect(hoisted.ensureSessionMountDir).not.toHaveBeenCalled()
  }, ROUTER_RELOAD_TIMEOUT)

  it('落点用 replace ⇒ `/` 不进历史（避免返回键回到 `/` 再被重定向）', async () => {
    const router = await freshRouter()
    await router.push('/vdfs/model')
    await router.push('/')
    await router.isReady()

    expect(router.currentRoute.value.path).toBe('/vdfs/session')

    // 断言「历史栈里没有 `/`」而不是 `router.back()` 的实际回退——happy-dom
    // 的 history 不实现真实 back 语义（回退不触发导航），测下去只会测到环境。
    // 判据直接取 vue-router 自己记的历史状态：`/` 若被 replace 掉就不会出现。
    const entries = router.getRoutes()
    expect(entries.some((r) => r.path === '/')).toBe(true)
    const stack = (router.options.history as unknown as { state?: { back?: string | null } }).state
    // `back` 指向上一页；它不该是 `/`（否则说明 `/` 留在了历史里）
    expect(stack?.back ?? '').not.toBe('/')
  }, ROUTER_RELOAD_TIMEOUT)

  /**
   * 以下三条是**回归用例**，对应打包后的 Tauri webview 里「落点根本没执行」。
   *
   * 上面那几条**全部**走 `router.push('/')`——那是冷启动守卫唯一"正常"的分支，
   * 所以它们带着一个致命 bug 也能全绿。真实现场是：打包环境用自定义协议加载
   * 资源，`createWebHistory()` 拿到的 `location.pathname` **不是 `'/'`**（实测
   * `/index.html`、`C:/…/index.html` 这类路径匹配不到任何具名路由），于是旧判据
   * `to.path !== '/'` **每次提前 return**，守卫从不介入。
   *
   * 所以要在这里**模拟那个环境**：从一个不认识的首段进入，断言它仍被兜底接回
   * 首页、守卫仍把落点定到会话目录。
   *
   * （这三条当初是为「位置记忆」写的，现在记忆已下线——但它们真正的价值是钉住
   * **「守卫在打包环境下会执行」**这件事，那是落点本身的前提，故保留。）
   */
  describe('打包环境回归（首段不是 `/`，落点仍须执行）', () => {
    it('不认识的首段（如 index.html）被兜底接回首页，而非渲染空白页', async () => {
      const router = await freshRouter()
      // 夹具前提：这个首段**不是**任何具名路由（它只被 `:unknown(.*)*` 兜住）。
      // 断言 `name === undefined` 而不是 `matched.length === 0`：兜底路由本身就是
      // 一条 matched——**加了兜底之后**这里必然是 2，用 0 当前提就自相矛盾了。
      const resolved = router.resolve('/index.html')
      const matched = resolved.matched
      expect(resolved.name, '夹具前提：index.html 不是具名路由').toBeUndefined()
      expect(
        matched[matched.length - 1]?.path,
        '夹具前提：它由兜底路由承接'
      ).toBe('/:unknown(.*)*')

      await router.push('/index.html')
      await router.isReady()

      // 兜底 `:unknown(.*)*` → 首页 → 守卫接管 → 落到会话目录
      expect(router.currentRoute.value.path).toBe('/vdfs/session')
    }, ROUTER_RELOAD_TIMEOUT)

    it('不认识的首段进入时，落点照样执行（不是停在那个不认识的地址）', async () => {
      const router = await freshRouter()
      await router.push('/index.html')
      await router.isReady()

      expect(router.currentRoute.value.path).toBe('/vdfs/session')
    }, ROUTER_RELOAD_TIMEOUT)

    it('判据用路由名而非 path：显式导航到具名 home 也能触发落点', async () => {
      const router = await freshRouter()
      await router.push({ name: 'home' })
      await router.isReady()

      expect(router.currentRoute.value.path).toBe('/vdfs/session')
    }, ROUTER_RELOAD_TIMEOUT)
  })
})
