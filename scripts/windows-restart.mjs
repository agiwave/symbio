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
  { cwd: process.cwd(), timeoutMs: exitWaitMs + 15000, capture: true });
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
async function protectedPids() {
  const code = `$ErrorActionPreference='Stop'; $n=${process.pid}; $seen=@{}; while($n -gt 0 -and -not $seen.ContainsKey($n)){ $seen[$n]=$true; $n; $p=Get-CimInstance Win32_Process -Filter "ProcessId=$n"; if($null -eq $p){break}; $n=[int]$p.ParentProcessId }`;
  const output = await run(powershell(), ['-NoProfile', '-NonInteractive', '-EncodedCommand',
    Buffer.from(code, 'utf16le').toString('base64')], { cwd: process.cwd(), timeoutMs: 15000, capture: true });
  return [...new Set([process.pid, process.ppid, ...output.trim().split(/\s+/).map(Number),
    ...(process.env.SYMBIO_RESTART_PROTECTED_PIDS || '').split(',').map(Number)])].filter(n => n > 0);
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
  const protectedIds = await protectedPids();
  requireThat(!protectedIds.includes(c.target.pid), 'Refusing to stop supervisor/current host/ancestor');
  const lock = `${c.statusFile}.lock`;
  const lockFd = fs.openSync(lock, 'wx');
  fs.writeFileSync(lockFd, JSON.stringify({ supervisorPid: process.pid, targetPid: c.target.pid }));
  let stage, child, stopped = false;
  let status = { supervisorPid: process.pid, target: c.target, applicationReady: false, startedAt: new Date().toISOString(), events: [] };
  const phase = (name, extra = {}) => {
    const at = new Date().toISOString();
    status = { ...status, ...extra, phase: name, updatedAt: at, stage,
      events: [...status.events, { phase: name, at }] };
    fs.writeFileSync(`${c.statusFile}.tmp`, JSON.stringify(status, null, 2));
    fs.renameSync(`${c.statusFile}.tmp`, c.statusFile);
    if (stage) fs.appendFileSync(path.join(stage, 'supervisor.log'), `${at} ${name} ${JSON.stringify(extra)}\n`);
  };
  const deadline = Date.now() + c.timeoutSeconds * 1000;
  const remaining = () => { const ms = deadline - Date.now(); requireThat(ms > 0, 'Restart deadline exceeded'); return ms; };
  const cancelFiles = [c.cancelFile];
  const check = () => { remaining(); for (const f of cancelFiles) requireThat(!fs.existsSync(f), 'Cancelled'); };
  try {
    stage = fs.mkdtempSync(path.join(c.stageRoot, 'symbio-restart-'));
    phase('verifying'); check();
    assertIdentity(c.target, await processIdentity(c.target.pid));
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
