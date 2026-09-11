#!/usr/bin/env node
// 构建 symbio-cli（纯 Rust CLI 前端）。
//
// ## 为什么需要一个脚本，而不是直接 `cargo build`
//
// 1. **共享 target 缓存**：`cli/.cargo/config.toml` 把 target-dir 指到
//    `../symbio/target`，以复用 symbio 已经预热好的依赖（离线环境下没有第二次
//    机会去下载 aws-lc-sys / ort / sqlite 这些带 C 构建脚本的 crate）。
//
// 2. **C 工具链必须显式给到 cc-rs**（2026-09-11 实测）。symbio 的依赖树里有
//    `aws-lc-sys`（reqwest→rustls 的加密后端）、`ring`、`onig_sys`、
//    `libsqlite3-sys` 等需要 C 编译器的 crate。它们通过 cc-rs 找编译器：
//    在没有 VS 开发环境的终端里，cc-rs 找不到 `cl.exe`，构建脚本会**静默挂死**
//    （进程存活、CPU≈0、无子进程，看起来像 cargo 死锁，实为等编译器）。
//
//    本脚本只注入 cc-rs 真正需要的最小变量：
//      CC / CXX  → cl.exe 绝对路径
//      INCLUDE   → MSVC + Windows SDK 头文件
//      LIB       → MSVC + Windows SDK 导入库（rust-lld 链接也要用）
//    **刻意不动 PATH**（保持 Agent/CI 环境的 PATH 原样，避免污染其它工具链探测）。
//
// 3. aws-lc-sys 0.44 自带 prebuilt NASM 对象，**不需要 cmake、也不需要装 nasm**
//    （见 crate 内 `builder/prebuilt-nasm`）。
//
// ## 用法
//
//   node scripts/build-cli.mjs                     # cargo build --offline
//   node scripts/build-cli.mjs --check             # cargo check --offline
//   node scripts/build-cli.mjs --release           # 追加 --release
//   node scripts/build-cli.mjs -- -v               # `--` 之后的参数原样透传
//
// 可用环境变量覆盖自动探测结果：
//   VS_ROOT       Visual Studio 安装根（默认扫几个常见位置）
//   WINSDK_ROOT   Windows Kits\10 根
//   MSVC_VER      VC/Tools/MSVC 下的版本目录名
//   SDK_VER       Windows Kits\10\Include 下的版本目录名
//
// 产物：`<repo>/symbio/target/debug/symbio-cli.exe`

import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CLI_DIR = path.resolve(HERE, '..');

const argv = process.argv.slice(2);
const sepIdx = argv.indexOf('--');
const ownArgs = sepIdx === -1 ? argv : argv.slice(0, sepIdx);
const passthrough = sepIdx === -1 ? [] : argv.slice(sepIdx + 1);

const MODE = ownArgs.includes('--check') ? 'check' : 'build';
const extra = ownArgs.filter((a) => a !== '--check');

function dirs(p) {
  return existsSync(p)
    ? readdirSync(p, { withFileTypes: true })
        .filter((d) => d.isDirectory())
        .map((d) => d.name)
    : [];
}

/** 取版本目录里「最大」的一个（按数值分段比较，避免字符串序把 10.0.9 排在 10.0.10 之后）。 */
function latest(names) {
  const key = (s) =>
    s.split(/[^0-9]+/).filter(Boolean).map((n) => Number(n).toString().padStart(6, '0')).join('.');
  return [...names].sort((a, b) => (key(a) < key(b) ? 1 : key(a) > key(b) ? -1 : 0))[0];
}

function firstExisting(candidates) {
  return candidates.find((p) => existsSync(p));
}

// ── 定位 Visual Studio ────────────────────────────────────────────────────
const VS_ROOT =
  process.env.VS_ROOT ||
  firstExisting([
    'D:\\Apps\\Visual Studio 2022',
    'C:\\Program Files\\Microsoft Visual Studio\\2022\\Enterprise',
    'C:\\Program Files\\Microsoft Visual Studio\\2022\\Professional',
    'C:\\Program Files\\Microsoft Visual Studio\\2022\\Community',
    'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools',
  ]);
