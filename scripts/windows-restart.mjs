#!/usr/bin/env node
// One-shot Windows restart supervisor. Default dry-run; only --execute performs the real switch.
// Reuses prepare's build path (scripts/windows-restart-prepare.mjs). Never touches the current host.
// Flow: verify identity/hash/cancel -> stop explicit PID via one PowerShell process-object handle ->
// launch candidate with user-asserted cwd/args -> survival window -> status.json. Old exe file is
// never modified or deleted; rollback relaunches it. Survival is NOT application readiness.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { validateConfig, run, processIdentity, assertIdentity, inspectPE, prepare } from './windows-restart-prepare.mjs';

function requireThat(ok, message) { if (!ok) throw new Error(message); }
function absolute(value, label) {
  requireThat(typeof value === 'string' && path.isAbsolute(value), `${label}: absolute path required`);
  return value;
}
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const sameFile = (a, b) => {
  try { return fs.realpathSync.native(a).toLowerCase() === fs.realpathSync.native(b).toLowerCase(); }
  catch { return false; }
};
const powershell = () => path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
const psQuote = s => `'` + String(s).replace(/'/g, `''`) + `'`;

export function validateRestartConfig(c) {
  requireThat(c && typeof c === 'object', 'Configuration object required');
  requireThat(Object.keys(c).every(k => ['target', 'cargoExe', 'buildCwd', 'manifest', 'bin', 'triple', 'stageRoot',
    'cancelFile', 'timeoutSeconds', 'statusFile', 'readyFile', 'aliveSeconds'].includes(k)), 'Unknown configuration field');
  validateConfig({ target: c.target, cargoExe: c.cargoExe, buildCwd: c.buildCwd, manifest: c.manifest,
    bin: c.bin, triple: c.triple, stageRoot: c.stageRoot, cancelFile: c.cancelFile, timeoutSeconds: c.timeoutSeconds });
  absolute(c.statusFile, 'statusFile');
  if (c.readyFile !== undefined) {
    absolute(c.readyFile, 'readyFile');
    requireThat(fs.statSync(c.readyFile).isFile(), 'readyFile: wrong type');
  }
  if (c.aliveSeconds !== undefined) {
    requireThat(Number.isInteger(c.aliveSeconds) && c.aliveSeconds >= 1 && c.aliveSeconds <= 600, 'aliveSeconds must be 1..600');
  }
  return c;
}

// Stop a process after verifying identity on the SAME PowerShell Process object used for Kill,
// so a recycled PID can never pass the check. Waits for confirmed exit; never launches anything.
export async function stopVerified(pid, startTimeUtcTicks, exePath, exitWaitMs = 30000, gate = {}) {
  requireThat(Number.isSafeInteger(pid) && pid > 0, 'Invalid PID');
  const code = [
    `$ErrorActionPreference='Stop'`,
    `$p=Get-Process -Id ${pid} -ErrorAction Stop`,
    `$null=$p.Handle`,
    `$exe=$p.Path; $ticks=$p.StartTime.ToUniversalTime().Ticks.ToString()`,
    `if($p.Id -ne ${pid}){throw 'identity mismatch'}`,
    `if($ticks -ne ${psQuote(startTimeUtcTicks)}){throw 'identity mismatch'}`,
    `if(-not [string]::Equals($exe,${psQuote(exePath)},[StringComparison]::OrdinalIgnoreCase)){throw 'identity mismatch'}`,
    ...(gate.candidate ? [
      `$stream=[IO.File]::OpenRead(${psQuote(gate.candidate)})`,
      `$sha=[Security.Cryptography.SHA256]::Create()`,
      `try{$hash=[BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','')}finally{$stream.Dispose();$sha.Dispose()}`,
      `if($hash -ne ${psQuote(gate.sha256)}){throw 'Candidate hash mismatch'}`,
    ] : []),
    ...(gate.cancelFiles || []).map(f => `if(Test-Path -LiteralPath ${psQuote(f)}){throw 'Cancelled before stop'}`),
    `$p.Kill()`,
    `if(-not $p.WaitForExit(${Math.max(1000, Math.floor(exitWaitMs))})){throw 'process did not exit in time'}`,
    `if(-not $p.HasExited){throw 'process still running'}`,
    `'STOPPED'`,
  ].join('; ');
  const out = await run(powershell(), ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand',
    Buffer.from(code, 'utf16le').toString('base64')],
  // 外层预算 = **进程启动 + 这段脚本本身的耗时**，与判据无关：判据是脚本里的
  // `WaitForExit(exitWaitMs)`（它才是"目标必须在 exitWaitMs 内退出"）。原先外层只给
  // `exitWaitMs + 15s` 余量，而冷全量编译压满机器时，单是 `powershell.exe` 冷启动 +
  // `Get-Process` + 对 candidate 算一次 SHA256 就可能吃掉十几秒 ⇒ 外层先超时，
  // 报出来的是「Timeout: no process killed」——看上去像"没杀掉"，其实是"没来得及判"。
  // 实测两次：并发 3 的实验、以及 rebase 后首次全量（backend 10m58s）。
  // 余量放宽**不放松任何一条判据**：exitWaitMs 仍然原样传给 WaitForExit。
  { cwd: process.cwd(), timeoutMs: exitWaitMs + 60000, capture: true });
  requireThat(out.includes('STOPPED'), `Stop failed: ${out.trim() || 'no output'}`);
}

