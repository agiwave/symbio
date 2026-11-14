/**
 * audit-assert — 审计回归测试的共用断言原语。
 *
 * 一个判据的对外契约只有两样：退出码，以及 stdout / stderr 里必须出现（或不出现）
 * 的形状。此前 57 份回归测试把这个契约各写一遍 `assert.equal(r.status, …)` +
 * 若干 `assert.match(r.stdout|r.stderr, …)`——287 组、652 行样板，改判据输出文案时
 * 要顺着每个用例改断言写法。这里收成一处：用例只给「期望码 + 一组正则」。
 *
 * 只收编**形状完全相同**的组（失败消息为 stdout/stderr、断言变量为 `r`、正则无第三参），
 * 带自定义失败消息 / 自定义断言的用例保持原样——它们写的不是同一件事。
 */
// @gate lib

import assert from 'node:assert/strict'
import { test } from 'node:test'

/** 标一条必须在 **stderr** 出现的形状；默认是 stdout（审计的 ERROR 走 stdout）。 */
export const onStderr = (re) => ({ stream: 'stderr', re })

/**
 * @param {{status:number, stdout:string, stderr:string}} r spawnSync 结果
 * @param {number} status 期望退出码
 * @param {Array<RegExp | {stream:string, re:RegExp}>} patterns 输出里必须出现的形状
 */
export function expectAudit(r, status, patterns = []) {
  assert.equal(r.status, status, r.stdout + r.stderr)
  for (const p of patterns) {
    if (p && typeof p === 'object' && !(p instanceof RegExp) && p.stream === 'stderr') {
      assert.match(r.stderr, p.re)
    } else {
      assert.match(r.stdout, p)
    }
  }
}

/**
 * 表驱动用例：`注入一组夹具 → 断言退出码 + 输出形状` 这一族用例只写数据，不写壳。
 *
 * 原先每个用例都要写四行：`test(name, () => {` / `const r = audit(...)` /
 * `expectAudit(r, status, patterns)` / `})`。这里把「用例名 + 夹具 + 期望码 +
 * 输出形状」压成一条 `auditCase(...)` 数据行，`test` / `r` / 断言调用由运行时承担。
 *
 * 用**零参 thunk** 交结果（而不是交夹具值）：
 *   1. 夹具仍在用例执行时构造，与手写块一致——不会把表达式提到模块加载期踩 TDZ；
 *   2. runner 是单参、多参、夹具跨多行，都能用同一条数据行表达。
 *
 * @param {string} name 用例名（进测试报告，保持与原来逐字相同）
 * @param {() => object} run 惰性产出 spawnSync 结果，如 `() => audit({...})`
 * @param {number} status 期望退出码
 * @param {Array<RegExp | {stream:string, re:RegExp}>} patterns 输出里必须出现的形状
 */
export function auditCase(name, run, status, patterns = []) {
  return test(name, () => expectAudit(run(), status, patterns))
}
