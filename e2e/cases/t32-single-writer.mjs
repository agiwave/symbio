import './_selfrun.mjs';
// T32 单写者（[plan/11 批 0](../../docs/003-design/011-Agent目录规范.md "PL-009-多执行器与多主体加固实施方案") 判据）：
// **两个进程写同一会话** ⇒ 第二个拿到 `NotTheWriter`（可判别的信号），
// 历史里只有甲的问答。
//
// ## 本用例钉的是什么
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 单写者是**跨进程**的（不是进程内 `RwLock`） | 两个真进程、同一会话 |
// | 第二个写者拿到的是**信号**而不是静默坏格 | 乙进程的退出码 + 输出里的 `NotTheWriter` |
// | 甲这一轮完整收束 | 甲的退出码 + stdout 里的答复 |
// | 被拒的写**没有**进历史 | 两进程退出后起长活 CLI 经读出口读回：只有甲的问答，乙的话不在 |
//
// 单测（`wal.test.rs::second_writer_gets_not_the_writer_and_nothing_is_lost`）证的是
// **同进程**两个 `WalStore` 交错；进程内的 `RwLock` 也能挡住那种情形。跨进程才是本批
// 真正补上的缺口——那个缺口在单测里**看不见**，因为单测跑在一个进程里。存储层不变量
// （seq 连续 / 事件号唯一 / 被拒写不落盘）由该单测系守住，e2e 只判用户可见面。
//
// ## 为什么甲必须是个**慢轮**
//
// 判据要成立，两个写者的写窗口必须真的重叠。若甲在乙启动前就跑完，乙会正常接手
// （这是合法的顺序轮），用例就变成在断言「不重叠时不会冲突」——什么都没证。
// 因此甲的场景用 `chunkDelayMs` 把一轮拉长到 ~10 秒：甲在 `turn_exec` 起步就
// `open` 并持住写者令牌（[`turn_exec`](../../crates/symbio-plugin-loop/src/turn_exec.rs)
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
// `AppendError::NotTheWriter`。这个错误经 `turn_exec` 映射成
// `PluginError::InternalError("v2 运行器落格失败：NotTheWriter")` ⇒ 乙这一轮失败、
// 退出码非零。**失败即正确**：信号可达本身就是本批要证的事。
import { spawn } from 'node:child_process';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  cliExe,
  startLongLivedCli,
  readSessionMessages,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
} from '../helpers.mjs';

/** 本用例的会话 id（两个进程共用同一个） */
const SID = 'e2e-t32';

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
  'T32 单写者：两个进程写同一会话 ⇒ 第二个被写者令牌拒写（NotTheWriter），历史里只有甲的问答',
  async () => {
    const llm = await new MockLlm([
      // 甲：慢轮——把写者令牌按住 ~10 秒（2 个 SSE 事件 × 5000ms）。
      { id: 'slow-a', match: TEXT_A, chunks: ['甲的答复'], chunkDelayMs: 5000 },
      // 乙：常规快轮。若它**没有**被拒，就会落第二条用户发言 ⇒ 断言直接抓住。
      { id: 'b-reply', match: TEXT_B, content: '乙的答复' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 读回要走读出口（gateway `invoke` → `vdfs/read`），入站口必须开着
        // （与 T28/T31 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 轮次走 `turn_exec`：store 在**轮首** open、持整轮。
        session: { ...DIALOG_FACE_OFF },
      },
    });
    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };

    try {
      // ── 甲先上：首条 LLM 请求到达即「甲已开轮并持住写者令牌」──────────────
      // （判据是用户可见面：mock 收到的第一个请求体属于甲这一轮。）
      const a = spawnCli({ ...base, session: SID, message: TEXT_A });
      await waitFor(async () => (await llm.requests()).length >= 1, {
        what: '甲的首个 LLM 请求（= 甲已持写者令牌）',
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

      // ── 证据 ③：两进程都退出后，经**生产读出口**读回历史——被拒的写没进去 ──
      // 起一个长活 CLI（同 homedir 同会话）读会话叶子（`vdfs/read`，与前端加载
      // 历史同一出口）：只有甲的问答在；乙的那句话一个字都不在。
      // 存储层不变量（seq 连续 / 事件号唯一）由 `wal.test.rs` 的单测系守住。
      const api = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await api.waitGatewayReady(20_000, 'T32 读回进程');
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T32');
        const userTexts = msgs
          .filter((m) => msgRoleOf(m) === 'user')
          .map((m) => (typeof m.content === 'string' ? m.content : ''));
        assertEq(
          userTexts.filter((t) => t.includes(TEXT_A)).length,
          1,
          `历史里应恰有甲那一句用户发言（实际: ${JSON.stringify(userTexts)}）`,
        );
        assert(
          userTexts.every((t) => !t.includes(TEXT_B)),
          `乙的用户发言不得进历史（被拒的写整条不落；实际: ${JSON.stringify(userTexts)}）`,
        );
        const bodyTexts = msgs.map((m) => (typeof m.content === 'string' ? m.content : ''));
        assert(
          bodyTexts.some((t) => t.includes('甲的答复')),
          '甲的答复应在历史里（甲的轮次真的收束了，不是只剩用户发言的半个轮）',
        );
        assert(
          bodyTexts.every((t) => !t.includes(TEXT_B)),
          `乙的正文一个字都不该进历史（实际: ${JSON.stringify(bodyTexts)}）`,
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28/t30/t31 同一约定：Windows 上带活子进程
      // 退出会撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'one-shot CLI / 长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
