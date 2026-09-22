// e2e runner：发现式加载 `cases/` 目录，每个用例在**独立子进程**中运行。
//
// - `node e2e/run-tests.mjs`          全量（按文件名序，逐个跑）
// - `node e2e/run-tests.mjs T2`       按名称过滤（匹配用例名或文件名）
// - `node e2e/cases/t5-llm-http-error.mjs`   单独跑某个用例（子进程自执行模式）
// - `E2E_DEBUG=1`                     失败时输出错误堆栈
//
// 为什么子进程隔离：用例各自起 mock、各自建临时 homedir、CLI 走 `spawnSync`，
// 一个用例卡死或泄漏句柄不应拖垮别的用例；单独跑一个用例也无需任何额外入口。
// （CLI 自身是进程内直连后端的，隔离在 CLI 进程之外再多一层，成本可忽略。）

import { spawnSync } from 'node:child_process';
import { readdirSync } from 'node:fs';
import { join, dirname, basename } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { red, green, yellow, dim, bold } from '../scripts/color.mjs';

const here = dirname(fileURLToPath(import.meta.url));

// ---------- 子进程自执行：`node cases/xxx.mjs` 直接跑该用例 ----------
// 用例文件被 import 时只做定义（defineCase 不执行）；直接运行时走这里。
// 通过环境变量 `E2E_CASE_SELF` 标记「正在自执行」，避免 import 联动误触发。
const argv = process.argv.slice(2);
const isSelfRun = process.env.E2E_CASE_SELF === '1';
const directFile = argv[0] && !argv[0].startsWith('-') ? null : null;

// ---------- 发现 ----------
function discoverCases() {
  return readdirSync(join(here, 'cases'))
    .filter((f) => f.endsWith('.mjs') && !f.startsWith('_'))
    .sort()
    .map((f) => ({
      file: join(here, 'cases', f),
      name: f.replace(/\.mjs$/, ''),
    }));
}

// ---------- 主模式 ----------
async function main() {
  const filter = argv[0] ?? null;
  const cases = discoverCases().filter((c) => !filter || c.name.includes(filter));
  if (cases.length === 0) {
    console.log(red(`没有匹配的用例（filter=${filter ?? '无'}）`));
    process.exit(1);
  }

  console.log(bold(`══ e2e（${cases.length} 个用例）══`));
  let pass = 0;
  const failures = [];
  for (const c of cases) {
    const t0 = Date.now();
    const r = spawnSync(
      process.execPath,
      [c.file],
      {
        encoding: 'utf8',
        timeout: 180_000,
        maxBuffer: 16 * 1024 * 1024,
        env: { ...process.env, E2E_CASE_SELF: '1' },
      },
    );
    const ms = Date.now() - t0;
    const ok = r.status === 0;
    if (ok) {
      pass++;
      console.log(`  ✓ ${c.name} ${dim(`(${ms}ms)`)}`);
    } else {
      failures.push(c.name);
      const detail = (r.stderr || r.stdout || '').trim().split('\n').slice(-8).join('\n      ');
      console.log(`  ✗ ${c.name} ${dim(`(${ms}ms)`)}\n      ${red(detail)}`);
    }
  }

  console.log();
  console.log(
    failures.length === 0
      ? green(`  全部通过（${pass}/${cases.length}）`)
      : red(`  通过 ${pass}/${cases.length}，失败: ${failures.join('、')}`),
  );
  process.exit(failures.length === 0 ? 0 : 1);
}

void main();
