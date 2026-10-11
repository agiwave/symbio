// e2e runner：发现式加载 `cases/` 目录，每个用例在**独立子进程**中运行。
//
// - `node e2e/run-tests.mjs`          全量（按文件名序，**并发**跑）
// - `node e2e/run-tests.mjs T2`       按名称过滤（匹配用例名或文件名）
// - `node e2e/cases/t5-llm-http-error.mjs`   单独跑某个用例（子进程自执行模式）
// - `E2E_DEBUG=1`                     失败时输出错误堆栈
// - `E2E_CONCURRENCY=N`               覆盖并发度（默认见下）
//
// 为什么子进程隔离：用例各自起 mock、各自建临时 homedir、CLI 走子进程，
// 一个用例卡死或泄漏句柄不应拖垮别的用例；单独跑一个用例也无需任何额外入口。
// （CLI 自身是进程内直连后端的，隔离在 CLI 进程之外再多一层，成本可忽略。）
//
// ## 为什么并发（与门禁同口径）
//
// 与 `scripts/gate.d/58-e2e.mjs` 的「e2e 用例集」任务**逐字同一套**判据与前提：
//
// - **默认并发 6**（`E2E_CONCURRENCY` 可覆盖，核数不足往下收）。理论最优 =
//   总工作量 / 最慢单例 ≈ 5.7，取 6（更多只买到调度抖动，见 58-e2e.mjs 的长注释）。
// - **端口分段**是并行的硬前提：按**用例序号**注入 `E2E_PORT_BASE`（不是按 worker
//   序号），每个用例的 mock LLM / MCP / gateway 各占一段。少了它，40+ 子进程会全从
//   18080 起算、互相撞端口，症状是「mock-llm 收到 31 次请求」——比串行更难排查。
//
// 早先这里是**串行** for 循环（`spawnSync`），与门禁的并发逻辑分叉了两份；
// 于是「门禁 19s、本机 88s」是同一套用例的两个耗时，且慢例各占满自己的超时、
// 全量动辄几分钟。现在两处收敛到同一口径（本文件是唯一入口，门禁那套保持独立）。

import { spawn } from 'node:child_process';
import { readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import os from 'node:os';
import { red, green, dim, bold } from '../scripts/color.mjs';
import { ensureCliBinary } from '../scripts/cli-binary.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..');

/** 单例硬上限（毫秒）。用例自己的等待超时都远小于它；它只兜「真挂死」。 */
const CASE_TIMEOUT_MS = 180_000;

// ---------- 子进程自执行：`node cases/xxx.mjs` 直接跑该用例 ----------
// 用例文件被 import 时只做定义（defineCase 不执行）；直接运行时由 `cases/_selfrun.mjs`
// 依据环境变量 `E2E_CASE_SELF=1` 触发执行——本文件只负责**作为 runner 时**注入它。
const argv = process.argv.slice(2);

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

