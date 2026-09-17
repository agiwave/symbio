import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { processIdentity, assertIdentity, inspectPE, run, validateConfig } from './windows-restart-prepare.mjs';

const script = fileURLToPath(new URL('./windows-restart-prepare.mjs', import.meta.url));
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

test('Windows prepare-only: real Cargo fixture and safety boundaries', { skip: process.platform !== 'win32' }, async t => {
  // The explicit target is THIS test process, never a discovered Symbio host.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio prepare test '));
  console.log(`Test evidence (retained): ${root}`);
  const project = path.join(root, 'fixture');
  const stages = path.join(root, 'stages');
  fs.mkdirSync(path.join(project, 'src'), { recursive: true });
  fs.mkdirSync(stages);
  fs.writeFileSync(path.join(project, 'Cargo.toml'), '[package]\nname = "prepare-fixture"\nversion = "0.1.0"\nedition = "2021"\n[workspace]\n');
  fs.writeFileSync(path.join(project, 'Cargo.lock'), 'version = 4\n\n[[package]]\nname = "prepare-fixture"\nversion = "0.1.0"\n');
  fs.writeFileSync(path.join(project, 'src/main.rs'), 'fn main() { println!("fixture only"); }\n');
  const where = spawnSync('where.exe', ['cargo.exe'], { encoding: 'utf8' });
  assert.equal(where.status, 0, where.stderr);
  const cargo = process.env.PREPARE_TEST_CARGO || where.stdout.trim().split(/\r?\n/)[0];
  const target = { ...await processIdentity(process.pid), cwd: root, args: [...process.argv.slice(1)] };
  const baseline = fs.readFileSync(target.exe);
  const config = { target, cargoExe: cargo, buildCwd: project, manifest: path.join(project, 'Cargo.toml'),
    bin: 'prepare-fixture', triple: 'x86_64-pc-windows-msvc', stageRoot: stages,
    cancelFile: path.join(root, 'cancel.flag'), timeoutSeconds: 60 };
  const configFile = path.join(root, 'config.json');
  const invoke = async (c, execute = true) => {
    fs.writeFileSync(configFile, JSON.stringify(c));
    return run(process.execPath, [script, '--config', configFile, ...(execute ? ['--execute'] : [])],
      { cwd: root, timeoutMs: 90000, capture: true });
  };
  const stageNames = () => fs.readdirSync(stages);
  const assertUntouched = async () => {
    assertIdentity(target, await processIdentity(target.pid));
    assert.deepEqual(fs.readFileSync(target.exe), baseline);
  };

  await t.test('default dry-run creates no files and does not run cargo', async () => {
    const output = await invoke({ ...config, cargoExe: process.execPath }, false);
    assert.match(output, /dry-run/);
    assert.deepEqual(stageNames(), []);
    await assertUntouched();
  });
  await t.test('missing explicit args and unknown fields rejected', () => {
    assert.throws(() => validateConfig({ ...config, target: { ...target, args: undefined } }), /args/);
    assert.throws(() => validateConfig({ ...config, restart: true }), /Unknown/);
    assert.throws(() => validateConfig({ ...config, target: { ...target, pid: 0 } }), /PID/);
  });
  await t.test('pre-existing cancellation prevents any stage creation', async () => {
    fs.writeFileSync(config.cancelFile, 'cancel');
    await assert.rejects(invoke(config), /Cancelled before prepare/);
    assert.deepEqual(stageNames(), []);
    fs.unlinkSync(config.cancelFile);
    await assertUntouched();
  });
  await t.test('PID reuse guard rejects wrong start time before build', async () => {
    await assert.rejects(invoke({ ...config, target: { ...target, startTimeUtcTicks: '100000000000000000' } }), /identity mismatch/);
    const stage = path.join(stages, stageNames().at(-1));
    assert.equal(fs.existsSync(path.join(stage, 'target')), false);
    assert.equal(fs.existsSync(path.join(stage, 'ready.json')), false);
    assert.match(fs.readFileSync(path.join(stage, 'supervisor.log'), 'utf8'), /FAILED/);
    await assertUntouched();
  });
  await t.test('real offline isolated Cargo build yields static validation + handoff only', async () => {
    const before = new Set(stageNames());
    try { assert.match(await invoke(config), /prepare-only/); }
    catch (e) {
      for (const n of stageNames().filter(n => !before.has(n))) {
        const log = path.join(stages, n, 'build.stderr.log');
        if (fs.existsSync(log)) console.error(fs.readFileSync(log, 'utf8'));
      }
      throw e;
    }
    const name = stageNames().find(n => !before.has(n));
    const record = JSON.parse(fs.readFileSync(path.join(stages, name, 'ready.json'), 'utf8'));
    assert.ok(record.candidate.startsWith(path.join(stages, name, 'target')));
    assert.equal(record.validation.sha256, inspectPE(record.candidate, config.triple).sha256);
    assert.equal(fs.existsSync(path.join(project, 'target')), false);
    assert.match(record.limitation, /Not a readiness test/);
    await assertUntouched();
  });
  await t.test('real Cargo compilation failure leaves no ready record and target intact', async () => {
    fs.writeFileSync(path.join(project, 'src/main.rs'), 'this is deliberately invalid Rust');
    const before = new Set(stageNames());
    await assert.rejects(invoke(config), /Command failed/);
    const name = stageNames().find(n => !before.has(n));
    assert.equal(fs.existsSync(path.join(stages, name, 'ready.json')), false);
    assert.match(fs.readFileSync(path.join(stages, name, 'build.stderr.log'), 'utf8'), /error/);
    await assertUntouched();
  });
  await t.test('static validator rejects corrupt PE, wrong architecture and DLL', () => {
    const bad = path.join(root, 'bad.exe');
    fs.writeFileSync(bad, 'not an executable');
    assert.throws(() => inspectPE(bad, config.triple), /Invalid PE/);
    const pe = Buffer.alloc(128);
    pe.writeUInt16LE(0x5a4d, 0); pe.writeUInt32LE(64, 0x3c);
    pe.writeUInt32LE(0x4550, 64); pe.writeUInt16LE(0x14c, 68); pe.writeUInt16LE(2, 86);
    fs.writeFileSync(bad, pe);
    assert.throws(() => inspectPE(bad, config.triple), /architecture/);
    pe.writeUInt16LE(0x8664, 68); pe.writeUInt16LE(0x2002, 86);
    fs.writeFileSync(bad, pe);
    assert.throws(() => inspectPE(bad, config.triple), /not an executable/);
  });
  for (const mode of ['timeout', 'cancel']) {
    await t.test(`${mode} returns without killing child; child exits naturally`, async () => {
      const marker = path.join(root, `${mode}.done`);
      const flag = path.join(root, `${mode}.flag`);
      const out = fs.openSync(path.join(root, `${mode}.stdout.log`), 'wx');
      const err = fs.openSync(path.join(root, `${mode}.stderr.log`), 'wx');
      const source = `setTimeout(() => require('node:fs').writeFileSync(${JSON.stringify(marker)}, 'natural exit'), 1000)`;
      const timer = mode === 'cancel' ? setTimeout(() => fs.writeFileSync(flag, 'cancel'), 200) : undefined;
      try {
        await assert.rejects(run(process.execPath, ['-e', source], { cwd: root, timeoutMs: mode === 'timeout' ? 200 : 10000,
          out, err, cancelFile: mode === 'cancel' ? flag : undefined }), mode === 'timeout' ? /Timeout/ : /Cancelled/);
      } finally { clearTimeout(timer); fs.closeSync(out); fs.closeSync(err); }
      for (let i = 0; i < 100 && !fs.existsSync(marker); i++) await sleep(100);
      assert.equal(fs.readFileSync(marker, 'utf8'), 'natural exit');
      await assertUntouched();
    });
  }
});
