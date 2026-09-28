import { createRouter, createWebHistory } from 'vue-router'
import MainLayout from '../views/MainLayout.vue'
import VdfsView from '../views/VdfsView.vue'
import { ensureSessionMountDir } from '../services/vdfsScheme'
import { logger } from '../utils/logger'

const MODULE_TAG = 'router'

/**
 * 冷启动落点：**恒为会话目录**（不是资源根）。
 *
 * 「打开应用就开始一段新对话」——冷启动不该落在资源根让用户自己找会话在哪，
 * 也不该去猜他上次在哪儿（见下）。
 *
 * ## 为什么不再「回到上次浏览的地址」
 *
 * 曾有过一级「有位置记忆 ⇒ 回上次地址」，**已整体下线**（2026-09-28）。它先后
 * 露出两个不可能在本机制内修好的毛病：
 *
 * 1. **判据拿不到真实的入口地址**：打包后的 webview 里 `location.pathname` 不是
 *    `/`，守卫的 `to.path !== '/'` 判据每次提前 return，记忆从未被读过——
 *    现象是「无论上次关在哪，每次都进会话」；
 * 2. **修好判据之后它反而是"正确地做错事"**：用户退出某智能体内部、重启后又被
 *    送回该智能体——那正是他上次停留的地方，还原**成功了**，只是用户要的不是
 *    这个。缺的是「回到中性位置」的入口，而那属于导航模型，不属于落点机制。
 *
 * 判「该不该回上次」需要一个**稳定的"中性位置"概念**，而 VDFS 的地址是分形的
 * （子空间与根空间的挂载名完全同名），纯地址形状分不出两者。与其继续加判据，
 * 不如把这一级去掉：**落点只有会话目录一个**，行为恒定、可预测。
 *
 * 盘上若残留旧版本的 `symbio.nav`，它已无人读取，不影响启动（不主动清理：
 * 那是用户存储，删别人的键比留着更糟）。
 *
 * 为什么落点要去解析挂载目录而不是写死 `/vdfs/session`：会话挂载段的段名由
 * 后端 provider 决定（前端零硬编码是全局约定），写死会在段名变更时静默跳到空目录。
 * 解析失败（根清单为空 / 列目录失败）退回 `/vdfs`——**降级不能阻断启动**，
 * 落在资源根仍可用，只是少了那一步「已经在新会话里了」的便利。
 *
 * ⚠️ 本函数是 **async，故只能用在导航守卫里，不能用作 `redirect`**：
 * vue-router（5.x 实测）的 `redirect` 返回值类型**不允许 Promise**（写
 * `redirect: () => coldStartPath()` 会直接 `TS2322`）。守卫是它提供的异步口子。
 */
