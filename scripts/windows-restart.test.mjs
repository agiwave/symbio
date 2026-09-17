import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
import { run, processIdentity, assertIdentity, inspectPE } from './windows-restart-prepare.mjs';
import { stopVerified } from './windows-restart.mjs';

const script = fileURLToPath(new URL('./windows-restart.mjs', import.meta.url));
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const triple = 'x86_64-pc-windows-msvc';
const source = `fn main() {
  println!("cwd={:?} args={:?}", std::env::current_dir().unwrap(), std::env::args().collect::<Vec<_>>());
  if std::env::current_exe().unwrap().file_name().unwrap() == "fail.exe" { std::process::exit(7); }
  std::thread::sleep(std::time::Duration::from_secs(180));
}`;

async function waitFor(fn, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const result = fn(); if (result) return result; await sleep(100); }
  throw new Error('waitFor timed out');
}

test('Windows restart: independent real processes only', { skip: process.platform !== 'win32', timeout: 240000 }, async t => {
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
  await run(cargoExe, ['build', '--offline', '--locked', '--release', '--target', triple], { cwd: project, timeoutMs: 60000, capture: true });
  const built = path.join(project, 'target', triple, 'release', 'restart-fixture.exe');
  const oldExe = path.join(root, 'old.exe');
  const goodExe = path.join(root, 'candidate.exe');
  const failExe = path.join(root, 'fail.exe');
  for (const exe of [oldExe, goodExe, failExe]) fs.copyFileSync(built, exe);
  const oldBytes = fs.readFileSync(oldExe);
  let n = 0;
  async function fixture() {
    const dir = path.join(root, `case-${++n}`); fs.mkdirSync(dir);
    const args = ['argument with spaces', '引号"参数', 'trailing\\'];
    const child = spawn(oldExe, args, { cwd: dir, stdio: 'ignore', windowsHide: true });
    child.on('error', () => {});
    await new Promise(resolve => child.once('spawn', resolve));
    const target = { ...await processIdentity(child.pid), cwd: dir, args };
    const c = { target, cargoExe, buildCwd: project, manifest: path.join(project, 'Cargo.toml'),
      bin: 'restart-fixture', triple, stageRoot: dir, cancelFile: path.join(dir, 'cancel'), timeoutSeconds: 60,
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
      return run(process.execPath, [script, '--config', config, ...flags], { cwd: dir, timeoutMs: 90000, capture: true });
    };
    const status = () => { try { return JSON.parse(fs.readFileSync(c.statusFile, 'utf8')); } catch { return null; } };
    const untouched = async () => {
      assertIdentity(target, await processIdentity(target.pid));
      assert.deepEqual(fs.readFileSync(oldExe), oldBytes);
      assert.equal(status()?.candidatePid, undefined);
    };
    const cleanup = async () => {
      // Only original fixture and exact PIDs published by this fixture's supervisor; never name-based stop.
      const s = status();
      for (const [pid, exe] of [[target.pid, oldExe], [s?.candidatePid, s?.candidate], [s?.rollbackPid, oldExe]]) {
        if (!pid) continue;
        try {
          const id = await processIdentity(pid);
          assert.equal(id.exe.toLowerCase(), exe.toLowerCase());
          await stopVerified(pid, id.startTimeUtcTicks, id.exe);
        } catch (e) { if (!/Command failed/.test(e.message)) throw e; }
      }
    };
    return { c, dir, invoke, makeReady, readyFile, status, untouched, cleanup, target };
  }
  async function scenario(name, fn) {
    await t.test(name, async () => { const f = await fixture(); try { await fn(f); } finally { await f.cleanup(); } });
  }

  await scenario('default dry-run writes no state/stage; background requires --execute', async f => {
    const before = fs.readdirSync(f.dir);
    assert.match(await f.invoke(f.c, []), /dry-run/);
    assert.deepEqual(fs.readdirSync(f.dir), [...before, 'config.json'].sort());
    await assert.rejects(f.invoke(f.c, ['--background']), /requires explicit/);
    await f.untouched();
  });
  await scenario('actual Cargo compile failure never stops target', async f => {
    fs.writeFileSync(path.join(project, 'src/main.rs'), 'deliberately invalid Rust');
    try { await assert.rejects(f.invoke(), /Command failed/); }
    finally { fs.writeFileSync(path.join(project, 'src/main.rs'), source); }
    assert.equal(f.status().phase, 'failed-before-confirmed-stop');
    assert.equal(f.status().events.some(e => e.phase === 'stopping-target'), false);
    await f.untouched();
  });
  await scenario('wrong identity before build never stops target', async f => {
    await assert.rejects(f.invoke({ ...f.c, target: { ...f.target, startTimeUtcTicks: '100000000000000000' } }), /identity mismatch/);
    await f.untouched();
  });
  await scenario('same-handle stop gate refuses wrong ticks/path/hash/cancel', async f => {
    await assert.rejects(stopVerified(f.target.pid, '100000000000000000', oldExe), /identity mismatch/);
    await assert.rejects(stopVerified(f.target.pid, f.target.startTimeUtcTicks, goodExe), /identity mismatch/);
    await assert.rejects(stopVerified(f.target.pid, f.target.startTimeUtcTicks, oldExe, 3000,
      { candidate: goodExe, sha256: '0'.repeat(64) }), /hash mismatch/);
    fs.writeFileSync(f.c.cancelFile, 'cancel');
    await assert.rejects(stopVerified(f.target.pid, f.target.startTimeUtcTicks, oldExe, 3000,
      { cancelFiles: [f.c.cancelFile] }), /Cancelled/);
    await f.untouched();
  });
  await scenario('ready hash and ready identity mismatch never stop target', async f => {
    await assert.rejects(f.invoke(f.makeReady(goodExe, { validation: { sha256: '0'.repeat(64) } })), /hash mismatch/);
    await assert.rejects(f.invoke(f.makeReady(goodExe, { target: { ...f.target, startTimeUtcTicks: '100000000000000000' } })), /identity mismatch/);
    await f.untouched();
  });
  await scenario('ready cancellation and config cancellation both prevent stop', async f => {
    const otherCancel = path.join(f.dir, 'prepare.cancel'); fs.writeFileSync(otherCancel, 'cancel');
    await assert.rejects(f.invoke(f.makeReady(goodExe, { cancelFile: otherCancel })), /Cancelled/);
    fs.writeFileSync(f.c.cancelFile, 'cancel');
    await assert.rejects(f.invoke(f.makeReady()), /Cancelled/);
    await f.untouched();
  });
  await scenario('real prepare build -> confirmed stop -> candidate survives with cwd/args/logs', async f => {
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
  await scenario('--ready skips invalid build; early candidate exit rolls back only after exit', async f => {
    const c = f.makeReady(failExe);
    fs.writeFileSync(path.join(project, 'src/main.rs'), 'invalid');
    try { await assert.rejects(f.invoke(f.c, ['--ready', c.readyFile, '--execute']), /Process exited/); }
    finally { fs.writeFileSync(path.join(project, 'src/main.rs'), source); }
    const s = f.status(); assert.equal(s.phase, 'rolled-back');
    assert.equal(s.events.some(e => e.phase === 'building'), false);
    const phases = s.events.map(e => e.phase);
    assert.ok(phases.indexOf('candidate-exit-confirmed') < phases.indexOf('rollback-window'));
    await assert.rejects(processIdentity(s.candidatePid), /Command failed/);
    assert.equal((await processIdentity(s.rollbackPid)).exe.toLowerCase(), oldExe.toLowerCase());
    assert.deepEqual(fs.readFileSync(oldExe), oldBytes);
  });
  await scenario('cancel during survival stops live candidate before rollback', async f => {
    const c = { ...f.makeReady(), aliveSeconds: 3 };
    const pending = f.invoke(c); // attach rejection handler immediately
    const outcome = pending.then(() => null, e => e);
    await waitFor(() => f.status()?.phase === 'survival-window');
    fs.writeFileSync(c.cancelFile, 'cancel');
    assert.match((await outcome).message, /Cancelled/);
    const s = f.status(); assert.equal(s.phase, 'rolled-back');
    await assert.rejects(processIdentity(s.candidatePid), /Command failed/);
    const phases = s.events.map(e => e.phase);
    assert.ok(phases.indexOf('candidate-exit-confirmed') < phases.indexOf('rollback-window'));
    assert.equal((await processIdentity(s.rollbackPid)).exe.toLowerCase(), oldExe.toLowerCase());
  });
  await scenario('background launcher exits while detached supervisor survives and finishes', async f => {
    const c = { ...f.makeReady(), aliveSeconds: 4 };
    const output = await f.invoke(c, ['--execute', '--background']);
    // run resolves only after launcher process exit/closed pipes.
    const launched = JSON.parse(output.trim());
    const supervisor = await processIdentity(launched.supervisorPid);
    assert.equal(supervisor.exe.toLowerCase(), process.execPath.toLowerCase());
    const s = await waitFor(() => { const s = f.status(); return s?.phase === 'survived' && s; });
    assert.equal(s.supervisorPid, launched.supervisorPid);
    assert.equal(s.applicationReady, false);
    assert.equal((await processIdentity(s.candidatePid)).exe.toLowerCase(), goodExe.toLowerCase());
    assert.ok(fs.existsSync(path.join(launched.launcherLogs, 'supervisor.stdout.log')));
    assert.ok(fs.existsSync(path.join(launched.launcherLogs, 'supervisor.stderr.log')));
    await sleep(500);
    await assert.rejects(processIdentity(supervisor.pid), /Command failed/);
  });
  await scenario('current launching test host is protected even with --execute', async f => {
    const host = { ...await processIdentity(process.pid), cwd: f.dir, args: [] };
    await assert.rejects(f.invoke({ ...f.c, target: host }), /current host/);
    assertIdentity(host, await processIdentity(process.pid));
    await f.untouched();
  });
});
