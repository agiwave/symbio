// `lists` 判据族（RG-026…RG-029）的反向验证 —— 固化成回归测试。
//
// 为什么必须反向验证：这四条判据的**失效形态全是恒真**。RG-027 判「同一 id 两个
// owner」，而 owner 常常是一份**生成物**（CURRENT.md 由 facts/current 生成、在
// SCAN_SKIP_FILES 里）。它若对判据隐形，「两个 owner」永远只看得见一个，RG-027
// 就成了一条**永远通过**的装饰——首次接入时反向验证当场抓到这一条漏过。
//
// 所以这里注入四种坏形态，断言每一种都判红。判据自己红自己，是它唯一的自证。
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
    apply: (t) => t.replace('lists: [routes=view]', 'lists: [routes=owner]').replace('<!-- list:routes view -->', '<!-- list:routes owner -->'),
  },
  {
    name: 'RG-029 声明未落实：front-matter 声明了，正文标记却没有',
    code: /RG-029/,
    apply: (t) => t.replace('<!-- list:routes view -->', ''),
  },
  {
    name: 'RG-029 声明漏写：正文有标记，front-matter 没声明',
    code: /RG-029/,
    apply: (t) => t.replace('lists: [routes=view]\n', ''),
  },
  {
    name: 'RG-026 role 词表封闭：想写第三个角色 `mirror`',
    code: /RG-026/,
    apply: (t) => t.replace('lists: [routes=view]', 'lists: [routes=mirror]'),
  },
  {
    name: 'RG-028 无主副本：声明 view 却不链到 owner',
    code: /RG-028/,
    // 把**所有** owner 提及都去掉。三次都栽在同一类错误上：**注入脚本假设了旧路径，
    // 而它自己没跟着搬**（先只删含一级相对路径的行、后只删文件名、
    // RE-006 搬进 `docs/ref/` 后两者都不对了）。故按**编号**删，不按路径删——
    // 编号是身份，路径是位置，位置会动。
    //
    // （这行注释里刻意不用反引号写旧路径：D-010 会把反引号里的路径当真路径判，
    //  而「讲一个已经不存在的东西」正是它要判的形态——判据的注释也在判据范围内。）
    apply: (t) => t.split('RE-006-当前事实表.md').join('那个文档.md').split('RE-006-当前事实表').join('那个文档'),
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
