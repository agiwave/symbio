#!/usr/bin/env node
/**
 * tauri-binary — 桌面壳（Tauri）产物的**路径解析**与**新鲜度机制**。
 *
 * ## 为什么要有这个文件：一个已经付过代价的假信号
 *
 * 排查会话日志时，日志里出现了几条**当前源码里根本不存在**的行（格式对不上：
 * 日志是 `[model INFO] 请求发起 (provider=…)`，而源码里只有
 * `[LLM] 请求发起 {} (model=…)`）。第一反应是去读代码找「是不是没接上」——
 * 真相却是**跑的是过期产物**，排查方向从第一步就错了。这与 CLI 侧那次假失败
 * （见 `cli-binary.mjs` 的文件头）是同一类失败，只是这次发生在壳上。
 *
 * 结论与 CLI 侧一致：**产物必须能自报身份**。本文件提供两件事：
 *
 * 1. **构建戳**（`.symbio-tauri.build-stamp`，与二进制同目录 ⇒ 落在 `target/` 内、
 *    天然被 gitignore）：内容是构建输入的**内容指纹**。
 *    - `--stamp` 在构建**前**写它——`tauri.conf.json` 的 `beforeDevCommand` /
 *      `beforeBuildCommand` 会先调一次，因此走 `npm run tauri dev|build` 就自动带上；
 *    - `ensureTauriBinary` 在构建**后**写它（手工重建那条路）。
 * 2. **运行时回显**：壳启动时读**自己旁边**那个戳文件，把值打进日志首行
 *    （`tauri/src-tauri/src/main.rs` 的 `build_id`）。于是任何一段日志自带
 *    「我是哪份源码产出的」——与 `node scripts/tauri-binary.mjs --print` 一比即知。
 *
 * ## 为什么指纹在 JS 侧算，而不是在 Rust 的 `build.rs` 里
 *
 * 指纹的**唯一真相**在这里（门禁回归、人工诊断、壳运行时回显共用同一个值）。
 * 在 Rust 里再实现一遍哈希 = 两份必须逐字节一致的实现，迟早分叉；而把它塞进
 * `build.rs` 还要处理 `rerun-if-changed` 的语义——**一旦 emit 任何一条，cargo
 * 的默认启发式（「包内任何文件变了就重跑」）就失效**，漏一个输入的表现恰好是
 * 「代码是新的、指纹是旧的」，正是本机制要消灭的那种不一致。
 * 因此 Rust 侧只做一件零风险的事：**读旁边那个文件**。
 *
 * ## 判据为什么是内容指纹而不是 mtime
 *
 * 与 `cli-binary.mjs` 同款理由：mtime 会被 checkout / 解压 / 复制 / 恢复备份改写，
 * 方向还不确定——既可能假旧（多一次无害的重建），也可能**假新**（把过期产物当
 * 最新，正是要防的）。内容指纹只问一件事：**构建输入变了吗**。
 *
 * 大文件改用 **(大小, mtime)**（阈值见 `LARGE_INPUT_BYTES`）：它每次变动大小或
 * 时间必变，而每次为它读几 MB 纯属浪费。
 *
 * ## 戳为什么还要配一条 mtime 判据
 *
 * 戳是**构建前**写的声明（「这一刻的构建输入是这些」），因此它只在那次构建真的
 * 产出产物之后才是**关于这份产物**的事实。若构建失败，旧产物仍在原处，而戳已经
 * 指向新输入——那正是「把过期产物当最新」，本机制存在的全部理由就是防它。
 *
 * 所以再加一条：**产物不得比戳更旧**。这不是在用 mtime 判断「内容有没有变」
 * （那是指纹的活），而是问一个**因果顺序**问题——「写这条声明之后，构建有没有
 * 产出它」——而这恰好是 mtime 唯一可靠回答的那类问题。
 *
 * 配套：`writeTauriStamp` 在**内容未变时不重写**。否则每次「什么都不用重建」的
 * 构建都会把戳刷新到比产物新，这条判据立刻变成假警报。
 *
 * ## 构建输入 = 什么
 *
 * - `tauri/src-tauri/src`、`capabilities`、`Cargo.toml` / `Cargo.lock` /
 *   `build.rs` / `tauri.conf.json`（壳自身）；
 * - `symbio/src` 与它的清单——壳的 `Cargo.toml` 写着
 *   `symbio = { path = "../../symbio" }`，**整棵插件树被编译进壳**；
 *   那几条「源码里不存在」的日志正是从这里来的。
 *
 * **不含 `tauri/src`（前端）**：dev 下前端由 Vite 现服（改了就热更，产物无需重建），
 * release 下前端由**同一条命令**先行构建（`beforeBuildCommand: npm run build`）。
 * 因此「前端新不新鲜」不是这个指纹能回答、也不需要它回答的问题。
 *
 * ## 已知边界（写清楚，免得把绿灯当证明）
 *
 * 绕过 `npm run tauri …` 直接 `cargo build` 时**不会写戳** ⇒ 状态退化为「没有
 * 构建戳」，壳回显 `unknown`。这是刻意的：与其猜一个来源，不如承认不知道。
 *
 * ## 用法
 *
 *   node scripts/tauri-binary.mjs            # 需要就重建；打印一行结论
 *   node scripts/tauri-binary.mjs --check    # 只判不建；不可信则退出码 1
 *   node scripts/tauri-binary.mjs --print    # 只打印当前指纹（壳回显的就是它）
 *   node scripts/tauri-binary.mjs --stamp    # 把当前指纹写进构建戳（构建前置步骤）
 */

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const EXE = `symbio-tauri${process.platform === 'win32' ? '.exe' : ''}`

