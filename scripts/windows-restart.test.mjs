// `scripts/windows-restart.mjs` 的回归测试：**只用真实进程**验证换壳监督器。
//
// ## 为什么这些场景是**并发**的（2026-10-18）
//
// 每个场景都在验一件独立的事（拒绝停机 / 哈希不符 / 回滚 / 取消 / 后台存活），
// 彼此不共享可变状态——除了 `project/` 那份 cargo fixture，而**只有两个场景会写它**
// （见下 `ownProject`）。可它们原先是一个一个 `await` 的，于是墙钟 = 全部之和。
//
// 实测代价（本机 Windows / 8 逻辑核 i7-6700HQ / 16GB，`node --test scripts/windows-restart.test.mjs`）：
//
//   串行（原样）      358s —— 超过本文件 240s 的 timeout ⇒ **整份测试判红**
//                             （前 7 个场景都过了，第 8 个被 `parentAlreadyFinished` 取消，
//                              报出来的是「test timed out」，看上去像功能坏了）
//   并发（现状）      见下方 `SCENARIO_CONCURRENCY` 注释处的实测数
//
// 单个场景的 14–38s 几乎全是**进程启动**，不是被测逻辑：每次 `invoke()` 起一个
// `node`（本机实测 ~300ms 固定成本）再在监督器里起 4–6 个 `powershell.exe`
// （本机实测**冷启动 2.2–2.6s**，一次 `Get-Process` 查询 ~1s）。串行时这些等待
// 一次也重叠不了；并发时它们互相重叠，而 CPU 几乎不参与——这正是能并发的那类等待。
//
// ⚠️ 并发**不改变任何判据**：每条断言仍逐字保留，每个场景仍用自己的 `case-N/` 目录、
// 自己的目标进程、自己的 config/status/cancel/ready 文件。唯一被并发碰到的是共享的
// `project/`，处置见 `fixture({ ownProject })`。
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
import { run, processIdentity, assertIdentity, inspectPE } from './windows-restart-prepare.mjs';
import { stopVerified } from './windows-restart.mjs';
import { machine, budget } from './gate.d/_machine.mjs';

const script = fileURLToPath(new URL('./windows-restart.mjs', import.meta.url));
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const triple = 'x86_64-pc-windows-msvc';
const source = `fn main() {
  println!("cwd={:?} args={:?}", std::env::current_dir().unwrap(), std::env::args().collect::<Vec<_>>());
  if std::env::current_exe().unwrap().file_name().unwrap() == "fail.exe" { std::process::exit(7); }
  std::thread::sleep(std::time::Duration::from_secs(180));
}`;

// 轮询等待一个状态迁移。默认预算必须**严格大于**监督器自己的截止时间
// （`timeoutSeconds`，本文件用 120）——不是「不小于」：相等时两边会在同一刻各自
// 宣布超时，谁先谁后看调度，于是同一份代码时而报「监督器超时」时而报
// 「waitFor timed out」。比它短就更糟：被测系统还在合法地干活，测试先宣布超时
// （实测踩过，那一刻监督器正走到 `pre-switch`）。留 30s 余量。
async function waitFor(fn, timeout = 150000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const result = fn(); if (result) return result; await sleep(100); }
  throw new Error('waitFor timed out');
}

/**
 * 监督器**已经写下终态失败**时，把它的原话抛出来。
 *
 * 不这么做的话，`waitFor` 会一路等满 120s 再报「waitFor timed out」——而那一刻
 * `status.json` 里明明写着 `failed-before-confirmed-stop` 和 `error` 字段。
 * 实测（2026-10-19）：一次 `rename` 撞 EPERM 让监督器崩在 `verifying`，测试却报了
 * 「waitFor timed out」，排查方向被引到"是不是太慢"上，而真因在另一个文件里。
 * **一条只能报"超时"的等待，把"已经失败"和"还没到"说成了同一件事。**
 */
const FAILED_PHASES = new Set(['failed-before-confirmed-stop', 'recovery-failed']);
function throwIfFailed(status) {
  if (status?.phase && FAILED_PHASES.has(status.phase)) {
    throw new Error(`监督器已失败于 ${status.phase}：${status.error ?? '(status.error 为空)'}`);
  }
}

