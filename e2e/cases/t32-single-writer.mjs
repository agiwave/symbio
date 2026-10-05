import './_selfrun.mjs';
// T32 单写者（[plan/11 批 0](../../docs/plan/11-多执行器与多主体加固实施方案.md) 判据）：
// **两个进程写同一会话** ⇒ 第二个拿到 `NotTheWriter`（可判别的信号），
// 网格不重号、盘上零丢失。
//
// ## 本用例钉的是什么
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 单写者是**跨进程**的（不是进程内 `RwLock`） | 两个真进程、同一份 `v2-events.wal` |
// | 第二个写者拿到的是**信号**而不是静默坏格 | 乙进程的退出码 + 输出里的 `NotTheWriter` |
// | 盘上零丢失、seq 不重号 | 甲的每一格都在，`seq` 连续 `0..n-1`、`event_id` 无重复 |
// | 被拒的写**没有**落盘 | 事实源里恰好一格 `user.message`，且是甲那句话 |
//
// 单测（`wal.test.rs::second_writer_gets_not_the_writer_and_nothing_is_lost`）证的是
// **同进程**两个 `WalStore` 交错；进程内的 `RwLock` 也能挡住那种情形。跨进程才是本批
// 真正补上的缺口——那个缺口在单测里**看不见**，因为单测跑在一个进程里。
//
// ## 为什么甲必须是个**慢轮**
//
// 判据要成立，两个写者的写窗口必须真的重叠。若甲在乙启动前就跑完，乙会正常接手
// （这是合法的顺序轮），用例就变成在断言「不重叠时不会冲突」——什么都没证。
// 因此甲的场景用 `chunkDelayMs` 把一轮拉长到 ~10 秒：甲在 `execute_turn` 开头就
// `open` 并持住写者令牌（[`v2_exec`](../../symbio/src/plugins/session/v2_exec.rs)
// 的 store 是整轮的局部量），足够乙从冷启动走到自己的 `append`。
//
// 余量怎么定的：实测乙从 `spawn` 到失败退出约 3 秒（本用例整轮 11 秒、其中甲的
// 窗口 8 秒），10 秒窗口留了 3 倍余量。**这个余量是本用例唯一的时序假设**——
// 若哪天 CLI 冷启动变慢到 10 秒，本用例会红，且红的原因就是那一句：把窗口调大，
// 不要改成「不重叠也算过」（那等于把用例退化成什么都没证）。
//
// ## 为什么乙的断言是「失败」而不是「跳过」
//
// 批 0 的语义就是**单写者**：第二个写者不该静默地写出重复 seq，而该拿到
// `AppendError::NotTheWriter`。这个错误经 `v2_exec` 映射成
// `PluginError::InternalError("v2 运行器落格失败：NotTheWriter")` ⇒ 乙这一轮失败、
// 退出码非零。**失败即正确**：信号可达本身就是本批要证的事。
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  cliExe,
  readFileSyncSafe,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 本用例的会话 id（两个进程共用同一个） */
const SID = 'e2e-t32';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

const TEXT_A = '甲进程的第一句话';
const TEXT_B = '乙进程的第二句话';

/**
 * 后台起一个 one-shot CLI。
 *
 * `runCli` 是 `spawnSync`——它会阻塞本进程，两个轮次就**不可能重叠**。本用例要的
 * 正是重叠，所以这里用 `spawn` 自己起，并把退出码包成 Promise。
 */
function spawnCli({ homedir, workdir, provider, session, message }) {
  const argv = [
    '--homedir', homedir,
    '--workdir', workdir,
    '--mode', 'auto',
    '--provider', provider,
    '--session', session,
    '-m', message,
  ];
  const child = spawn(cliExe(), argv, {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, E2E_STDIO: '1' },
  });
  const out = { stdout: '', stderr: '', code: null };
  child.stdout.on('data', (d) => { out.stdout += d; });
  child.stderr.on('data', (d) => { out.stderr += d; });
  out.done = new Promise((resolve) => {
    child.on('exit', (code) => {
      out.code = code;
      resolve(code);
    });
  });
  return out;
}