/** 超过此体积的构建输入按 (大小, mtime) 判，不读内容（内嵌资产） */
const LARGE_INPUT_BYTES = 1024 * 1024

/** 戳文件名（与二进制同目录） */
const STAMP_NAME = '.symbio-tauri.build-stamp'

/**
 * 壳二进制的落点：根 workspace 统一后，产物落在仓库根 `target/<profile>/`
 * （不再在 `tauri/src-tauri/target/`）。两个 profile 都探：dev 走 debug、打包走
 * release，而「哪一份是你手上那个」不该由调用方猜。
 *
 * ⚠️ debug 在前：dev 是日常路径，而「候选都还没构建出来」时默认要建的就是它。
 */
export function tauriBinaryForProfile(repoRoot, profile) {
  return path.join(repoRoot, 'target', profile, EXE)
}

export function tauriBinaryCandidates(repoRoot) {
  return [tauriBinaryForProfile(repoRoot, 'debug'), tauriBinaryForProfile(repoRoot, 'release')]
}

/**
 * 取**最新**的那个已存在候选，而不是第一个。
 *
 * 两个 profile 可能各躺着一份（dev 跑过、也打过包）。此时「第一个」可能是几个月前
 * 那份——正是要防的过期产物。构建产物里最新的一份才对应最近一次构建。
 */
export function tauriBinaryPath(repoRoot) {
  const candidates = tauriBinaryCandidates(repoRoot)
  let best = null
  for (const p of candidates) {
    let st
    try {
      st = fs.statSync(p)
    } catch {
      continue
    }
    if (!best || st.mtimeMs > best.mtimeMs) best = { p, mtimeMs: st.mtimeMs }
  }
  return best ? best.p : candidates[0]
}

/** 由二进制路径反推它属于哪个 profile（戳就写在它旁边，故只需目录名） */
export function profileOfBinary(binaryPath) {
  const dir = path.basename(path.dirname(binaryPath))
  return dir === 'release' ? 'release' : 'debug'
}

/** 递归列出目录下的全部文件（目录不存在 ⇒ 空，不抛） */
function walkFiles(dir, out = []) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walkFiles(p, out)
    else if (e.isFile()) out.push(p)
  }
  return out
}

/**
 * 构建输入清单 = 壳的源码树 + 它挂载的插件树 + 各自的清单。
 *
 * 用「列目录」而不是「读 Cargo 的依赖图」：依赖图要跑 `cargo metadata`（几百毫秒
 * 且需要网络语义），而这里只需要**宁可多算**——多纳入一个文件只会多触发一次无害的
 * 增量构建，漏掉一个才会让过期产物蒙混过关。
 */
export function tauriBuildInputs(repoRoot) {
  const files = []
  for (const sub of ['tauri/src-tauri/src', 'tauri/src-tauri/capabilities', 'symbio/src']) {
    files.push(...walkFiles(path.join(repoRoot, sub)))
  }
  for (const rel of [
    'tauri/src-tauri/Cargo.toml',
    'tauri/src-tauri/Cargo.lock',
    'tauri/src-tauri/build.rs',
    'tauri/src-tauri/tauri.conf.json',
    'symbio/Cargo.toml',
    'symbio/Cargo.lock',
  ]) {
    const p = path.join(repoRoot, rel)
    if (fs.existsSync(p)) files.push(p)
  }
  return files.sort()
}