// For our own children, exit is observed from spawn onward (never infer exit from kill failure).
export async function stopSpawned(child, timeoutMs = 10000) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  if (!child.pid) { await child.finished; return; }
  child.kill();
  let timer;
  try {
    await Promise.race([child.finished, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Candidate exit unconfirmed; rollback forbidden')), timeoutMs);
    })]);
  } finally { clearTimeout(timer); }
}

function launch(exe, target, stage, prefix) {
  const out = fs.openSync(path.join(stage, `${prefix}.stdout.log`), 'wx');
  const err = fs.openSync(path.join(stage, `${prefix}.stderr.log`), 'wx');
  let child;
  try {
    child = spawn(exe, target.args, { cwd: target.cwd, detached: true, windowsHide: true,
      stdio: ['ignore', out, err] });
  } finally { fs.closeSync(out); fs.closeSync(err); }
  child.finished = new Promise(resolve => {
    child.once('error', error => {
      child.failure = error.message;
      // A failed kill is not evidence of exit. Only spawn failure (no PID) may resolve here.
      if (!child.pid) resolve();
    });
    child.once('exit', (code, signal) => { child.failure = `Process exited (${code}, ${signal})`; resolve(); });
  });
  child.unref();
  return child;
}

async function survival(child, seconds, check) {
  const end = Date.now() + seconds * 1000;
  do {
    check();
    requireThat(!child.failure, child.failure);
    await Promise.race([sleep(Math.min(100, Math.max(0, end - Date.now()))), child.finished]);
    requireThat(!child.failure, child.failure);
  } while (Date.now() < end);
  check();
}

// Include the launching host even when the short-lived background launcher has already exited.
//
// ## 为什么把「祖先链」与「目标身份」合成**一次** PowerShell 调用
//
// 两者问的是「同一时刻、同一台机器」的两个事实，分成两次调用只多付一次
// `powershell.exe` 冷启动——本机实测**每次 1–2.5s**，而本脚本一次换壳要问好几轮。
// 合并只动**取数方式**，不动判据：`assertIdentity` 的比较逻辑一字未改，
// `stopVerified` 里「同一个 Process 对象上校验再 Kill」才是权威判据，仍独立执行。
//
// 祖先链顺带从「每层一次 `Get-CimInstance -Filter`」改成「一次全表 + 走链」：
// 原实现 5 层就是 5 次 WMI 过滤查询（实测约 1s），而一次全表查询够回答整条链。
//
// `pid` 为空时只取祖先链（`background` 只要这个），此时不发 `Get-Process`——
// 少一次进程查询，也避免在「还不知道要不要停」的阶段就把目标查活。
async function probe(pid) {
  const code = [
    `$ErrorActionPreference='Stop'`,
    `$parent=@{}; Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId | ForEach-Object { $parent[[int]$_.ProcessId]=[int]$_.ParentProcessId }`,
    `$n=${process.pid}; $seen=@{}; $chain=@()`,
    `while($n -gt 0 -and -not $seen.ContainsKey($n)){ $seen[$n]=$true; $chain+=$n; $n=[int]$parent[$n] }`,
    ...(pid ? [
      `$p=Get-Process -Id ${pid} -ErrorAction Stop`,
      `$identity=@{ pid=$p.Id; exe=$p.Path; startTimeUtcTicks=$p.StartTime.ToUniversalTime().Ticks.ToString() }`,
    ] : [`$identity=$null`]),
    `@{ protected=@($chain); identity=$identity } | ConvertTo-Json -Compress -Depth 4`,
  ].join('; ');
  const output = await run(powershell(), ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand',
    Buffer.from(code, 'utf16le').toString('base64')],
  // 一次全表 WMI 查询 + 走链 + 一次 `Get-Process`。原先给 20s：空闲时约 1.5–3s，
  // 但机器被 cargo 压满时实测能吃掉 20s 以上（`Get-CimInstance Win32_Process` 是
  // 全表枚举，越忙越慢）⇒ 超时后抛的是 `run` 的「Command failed」，与「目标已经没了」
  // 同形，会把"没读到"说成"不存在"。给它 60s。
  { cwd: process.cwd(), timeoutMs: 60000, capture: true });
  const parsed = JSON.parse(output.replace(/^\uFEFF/, '').trim());
  // PowerShell 5.1 的 `ConvertTo-Json` 会把单元素数组压成标量 ⇒ 两种形态都接。
  const chain = parsed.protected == null ? [] : [].concat(parsed.protected);
  return {
    protected: [...new Set([process.pid, process.ppid, ...chain.map(Number),
      ...(process.env.SYMBIO_RESTART_PROTECTED_PIDS || '').split(',').map(Number)])].filter(n => n > 0),
    identity: parsed.identity ?? null,
  };
}

