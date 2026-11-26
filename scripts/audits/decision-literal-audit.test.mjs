import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import { withTempTree } from '../fs-walk.mjs'
import { auditDecisionLiterals } from './decision-literal-audit.mjs'

const forbidden = (n) => `D-${900 + n}`
const registration = '// @codes D-001\n'

test('DL-001：审计码与文档登记 ID 保留，架构判例标题与引用判红', () => {
  withTempTree({
    'scripts/audits/registered.mjs': registration,
    'docs/rule.md': '审计 D-001；登记见 DE-001 / SP-003。\n',
  }, (root) => {
    const clean = auditDecisionLiterals(root)
    assert.equal(clean.fatal, undefined)
    assert.deepEqual(clean.violations, [])

    const badId = forbidden(1)
    fs.writeFileSync(`${root}/docs/rule.md`, `### ${badId} 具体判例\n\n依据 ${badId}。\n`)
    fs.writeFileSync(`${root}/crate.rs`, `const OLD: &str = "${forbidden(2)}";\n`)
    const result = auditDecisionLiterals(root)
    assert.equal(result.violations.length, 3)
    assert.ok(result.violations.some((v) => v.kind === '决策标题'))
    assert.ok(result.violations.some((v) => v.key === 'crate.rs:1'))
  })
})

test('DL-001：已注册的审计码即使写成标题也不是架构判例', () => {
  withTempTree({
    'scripts/audits/registered.mjs': registration,
    'docs/rule.md': '### D-001 审计规则\n',
  }, (root) => {
    assert.deepEqual(auditDecisionLiterals(root).violations, [])
  })
})

test('DL-001：扫描范围或审计码名单读不出来 ⇒ 判据失效', () => {
  withTempTree({}, (root) => {
    assert.match(auditDecisionLiterals(root).fatal, /没有可读|没有读到任何审计/)
  }, { dirs: ['scripts'] })
  withTempTree({
    'scripts/audits/registered.mjs': '// @gate guard\n',
    'docs/rule.md': '# rule\n',
  }, (root) => {
    assert.match(auditDecisionLiterals(root).fatal, /没有可读|没有读到任何审计/)
  })
  assert.match(auditDecisionLiterals('/path/that/does/not/exist').fatal, /判据没有输入/)
})

test('DL-001：测试夹具或导入后的伪声明不能白名单编号', () => {
  withTempTree({
    'scripts/fake.mjs': `import fs from 'node:fs'\nconst fixture = \`\n// @codes ${forbidden(4)}\n\`\n`,
    'scripts/fake.test.mjs': `// @codes ${forbidden(5)}\n`,
    'docs/rule.md': '# rule\n',
  }, (root) => {
    assert.match(auditDecisionLiterals(root).fatal, /没有读到任何审计/)
  })
})

test('DL-001：临时目录不进入活跃文档扫描面', () => {
  withTempTree({
    'scripts/audits/registered.mjs': registration,
    'tmp/history.md': `### ${forbidden(3)} 历史草稿\n`,
  }, (root) => {
    assert.deepEqual(auditDecisionLiterals(root).violations, [])
  })
})