// ── `writeStatusAtomic`：报告通道失败 ≠ 操作失败 ──────────────────────────
//
// 这条判据来自一次真实事故（2026-10-19）：`phase('pre-switch')` 的 `rename` 撞上
// Windows 的偶发 EPERM（并发读者 / 杀软扫描 `.tmp`），异常冒进 `restart()` 的 catch，
// catch 里又写一次状态、**又抛** ⇒ 进程崩在 `verifying`，而目标进程还活着、候选没起。
//
// 「偶发 EPERM」没法在测试里复现，所以让它**必然**失败：把目标占成一个**目录**，
// 于是 `renameSync(tmp -> 目标)` 一定失败。断言的是两件事：
//   ① 返回 `false` 而**不抛**（这条路径不许变成异常）；
//   ② 正常路径仍然真的落盘（否则"不抛"可以用"什么都不做"骗过去）。
test('writeStatusAtomic：rename 必然失败时返回 false 不抛；正常时真的落盘', async () => {
  const { writeStatusAtomic } = await import('./windows-restart.mjs');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-status-'));
  try {
    const ok = path.join(dir, 'ok.json');
    assert.equal(writeStatusAtomic(ok, { phase: 'survived' }), true, '正常路径必须返回 true');
    assert.deepEqual(JSON.parse(fs.readFileSync(ok, 'utf8')), { phase: 'survived' }, '内容必须是这份 payload');
    assert.ok(!fs.existsSync(`${ok}.tmp`), '成功时 .tmp 必须已被 rename 走');

    const blocked = path.join(dir, 'blocked.json');
    fs.mkdirSync(blocked); // rename 到一个已存在的目录 ⇒ 必然失败
    assert.equal(writeStatusAtomic(blocked, { phase: 'pre-switch' }), false, '写不出去要返回 false');
    assert.ok(fs.statSync(blocked).isDirectory(), '失败时不许把目标搞坏（它还是个目录）');
  } finally {
    try { fs.rmSync(dir, { recursive: true, force: true }) } catch { /* 临时目录残留无害 */ }
  }
})

// 场景并发度**不是常量，由机器档案决定**（`gate.d/_machine.mjs::slots.process`）。
//
// 下面这组实测是在**本机**（Windows / 8 逻辑核 i7-6700HQ / 16GB）上做的，
// 它说明的是「这条曲线的形状」，不是「全世界的答案」：
//
//   串行（原样）                     358s  ⇒ 超过 240s 的 timeout，**整份测试判红**
//   串行 + 本次削减                  272s
//   并发 2 + 本次削减                206s   ← 本机的拐点
//   并发 3 + 本次削减                239s   3 个场景假红（`stopVerified` 的 PowerShell 在
//                                          45s 内没起来 ⇒ 报「Timeout: no process killed」）
//   并发 3（预算放宽后复测两轮）      192s 通过 / 212s **2 项假红**，同一形态：
//                                          `same-handle stop gate` 那条用
//                                          `stopVerified(..., 3000, {candidate})` ⇒ 子进程预算
//                                          18s，而它要在其中起一个 PowerShell **再算一遍
//                                          candidate.exe 的 SHA256**。预算放宽治不了这条：
//                                          它测的就是"确认没杀掉"，本来就不该等这么久。
//
// 读法：**瓶颈是 Windows 进程创建，不是 CPU**——再往上就是让 PowerShell 冷启动互相
// 排队，反而更慢且不稳。所以「并发几」取决于**这台机器起进程有多快**，而那是
// `slots.process` 用单核探针 + 核数估出来的（强机 3 / 中档 2 / 弱机 1）。
//
// ⚠️ **这个数曾经写死成 2**，理由是「本机实测拐点」。问题在于：拐点位置随机器变。
// 8 核机器上 2 是对的；32 核的新机器上它是**白等**（每波只跑 2 个，而进程创建
// 完全跟得上）；2 核老机器上它又太大（两个 PowerShell 冷启动互相拖，双双超时）。
// 一个数字不可能同时正确三次。
//
// `RESTART_TEST_CONCURRENCY=<n>` 仍可覆盖：换机器后若想复核档案给的数，A/B 一次即可，
// 但**默认值不再需要人记得改**。
const MACHINE = await machine();
const SCENARIO_CONCURRENCY = MACHINE.slots.process;

