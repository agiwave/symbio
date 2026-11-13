import './_selfrun.mjs';
// T107 义务环符合性 **运行期** 半边：一次真实 CLI 会话的 WAL 与模型 HTTP 调用对账。
//
// ## 本用例钉的是什么
//
// `scripts/obligation-ring-accept.mjs`（J1…J9）是义务环符合性的 CLI 实测判定器，
// 但它的输入是「一次真实 CLI 跑出来的 WAL + 日志」——门内此前**没有任何地方产生
// 这样的输入**，于是它只能在脚本头声明 `// @gate off`（声明的意思是「本体进不了门，
// 进门的只有它的回归测试」，测的是读法，不是判据对一次真实运行成立）。
//
// 本用例补的就是这个缺口（DE-051 / SP-033 TC-4 的运行期半边）：
//
//   MockLlm ──HTTP──> CLI 一轮 / 两轮 ──> <homedir>/session/<sid>/v2-events.wal
//                                              + CLI stderr（`[LLM] 请求发起`）
//                                    ↓
//                        node scripts/obligation-ring-accept.mjs --strict
//
// 判据是**判定器的退出码**，不是本文件里的任何手写断言：
//   - J1 按 `(port,id,attempt)` 分组齐整（唯一 request / ≥1 sent / 恰一个终态或 retry）
//   - J2 request 落在已开且未关的 scope 里（前面有 user.message）
//   - J3 最早的 sent 严格早于终态
//   - J4 收束格（final | fallback）reason 非空
//   - J5 produced_by 指向已存在且更早的 seq
//   - J6 孤名 = 0（`ring:` 身份里的端口都在声明集里）
//   - J7 模型往返「观测 ⇔ 三格」双向对账（classify/decide、compose/wording）
//   - J8 真实 HTTP 调用数 == 模型端口（model / model.assist）的 sent 数
//   - J9 压缩产物 ⇔ 摘要往返（本会话无压缩 ⇒ 0/0 平凡成立；有压缩时是 T8 的读数）
//
// 判据的读法锚在 `event_id` 的 `ring:{port}:{id}:{attempt}:{kind}[:{shard}]` 上，
// 不是载荷——这是 2026-11 重锚之后的形状（见 `scripts/obligation-ring-accept.mjs` 文件头）。
//
// ## 为什么是「两轮 + 默认对话面」
//
// 一轮只能证单次往返；两轮让 J2/J5 跨轮成立（第二轮的 request 不能挂到第一轮的
// 输入格上），也让端口往返组数 > 1。对话面**必须默认开启**（不能套 `DIALOG_FACE_OFF`）：
// J7 的 `classify/decide` 与 `compose/wording` 两条产线正是对话面的模型往返，关掉
// 它们，J7 就只剩「0 观测 ⇔ 0 三格」的平凡真——判据没被真正行使。
//
// 第二轮刻意走 `from_context`（`direct` 判决）：它同时行使 **classify**（判决那一次
// 调用）与 **compose**（措辞那一次调用）两条产线，J7 的两条模型路径都落到「有观测、
// 有三格」的正向对账上。
//
// 简单会话没有压缩 ⇒ J9 的「产物 0 / 摘要往返 0」平凡成立。要覆盖压缩的产物配对，
// 是 T8 / T11 的主题，不在这里重复。
//
// ## 红了先看什么
//
// 失败信息里附了判定器的**原始输出**。它是判据的唯一读法：
//   - `J7 ... 观测 N 次、入账三格 M 组` ⇒ 有一条模型调用绕开了债环（真实缺口，别放宽）；
//   - `J8 真实模型调用 X 次 ≠ 入账 ...` ⇒ 日志与 WAL 的跨度对不上（本用例的日志
//     跨两轮 CLI 进程，拼接顺序不影响计数）；
//   - `J2/J5 ...` ⇒ 事实层的溯源 / 输入闭包破了。
//
// ## 注意：不要为「让它过」而加 `--strict` 之外的白名单
//
// 判定器本体是判据；本用例只负责**喂给它一次真实运行**。任何一处判红都应作为
// 产品缺陷上报，而不是在这里挑软判据。
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

/** 本用例的会话 id（两轮共用，J2/J5 的跨轮判据才有对象）。 */
const SID = 'e2e-t107';

/** 第一轮：需要干活 ⇒ 分类一次 + 主循环一次模型往返。 */
const MSG_WORK = '把这件事记下来';
/** 第二轮：答案已在对话里 ⇒ 分类一次 + 措辞产线一次（`from_context`）。 */
const MSG_DIRECT = '刚才那件事是什么';