const STATUS_SLEEP = new Int32Array(new SharedArrayBuffer(4));

/**
 * 把 `payload` 原子地落成 `file`（先写 `.tmp` 再 rename，读者永远看到完整 JSON）。
 *
 * 返回 `true` = 落盘成功；`false` = **没落成，但不抛**。
 *
 * ## 为什么"写不出去"必须返回 false 而不是抛
 *
 * 这是实测出来的，不是防御性编程。Windows 上 `rename` 会因并发读者 / 杀软扫描临时
 * 文件而偶发 `EPERM`。实测（2026-10-19 门禁 docs 阶段）：回归测试每 100ms 读一次
 * `status.json`，`phase('pre-switch')` 的 rename 抛 EPERM ⇒ 异常冒进 `restart()` 的
 * catch，而 catch 里第一件事又是 `phase(...)`，**再抛一次** ⇒ 进程直接崩在 `verifying`。
 * 后果最难堪：**目标进程还活着、候选没起、status.json 停在半路**，调用方永远等不到
 * 结论——一次"报告通道写失败"被当成了"换壳失败"。
 *
 * 所以：短退避重试（读者 / 扫描通常只持有几毫秒），仍不行就降级为"这一帧没报出去"，
 * 操作照走。进度不会因此丢证据：`stage/supervisor.log` 里有同一份流水，而
 * `restart()` 的返回值是权威结果。
 *
 * 重试用 `Atomics.wait` 做**同步**睡眠：调用方（`phase`）是同步的，为一次 rename 把
 * 整条换壳流程改成 async 得不偿失。
 *
 * 判据：`windows-restart.test.mjs`「writeStatusAtomic」——把目标占成目录让 rename
 * **必然**失败，断言它返回 false 且不抛（"偶发 EPERM"没法在测试里复现，就让失败必然发生）。
 */
export function writeStatusAtomic(file, payload) {
  const tmp = `${file}.tmp`;
  let text;
  try {
    text = JSON.stringify(payload, null, 2);
  } catch {
    return false; // 循环引用等：连文本都产不出
  }
  try {
    fs.writeFileSync(tmp, text);
  } catch {
    return false; // 临时文件都写不出（磁盘满 / 目录被删）⇒ 同上
  }
  for (let attempt = 0; attempt < 6; attempt++) {
    try {
      fs.renameSync(tmp, file);
      return true;
    } catch {
      if (attempt === 5) return false; // 见上：报告通道失败 ≠ 操作失败
      Atomics.wait(STATUS_SLEEP, 0, 0, 15 * (attempt + 1));
    }
  }
  return false;
}

async function protectedPids() {
  return (await probe(null)).protected;
}
const baseConfig = c => Object.fromEntries(['target', 'cargoExe', 'buildCwd', 'manifest', 'bin', 'triple',
  'stageRoot', 'cancelFile', 'timeoutSeconds'].map(k => [k, c[k]]));