export default defineCase(
  'T32 单写者：两个进程写同一会话 ⇒ 第二个被写者令牌拒写（NotTheWriter），网格不重号、零丢失',
  async () => {
    const llm = await new MockLlm([
      // 甲：慢轮——把写者令牌按住 ~10 秒（2 个 SSE 事件 × 5000ms）。
      { id: 'slow-a', match: TEXT_A, chunks: ['甲的答复'], chunkDelayMs: 5000 },
      // 乙：常规快轮。若它**没有**被拒，就会落第二格 `user.message` ⇒ 断言直接抓住。
      { id: 'b-reply', match: TEXT_B, content: '乙的答复' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      // `full` 档 = 轮次走 `v2_exec`：store 在**轮首** open、持整轮。
      // `bridge` 档要等收束转写才落格，写窗口太短，测不出重叠。
      pluginConfigs: { session: { ...DIALOG_FACE_OFF, v2_mode: 'full' } },
    });
    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const walPath = join(hd.homedir, 'session', SID, V2_WAL);

    try {
      // ── 甲先上：WAL 出现即「甲已过 open 且落了用户格」⇒ 写者令牌在甲手上 ──
      const a = spawnCli({ ...base, session: SID, message: TEXT_A });
      await waitFor(() => existsSync(walPath), {
        what: '甲的事实源出现（= 甲已持写者令牌）',
        timeoutMs: 60_000,
      });

      // ── 乙上：同一会话、另一个进程。此刻令牌在甲手里 ⇒ 乙应被拒 ──
      const b = spawnCli({ ...base, session: SID, message: TEXT_B });
      const bCode = await b.done;
      const aCode = await a.done;

      // ── 证据 ①：甲这一轮正常收束（令牌没被乙搅坏）────────────────────────
      assertEq(aCode, 0, `甲的退出码（stderr: ${a.stderr.slice(0, 800)}）`);
      assert(
        a.stdout.includes('甲的答复'),
        `甲应拿到自己的正文（实际 stdout: ${JSON.stringify(a.stdout.slice(0, 300))}）`,
      );

      // ── 证据 ②：乙拿到的是**信号**，不是静默写坏 ──────────────────────────
      assert(bCode !== 0, `乙应因写者令牌被拒而失败（实际退出码 ${bCode}）`);
      const bOut = `${b.stderr}\n${b.stdout}`;
      assert(
        bOut.includes('NotTheWriter'),
        `乙的失败应点名 NotTheWriter（可判别的信号）——实际输出: ${JSON.stringify(bOut.slice(0, 800))}`,
      );

      // ── 证据 ③：盘上零丢失、不重号 ────────────────────────────────────────
      const raw = readFileSyncSafe(walPath);
      assert(raw.length > 0, `事实源应非空：${walPath}`);
      const events = raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));

      // seq 连续 0..n-1：并发写坏的第一症状就是重号或跳号。
      assertEq(
        events.map((e) => e.seq),
        events.map((_, i) => i),
        `seq 必须连续无重号（实际: ${JSON.stringify(events.map((e) => e.seq))}）`,
      );
      // event_id 唯一：`u-{turn}` 这类计数派生的 id 在并发下会撞，撞了就看得见。
      const ids = events.map((e) => e.event_id);
      assertEq(
        new Set(ids).size,
        ids.length,
        `event_id 必须唯一（实际: ${JSON.stringify(ids)}）`,
      );

      // ── 证据 ④：被拒的写**没有**落盘（乙那句话一个字都没进事实源）──────────
      const users = events.filter((e) => e.kind === 'user.message');
      assertEq(users.length, 1, `只该有甲那一格用户发言（实际 ${users.length}）`);
      assertEq(
        users[0].payload?.text,
        TEXT_A,
        `唯一那格用户发言应是甲的（实际: ${JSON.stringify(users[0].payload?.text)}）`,
      );
      assert(
        !raw.includes(TEXT_B),
        '乙的正文一个字都不该进事实源——被拒的写必须整条不落',
      );

      // 甲的轮次真的收束了（不是「只剩用户格」的半个轮）。
      assert(
        events.some((e) => e.kind === 'chat.assistant.final'),
        `甲应落收束格（实际 kinds: ${JSON.stringify(events.map((e) => e.kind))}）`,
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28/t30/t31 同一约定：Windows 上带活子进程
      // 退出会撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'one-shot CLI / mock-llm 子进程退出',
      });
    }
  },
);
