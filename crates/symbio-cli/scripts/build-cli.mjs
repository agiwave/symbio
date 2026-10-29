#!/usr/bin/env node
// 构建 symbio-cli（纯 Rust CLI 前端）。
//
// ## 为什么需要一个脚本，而不是直接 `cargo build`
//
// 1. **离线**：显式传 `--offline`，构建永不触网。新增或升级依赖后先跑一次
//    `node scripts/cargo-offline-refresh.mjs` 把新 crate 拉进本地缓存。
// 2. **产物自证**：`cli` 是根 workspace 的成员，产物只有一个去处——
//    `<仓库根>/target/<profile>/symbio-cli(.exe)`。脚本构建成功后打印这条
//    绝对路径，读者不必猜 target 落在哪。
//
// ## 用法
//
//   node crates/symbio-cli/scripts/build-cli.mjs            # cargo build --offline
//   node crates/symbio-cli/scripts/build-cli.mjs --check    # cargo check --offline
//   node crates/symbio-cli/scripts/build-cli.mjs --release  # 追加 --release
//   node crates/symbio-cli/scripts/build-cli.mjs -- -v      # `--` 之后的参数原样透传

import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { cliArtifactRelPath, cliBinaryPath } from '../../../scripts/cli-binary.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CLI_DIR = path.resolve(HERE, '..');
const REPO_ROOT = path.resolve(CLI_DIR, '..', '..');

const argv = process.argv.slice(2);
const sepIdx = argv.indexOf('--');
const ownArgs = sepIdx === -1 ? argv : argv.slice(0, sepIdx);
const passthrough = sepIdx === -1 ? [] : argv.slice(sepIdx + 1);

const MODE = ownArgs.includes('--check') ? 'check' : 'build';
const extra = ownArgs.filter((a) => a !== '--check');
// profile 只由命令行决定：`--release` 在自身参数里或 `--` 之后透传都算
const PROFILE = [...extra, ...passthrough].includes('--release') ? 'release' : 'debug';

console.log(`cargo     : ${MODE} --offline ${extra.join(' ')} ${passthrough.join(' ')}`.trim());

const r = spawnSync('cargo', [MODE, '--offline', ...extra, ...passthrough], {
  cwd: CLI_DIR,
  stdio: 'inherit',
  shell: false,
});
const status = r.status ?? 1;
if (MODE === 'build' && status === 0) {
  const actual = cliBinaryPath(REPO_ROOT, PROFILE);
  if (!existsSync(actual)) {
    console.error(`✖ 构建返回 0，但找不到产物：${actual}`);
    console.error('  本机若用 .cargo/config.toml 或 CARGO_TARGET_DIR 改过 target-dir，产物就离开了规范位置。');
    process.exit(3);
  }
  console.log(`产物：${actual}`);
  console.log(`      （规范路径 ${cliArtifactRelPath(PROFILE)}）`);
}
process.exit(status);