test('Windows restart: independent real processes only', {
  skip: process.platform !== 'win32',
  // 父测试的 timeout 覆盖**全部**子场景。原值 240s 是按串行写的，于是它先把测试判红、
  // 再让第 8 个场景被取消——一个**代价判据**冒充了功能判据。
  //
  // 现在按**上一轮实测**给（`budget` = 中位数 × 2.5，钳在静态基准的 1–6 倍）：
  // 慢机器上它自动变宽（不至于一上来就假红），快机器上它自动收紧（真挂死能早点发现）。
  // 首跑没有历史时退回静态基准 × 单核探针系数。
  timeout: budget(MACHINE, 'docs::windows-restart 回归测试', 480000),
  concurrency: true,
}, async t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio restart test '));
  console.log(`Restart test evidence (retained): ${root}`);
  const project = path.join(root, 'fixture');
  fs.mkdirSync(path.join(project, 'src'), { recursive: true });
  fs.writeFileSync(path.join(project, 'Cargo.toml'), '[package]\nname = "restart-fixture"\nversion = "0.1.0"\nedition = "2021"\n[workspace]\n');
  fs.writeFileSync(path.join(project, 'Cargo.lock'), 'version = 4\n\n[[package]]\nname = "restart-fixture"\nversion = "0.1.0"\n');
  fs.writeFileSync(path.join(project, 'src/main.rs'), source);
  const where = spawnSync('where.exe', ['cargo.exe'], { encoding: 'utf8' });
  assert.equal(where.status, 0, where.stderr);
  const cargoExe = process.env.PREPARE_TEST_CARGO || where.stdout.trim().split(/\r?\n/)[0];
  // 共享 fixture 只编一次：三个 exe 都是它的副本。单个场景**不重编**这份（见 ownProject）。
  await run(cargoExe, ['build', '--offline', '--locked', '--release', '--target', triple], { cwd: project, timeoutMs: 60000, capture: true });
  const built = path.join(project, 'target', triple, 'release', 'restart-fixture.exe');
  const oldExe = path.join(root, 'old.exe');
  const goodExe = path.join(root, 'candidate.exe');
  const failExe = path.join(root, 'fail.exe');
  for (const exe of [oldExe, goodExe, failExe]) fs.copyFileSync(built, exe);
  const oldBytes = fs.readFileSync(oldExe);
  let n = 0;

  /**
   * 建一个场景的夹具。
   *
   * `ownProject: true` 给该场景一份**自己的** cargo fixture 副本：原实现里有两个场景
   * （「编译失败不停目标」与「`--ready` 跳过构建」）会改写**共享** `project/src/main.rs`
   * 来制造编译失败——串行时靠 `finally` 复原，并发时就是「A 把源码写成非法、B 正在
   * 编它」。这两处改为各持一份副本，共享的那份此后**只读**。
   *
   * 副本不触发重编：前者要的是**编译失败**（解析期就失败，不产出任何东西），后者要的是
   * **根本不进构建**（`--ready` 直接跳过）——两者都不需要一次成功的构建。
   */
  async function fixture({ ownProject = false } = {}) {
    const dir = path.join(root, `case-${++n}`); fs.mkdirSync(dir);
    const proj = ownProject ? path.join(dir, 'fixture') : project;
    if (ownProject) {
      fs.mkdirSync(path.join(proj, 'src'), { recursive: true });
      for (const rel of ['Cargo.toml', 'Cargo.lock', 'src/main.rs']) {
        fs.copyFileSync(path.join(project, rel), path.join(proj, rel));
      }
    }
    const args = ['argument with spaces', '引号"参数', 'trailing\\'];
    const child = spawn(oldExe, args, { cwd: dir, stdio: 'ignore', windowsHide: true });
    child.on('error', () => {});
    await new Promise(resolve => child.once('spawn', resolve));
    const target = { ...await processIdentity(child.pid), cwd: dir, args };    // `timeoutSeconds` 取 120（原 60）：并发下进程创建排队，60s 的监督器截止时间
    // 会先于被测行为触发，而它只会把失败归因成「Restart deadline exceeded」。
    // 它是**截止时间**不是判据——本文件没有一条断言依赖它恰好是 60。
    const c = { target, cargoExe, buildCwd: proj, manifest: path.join(proj, 'Cargo.toml'),
      bin: 'restart-fixture', triple, stageRoot: dir, cancelFile: path.join(dir, 'cancel'), timeoutSeconds: 120,
      statusFile: path.join(dir, 'status.json'), aliveSeconds: 1 };
    const config = path.join(dir, 'config.json');
    const readyFile = path.join(dir, 'ready.json');
    const makeReady = (exe = goodExe, patch = {}) => {
      // Explicit fixture handoff with the same schema as prepare's generated ready.json.
      const record = { mode: 'prepare-only', target, triple, cancelFile: c.cancelFile,
        candidate: exe, validation: inspectPE(exe, triple), ...patch };
      console.log('fixture identity', JSON.stringify({ spawnedPid: child.pid, readyPid: record.target.pid,
        targetCommand: [oldExe, ...args], candidateCommand: [exe, ...args] }));
      assert.equal(child.pid, record.target.pid);
      fs.writeFileSync(readyFile, JSON.stringify(record));
      return { ...c, readyFile };
    };
    const invoke = (cfg = c, flags = ['--execute']) => {
      fs.writeFileSync(config, JSON.stringify(cfg));
      // 子进程预算必须**大于**监督器自己的截止时间（`timeoutSeconds`）+ 余量：
      // 卡在两者之间时，杀掉监督器的会是我们，而失败会被归因成「超时」而不是它自己的
      // 结论——诊断信息正好丢在最需要它的地方。
      //
      // **由 `timeoutSeconds` 派生，不写死**：原先写 150000（= 120s + 30s），
      // 于是有人把 `timeoutSeconds` 调到 180 时，外层预算就悄悄变成了「比内层短」，
      // 而症状是「Timeout: no process killed」——看上去像功能坏了。
      const invokeTimeoutMs = (cfg.timeoutSeconds ?? 120) * 1000 + 30000;
      return run(process.execPath, [script, '--config', config, ...flags], { cwd: dir, timeoutMs: invokeTimeoutMs, capture: true });
    };
    const status = () => { try { return JSON.parse(fs.readFileSync(c.statusFile, 'utf8')); } catch { return null; } };
    const untouched = async () => {
      assertIdentity(target, await processIdentity(target.pid));
      assert.deepEqual(fs.readFileSync(oldExe), oldBytes);
      assert.equal(status()?.candidatePid, undefined);
    };
    const cleanup = async () => {
      // 目标进程是**本测试自己 spawn 的** ⇒ 用自己手里的句柄关，比「按 pid 再查一遍身份
      // 再 kill」更直接，也省掉一次 PowerShell 冷启动（本机实测 1–2.5s/次，而这份测试
      // 一轮要起近百次）。句柄在这里比 pid 更强：它不可能指向别人。
      //
      // 候选 / 回滚进程是**监督器**spawn 的，测试手上只有 pid ⇒ 仍走 `stopVerified`
      // ——那是唯一能保证「pid 没被回收」的路径，也正是本文件要验的原语，不能因为
      // 「省时间」换成按名字 / 按 pid 盲杀。
      if (child.exitCode === null && child.signalCode === null) {
        child.kill();
        // 有界等待：目标可能已被监督器停掉，此时 `kill()` 返回 false 且不会再有
        // `exit` 事件——无界 await 会把清理挂死（那会把失败归因到「卡住」）。
        await Promise.race([new Promise(resolve => child.once('exit', resolve)), sleep(5000)]);
      }
      const s = status();
      for (const [pid, exe] of [[s?.candidatePid, s?.candidate], [s?.rollbackPid, oldExe]]) {
        if (!pid) continue;
        try {
          const id = await processIdentity(pid);
          assert.equal(id.exe.toLowerCase(), exe.toLowerCase());
          await stopVerified(pid, id.startTimeUtcTicks, id.exe);
        } catch (e) { if (!/Command failed/.test(e.message)) throw e; }
      }
    };
    return { c, dir, project: proj, invoke, makeReady, readyFile, status, untouched, cleanup, target, child };
  }

  // 场景**只登记不执行**：`await t.test(...)` 会一个接一个地跑（await 串行化），
  // 并发要求「先全部挂上、再统一等」——见下方 `Promise.all`。
  const scenarios = [];
  function scenario(name, fn, opts = {}) {
    scenarios.push({ name, fn, opts });
  }

  scenario('default dry-run writes no state/stage; background requires --execute', async f => {
    const before = fs.readdirSync(f.dir);
    assert.match(await f.invoke(f.c, []), /dry-run/);
    assert.deepEqual(fs.readdirSync(f.dir), [...before, 'config.json'].sort());
    await assert.rejects(f.invoke(f.c, ['--background']), /requires explicit/);
    await f.untouched();
  });
  scenario('actual Cargo compile failure never stops target', async f => {
    fs.writeFileSync(path.join(f.project, 'src/main.rs'), 'deliberately invalid Rust');
    try { await assert.rejects(f.invoke(), /Command failed/); }
    finally { fs.writeFileSync(path.join(f.project, 'src/main.rs'), source); }
    assert.equal(f.status().phase, 'failed-before-confirmed-stop');
    assert.equal(f.status().events.some(e => e.phase === 'stopping-target'), false);
    await f.untouched();
  }, { ownProject: true });
  scenario('wrong identity before build never stops target', async f => {
    await assert.rejects(f.invoke({ ...f.c, target: { ...f.target, startTimeUtcTicks: '100000000000000000' } }), /identity mismatch/);
    await f.untouched();
  });
  scenario('same-handle stop gate refuses wrong ticks/path/hash/cancel', async f => {
    await assert.rejects(stopVerified(f.target.pid, '100000000000000000', oldExe), /identity mismatch/);
    await assert.rejects(stopVerified(f.target.pid, f.target.startTimeUtcTicks, goodExe), /identity mismatch/);
    await assert.rejects(stopVerified(f.target.pid, f.target.startTimeUtcTicks, oldExe, 3000,
      { candidate: goodExe, sha256: '0'.repeat(64) }), /hash mismatch/);
    fs.writeFileSync(f.c.cancelFile, 'cancel');
    await assert.rejects(stopVerified(f.target.pid, f.target.startTimeUtcTicks, oldExe, 3000,
      { cancelFiles: [f.c.cancelFile] }), /Cancelled/);
    await f.untouched();
  });
  scenario('ready hash and ready identity mismatch never stop target', async f => {
    await assert.rejects(f.invoke(f.makeReady(goodExe, { validation: { sha256: '0'.repeat(64) } })), /hash mismatch/);
    await assert.rejects(f.invoke(f.makeReady(goodExe, { target: { ...f.target, startTimeUtcTicks: '100000000000000000' } })), /identity mismatch/);
    await f.untouched();
  });
  scenario('ready cancellation and config cancellation both prevent stop', async f => {
    const otherCancel = path.join(f.dir, 'prepare.cancel'); fs.writeFileSync(otherCancel, 'cancel');
    await assert.rejects(f.invoke(f.makeReady(goodExe, { cancelFile: otherCancel })), /Cancelled/);
    fs.writeFileSync(f.c.cancelFile, 'cancel');
    await assert.rejects(f.invoke(f.makeReady()), /Cancelled/);
    await f.untouched();
  });
  scenario('real prepare build -> confirmed stop -> candidate survives with cwd/args/logs', async f => {
    await f.invoke();
    const s = f.status();
    assert.equal(s.phase, 'survived'); assert.equal(s.applicationReady, false);
    assert.equal(s.survivedSeconds, 1);
    await assert.rejects(processIdentity(f.target.pid), /Command failed/);
    assert.equal((await processIdentity(s.candidatePid)).exe.toLowerCase(), s.candidate.toLowerCase());
    const log = fs.readFileSync(path.join(s.stage, 'candidate.stdout.log'), 'utf8');
    assert.ok(log.includes('argument with spaces'));
    assert.ok(log.includes('引号\\\"参数'));
    assert.ok(log.includes(f.dir.replaceAll('\\', '\\\\')));
    assert.deepEqual(fs.readFileSync(oldExe), oldBytes);
    assert.ok(fs.existsSync(path.join(s.stage, 'candidate.stderr.log')));
  });
  scenario('--ready skips invalid build; early candidate exit rolls back only after exit', async f => {
    const c = f.makeReady(failExe);
    fs.writeFileSync(path.join(f.project, 'src/main.rs'), 'invalid');
    try { await assert.rejects(f.invoke(f.c, ['--ready', c.readyFile, '--execute']), /Process exited/); }
    finally { fs.writeFileSync(path.join(f.project, 'src/main.rs'), source); }
    const s = f.status(); assert.equal(s.phase, 'rolled-back');
    assert.equal(s.events.some(e => e.phase === 'building'), false);
    const phases = s.events.map(e => e.phase);
    assert.ok(phases.indexOf('candidate-exit-confirmed') < phases.indexOf('rollback-window'));
    await assert.rejects(processIdentity(s.candidatePid), /Command failed/);
    assert.equal((await processIdentity(s.rollbackPid)).exe.toLowerCase(), oldExe.toLowerCase());
    assert.deepEqual(fs.readFileSync(oldExe), oldBytes);
  }, { ownProject: true });
  scenario('cancel during survival stops live candidate before rollback', async f => {
    const c = { ...f.makeReady(), aliveSeconds: 3 };
    const pending = f.invoke(c); // attach rejection handler immediately
    const outcome = pending.then(() => null, e => e);
    // 两种"等不到"必须分开：监督器**已经失败**（status 写了终态 + error）要立刻把
    // 原话抛出来；监督器**已经退出**（promise 落定）也不该继续等。原先只写一个
    // `waitFor`，两种情况都退化成「120s 后报 waitFor timed out」，真因被埋在日志里。
    await Promise.race([
      waitFor(() => {
        const st = f.status();
        throwIfFailed(st);
        return st?.phase === 'survival-window';
      }),
      outcome.then((e) => {
        if (e) throw new Error(`监督器提前退出，未到 survival-window：${e.message}`);
      }),
    ]);
    fs.writeFileSync(c.cancelFile, 'cancel');
    assert.match((await outcome).message, /Cancelled/);
    const s = f.status(); assert.equal(s.phase, 'rolled-back');
    await assert.rejects(processIdentity(s.candidatePid), /Command failed/);
    const phases = s.events.map(e => e.phase);
    assert.ok(phases.indexOf('candidate-exit-confirmed') < phases.indexOf('rollback-window'));
    assert.equal((await processIdentity(s.rollbackPid)).exe.toLowerCase(), oldExe.toLowerCase());
  });
  scenario('background launcher exits while detached supervisor survives and finishes', async f => {
    const c = { ...f.makeReady(), aliveSeconds: 4 };
    const output = await f.invoke(c, ['--execute', '--background']);
    // run resolves only after launcher process exit/closed pipes.
    const launched = JSON.parse(output.trim());
    const supervisor = await processIdentity(launched.supervisorPid);
    assert.equal(supervisor.exe.toLowerCase(), process.execPath.toLowerCase());
    const s = await waitFor(() => {
      const st = f.status();
      throwIfFailed(st); // 分离的监督器没有 promise 可等，只能靠 status 的终态失败止损
      return st?.phase === 'survived' && st;
    });
    assert.equal(s.supervisorPid, launched.supervisorPid);
    assert.equal(s.applicationReady, false);
    assert.equal((await processIdentity(s.candidatePid)).exe.toLowerCase(), goodExe.toLowerCase());
    assert.ok(fs.existsSync(path.join(launched.launcherLogs, 'supervisor.stdout.log')));
    assert.ok(fs.existsSync(path.join(launched.launcherLogs, 'supervisor.stderr.log')));
    await sleep(500);
    await assert.rejects(processIdentity(supervisor.pid), /Command failed/);
  });
  scenario('current launching test host is protected even with --execute', async f => {
    const host = { ...await processIdentity(process.pid), cwd: f.dir, args: [] };
    await assert.rejects(f.invoke({ ...f.c, target: host }), /current host/);
    assertIdentity(host, await processIdentity(process.pid));
    await f.untouched();
  });

  // 并发执行：**按波次**（而不是游标）——`t.test()` 是「挂上就返回」，游标 worker
  // 会在一个微任务里把 8 个场景全部挂上，并发度形同虚设（实测 8 个同时起，把
  // `stopVerified` 的 PowerShell 拖到 45s 超时，8 个场景一起假红）。
  // 波次简单且可预测：每波 `SCENARIO_CONCURRENCY` 个，等这波全部落定再起下一波。
  // 结果仍按**登记序**，完成先后不影响 TAP 里的顺序。
  const waveSize = Math.max(1, Number(process.env.RESTART_TEST_CONCURRENCY) || SCENARIO_CONCURRENCY);
  for (let i = 0; i < scenarios.length; i += waveSize) {
    await Promise.all(
      scenarios.slice(i, i + waveSize).map(({ name, fn, opts }) =>
        t.test(name, async () => {
          const f = await fixture(opts);
          try { await fn(f); } finally { await f.cleanup(); }
        }),
      ),
    );
  }
});
