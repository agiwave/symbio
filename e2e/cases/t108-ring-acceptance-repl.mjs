import './_selfrun.mjs';
// T108 义务环符合性 **第二条**真实行使路径：REPL 单进程、多行 stdin 的真实会话。
//
// ## 为什么需要第二条
//
// `scripts/obligation-ring-accept.mjs`（J1…J9）的输入是「一次真实 CLI 跑出来的 WAL +
// 日志」。T107 之前它是唯一的真实行使路径——一条路径行使九条判据，任何一条被悄悄
// 绕过都没有第二条路径发现。本用例补第二条：**同一个判定器、不同的执行形状**。
//
//   MockLlm ──HTTP──> CLI（--repl，一次进程、两行 stdin）──> <homedir>/session/<sid>/v2-events.wal
//                                                              + CLI stderr（`[LLM] 请求发起`）
//                                                   ↓
//                                       node scripts/obligation-ring-accept.mjs --strict
//
// 与 T107 的形状差：T107 是**两次独立 CLI 进程**（`-m`，每轮一个进程）；本用例是
// **一次 REPL 进程吃两行 stdin**——两条 `user.message` 落在同一个 WAL 且同一个进程
// 写出，J2（scope 开关）与 J5（溯源链）判的是**进程内连续**的序列，不是两个进程
// 各自的开头。执行面也不同：REPL 的输入经 `runCli` 的提示符同步关闭喂入
// （`e2e/helpers.mjs::runCli`），这条路径同时就是该喂入方式的真实回归——
// `t4-multi-turn` 并发 7 下的 stdin 竞态由本用例与它共同钉住。
//
// 判据仍是**判定器的退出码**，不是本文件里的手写断言；J1…J9 的读法见
// `scripts/obligation-ring-accept.mjs` 文件头与 T107 的文件头（那里逐条展开过，本
// 文件不重复）。对话面**必须默认开启**：J7 的 `classify/decide` 与 `compose/wording`
// 两条产线是它的模型往返，关掉 J7 就只剩「0 观测 ⇔ 0 三格」的平凡真。
//
// ## 红了先看什么
//
// 与 T107 同一口径：失败信息里附了判定器的**原始输出**，它是判据的唯一读法。
// `J7 ... 观测 N 次、入账三格 M 组` ⇒ 有模型调用绕开了债环；`J8` 差额 ⇒ 日志与 WAL
// 的跨度对不上（本用例单进程，stderr 即整段日志）；`J2/J5` ⇒ 溯源 / 输入闭包破了。
// 若失败形态是「stdout 只剩空 prompt、stderr 以『再见。』收尾」，那不是判定器的红，
// 是 REPL 没吃到第二轮输入——查 `runCli` 的 stdin 关闭时机，别去放宽判据。
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, '..', '..');
const JUDGE = join(REPO_ROOT, 'scripts', 'obligation-ring-accept.mjs');

/** 本用例的会话 id（REPL 两轮共用一次进程）。 */
const SID = 'e2e-t108';

/** 第一行：需要干活 ⇒ 分类一次 + 主循环一次模型往返。 */
const MSG_WORK = '把这件事记下来';
/** 第二行：答案已在对话里 ⇒ 分类一次 + 措辞产线一次（`from_context`）。 */
const MSG_DIRECT = '刚才那件事是什么';

