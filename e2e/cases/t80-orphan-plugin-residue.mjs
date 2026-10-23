import './_selfrun.mjs';
// T80 孤儿插件目录（①）：`.symbio/` 是跨构建存活的数据目录，旧构建留下的
// `PLUGIN.yml` 指向一个本构建不存在的工厂 ⇒ 装配时**不是异常**，不许喊 WARN。
//
// 判据分两段，对应退出条件的两半：
//  ① 干净系统目录启动零条 `[composite WARN]`；
// ② 种下孤儿目录（与 实测同一形态：`projection/PLUGIN.yml` 声明
//     `plugin_provider: projection`，全仓无此工厂）后再启动，WARN 仍为零，
//     且默认级别下连目录名都不出现（降级到 debug，不是换句措辞继续喊）。
//
// 为什么用真进程而不是单测：级别闸门在 CLI 的 `logger_set_min_level`（默认 INFO），
// 单测在进程内看不到「默认配置下会不会打出来」。
import { join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import {
  cleanupHomedir,
  defineCase,
  makeHomedir,
  providerConfig,
  PROVIDER_ID,
  runCli,
  assert,
  assertEq,
} from '../helpers.mjs';

/** 只启动、装配、退出——不发消息，因此不需要 mock LLM 在听 */
const START_AND_EXIT = '/exit\n';

export default defineCase(
  'T80 孤儿插件目录：干净启动与残渣启动都零条 WARN（目录仍可停用/卸载）',
  async () => {
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(1) }],
    });
    try {
      // ① 干净的系统目录：装配期不应有任何 composite WARN
      const clean = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        stdinText: START_AND_EXIT,
      });
      assertEq(clean.code, 0, `干净启动应正常退出（stderr: ${clean.stderr.slice(-800)}）`);
      assert(
        !clean.stderr.includes('[composite WARN]'),
        `干净系统目录启动不许有 composite WARN（实际 stderr: ${clean.stderr}）`,
      );

      // ② 残渣：插件目录合格（PLUGIN.yml 完整）但工厂不在本构建里
      const orphan = join(hd.homedir, 'projection');
      mkdirSync(orphan, { recursive: true });
      writeFileSync(
        join(orphan, 'PLUGIN.yml'),
        'plugin_provider: projection\nplugin_name: projection\n',
      );

      const residue = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        stdinText: START_AND_EXIT,
      });
      assertEq(residue.code, 0, `残渣启动应正常退出（stderr: ${residue.stderr.slice(-800)}）`);
      assert(
        !residue.stderr.includes('[composite WARN]'),
        `孤儿插件目录不许喊 WARN（实际 stderr: ${residue.stderr}）`,
      );
      assert(
        !residue.stderr.includes('projection'),
        `孤儿目录名不应出现在默认级别的启动日志里（降级到 debug，实际 stderr: ${residue.stderr}）`,
      );
    } finally {
      cleanupHomedir(hd);
    }
  },
);
