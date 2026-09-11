#!/usr/bin/env node
// 修复被「不同 rustc 版本」污染的 cargo target 缓存。
//
// ## 背景（2026-09-11 实测定论）
//
// cargo 产物文件名里的哈希**不含编译器版本**（`deps/lib<krate>-<hash>.rmeta`），
// 所以两代编译器会共用同一个文件路径。当 target 目录里混入了另一版编译器写下的
// `.rmeta`，而 cargo 又把它当作「新鲜」复用时，下游 crate 编译就会报：
//
//   error[E0514]: found crate `X` compiled by an incompatible version of rustc
//     = note: crate `X` compiled by rustc 1.98.0 ...: ...\deps\libX-<hash>.rmeta
//             = help: please recompile that crate using this compiler (rustc 1.93.1 ...)
//
// 典型触发：`cli/` 与 `symbio/` 是兄弟目录，`../symbio/rust-toolchain.toml`（锁 1.93.1）
// 管不到 cli/，于是 cli 曾用 default channel（本机 stable = 1.98）构建，把 1.98 的产物
// 写进了 symbio 共用的 target 目录。（已由 `cli/rust-toolchain.toml` 根治，本脚本
// 只负责清掉历史污染。）
//
// ## 做法：以构建日志为唯一证据
//
// 不去猜「哪个 rustc 哈希才是当前编译器」——实测 fingerprint 里的 `"rustc"` 字段
// **不是**编译器版本哈希（1.93.1 与 1.98 可以写出同一个值），据此判断会误伤。
// 改为直接读 E0514 报错：报错里点名了每个被判定为「异版本」的产物路径与哈希，
// 那就是确凿的过期单元。删掉它们的 `.fingerprint/<pkg>-<hash>/`，
// cargo 的硬保证是「没有 fingerprint 文件 ⇒ 必然重编」，下一轮就会用当前编译器重编。
//
// 循环「构建 → 收集 E0514 证据 → 清指纹」直到构建成功或不再有进展。
//
// ## 安全边界
//
// - **只删 `.fingerprint/` 目录**，不删 `deps/` 产物（指纹缺失即触发重编并覆盖产物）。
// - 命中 `BLOCKLIST` 的 crate **绝不删除**：其构建脚本含 C/汇编编译，在无 cmake/nasm/cl
//   的环境里重跑会**挂死**（本机 Agent 环境即是）。若这些单元也过期，脚本明确报错，
//   交回开发者 shell 处理。
// - 默认 dry-run，必须显式 `--fix` 才动文件。
//
// 用法：
//   node scripts/repair-target-cache.mjs            # 仅调查
//   node scripts/repair-target-cache.mjs --fix      # 执行修复