export default defineCase(
  'T108 义务环符合性第二条路径：REPL 真实会话的 WAL 与模型 HTTP 调用对账（J1…J9 --strict）',
  async () => {
    // 场景表与 T107 同形：两轮各一次分类，第一轮走主循环、第二轮走措辞产线。
    const llm = await new MockLlm([
      { id: 'cls-work', match: MSG_WORK, content: 'work', once: true },
      { id: 'final-work', content: '已经记下来了。' },
      { id: 'cls-direct', match: MSG_DIRECT, content: 'direct', once: true },
      { id: 'wording-direct', content: '刚才那件事是：把它记下来。' },
    ]).start();

    // 不套 `DIALOG_FACE_OFF`：对话面是 J7 两条产线的来源（理由见文件头）。
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    });

    try {
      // 一次 REPL 进程吃两行 stdin：两条 user.message 出自同一进程的连续序列。
      const r = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: SID,
        stdinText: `${MSG_WORK}\n${MSG_DIRECT}\n`,
      });
      assertEq(r.code, 0, `REPL CLI 退出码（stderr 末 800 字: ${r.stderr.slice(-800)}）`);
      // 第一轮的回复文本可以断言（`final-work` 是无 `match` 的兜底场景，任何
      // 主循环请求都命中它）；**措辞轮的文本不断言**——mock-llm 的选场语义是
      // 无 `match` 场景只在无位置命中时兜底且取数组第一个（`choose()` 里
      // `else if (best === null) best = s`），`wording-direct` 永远排在
      // `final-work` 之后选不中，断言它的文本等于假设了 mock 的内部行为。
      // 措辞产线是否真的被行使，交给判定器 J7（观测 ⇔ 三格双向对账）与下方
      // `http >= 4` 的读数自证——判据本体是判定器，不是手写文本断言（与 T107
      // 同口径，T107 从不断言措辞文本）。
      assert(
        r.stdout.includes('已经记下来了。'),
        `至少一轮有答复产出（stdout 末 400 字: ${JSON.stringify(r.stdout.slice(-400))}）`,
      );

      // ── 事实层先自证：这次会话确实跑出了可判的形状 ─────────────────────
      const walText = readFileSyncSafe(join(hd.homedir, 'session', SID, 'v2-events.wal'));
      const events = walText
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line));
      assert(events.length > 0, '会话 WAL 为空：CLI 没有落下任何事实');
      assertEq(
        events.filter((e) => e.kind === 'user.message').length,
        2,
        `两行 stdin 应落两条 user.message（实际事实：${events.length} 格）`,
      );
      assert(
        events.some((e) => e.kind === 'chat.assistant.final' || e.kind === 'chat.assistant.fallback'),
        '两轮跑完应至少有一条收束格（final | fallback）',
      );

      // ── CLI 日志 = J8 / J9 的账实读数（单进程 ⇒ stderr 即整段日志）──────
      const cliLog = join(hd.homedir, 'cli.log');
      writeFileSync(cliLog, r.stderr);

      // ── 判据本体：真实 REPL 会话产物交给判定器，`--strict` 不许降级 ─────
      const j = spawnSync(
        process.execPath,
        [
          JUDGE,
          `--homedir=${hd.homedir}`,
          `--session=${SID}`,
          `--cli-log=${cliLog}`,
          '--strict',
        ],
        {
          cwd: REPO_ROOT,
          encoding: 'utf8',
          timeout: 60_000,
          maxBuffer: 16 * 1024 * 1024,
          env: { ...process.env, NO_COLOR: '1' },
        },
      );
      const out = `${j.stdout ?? ''}${j.stderr ?? ''}`.trim();
      const tail = out.split('\n').slice(-20).join('\n');

      assertEq(j.status, 0, `判定器应以 0 退出（实际 ${j.status}）。原始输出（末 20 行）：\n${tail}`);
      assert(
        /J1…J7（事实层口径）全项通过/.test(out),
        `判定器应报 J1…J7 全项通过。原始输出（末 20 行）：\n${tail}`,
      );
      assert(out.includes('J8 通过'), `判定器应报 J8 通过（账实相符）。原始输出（末 20 行）：\n${tail}`);
      assert(out.includes('J9 通过'), `判定器应报 J9 通过。原始输出（末 20 行）：\n${tail}`);
      assert(
        out.includes('义务环实测判定：全项通过'),
        `判定器总体结论应为全项通过。原始输出（末 20 行）：\n${tail}`,
      );
      assert(!out.includes('✗'), `判定器输出里不得有失效项。原始输出（末 20 行）：\n${tail}`);

      // ── 读数自证：判的不是一个「什么模型都没打」的平凡会话 ──────────────
      const http = Number((out.match(/整轮：HTTP (\d+)/) ?? [])[1] ?? NaN);
      assert(Number.isFinite(http) && http >= 4, `两轮对话面会话至少应有 4 次模型 HTTP 往返（实际 ${out.includes('整轮：HTTP') ? http : '缺失'}）`);
    } finally {
      if (process.env.E2E_KEEP) console.log('DEBUG homedir:', hd.homedir);
      else {
        llm.stop();
        cleanupHomedir(hd);
        await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
          what: 'REPL CLI / mock-llm 子进程退出',
        });
      }
    }
  },
);
