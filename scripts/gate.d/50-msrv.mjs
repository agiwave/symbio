// msrv 阶段：用 `rust-version` 声明的**最低**工具链实编译验证。
//
// 范围由声明自己圈出来：`cargo metadata --no-deps` 给出每个 workspace 成员的
// `rust_version`，**声明了的**才纳入本判据（不声明的不在，比如 `symbio-tauri`——
// CI 的 msrv job 不装 GTK，把它编进来只会红在缺库上，与 MSRV 无关）。
//
// 两个硬约束（都是踩出来的）：
// ① `RUSTUP_TOOLCHAIN` 只对 rustup 安装的 cargo 生效——发行版包会**静默忽略**它，
//    拿默认编译器冒充 MSRV 结论比不检查更糟。故先在同一环境探 `rustc --version`，
//    实际版本 ≠ 声明值就跳过（CI 的 msrv job 装了对应工具链，那里一定真跑）。
// ② 换编译器会让 cargo fingerprint 全部失效 ⇒ 用独立 CARGO_TARGET_DIR（按版本分
//    目录），不污染日常构建缓存；与 backend 不争 `.cargo-lock`，所以本阶段可并发。
//
// ⚠️ 这一层曾经是**空转**的：jobs 写死了 D-191 之前那两个顶层目录（symbio 与 cli），
// 拆成 crate 之后路径不存在 ⇒ `jobs.length === 0` ⇒ 每轮只产出一条 `skipped`
// （校准里连续 5 轮墙钟 0s），`rust-version` 又变回「没人验证的一句话」。所以判据
// 现在从**仓里真实的声明**派生：路径搬了它跟着搬，加一个 crate 它自动多编一个。
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, stripAnsi } from '../color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const msrvTargetDir = path.join(repoRoot, '.workbuddy-ai', 'msrv-target')

/**
 * 声明了 `rust-version` 的 workspace 成员，按版本分组：`Map<version, [crate]>`。
 *
 * 读不到（没有 cargo、清单损坏、非 workspace 仓）返回 null——**不能返回空表**：
 * 空表会被当成「本机没有需要验证的 MSRV」，而真实含义是「这条判据没得到信号」。
 *
 * 进程内记忆化：一轮门禁里 `tasks()` 被物化两遍（起跑前收校准键、`runStage` 里编排），
 * 而 `cargo metadata` 一次约 0.5s。同进程内不该为一个不变的事实付两遍。
 */
let msrvGroups = null
let msrvGroupsRead = false

function cratesByMsrv() {
  if (msrvGroupsRead) return msrvGroups
  msrvGroupsRead = true
  msrvGroups = readMsrvGroups()
  return msrvGroups
}

function readMsrvGroups() {
  const r = spawnSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: false,
    maxBuffer: 64 * 1024 * 1024,
  })
  if (r.status !== 0 || !r.stdout) return null
  let meta
  try {
    meta = JSON.parse(r.stdout)
  } catch {
    return null
  }
  const byVersion = new Map()
  for (const p of meta.packages ?? []) {
    const declared = p.rust_version
    if (!declared) continue
    // rustup 的工具链名带 patch，而 `rust-version = "1.91"` 不带；补全后才能比对。
    const parts = String(declared).trim().split('.')
    while (parts.length < 3) parts.push('0')
    const version = parts.join('.')
    if (!byVersion.has(version)) byVersion.set(version, [])
    byVersion.get(version).push(p.name)
  }
  return byVersion
}

export default {
  id: 'msrv',
  title: 'MSRV（rust-version 实编译校验）',
  // 阶段级并发：CARGO_TARGET_DIR 指向独立的 `.workbuddy-ai/msrv-target/<版本>`，与
  // backend 的编译**不争 `.cargo-lock`**；1.91 与 1.93 的产物本就互不复用，并发只是让
  // 两套 rustc 同时推进（单 crate 编译的 CPU 上不去，两条编译流水线叠着跑才用得起来）。
  parallel: true,
  *tasks(ctx) {
    const byVersion = cratesByMsrv()
    if (byVersion === null) {
      yield {
        label: 'MSRV',
        run: async () => ({
          ok: false,
          note: 'cargo metadata 读不到 ⇒ 无法确定有哪些 rust-version 声明（判据没拿到信号，不算通过）',
        }),
      }
      return
    }
    // 版本升序：多数仓里只有一个声明；真有多个时，低版本那条是更硬的下限，先跑先暴露。
    const versions = [...byVersion.keys()].sort((a, b) => a.localeCompare(b, undefined, { numeric: true }))
    if (versions.length === 0) {
      yield {
        label: 'MSRV',
        run: async () => ({ ok: false, note: '没有任何成员声明 rust-version ⇒ 声明本身消失了' }),
      }
      return
    }

    for (const msrv of versions) {
      const crates = byVersion.get(msrv).sort()
      yield {
        label: `${crates.length} crates @ ${msrv}`,
        run: async () => {
          const probe = await ctx.run({
            label: `rustc --version（要求 ${msrv}）`,
            cmd: 'rustc',
            args: ['--version'],
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
            label: `cargo check --all-targets @ ${msrv}`,
            cmd: 'cargo',
            args: ['check', '--locked', '--all-targets', ...crates.flatMap((name) => ['-p', name])],
            env: { RUSTUP_TOOLCHAIN: msrv, CARGO_TARGET_DIR: path.join(msrvTargetDir, msrv) },
          })
          if (!r.ok) {
            console.log(
              red(`      ↳ ${crates.length} 个声明了 ${msrv} 的 crate 编不过 ⇒ rust-version 声明与实际不符`),
            )
            return { ok: false, note: `exit=${r.code}`, logFile: r.logFile }
          }
          return { ok: true }
        },
      }
    }
  },
}