/** 构建输入的 sha256 指纹（同一份输入 ⇒ 同一个值） */
export function tauriBuildFingerprint(repoRoot) {
  const h = createHash('sha256')
  for (const p of tauriBuildInputs(repoRoot)) {
    const rel = path.relative(repoRoot, p).split(path.sep).join('/')
    let st
    try {
      st = fs.statSync(p)
    } catch {
      continue
    }
    if (st.size > LARGE_INPUT_BYTES) {
      h.update(`L\0${rel}\0${st.size}\0${Math.trunc(st.mtimeMs)}\n`)
    } else {
      h.update(`T\0${rel}\0`)
      h.update(fs.readFileSync(p))
      h.update('\0\n')
    }
  }
  return h.digest('hex')
}

/** 二进制旁的构建戳路径 */
export function tauriStampPath(binaryPath) {
  return path.join(path.dirname(binaryPath), STAMP_NAME)
}

/** 某个 profile 的构建戳路径（`--stamp` 用：构建前还不知道二进制在哪一份里） */
export function tauriStampForProfile(repoRoot, profile) {
  return tauriStampPath(tauriBinaryForProfile(repoRoot, profile))
}

/**
 * 把当前指纹写进构建戳。
 *
 * 两个调用点：`tauri.conf.json` 的 `before*Command`（`npm run stamp:dev|release`）
 * 与 `ensureTauriBinary`——都是构建**前**。
 *
 * ⚠️ **内容没变就不重写。** 戳的 mtime 是判据的一部分（「构建必须留下不比它更旧的
 * 产物」，见 `tauriBinaryState`）。无谓地刷新它，会把每一次「什么都不用重建」的
 * 构建变成一次假警报——而假警报会让这条判据被学会忽略，比没有更糟。
 */
export function writeTauriStamp(repoRoot, { profile = 'debug' } = {}) {
  const stamp = tauriStampForProfile(repoRoot, profile)
  const fingerprint = tauriBuildFingerprint(repoRoot)
  const existing = readStampFile(stamp)
  const rewritten = existing !== fingerprint
  if (rewritten) {
    fs.mkdirSync(path.dirname(stamp), { recursive: true })
    fs.writeFileSync(stamp, fingerprint, 'utf8')
  }
  return { stamp, fingerprint, rewritten }
}

function readStampFile(stampPath) {
  try {
    return fs.readFileSync(stampPath, 'utf8').trim()
  } catch {
    return null
  }
}

function readStamp(binaryPath) {
  return readStampFile(tauriStampPath(binaryPath))
}

function mtimeMs(p) {
  try {
    return fs.statSync(p).mtimeMs
  } catch {
    return null
  }
}

/**
 * 产物当前状态：`fresh` 为真当且仅当三条同时成立——
 *
 * 1. 产物**存在**；
 * 2. 戳 == 当前指纹（**内容**对得上：这份产物是按这些输入构建的）；
 * 3. 产物**不比戳旧**（**顺序**对得上：写戳之后的那次构建真的产出了它）。
 *
 * 第 3 条防的是「构建失败，旧产物留在原处、戳已指向新输入」——那时第 2 条会
 * 亮绿灯而事实相反。详见文件头「戳为什么还要配一条 mtime 判据」。
 *
 * `staleReason` 是给人看的一句话——日志溯源最贵的是排查方向，把原因直接说出来。
 */
export function tauriBinaryState(repoRoot) {
  const binaryPath = tauriBinaryPath(repoRoot)
  const exists = fs.existsSync(binaryPath)
  const fingerprint = tauriBuildFingerprint(repoRoot)
  const stamped = exists ? readStamp(binaryPath) : null
  const stampTime = exists ? mtimeMs(tauriStampPath(binaryPath)) : null
  const binaryTime = exists ? mtimeMs(binaryPath) : null
  const builtAfterStamp = stampTime == null || binaryTime == null || binaryTime >= stampTime
  const fresh = exists && stamped === fingerprint && builtAfterStamp
  let staleReason = ''
  if (!exists) staleReason = '产物不存在'
  else if (stamped == null) staleReason = '没有构建戳（直接用 cargo build 构建的，壳会回显 unknown）'
  else if (stamped !== fingerprint) staleReason = '构建输入自上次构建后已改变'
  else if (!builtAfterStamp) staleReason = '产物比构建戳旧（写戳之后的那次构建没有产出它）'
  return {
    binaryPath,
    exists,
    fingerprint,
    stamped,
    fresh,
    staleReason,
    builtAfterStamp,
    stampPath: tauriStampPath(binaryPath),
  }
}