async function coldStartPath(): Promise<string> {
  try {
    const mountDir = await ensureSessionMountDir()
    // 挂载目录是**数据地址**（`<根>/session`），浏览器地址要去掉根锚点前缀。
    // `vdfsRoot()` 返回的锚点带前后斜杠，故按段裁掉第一段。
    const segs = mountDir.split('/').filter(Boolean)
    return segs.length > 1 ? `/vdfs/${segs.slice(1).join('/')}` : '/vdfs'
  } catch (e) {
    logger.info(MODULE_TAG, 'cold start: session mount unresolved, falling back to /vdfs', e)
    return '/vdfs'
  }
}

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      component: MainLayout,
      children: [
        // 首页 = 上次浏览的地址；无记忆时 = 会话目录。两者都由 `beforeEach`
        // 解析（见下）——此处留空路径，由守卫决定去哪。
        { path: '', component: VdfsView, name: 'home' },
        // 首页 = `<根>` 目录本身；`/vdfs/<dir…>` = `<根>/<dir…>` 地址页
        // （push 出来的，如会话内部）。两者都是同一个 VdfsView：它把浏览器地址
        // 换算成**数据地址**（`<根>` / `<根>/<dir…>`）绑定给唯一的三栏控件
        // VdfsWorkbench——数据地址与浏览器地址是两个概念，路由只做承载。
        // :dir 可选 = `<根>` 之下的相对路径（可多级，如 /vdfs/session/<id>/workdir），
        // 深链与「浏览内部」push 出来的地址页都由本路由承接。
        { path: 'vdfs/:dir(.*)*', name: 'vdfs', component: VdfsView },
        // 旧地址的兼容 redirect（书签 / 深链）：指向同名挂载的 VDFS 页。
        // 机制与沿革见 docs/design/vdfs.md，此处只保留 URL 字面量。
        {
          path: 'entities/:types?',
          redirect: (to) => {
            const first = String(to.params.types ?? '').split(',')[0]
            return first && first !== 'all' ? `/vdfs/${first}` : '/vdfs'
          },
        },
        // 同理：各专项页面的旧地址一律指向 `/vdfs/<挂载名>`（挂载名与 kind 同名）
        { path: 'model-providers', redirect: () => '/vdfs/model' },
        { path: 'mcp', redirect: () => '/vdfs/mcp' },
        { path: 'skill', redirect: () => '/vdfs/skill' },
        { path: 'agent', redirect: () => '/vdfs/agent' },
        { path: 'settings', redirect: () => '/vdfs/plugin_manager' },
        // 兜底：**不认识的首段一律回首页**。
        //
        // 为什么必须有它（而不是让它 404）：打包后的 Tauri webview 从自定义协议
        // 加载前端资源，`location.pathname` 可能是 `/index.html` 或
        // `C:/…/index.html` 这类**任何路由都不匹配**的路径（实测二者
        // `matched.length === 0`）。没有兜底时应用会渲染成**空白页**——而这一条
        // 兜底把它接回首页，冷启动落点逻辑（守卫）随即接手。
        //
        // 放 `vdfs/:dir(.*)*` **之后**：那条更具体，先匹配；只有它接不住的
        // 首段（如 `index.html`）才落到这里。回首页用 `'/'` 而非 `/vdfs`——
        // 首页才是「交给守卫决定去哪」的入口。
        { path: ':unknown(.*)*', redirect: () => '/' }
      ]
    },
    // 容器内部的整页入口不再需要：内部结构由 VDFS 的
    // `<id>/<子类别>/<条目>` 寻址承担（钻入 = push 一个地址页，见 docs/design/vdfs.md）。
  ]
})

/**
 * 冷启动落地的**唯一实现**：把 `home` 路由镜像到实际落点。
 *
 * ## 判据为什么是**路由名**而不是 `to.path === '/'`
 *
 * 曾经写的是 `if (to.path !== '/') return true`。**在打包后的 Tauri webview 里
 * 这是个静默失效的判断**：前端资源走自定义协议加载，`createWebHistory()` 取
 * `location.pathname`，它**不保证恰好是 `'/'`**（WebView2 上可能是
 * `C:/…/index.html` 或带前缀的路径）。于是守卫**每次都提前 return**，位置记忆
 * 从未被读过——用户看到的现象正是「还原根本没有执行，每次都打开会话」。
 *
 * 而 dev 下 `location.pathname` 就是 `'/'`，**所以这不是单元测试能发现的**：
 * 单测里 `router.push('/')` 走的正是那条唯一正确的分支。
 *
 * 改用**路由名**（`home` = 路径为空的首页子路由）作判据：名字由本文件的配置
 * 决定，与运行环境的 URL 形状无关。`||` 保留 `'/'` 是为了兼容"名字尚未解析"
 * 的极早期调用（此时按旧判据兜底，行为不变）。
 *
 * ## 为什么用守卫而不是 `redirect`
 *
 * 落点要**异步**解析（`ensureSessionMountDir` 是一次目录读取），而 vue-router 的
 * `redirect` 不接受 Promise（见 coldStartPath 的注释）。守卫的 `return <path>`
 * 天然支持它返回的 Promise 被 await。
 *
 * `replace: true` 是刻意的：首页不该留在历史里——否则用户按返回键会回到首页，
 * 又被守卫重定向一次，形成「返回键无效」的死循环观感。
 *
 * 只在**首页**介入，其余地址（含 `/vdfs/…` 深链、刷新当前页）一律放行：
 * 那些是用户明确所在的位置，不该被"冷启动落点"改写。
 */
router.beforeEach(async (to) => {
  if (to.name !== 'home' && to.path !== '/') return true
  return { path: await coldStartPath(), replace: true }
})

export default router