export async function restart(config, execute = false) {
  requireThat(process.platform === 'win32', 'Windows only');
  const c = validateRestartConfig(config);
  const aliveSeconds = c.aliveSeconds ?? 20;
  console.log(JSON.stringify({ mode: execute ? 'execute' : 'dry-run', target: c.target,
    readyFile: c.readyFile, statusFile: c.statusFile, aliveSeconds,
    note: 'Survival only, NOT application readiness. Old exe is never overwritten.' }, null, 2));
  if (!execute) return { dryRun: true }; // no subprocesses/files in dry-run
  // 一次探测同时给出「祖先链」与「目标身份」：前者判「是不是自己人」，后者是第一道
  // 身份校验。权威校验仍是 `stopVerified`（同一 Process 对象上校验再 Kill）。
  const pre = await probe(c.target.pid);
  requireThat(!pre.protected.includes(c.target.pid), 'Refusing to stop supervisor/current host/ancestor');
  const lock = `${c.statusFile}.lock`;
  const lockFd = fs.openSync(lock, 'wx');
  fs.writeFileSync(lockFd, JSON.stringify({ supervisorPid: process.pid, targetPid: c.target.pid }));
  let stage, child, stopped = false;
  let status = { supervisorPid: process.pid, target: c.target, applicationReady: false, startedAt: new Date().toISOString(), events: [] };
  // 落盘走 `writeStatusAtomic`（**失败不抛**，理由见它的文档注释）：报告通道坏了
  // 不许等于操作失败。
  const writeStatus = () => writeStatusAtomic(c.statusFile, status);
  const phase = (name, extra = {}) => {
    const at = new Date().toISOString();
    status = { ...status, ...extra, phase: name, updatedAt: at, stage,
      events: [...status.events, { phase: name, at }] };
    writeStatus();
    if (stage) fs.appendFileSync(path.join(stage, 'supervisor.log'), `${at} ${name} ${JSON.stringify(extra)}\n`);
  };
  const deadline = Date.now() + c.timeoutSeconds * 1000;
  const remaining = () => { const ms = deadline - Date.now(); requireThat(ms > 0, 'Restart deadline exceeded'); return ms; };
  const cancelFiles = [c.cancelFile];
  const check = () => { remaining(); for (const f of cancelFiles) requireThat(!fs.existsSync(f), 'Cancelled'); };
  try {
    stage = fs.mkdtempSync(path.join(c.stageRoot, 'symbio-restart-'));
    phase('verifying'); check();
    // 与祖先链同一次读取到的身份（见 `probe`）：这是**第一道**校验，用来在花掉一次
    // 构建之前就发现「目标已经换了个进程」。权威校验在 `stopVerified`。
    assertIdentity(c.target, pre.identity ?? await processIdentity(c.target.pid));
    let ready;
    if (c.readyFile) {
      ready = JSON.parse(fs.readFileSync(c.readyFile, 'utf8').replace(/^\uFEFF/, ''));
      requireThat(ready.mode === 'prepare-only', 'Expected prepare-only ready record');
      assertIdentity(c.target, ready.target);
      requireThat(ready.triple === c.triple, 'Ready triple mismatch');
      cancelFiles.push(absolute(ready.cancelFile, 'ready.cancelFile'));
    } else {
      phase('building');
      const result = await prepare({ ...baseConfig(c), stageRoot: stage,
        timeoutSeconds: Math.min(7200, Math.ceil(remaining() / 1000)) }, true);
      ready = JSON.parse(fs.readFileSync(path.join(result.stage, 'ready.json'), 'utf8'));
    }
    check();
    const candidate = fs.realpathSync.native(absolute(ready.candidate, 'candidate'));
    requireThat(!sameFile(candidate, c.target.exe), 'Candidate must not be old exe');
    const sha256 = ready.validation?.sha256;
    requireThat(typeof sha256 === 'string' && /^[a-f0-9]{64}$/i.test(sha256), 'Ready hash missing/invalid');
    requireThat(inspectPE(candidate, c.triple).sha256 === sha256.toLowerCase(), 'Candidate hash mismatch');
    const oldSha256 = inspectPE(c.target.exe, c.triple).sha256;
    phase('pre-switch', { candidate, candidateSha256: sha256, oldSha256 });
    assertIdentity(c.target, await processIdentity(c.target.pid));
    check();
    // Leave time for starting and observing the new process; rollback has an independent budget.
    requireThat(remaining() > aliveSeconds * 1000 + 2000, 'Insufficient time for survival window');
    phase('stopping-target');
    await stopVerified(c.target.pid, c.target.startTimeUtcTicks, fs.realpathSync.native(c.target.exe),
      Math.min(30000, remaining()), { candidate, sha256, cancelFiles });
    stopped = true;
    phase('target-stopped');
    check();
    requireThat(inspectPE(candidate, c.triple).sha256 === sha256.toLowerCase(), 'Candidate hash changed after stop');
    child = launch(candidate, c.target, stage, 'candidate');
    phase('survival-window', { candidatePid: child.pid, aliveSeconds });
    await survival(child, aliveSeconds, check);
    phase('survived', { survivedSeconds: aliveSeconds });
    return status;
  } catch (error) {
    if (!stopped) {
      phase('failed-before-confirmed-stop', { error: error.message,
        note: 'No candidate or rollback launched. If stop acknowledgement was lost, inspect target manually.' });
    } else {
      try {
        phase('recovering', { error: error.message });
        if (child) await stopSpawned(child);
        phase('candidate-exit-confirmed');
        requireThat(inspectPE(c.target.exe, c.triple).sha256 === status.oldSha256, 'Old exe hash changed; rollback forbidden');
        const rollback = launch(c.target.exe, c.target, stage, 'rollback');
        phase('rollback-window', { rollbackPid: rollback.pid });
        await survival(rollback, aliveSeconds, () => {}); // cancel never suppresses safety recovery
        phase('rolled-back', { rollbackSurvivedSeconds: aliveSeconds });
      } catch (recoveryError) {
        phase('recovery-failed', { recoveryError: recoveryError.message,
          note: 'Do not manually start another instance until candidate exit is verified.' });
      }
    }
    throw error;
  } finally { fs.closeSync(lockFd); fs.unlinkSync(lock); }
}