/**
 * 保证「接下来跑的壳对应当前源码」。
 *
 * - 已新鲜 ⇒ 直接返回（不启动 cargo）。
 * - 不新鲜且 `build` 为真 ⇒ 先写戳（构建前置，与 `npm run tauri dev` 同一条路），
 *   再 `cargo build`（默认 debug；已存在 release 产物时按它所属的 profile 走）。
 * - 不新鲜且 `build` 为假 ⇒ **抛错**。这是本机制的全部要点：宁可停，也不拿过期
 *   产物跑出一堆与源码矛盾的日志行。
 *
 * 构建失败时**不删戳**：戳与产物都在原处，状态会以更精确的那条理由
 * （「产物比构建戳旧」）报出来，而错误信息本身已经带上了 cargo 的尾部输出。
 *
 * 同步实现（`spawnSync`）：调用方本身就是同步的，异步化没有收益。
 */
export function ensureTauriBinary(repoRoot, { build = true, log = null, release = null } = {}) {
  const before = tauriBinaryState(repoRoot)
  if (before.fresh) return { ...before, rebuilt: false }

  if (!build) {
    throw new Error(
      `Tauri 壳产物不可信（${before.staleReason}）：${before.binaryPath}\n` +
        `请先构建：node scripts/tauri-binary.mjs（等价于 cd tauri/src-tauri && cargo build）`,
    )
  }

  const profile = release === true ? 'release' : release === false ? 'debug' : profileOfBinary(before.binaryPath)
  if (log) log(`${before.staleReason} ⇒ cargo build${profile === 'release' ? ' --release' : ''}`)

  writeTauriStamp(repoRoot, { profile })

  const args = ['build']
  if (profile === 'release') args.push('--release')
  const r = spawnSync('cargo', args, {
    cwd: path.join(repoRoot, 'tauri', 'src-tauri'),
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  })
  if (r.error) {
    throw new Error(`无法启动 cargo（${r.error.message}）——构建壳需要本机 Rust 工具链`)
  }
  if (r.status !== 0) {
    const tail = `${r.stderr || ''}\n${r.stdout || ''}`.trim().split('\n').slice(-12).join('\n')
    throw new Error(`cargo build 失败（exit=${r.status}）\n${tail}`)
  }

  const after = tauriBinaryState(repoRoot)
  if (!after.exists) {
    throw new Error(
      `cargo build 成功但找不到产物（预期其一：${tauriBinaryCandidates(repoRoot).join(' | ')}）`,
    )
  }
  if (!after.fresh) {
    throw new Error(`cargo build 成功但状态仍不可信（${after.staleReason}）：${after.binaryPath}`)
  }
  if (log) log(`已重建：${after.binaryPath}`)
  return { ...after, rebuilt: true }
}

// ==================== CLI 入口 ====================

const invokedDirectly =
  process.argv[1] != null && path.basename(process.argv[1]) === path.basename(fileURLToPath(import.meta.url))

if (invokedDirectly) {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
  const checkOnly = process.argv.includes('--check')
  const printOnly = process.argv.includes('--print')
  const stampOnly = process.argv.includes('--stamp')
  const wantRelease = process.argv.includes('--release')

  // `--print` 必须只输出指纹本身、且退出码 0：它会被别的程序（诊断脚本）当值读走。
  if (printOnly) {
    process.stdout.write(`${tauriBuildFingerprint(repoRoot)}\n`)
    process.exit(0)
  }

  if (stampOnly) {
    const { stamp, fingerprint, rewritten } = writeTauriStamp(repoRoot, {
      profile: wantRelease ? 'release' : 'debug',
    })
    const rel = path.relative(repoRoot, stamp)
    console.log(
      rewritten
        ? `      ✓ 构建戳已写：${rel}（${fingerprint.slice(0, 12)}）`
        : `      ✓ 构建输入未变，构建戳保持原样：${rel}（${fingerprint.slice(0, 12)}）`,
    )
    process.exit(0)
  }

  try {
    const st = ensureTauriBinary(repoRoot, {
      build: !checkOnly,
      log: (m) => console.log(`      ${m}`),
    })
    console.log(
      st.rebuilt
        ? `      ✓ Tauri 壳已重建（${path.relative(repoRoot, st.binaryPath)}）`
        : `      ✓ Tauri 壳产物与当前源码一致（${path.relative(repoRoot, st.binaryPath)}）`,
    )
    process.exit(0)
  } catch (e) {
    console.error(`      ✗ ${e.message}`)
    process.exit(1)
  }
}