export default defineCase(
  'T107 义务环符合性：真实 CLI 会话的 WAL 与模型 HTTP 调用对账（J1…J9 --strict）',
  async () => {
    // 场景表按数组顺序匹配；`match` 是对**最后一条 user 消息**的子串匹配，
    // `once` 让「分类请求」与「同一条用户话触发的后续请求」共用同一句话而不打架
    // （与 T21 同一条编排理由）。
    const llm = await new MockLlm([
      // 第一轮分类：判「要干活」（一次）
      { id: 'cls-work', match: MSG_WORK, content: 'work', once: true },
      // 第一轮主循环：正常终答
      { id: 'final-work', content: '已经记下来了。' },
      // 第二轮分类：判「能直接答」（一次）
      { id: 'cls-direct', match: MSG_DIRECT, content: 'direct', once: true },
      // 第二轮措辞产线（compose/wording）：组织面向用户的那句话
      { id: 'wording-direct', content: '刚才那件事是：把它记下来。' },
    ]).start();

    // **不套 `DIALOG_FACE_OFF`**：对话面是 J7 两条产线的来源，见文件头。
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    });

    try {
      const first = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: MSG_WORK,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 800)}）`);

      const second = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: MSG_DIRECT,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(second.code, 0, `第二轮 CLI 退出码（stderr: ${second.stderr.slice(0, 800)}）`);

      // ── 事实层先自证：这次会话确实跑出了可判的形状 ─────────────────────
      //
      // 没有这层自证，一个「会话根本没建起来」的空跑也可能让判定器在别的地方
      // 失败得莫名其妙；而这里能直接说清是「没跑起来」还是「跑起来了但不合规」。
      const walText = readFileSyncSafe(join(hd.homedir, 'session', SID, 'v2-events.wal'));
      const events = walText
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line));
      assert(events.length > 0, '会话 WAL 为空：CLI 没有落下任何事实');
      assertEq(
        events.filter((e) => e.kind === 'user.message').length,
        2,
        `两轮应有两条 user.message（实际事实：${events.length} 格）`,
      );
      assert(
        events.some((e) => e.kind === 'chat.assistant.final' || e.kind === 'chat.assistant.fallback'),
        '两轮跑完应至少有一条收束格（final | fallback）',
      );

      // ── CLI 日志 = J8 / J9 的账实读数 ────────────────────────────────
      //
      // `[LLM] 请求发起` 是 INFO（`bound_provider.rs:142`），CLI 默认把插件日志写
      // stderr ⇒ 不带 `-v` 也有。日志跨度必须覆盖**整段会话**（两轮），所以拼接
      // 两个进程的 stderr。
      const cliLog = join(hd.homedir, 'cli.log');
      writeFileSync(cliLog, `${first.stderr}\n${second.stderr}`);

      // ── 判据本体：真实 CLI 产物交给判定器，`--strict` 不许降级 ──────────
      const r = spawnSync(
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
      const out = `${r.stdout ?? ''}${r.stderr ?? ''}`.trim();
      const tail = out.split('\n').slice(-20).join('\n');

      assertEq(r.status, 0, `判定器应以 0 退出（实际 ${r.status}）。原始输出（末 20 行）：\n${tail}`);
      // 判定器的成功形态（读它的输出格式确认过）：
      //   `✓ <sid>: J1…J7（事实层口径）全项通过（N 格 / M 笔端口往返）`
      //   `✓ 整轮：J8 通过` / `✓ 整轮：J9 通过（…）`
      //   `义务环实测判定：全项通过`
      // J1…J6 在通过时不逐条打码，只在那一行里以 `J1…J7` 整体出现——断言按这个
      // 事实写，而不是去猜一个不存在的逐行格式。
      assert(
        /J1…J7（事实层口径）全项通过/.test(out),
        `判定器应报 J1…J7 全项通过。原始输出（末 20 行）：\n${tail}`,
      );
      assert(out.includes('J8 通过'), `判定器应报 J8 通过（账实相符）。原始输出（末 20 行）：\n${tail}`);
      assert(out.includes('J9 通过'), `判定器应报 J9 通过（具名项不超编）。原始输出（末 20 行）：\n${tail}`);
      assert(
        out.includes('义务环实测判定：全项通过'),
        `判定器总体结论应为全项通过。原始输出（末 20 行）：\n${tail}`,
      );
      assert(!out.includes('✗'), `判定器输出里不得有失效项。原始输出（末 20 行）：\n${tail}`);

      // ── 读数自证：判的不是一个「什么模型都没打」的平凡会话 ──────────────
      const http = Number((out.match(/整轮：HTTP (\d+)/) ?? [])[1] ?? NaN);
      const sentModel = Number((out.match(/sent\(模型端口\) (\d+)/) ?? [])[1] ?? NaN);
      assert(Number.isFinite(http) && http > 0, `本次会话应真的打过模型（HTTP 计数：${out.includes('整轮：HTTP') ? http : '缺失'}）`);
      assert(
        Number.isFinite(sentModel) && sentModel > 0,
        `本次会话应有模型端口的 sent 事实（实际 sent(模型端口) 解析：${sentModel}）`,
      );
      // 两轮 = 分类 2 次 + 主循环 1 次 + 措辞 1 次 = 4 次往返；断言「相等」由 J8 负责，
      // 这里只钉住「不是一条被空跑的会话」。
      assert(http >= 4, `两轮对话面会话至少应有 4 次模型 HTTP 往返（实际 ${http}）`);
    } finally {
      if (process.env.E2E_KEEP) console.log('DEBUG homedir:', hd.homedir);
      else {
        llm.stop();
        cleanupHomedir(hd);
        await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
          what: 'CLI / mock-llm 子进程退出',
        });
      }
    }
  },
);