import { spawnSync } from 'node:child_process';
import { readdirSync, rmSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CLI_DIR = path.resolve(HERE, '..');
const TARGET_DIR = path.resolve(CLI_DIR, '..', 'symbio', 'target', 'debug');
const FP_DIR = path.join(TARGET_DIR, '.fingerprint');

// 构建脚本含 C/汇编编译；在无 cmake/nasm/cl 的环境重跑会挂死 → 绝不清理
const BLOCKLIST = /^(aws-lc|ring|ort|libsqlite3-sys|zstd|lzma|bzip2|turbojpeg|esaxx|sentencepiece|onig|torch)/;

const argv = process.argv.slice(2);
const FIX = argv.includes('--fix');
const MAX_ITERATIONS = Number(argValue('--iterations') ?? 8);

function argValue(name) {
  const hit = argv.find((a) => a.startsWith(`${name}=`));
  return hit ? hit.slice(name.length + 1) : undefined;
}

function runCargo() {
  process.stdout.write('  · cargo build --offline --keep-going … ');
  const r = spawnSync('cargo', ['build', '--offline', '--keep-going'], {
    cwd: CLI_DIR,
    encoding: 'utf8',
    shell: false,
    maxBuffer: 256 * 1024 * 1024,
  });
  const out = `${r.stdout ?? ''}${r.stderr ?? ''}`;
  console.log(r.status === 0 ? 'OK' : `exit ${r.status}`);
  return { status: r.status, out };
}

/**
 * 从构建日志里提取被判定为「异版本编译器产物」的单元哈希。
 *
 * 只认 E0514 的 note 行——它们形如：
 *   crate `windows_sys` compiled by rustc 1.98.0 (...): \\?\...\deps\libwindows_sys-7eca5abb391c3263.rmeta
 * 哈希就是 cargo 眼里的单元标识，可直接定位 `.fingerprint/<pkg>-<hash>/`。
 */
function staleHashesFromLog(log) {
  const hashes = new Set();
  for (const line of log.split('\n')) {
    if (!line.includes('compiled by rustc')) continue;
    if (!line.includes('E0514') && !/compiled by rustc \d/.test(line)) continue;
    const m = line.match(/lib[A-Za-z0-9_]+-([0-9a-f]{16})\.(?:rmeta|rlib|d)\b/);
    if (m) hashes.add(m[1]);
  }
  return hashes;
}

function fingerprintDirsForHash(hash) {
  return readdirSync(FP_DIR, { withFileTypes: true })
    .filter((d) => d.isDirectory() && d.name.endsWith(`-${hash}`))
    .map((d) => ({ name: d.name, dir: path.join(FP_DIR, d.name) }));
}

function main() {
  console.log(`target: ${TARGET_DIR}`);
  console.log(`mode:   ${FIX ? '修复 (--fix)' : '仅调查 (dry-run)'}\n`);

  if (!statSync(FP_DIR, { throwIfNoEntry: false })?.isDirectory()) {
    console.error(`找不到 fingerprint 目录: ${FP_DIR}`);
    process.exit(2);
  }

  if (!FIX) {
    const { out } = runCargo();
    const hashes = staleHashesFromLog(out);
    console.log(`\n发现 ${hashes.size} 个异版本编译器单元：`);
    for (const h of hashes) {
      const dirs = fingerprintDirsForHash(h);
      const names = dirs.map((d) => d.name).join(', ') || '(无对应 fingerprint)';
      const blocked = dirs.some((d) => BLOCKLIST.test(d.name)) ? '  ⚠ 禁区' : '';
      console.log(`  ${h}  ${names}${blocked}`);
    }
    console.log('\n（仅调查：未改动任何文件。加 --fix 执行修复。）');
    return;
  }

  for (let i = 1; i <= MAX_ITERATIONS; i++) {
    console.log(`\n=== 第 ${i} 轮 ===`);
    const { status, out } = runCargo();
    if (status === 0) {
      console.log('\n✔ 构建成功，缓存已收敛到当前工具链。');
      return;
    }

    const e0514 = (out.match(/E0514/g) ?? []).length;
    const hashes = staleHashesFromLog(out);
    if (hashes.size === 0) {
      console.log(`\n✖ 构建失败（E0514 ${e0514} 处），但没有可清理的过期单元 → 属其它问题，停止。`);
      console.log(
        out
          .split('\n')
          .filter((l) => l.startsWith('error'))
          .slice(0, 25)
          .join('\n'),
      );
      process.exit(1);
    }

    const blocked = [];
    const removable = [];
    for (const h of hashes) {
      for (const d of fingerprintDirsForHash(h)) {
        (BLOCKLIST.test(d.name) ? blocked : removable).push(d);
      }
    }

    console.log(`  过期单元 ${hashes.size} 个 → 可清理 fingerprint ${removable.length} 个，禁区 ${blocked.length} 个`);
    if (blocked.length) {
      console.log('  ⚠ 禁区单元同样过期，脚本不动它们（重跑其构建脚本会挂死）：');
      for (const b of blocked) console.log(`      ${b.name}`);
      console.log('    请在具备 cmake/nasm/cl 的开发者 shell 中重建这些依赖。');
    }
    if (removable.length === 0) {
      console.log('  ✖ 无任何可清理项 → 停止，避免空转。');
      process.exit(1);
    }
    for (const d of removable) rmSync(d.dir, { recursive: true, force: true });
    console.log(`  已删除 ${removable.length} 个 fingerprint 目录，进入下一轮…`);
  }

  console.log('\n! 达到迭代上限仍未收敛，请人工介入。');
  process.exit(1);
}

main();