if (!VS_ROOT) fail('找不到 Visual Studio 安装目录（可用 VS_ROOT 环境变量指定）');

const msvcRoot = path.join(VS_ROOT, 'VC', 'Tools', 'MSVC');
const MSVC_VER = process.env.MSVC_VER || latest(dirs(msvcRoot));
if (!MSVC_VER) fail(`找不到 MSVC 工具集：${msvcRoot}`);

const msvcDir = path.join(msvcRoot, MSVC_VER);
const clExe = path.join(msvcDir, 'bin', 'Hostx64', 'x64', 'cl.exe');
if (!existsSync(clExe)) fail(`找不到 cl.exe：${clExe}`);

// ── 定位 Windows SDK ──────────────────────────────────────────────────────
const WINSDK_ROOT =
  process.env.WINSDK_ROOT ||
  firstExisting(['D:\\Windows Kits\\10', 'C:\\Program Files (x86)\\Windows Kits\\10']);
if (!WINSDK_ROOT) fail('找不到 Windows SDK（可用 WINSDK_ROOT 环境变量指定）');

const incRoot = path.join(WINSDK_ROOT, 'Include');
const libRoot = path.join(WINSDK_ROOT, 'Lib');
const SDK_VER = process.env.SDK_VER || latest(dirs(incRoot));
if (!SDK_VER) fail(`找不到 Windows SDK 版本目录：${incRoot}`);

// ── 组装环境（只加 cc-rs 与链接器需要的，不动 PATH）────────────────────────
//
// ⚠ 路径分隔符必须**归一化成 `/`**（2026-09-11 实测踩坑）：
//   cc-rs 会把 INCLUDE/LIB 记进构建指纹（`rerun-if-env-changed`），值一变就判定
//   整棵 C 依赖树过期。而 Git Bash 的 MSYS 在 spawn 原生 exe 时会重写
//   `;` 分隔的路径列表，把 `D:\Apps\...` 变成 `D:/Apps\...`。
//   如果这里用 path.join 产出的原生 `D:\...`，就会出现「一次用脚本构建、一次用
//   shell 构建」⇒ 两种字符串交替 ⇒ aws-lc-sys 反复全量重编（每次约 9 分钟）。
//   统一成 `/` 后，脚本与 shell 两条路径写下的指纹完全一致。
const sep = (s) => s.replace(/\\/g, '/');

const env = {
  ...process.env,
  CC: sep(clExe),
  CXX: sep(clExe),
  INCLUDE: [
    path.join(msvcDir, 'include'),
    path.join(incRoot, SDK_VER, 'ucrt'),
    path.join(incRoot, SDK_VER, 'um'),
    path.join(incRoot, SDK_VER, 'shared'),
  ]
    .map(sep)
    .join(';'),
  LIB: [
    path.join(msvcDir, 'lib', 'x64'),
    path.join(libRoot, SDK_VER, 'um', 'x64'),
    path.join(libRoot, SDK_VER, 'ucrt', 'x64'),
  ]
    .map(sep)
    .join(';'),
};

console.log(`VS        : ${VS_ROOT}  (MSVC ${MSVC_VER})`);
console.log(`WindowsKit: ${WINSDK_ROOT}  (SDK ${SDK_VER})`);
console.log(`cl.exe    : ${sep(clExe)}`);
console.log(`cargo     : ${MODE} ${extra.join(' ')} ${passthrough.join(' ')}`.trim());

const r = spawnSync('cargo', [MODE, '--offline', ...extra, ...passthrough], {
  cwd: CLI_DIR,
  env,
  stdio: 'inherit',
  shell: false,
});
process.exit(r.status ?? 1);

function fail(msg) {
  console.error(`✖ ${msg}`);
  process.exit(2);
}
