// `lists` 判据族（RG-026 / RG-027 / RG-029）的反向验证 —— 固化成回归测试。
//
// 为什么必须反向验证：这几条判据的**失效形态全是恒真**。RG-027 判「同一 id 两个
// owner」，而 owner 常常是一份**生成物**（CURRENT.md 由 facts/current 生成）——
// 它若对判据隐形，「两个 owner」永远只看得见一个，RG-027 就成了一条**永远通过**
// 的装饰。所以这里注入五种坏形态，断言每一种都判红。判据自己红自己，是它唯一的自证。
//
// 落点是**夹具根**，不是真实仓：本测试曾直接改写真实 `docs/004-ref/003-ROUTES.md`
// （注入 → 审计 → 还原），而门禁 30-docs 阶段各测试文件**并发**跑——真实仓审计类
// 测试（registry-audit.test 等）在注入窗口读到被改的 RE-003，报出注入态的 RG-027
// 假红（窗口小，平时撞不上，撞上一次就是一次门禁假警报）。夹具复刻同一形状：
// `routes` 的 owner 是一份生成物条目（RE-006 形）+ 手写契约页（RE-003 形，
// `lists: [route-contracts=owner]` + 正文标记），注入按**字段与标记的字面形态**匹配，
// 不按路径——改夹具的 front-matter 或标记时，同步改这里；每条的
// `assert.notEqual(injected, orig)` 就是锚点失配的报警器。（真实仓本身的 lists
// 健康由 registry-audit.test 的真实仓全绿测试覆盖，不在这里重复。）
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { allKeys, loadKinds } from './registry-lib.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const REPO = path.resolve(scriptDir, '..')
const AUDIT = path.join(scriptDir, 'registry-audit.mjs')
const KINDS = loadKinds(REPO).kinds

function runAudit(root) {
  try {
    execFileSync(process.execPath, [AUDIT, `--root=${root}`], { cwd: REPO, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })
    return { red: false, msg: '' }
  } catch (e) {
    return { red: true, msg: (e.stdout ?? '') + (e.stderr ?? '') }
  }
}

/** 夹具根：各 kind README（形状文件）读真仓；两个 ref 条目复刻「生成物 owner + 手写契约页」。 */
function makeRoot() {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), 'lists-fixture-'))
  for (const cfg of Object.values(KINDS)) {
    const text = fs.readFileSync(path.join(REPO, cfg.file ?? path.join(cfg.dir, 'README.md')), 'utf8')
    const dir = path.join(base, cfg.dir)
    fs.mkdirSync(dir, { recursive: true })
    fs.writeFileSync(path.join(dir, 'README.md'), text)
  }
  const fm = (meta) => {
    const lines = ['---']
    for (const [k, v] of Object.entries(meta)) lines.push(`${k}: ${v}`)
    lines.push('---', '')
    return lines.join('\n')
  }
  // RG-011 要求任何根都有 SP-001（schema 的文档侧真相）——字段表与 kind 形状对账
  fs.writeFileSync(
    path.join(base, 'docs', '005-spec', '001.md'),
    fm({ kind: 'spec', id: 'SP-001', title: '字段规范（夹具）', status: 'active', created: '2026-10-11', scope: '夹具' }) +
      '\n## 规范\n\n条文。\n\n## 字段\n\n| 字段 | 说明 |\n|---|---|\n' +
      allKeys(KINDS).map((k) => `| \`${k}\` | 说明 |`).join('\n') +
      '\n\n## 判据\n\n人工核对。\n',
  )
  // owner A：生成物条目（RE-006 形）——正常登记，`routes` 的唯一真 owner
  fs.writeFileSync(
    path.join(base, 'docs', '004-ref', '006-当前事实表.md'),
    fm({ kind: 'ref', id: 'RE-006-当前事实表', title: '当前事实表', status: 'active', created: '2026-10-11', lists: '[routes=owner]' }) +
      '# 当前事实表\n\n<!-- list:routes owner -->\n\n| 地址 | 一句话职责 |\n|---|---|\n| `a/x` | 示例 |\n',
  )
  // owner B（注入落点）：手写契约页（RE-003 形）——地址在这里是**键**不是第二张清单
  fs.writeFileSync(
    path.join(base, 'docs', '004-ref', '003-ROUTES.md'),
    fm({ kind: 'ref', id: 'RE-003-ROUTES', title: '路由参考', status: 'active', created: '2026-10-11', lists: '[route-contracts=owner]' }) +
      '# 路由参考\n\n<!-- list:route-contracts owner -->\n\n| 地址 | 调用方 | 返回类型 |\n|---|---|---|\n| `a/x` | `session` | `Verdict` |\n',
  )
  return base
}

const CASES = [
  {
    name: 'RG-027 分工重叠：两篇都声称 owner 同一份清单',
    code: /RG-027/,
    // `routes` 的 owner 是生成物条目——让契约页也声称拥有它（声明与标记同时改，
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

const TARGET = path.join('docs', '004-ref', '003-ROUTES.md')

for (const c of CASES) {
  test(`lists 判据反向验证：${c.name}`, () => {
    const base = makeRoot()
    const target = path.join(base, TARGET)
    const orig = fs.readFileSync(target, 'utf8')
    const injected = c.apply(orig)
    assert.notEqual(injected, orig, '注入失败：没匹配到落点，判据等于没被测')
    try {
      fs.writeFileSync(target, injected)
      const r = runAudit(base)
      assert.ok(r.red, '判据没红——它对这一种坏形态是恒真的')
      assert.match(r.msg, c.code, `红了但不是 ${c.code} 那条：判据张冠李戴`)
    } finally {
      fs.rmSync(base, { recursive: true, force: true })
    }
  })
}

test('lists 判据基线：未注入时夹具全绿（否则上面的红都是别的原因）', () => {
  const base = makeRoot()
  try {
    const r = runAudit(base)
    assert.ok(!r.red, `基线就红了，后果是上面五条都不作数：\n${r.msg}`)
  } finally {
    fs.rmSync(base, { recursive: true, force: true })
  }
})
