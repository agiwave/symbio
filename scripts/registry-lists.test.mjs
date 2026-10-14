// `lists` 判据族（RG-026 / RG-027 / RG-029）的反向验证 —— 固化成回归测试。
//
// 为什么必须反向验证：这几条判据的**失效形态全是恒真**。RG-027 判「同一 id 两个
// owner」，而 owner 常常是一份**生成物**（CURRENT.md 由 facts/current 生成、在
// SCAN_SKIP_FILES 里）。它若对判据隐形，「两个 owner」永远只看得见一个，RG-027
// 就成了一条**永远通过**的装饰——首次接入时反向验证当场抓到这一条漏过。
//
// 所以这里注入五种坏形态，断言每一种都判红。判据自己红自己，是它唯一的自证。
//
// 落点是 RE-003-ROUTES.md（手写，`lists: [route-contracts=owner]` + 正文标记
// `<!-- list:route-contracts owner -->`）。⚠️ 注入按**字段与标记的字面形态**匹配，
// 不按路径——RE-003 重写（T-102 取消 view 后它从生成物变回手写）时这些锚点集体
// 失效过一次，症状是「红了但不是预期那条」。改 RE-003 的 front-matter 或标记时，
// 同步改这里；每条的 `assert.notEqual(injected, orig)` 就是锚点失配的报警器。
import fs from 'node:fs'
import path from 'node:path'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const TARGET = path.join(REPO, 'docs', 'ref', 'RE-003-ROUTES.md')
const AUDIT = path.join(REPO, 'scripts', 'registry-audit.mjs')

function runAudit() {
  try {
    execFileSync(process.execPath, [AUDIT], { cwd: REPO, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })
    return { red: false, msg: '' }
  } catch (e) {
    return { red: true, msg: (e.stdout ?? '') + (e.stderr ?? '') }
  }
}

const CASES = [
  {
    name: 'RG-027 分工重叠：两篇都声称 owner 同一份清单',
    code: /RG-027/,
    // `routes` 的 owner 是 RE-006 §2.3——让 RE-003 也声称拥有它（声明与标记同时改，
    // 保持两面对账干净，红的就是 RG-027 那一条）。
    apply: (t) => t
      .replace('lists: [route-contracts=owner]', 'lists: [route-contracts=owner, routes=owner]')
      .replace('<!-- list:route-contracts owner -->', '<!-- list:route-contracts owner -->\n<!-- list:routes owner -->'),
  },
  {
    name: 'RG-029 声明未落实：front-matter 声明了，正文标记却没有',
    code: /RG-029/,
    apply: (t) => t.replace('<!-- list:route-contracts owner -->\n', ''),
  },
  {
    name: 'RG-029 声明漏写：正文有标记，front-matter 没声明',
    code: /RG-029/,
    apply: (t) => t.replace('lists: [route-contracts=owner]\n', 'lists: []\n'),
  },
  {
    name: 'RG-026 role 词表封闭：想写第三个角色 `mirror`',
    code: /RG-026/,
    apply: (t) => t.replace('route-contracts=owner', 'route-contracts=mirror'),
  },
  {
    name: 'RG-026 role 词表封闭：想复活已取消的 `view`（投影绕开重复）',
    code: /RG-026/,
    // view 角色已整体取消（生成的重复也是重复）。写它 = 直接被词表挡下；
    // 标记那侧随后也会报 RG-029（标记正则只认 owner）——两条都指向同一件事：
    // 这条路没有了。想读另一份清单就指路，不重渲。
    apply: (t) => t.replace('route-contracts=owner', 'route-contracts=view'),
  },
]

for (const c of CASES) {
  test(`lists 判据反向验证：${c.name}`, () => {
    const orig = fs.readFileSync(TARGET, 'utf8')
    const injected = c.apply(orig)
    assert.notEqual(injected, orig, '注入失败：没匹配到落点，判据等于没被测')
    try {
      fs.writeFileSync(TARGET, injected)
      const r = runAudit()
      assert.ok(r.red, '判据没红——它对这一种坏形态是恒真的')
      assert.match(r.msg, c.code, `红了但不是 ${c.code} 那条：判据张冠李戴`)
    } finally {
      fs.writeFileSync(TARGET, orig)
    }
  })
}

test('lists 判据基线：未注入时全绿（否则上面的红都是别的原因）', () => {
  const r = runAudit()
  assert.ok(!r.red, `基线就红了，后果是上面四条都不作数：\n${r.msg}`)
})
