#!/usr/bin/env node
// One-shot, prepare-only supervisor. No process termination or application launch.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { pathToFileURL } from 'node:url';

function requireThat(ok, message) { if (!ok) throw new Error(message); }
function absolute(value, label) {
  requireThat(typeof value === 'string' && path.isAbsolute(value), `${label}: absolute path required`);
  return value;
}
function existing(value, label, directory = false) {
  absolute(value, label);
  requireThat(directory ? fs.statSync(value).isDirectory() : fs.statSync(value).isFile(), `${label}: wrong type`);
  return fs.realpathSync.native(value);
}
export function validateConfig(c) {
  const keys = ['target', 'cargoExe', 'buildCwd', 'manifest', 'bin', 'triple', 'stageRoot', 'cancelFile', 'timeoutSeconds'];
  requireThat(c && Object.keys(c).every(k => keys.includes(k)), 'Unknown configuration field');
  requireThat(c.target && Object.keys(c.target).every(k => ['pid', 'exe', 'startTimeUtcTicks', 'cwd', 'args'].includes(k)), 'Invalid target');
  requireThat(Number.isSafeInteger(c.target.pid) && c.target.pid > 0, 'Explicit positive target PID required');
  requireThat(typeof c.target.startTimeUtcTicks === 'string' && /^\d{15,20}$/.test(c.target.startTimeUtcTicks), 'Explicit startTimeUtcTicks string required');
  requireThat(Array.isArray(c.target.args) && c.target.args.every(a => typeof a === 'string'), 'Explicit target args array required (including [])');
  existing(c.target.exe, 'target.exe');
  existing(c.target.cwd, 'target.cwd', true);
  existing(c.cargoExe, 'cargoExe');
  existing(c.buildCwd, 'buildCwd', true);
  existing(c.manifest, 'manifest');
  existing(c.stageRoot, 'stageRoot', true);
  absolute(c.cancelFile, 'cancelFile');
  requireThat(/^[a-zA-Z0-9_-]+$/.test(c.bin ?? ''), 'Invalid bin');
  requireThat(['x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'i686-pc-windows-msvc'].includes(c.triple), 'Explicit supported Windows MSVC triple required');
  requireThat(Number.isInteger(c.timeoutSeconds) && c.timeoutSeconds >= 1 && c.timeoutSeconds <= 7200, 'timeoutSeconds must be 1..7200');
  return c;
}

// Timeout never kills even the build process. It may finish in isolation, but cannot publish ready.json.
export function run(exe, args, { cwd, env = process.env, timeoutMs, out, err, capture = false, cancelFile }) {
  return new Promise((resolve, reject) => {
    const child = spawn(exe, args, { cwd, env, shell: false, windowsHide: true, stdio: ['ignore', capture ? 'pipe' : out, capture ? 'pipe' : err] });
    let text = '', errors = '', done = false;
    child.stdout?.on('data', b => { text += b; });
    child.stderr?.on('data', b => { errors += b; });
    const finish = (error, result) => {
      if (done) return;
      done = true;
      clearTimeout(timer); clearInterval(poller);
      if (error) {
        child.stdout?.destroy(); child.stderr?.destroy(); child.unref();
        reject(error);
      } else resolve(result);
    };
    const timer = setTimeout(() => finish(new Error('Timeout: no process killed; build children may still finish in isolated directory. Do not delete it until they exit.')), timeoutMs);
    const poller = setInterval(() => {
      if (cancelFile && fs.existsSync(cancelFile)) finish(new Error('Cancelled: no process killed; build children may still finish in isolation.'));
    }, 100);
    child.on('error', e => finish(e));
    child.on('close', code => finish(code === 0 ? null : new Error(`Command failed (${code}): ${errors.trim()}`), text));
  });
}

export async function processIdentity(pid, timeoutMs = 10000) {
  requireThat(Number.isSafeInteger(pid) && pid > 0, 'Invalid PID');
  const ps = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
  const code = `$ErrorActionPreference='Stop'; $p=Get-Process -Id ${pid}; @{ pid=$p.Id; exe=$p.Path; startTimeUtcTicks=$p.StartTime.ToUniversalTime().Ticks.ToString() } | ConvertTo-Json -Compress`;
  const output = await run(ps, ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(code, 'utf16le').toString('base64')], { cwd: process.cwd(), timeoutMs, capture: true });
  return JSON.parse(output.replace(/^\uFEFF/, '').trim());
}
export function assertIdentity(expected, actual) {
  requireThat(actual.pid === expected.pid && actual.startTimeUtcTicks === expected.startTimeUtcTicks &&
    typeof actual.exe === 'string' && fs.realpathSync.native(actual.exe).toLowerCase() === fs.realpathSync.native(expected.exe).toLowerCase(),
  'Target identity mismatch (PID/path/start time); refusing prepare');
}
export function inspectPE(file, triple) {
  const b = fs.readFileSync(file);
  requireThat(b.length >= 64 && b.readUInt16LE(0) === 0x5a4d, 'Invalid PE: DOS header');
  const pe = b.readUInt32LE(0x3c);
  requireThat(pe >= 64 && pe + 26 <= b.length && b.readUInt32LE(pe) === 0x4550, 'Invalid PE signature');
  const machine = { 'x86_64-pc-windows-msvc': 0x8664, 'aarch64-pc-windows-msvc': 0xaa64, 'i686-pc-windows-msvc': 0x14c }[triple];
  requireThat(b.readUInt16LE(pe + 4) === machine, 'PE architecture mismatch');
  const flags = b.readUInt16LE(pe + 22);
  requireThat((flags & 2) !== 0 && (flags & 0x2000) === 0, 'PE is not an executable application');
  return { sha256: crypto.createHash('sha256').update(b).digest('hex'), bytes: b.length, machine };
}

export async function prepare(config, execute = false) {
  requireThat(process.platform === 'win32', 'Windows only');
  const c = validateConfig(config);
  const plan = { mode: execute ? 'prepare-only' : 'dry-run', target: c.target, buildCwd: c.buildCwd, manifest: c.manifest,
    bin: c.bin, triple: c.triple, stageRoot: c.stageRoot, cancelFile: c.cancelFile,
    timeoutSeconds: c.timeoutSeconds, note: 'No stop, launch, heartbeat changes or automatic switch. Args/cwd are user assertions, not discovered process properties.' };
  console.log(JSON.stringify(plan, null, 2));
  if (!execute) return { dryRun: true }; // No subprocess, directory or log file in default mode.
  requireThat(!fs.existsSync(c.cancelFile), 'Cancelled before prepare');
  const stage = fs.mkdtempSync(path.join(fs.realpathSync.native(c.stageRoot), 'symbio-prepare-'));
  const log = message => fs.appendFileSync(path.join(stage, 'supervisor.log'), `${new Date().toISOString()} ${message}\n`);
  const deadline = Date.now() + c.timeoutSeconds * 1000;
  const remaining = () => { const ms = deadline - Date.now(); requireThat(ms > 0, 'Preparation deadline exceeded'); return ms; };
  const checkCancelled = () => requireThat(!fs.existsSync(c.cancelFile), 'Cancelled; no ready record published');
  const targetDir = path.join(stage, 'target');
  let stdout, stderr;
  console.log(`Stage: ${stage}`);
  try {
    log('START prepare-only; target untouched');
    assertIdentity(c.target, await processIdentity(c.target.pid, Math.min(10000, remaining())));
    checkCancelled();
    const args = ['build', '--locked', '--offline', '--release', '--manifest-path', c.manifest, '--bin', c.bin, '--target', c.triple, '--target-dir', targetDir];
    log(`BUILD ${JSON.stringify({ exe: c.cargoExe, args, cwd: c.buildCwd })}`);
    stdout = fs.openSync(path.join(stage, 'build.stdout.log'), 'wx');
    stderr = fs.openSync(path.join(stage, 'build.stderr.log'), 'wx');
    await run(c.cargoExe, args, { cwd: c.buildCwd, env: { ...process.env, CARGO_TARGET_DIR: targetDir }, timeoutMs: remaining(), out: stdout, err: stderr, cancelFile: c.cancelFile });
    checkCancelled(); remaining();
    const candidate = path.join(targetDir, c.triple, 'release', `${c.bin}.exe`);
    // Refuse links escaping the unique target directory before reading candidate bytes.
    const rel = path.relative(fs.realpathSync.native(targetDir), fs.realpathSync.native(candidate));
    requireThat(!rel.startsWith('..') && !path.isAbsolute(rel), 'Candidate escapes isolated target directory');
    const validation = inspectPE(candidate, c.triple);
    assertIdentity(c.target, await processIdentity(c.target.pid, Math.min(10000, remaining())));
    checkCancelled(); remaining();
    const record = { ...plan, candidate, validation, preparedAt: new Date().toISOString(),
      limitation: 'Static PE/hash checks only. Not a readiness test or restart authorization. Recheck identity, cancellation, hash, dependencies and user approval before any future manual switch.' };
    fs.writeFileSync(path.join(stage, 'ready.json'), JSON.stringify(record, null, 2), { flag: 'wx' });
    log('PREPARED; old instance still running; manual acceptance required');
    return { stage, candidate, validation };
  } catch (error) {
    log(`FAILED ${error.message}; target untouched; no switch`);
    throw error;
  } finally {
    if (stdout !== undefined) fs.closeSync(stdout);
    if (stderr !== undefined) fs.closeSync(stderr);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const args = process.argv.slice(2);
    requireThat(args.length >= 2 && args.length <= 3 && args[0] === '--config' && (args.length === 2 || args[2] === '--execute'),
      'Usage: node scripts/windows-restart-prepare.mjs --config ABSOLUTE_JSON_PATH [--execute]');
    const config = JSON.parse(fs.readFileSync(absolute(args[1], 'config'), 'utf8').replace(/^\uFEFF/, ''));
    await prepare(config, args[2] === '--execute');
  } catch (e) { console.error(e.message); process.exitCode = 1; }
}