export async function background(config) {
  validateRestartConfig(config);
  const ids = await protectedPids();
  requireThat(!ids.includes(config.target.pid), 'Refusing to stop current host/ancestor');
  const dir = fs.mkdtempSync(path.join(config.stageRoot, 'symbio-launch-'));
  const snapshot = path.join(dir, 'config.json');
  fs.writeFileSync(snapshot, JSON.stringify(config, null, 2), { flag: 'wx' });
  const out = fs.openSync(path.join(dir, 'supervisor.stdout.log'), 'wx');
  const err = fs.openSync(path.join(dir, 'supervisor.stderr.log'), 'wx');
  let child;
  try {
    child = spawn(process.execPath, [fileURLToPath(import.meta.url), '--config', snapshot, '--execute'],
      { detached: true, windowsHide: true, stdio: ['ignore', out, err],
        env: { ...process.env, SYMBIO_RESTART_PROTECTED_PIDS: ids.join(',') } });
    await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
    child.unref();
  } finally { fs.closeSync(out); fs.closeSync(err); }
  const result = { supervisorPid: child.pid, statusFile: config.statusFile, launcherLogs: dir,
    note: 'Spawn acknowledged only; query status.json for outcome.' };
  console.log(JSON.stringify(result));
  return result;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const args = process.argv.slice(2);
    let configFile, readyFile, execute = false, detached = false;
    const seen = new Set();
    while (args.length) {
      const arg = args.shift();
      requireThat(!seen.has(arg), `Duplicate option ${arg}`); seen.add(arg);
      if (arg === '--config') configFile = absolute(args.shift(), 'config');
      else if (arg === '--ready') readyFile = absolute(args.shift(), 'ready');
      else if (arg === '--execute') execute = true;
      else if (arg === '--background') detached = true;
      else throw new Error(`Unknown option ${arg}`);
    }
    requireThat(configFile, 'Usage: node scripts/windows-restart.mjs --config ABS_JSON [--ready ABS_READY] [--execute [--background]]');
    requireThat(!detached || execute, '--background requires explicit --execute');
    const config = JSON.parse(fs.readFileSync(configFile, 'utf8').replace(/^\uFEFF/, ''));
    if (readyFile) config.readyFile = readyFile;
    if (detached) await background(config); else await restart(config, execute);
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
