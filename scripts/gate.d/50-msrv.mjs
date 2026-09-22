// msrv 阶段：用 `rust-version` 声明的**最低**工具链实编译验证（symbio 与 cli）。
//
// 两个硬约束（都是踩出来的）：
// ① `RUSTUP_TOOLCHAIN` 只对 rustup 安装的 cargo 生效——发行版包会**静默忽略**它，
//    拿默认编译器冒充 MSRV 结论比不检查更糟。故先在同一环境探 `rustc --version`，
//    实际版本 ≠ 声明值就跳过（CI 的 msrv job 装了对应工具链，那里一定真跑）。
// ② 换编译器会让 cargo fingerprint 全部失效 ⇒ 用独立 CARGO_TARGET_DIR，
//    不污染日常构建缓存。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { red, stripAnsi } from '../color.mjs'
import { readMsrv } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const msrvTargetDir = path.join(repoRoot, '.workbuddy-ai', 'msrv-target')

export default {
  id: 'msrv',
  title: 'MSRV（rust-version 实编译校验）',
  *tasks(ctx) {
    const jobs = [
      ['symbio', path.join(repoRoot, 'symbio')],
      ['cli', path.join(repoRoot, 'cli')],
    ]
      .map(([name, dir]) => [name, dir, readMsrv(dir)])
      .filter(([, dir, msrv]) => msrv && fs.existsSync(path.join(dir, 'Cargo.toml')))

    if (jobs.length === 0) {
      yield {
        label: 'MSRV',
        run: async () => 'skipped',
        skipNote: '两个 workspace 都没有可读的 rust-version 声明',
      }
      return
    }

    for (const [name, dir, msrv] of jobs) {
      yield {
        label: `${name} @ ${msrv}`,
        run: async () => {
          const probe = await ctx.run({
            label: `${name}: rustc --version（要求 ${msrv}）`,
            cmd: 'rustc',
            args: ['--version'],
            cwd: dir,
            env: { RUSTUP_TOOLCHAIN: msrv },
            echo: 'none',
          })
          const actual = stripAnsi(probe.output).match(/^rustc (\d+\.\d+\.\d+)/m)?.[1] ?? null
          if (actual !== msrv) {
            const why = actual ? `当前生效的是 ${actual}` : '取不到 rustc 版本'
            console.log(`      ↳ 跳过（${why}）。要真验证需恰好装 ${msrv}：rustup toolchain install ${msrv}`)
            return 'skipped'
          }
          const r = await ctx.run({
            label: `${name}: cargo check --all-targets @ ${msrv}`,
            cmd: 'cargo',
            args: ['check', '--locked', '--all-targets'],
            cwd: dir,
            env: { RUSTUP_TOOLCHAIN: msrv, CARGO_TARGET_DIR: path.join(msrvTargetDir, name) },
          })
          if (!r.ok) {
            console.log(red(`      ↳ ${name} 在 ${msrv} 上编译失败 ⇒ rust-version 声明与实际不符`))
            return { ok: false, note: `exit=${r.code}` }
          }
          return { ok: true }
        },
      }
    }
  },
}
