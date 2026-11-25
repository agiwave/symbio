/**
 * runtime-boundary-audit 回归测试：证明生产、构建、开发依赖与源码引用都会红。
 * 用法：node --test scripts/audits/runtime-boundary-audit.test.mjs
 */

import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { withTempTree } from '../fs-walk.mjs'
import { spawnSync } from 'node:child_process'
import { expectAudit, onStderr } from '../audit-assert.mjs'

const script = fileURLToPath(new URL('./runtime-boundary-audit.mjs', import.meta.url))
const MANIFEST = 'crates/symbio-runtime/Cargo.toml'
const LIB = 'crates/symbio-runtime/src/lib.rs'

function tree(extra = {}) {
  return {
    [MANIFEST]: '[dependencies]\nsymbio-platform = { path = "../symbio-platform" }\n',
    [LIB]: 'pub struct Runtime;\n',
    ...extra,
  }
}

function audit(files) {
  return withTempTree(files, (root) => spawnSync(process.execPath, [script, `--root=${root}`], {
    cwd: root,
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
  }), { prefix: 'runtime-boundary-' })
}

test('干净夹具：运行时只依赖平台 ⇒ 通过', () => {
  expectAudit(audit(tree()), 0, [/RB-001/, /均不依赖/])
})

test('生产依赖装配根 ⇒ 红', () => {
  expectAudit(audit(tree({
    [MANIFEST]: '[dependencies]\nsymbio = { path = "../symbio" }\n',
  })), 1, [/RB-001/, /Cargo.toml/])
})

test('开发依赖装配根同样 ⇒ 红', () => {
  expectAudit(audit(tree({
    [MANIFEST]: '[dev-dependencies]\nsymbio = { path = "../symbio" }\n',
  })), 1, [/RB-001/, /Cargo.toml/])
})

test('构建依赖与带引号键 ⇒ 红；注释不算', () => {
  expectAudit(audit(tree({
    [MANIFEST]: '[build-dependencies]\n"symbio" = { path = "../symbio" }\n',
  })), 1, [/RB-001/, /Cargo.toml/])
  expectAudit(audit(tree({
    [MANIFEST]: '[dependencies]\n# symbio = { path = "../symbio" }\nsymbio-platform = { path = "../symbio-platform" }\n',
  })), 0, [/RB-001/])
})

test('生产源码引用装配根 ⇒ 红；注释不算', () => {
  expectAudit(audit(tree({ [LIB]: 'use symbio::init::create_root_plugin;\n' })), 1, [/RB-001/, /lib.rs/])
  expectAudit(audit(tree({ [LIB]: 'use symbio :: init::create_root_plugin;\n' })), 1, [/RB-001/, /lib.rs/])
  expectAudit(audit(tree({ [LIB]: '// use symbio::init::create_root_plugin;\npub struct Runtime;\n' })), 0, [/RB-001/])
})

test('清单缺失 ⇒ 退 2，不静默通过', () => {
  expectAudit(audit({ [LIB]: 'pub struct Runtime;\n' }), 2, [onStderr(/范围读不出/)])
})