/** 跑一个用例子进程；返回 `{ ok, ms, output }`（输出为 stdout+stderr 合并的尾部）。 */
function runCase(c, portBase) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    const child = spawn(process.execPath, [c.file], {
      cwd: repoRoot,
      env: { ...process.env, E2E_CASE_SELF: '1', E2E_PORT_BASE: String(portBase) },
      stdio: ['ignore', 'pipe', 'pipe'],
      // 独立进程组：超时时能整组杀（含用例起的 mock-llm / mock-mcp / gateway 孙进程）。
      // POSIX 下 detached 让子进程成为组长，`kill(-pid)` 可整组收；Windows 无此语义，
      // 走下面的 `taskkill /T`。
      detached: process.platform !== 'win32',
    });
    let out = '';
    let settled = false;
    let escalate;
    const done = (ok) => {
      if (settled) return;
      settled = true;
      clearTimeout(killer);
      clearTimeout(escalate);
      resolve({ ok, ms: Date.now() - t0, output: out });
    };
    const cap = (buf) => {
      out += buf.toString();
      if (out.length > 16 * 1024 * 1024) out = out.slice(-8 * 1024 * 1024);
    };
    child.stdout.on('data', cap);
    child.stderr.on('data', cap);

    // 真挂死兜底：超时先杀**进程树**，再给一小段宽限；仍不 close 就强判失败并放行。
    // 「强判放行」不可省——Windows 上孙进程可能仍持着 pipe 写端，令 close 永不触发，
    // 那会让**整个并发池**卡在一个用例上（正是要根治的「卡住不动」）。
    const killer = setTimeout(() => {
      if (process.platform === 'win32') {
        // `/T` 连子树、`/F` 强杀；`spawnSync` 由 node 直接拉起，不必等它。
        spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
      } else {
        try {
          process.kill(-child.pid, 'SIGKILL');
        } catch {
          child.kill('SIGKILL');
        }
      }
      escalate = setTimeout(() => done(false), 5_000);
    }, CASE_TIMEOUT_MS);

    child.on('close', (code) => done(code === 0));
    child.on('error', () => done(false));
  });
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

  // 被测系统先就位：判据是**内容指纹**（产物是否对应当前源码），不是「文件在不在」。
  // 放在这里而不是每个子进程里：重建只做一次，且输出看得见——子进程里静默重建
  // 会让人以为「怎么跑了 40 秒」。
  if (!process.env.E2E_CLI_EXE) {
    try {
      const st = ensureCliBinary(repoRoot, { log: (m) => console.log(dim(`  [cli-binary] ${m}`)) });
      console.log(dim(`  被测二进制：${st.binaryPath}${st.rebuilt ? '（已重建）' : ''}`));
    } catch (e) {
      console.log(red(`  ✗ ${String(e.message ?? e)}`));
      process.exit(1);
    }
  }

  // 并发度：与门禁 `58-e2e.mjs` 同一口径——默认 6，核数不足往下收，
  // `E2E_CONCURRENCY` 覆盖（与之前串行时的 `for` 循环等价性：并发 1 即串行）。
  const cpuCap = Math.max(1, Math.min(6, os.availableParallelism?.() ?? os.cpus().length));
  const limit = Math.max(1, Number(process.env.E2E_CONCURRENCY) || cpuCap);

  // 按**用例序号**注入端口基址（不是按 worker 序号）：每个用例的 mock LLM / MCP /
  // gateway 各占一段，与门禁同一条前提。过滤器命中时序号以**过滤后**的列表为准
  // （单独跑几例时不必去撞远处那段）。
  const results = new Array(cases.length);
  let cursor = 0;
  const worker = async () => {
    for (;;) {
      const i = cursor++;
      if (i >= cases.length) return;
      const c = cases[i];
      const r = await runCase(c, 18080 + i * 100);
      results[i] = r;
      const tag = r.ok ? green('✓') : red('✗');
      console.log(`  ${tag} ${c.name} ${dim(`(${r.ms}ms)`)}`);
      if (!r.ok) {
        const detail = r.output.trim().split('\n').slice(-8).join('\n      ');
        if (detail) console.log(`      ${red(detail)}`);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, cases.length) }, worker));

  let pass = 0;
  const failures = [];
  for (const [i, r] of results.entries()) {
    if (r.ok) pass++;
    else failures.push(cases[i].name);
  }

  // 并发度读数（与门禁同口径，只印不判红）：理论最优 = 总工作量 / 最慢单例。
  const times = results.filter(Boolean).map((r) => r.ms);
  if (times.length > 0) {
    const totalMs = times.reduce((a, b) => a + b, 0);
    const slowest = Math.max(...times);
    console.log(
      dim(
        `  并发 ${limit}｜总工作量 ${(totalMs / 1000).toFixed(1)}s` +
          `｜最慢单例 ${(slowest / 1000).toFixed(1)}s` +
          `｜理论最优 ${(totalMs / slowest).toFixed(1)}｜墙钟下界 ${(slowest / 1000).toFixed(1)}s`,
      ),
    );
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
