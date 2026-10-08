/**
 * gen-routes-ts / route-facts 回归测试
 *
 * 一个只会亮绿灯的守卫等于没有守卫。这里对**每一个判据方向**都注入一次：
 * 后端加一条臂 ⇒ 生成物必须有它；不是臂的字符串 ⇒ 不能有它；
 * 手改生成物 ⇒ 与代码不一致必须被抓住。
 *
 * 跑法：node --test scripts/gen-routes-ts.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { controlPlaneRoutes, extractRouteArms, routeConstName } from './route-facts.mjs'
import { render } from './gen-routes-ts.mjs'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const GEN_FILE = path.join(REPO, 'tauri', 'src', 'constants', 'routes.gen.ts')

/** 一棵只有 `demo` 插件的临时仓库（route-facts 读 `<root>/symbio/src/plugins`） */
function fixtureRepo(pluginName, routeSource) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'gen-routes-ts-'))
  const dir = path.join(root, 'symbio', 'src', 'plugins', pluginName)
  fs.mkdirSync(dir, { recursive: true })
  fs.writeFileSync(path.join(dir, 'plugin.rs'), routeSource)
  return root
}

const ROUTE_FN = (arms) => `
impl Plugin for DemoPlugin {
    async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> Result<Payload> {
        let path = ctx.get(PATH).unwrap_or_default();
        match path {
${arms}
            _ => Err(NotFound),
        }
    }
}
`

test('后端加一条臂 ⇒ 生成物里有同名同值的常量', () => {
  const root = fixtureRepo('demo', ROUTE_FN(`            "demo/ping" => ok(),`))
  try {
    const routes = controlPlaneRoutes(root)
    assert.deepEqual(routes, ['demo/ping'])
    assert.match(render(routes), /export const ROUTE_DEMO_PING = 'demo\/ping'/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('相对臂补回目录名前缀；home 是根容器，臂本身就是完整地址', () => {
  assert.deepEqual(extractRouteArms(ROUTE_FN(`            "ping" => ok(),`), 'demo'), [
    'demo/ping',
  ])
  assert.deepEqual(extractRouteArms(ROUTE_FN(`            "home/reload" => ok(),`), 'home'), [
    'home/reload',
  ])
})

test('复合臂（`"a/b" | "c"`）两条都收', () => {
  const arms = extractRouteArms(ROUTE_FN(`            "a/b" | "c/d" => ok(),`), 'x')
  assert.deepEqual(arms, ['x/a/b', 'x/c/d'])
})

test('不是臂的字符串不算路由（`get("…")` 里的是参数名）', () => {
  // 这正是提取口径的边界：整段抓字符串会把 `cfg.get("approved")` 报成一条路由，
  // 于是一个不存在的地址进了前端契约——它不会红，只会在运行期变成 NotFound。
  const src = ROUTE_FN(`            "demo/ping" => ok(),`) + `let v = cfg.get("approved");\n`
  assert.deepEqual(extractRouteArms(src, 'demo'), ['demo/ping'])
})

test('常量名与地址逐字同构：去 `ROUTE_` 前缀 = 地址大写（`/` → `_`）', () => {
  assert.equal(routeConstName('session/chat/send'), 'ROUTE_SESSION_CHAT_SEND')
  assert.equal(routeConstName('home/get_homedir'), 'ROUTE_HOME_GET_HOMEDIR')
})

test('真仓的生成物与后端路由一致（手改过它就红）', () => {
  const committed = fs.existsSync(GEN_FILE) ? fs.readFileSync(GEN_FILE, 'utf8') : ''
  assert.equal(committed, render(controlPlaneRoutes(REPO)))
})
