import './_selfrun.mjs';
// T90 主动开口是一条**插件能力**：摘掉 `heartbeat` 的装配行（插件目录 `PLUGIN.yml` 的
// `plugin_enabled: false`）之后，这条能力在系统里不该留下任何痕迹，而系统照常运行。
//
// ## 本用例钉的是「插件能承载能力」不是假想
//
// 主动开口是三个代表能力里唯一今天什么都不缺的：`heartbeat` 已是独立 crate，扫描循环、
// 空闲判定、收件箱写入全在包内，`loop` / `session` 都不认识它。它的价值不是「再搬一次」，
// 而是把拆分模板冻下来——**一条能力摘掉之后，四件事同时成立**：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 工具面没有它 | 首次模型请求的 `tools`：启用侧有 `heartbeat`，停用侧没有，其余工具仍在 |
// | 不再产生主动事实 | 停用侧一个完整触发窗内，转写里不出现那条提示词（LLM 侧也没收到过） |
// | 系统完整运行 | 停用侧照常跑完普通轮次，窗口之后还能再跑一轮（不是在等一个坏掉的系统） |
// | 触发确实到过点 | 同窗的启用侧真的把提示词落进了转写——否则「停用侧没有」可能只是还没到点 |
//
// ## 为什么两侧必须**同时**跑、同一个会话 id
//
// 调度器的相位由会话 id 的确定性哈希决定（`phase_ms`，0–30s），tick 是 15s。两个会话
// 各用各的 id 时，「停用侧没触发」与「停用侧的相位恰好还没到」在读数上无法区分。
// 同一 id + 同时起步 ⇒ 两侧的到点时刻相同 ⇒ 一侧触发过之后，另一侧的窗口必然已经过去。
// 停用侧因此不需要猜一个超时，而是等「启用侧实测的触发时刻 + 一个 tick」。
//
// ## 停用走的是插件目录，不是源码
//
// `plugin_enabled` 是 `.symbio/<插件>/PLUGIN.yml` 上的运行期装配位（与本仓
// `plugin_manager` 的停用同一条路）。用例只翻这一个键，装配侧没有任何分支。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readSessionMessages,
  msgRoleOf,
} from '../helpers.mjs';

/** 两侧同用一个会话 id：相位（抖动）因此相同，触发时刻才可对照。 */
const SID = 'e2e-t90-heartbeat';

/** 普通一轮的用户发言：证明系统在能力缺席时照常工作。 */
const MSG = '开始工作（T90）';
const MSG_AGAIN = '继续（T90）';

/** 心跳提示词：它作为 user 消息落进转写，就是「主动开口发生过」的唯一读数。 */
const PROMPT = '自主推进（T90）';

/** 心跳间隔取 1s：到点只由 tick（15s）与相位（≤30s）决定，不再叠一个长间隔。 */
const INTERVAL = 1;

/** 启用侧等触发的上限：15s tick + 30s 相位 + 余量。 */
const FIRE_TIMEOUT_MS = 90_000;

const gatewayConfig = (port) => ({
  inbound_enabled: true,
  inbound_protocol: 'http',
  inbound_bind: '127.0.0.1',
  inbound_port: port,
  inbound_token: '',
  inbound_readonly: false,
});

/**
 * 起一侧：自己的 mock LLM、homedir、长驻 CLI。
 *
 * `heartbeatEnabled: false` 只写一个键——`heartbeat/PLUGIN.yml` 的 `plugin_enabled`。
 */
async function bootSide(label, { heartbeatEnabled }) {
  const llm = await new MockLlm([{ id: 'rest', content: '收到。' }]).start();
  const gatewayPort = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: {
      gateway: gatewayConfig(gatewayPort),
      session: { ...DIALOG_FACE_OFF },
      ...(heartbeatEnabled ? {} : { heartbeat: { plugin_enabled: false } }),
    },
  });
  const cli = startLongLivedCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    // 中立会话 id：REPL 自己那一轮的 `updated_at` 会抹掉被测会话的空闲基线。
    session: `e2e-t90-live-${label}`,
    provider: PROVIDER_ID,
    gatewayPort,
  });

  const root = async () => {
    const resp = await cli.invoke('vdfs/root', {});
    const path = resp.body?.data?.path;
    assert(typeof path === 'string' && path.length > 0, 'vdfs/root 应返回根地址');
    return path.replace(/\/+$/, '');
  };

  const side = {
    label,
    llm,
    cli,
    hd,
    /** 建会话时就带上心跳配置：停用侧也**有**这份数据，缺的是执行它的插件。 */
    async createSession() {
      const addr = `${await root()}/session/${SID}`;
      const created = await cli.invoke('vdfs/write', {
        path: addr,
        create: true,
        text: JSON.stringify({
          metadata: {
            workdir: hd.workdir,
            mode: 'auto',
            risk_level: 'medium',
            heartbeat: {
              enabled: true,
              interval_seconds: INTERVAL,
              prompt: PROMPT,
              include_history: true,
            },
          },
        }),
      });
      assertEq(created.status, 200, `创建会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);
      return addr;
    },
    async say(addr, text) {
      const w = await cli.invoke('vdfs/write', { path: `${addr}/inbox`, text });
      assertEq(w.status, 200, `写收件箱（${JSON.stringify(w.body)?.slice(0, 300)}）`);
    },
    /** 转写里的 user 发言正文（读出口 = 前端加载历史同一出口）。 */
    async userTexts() {
      const msgs = (await readSessionMessages(cli, SID)) ?? [];
      return msgs.filter((m) => msgRoleOf(m) === 'user').map((m) => String(m.content ?? ''));
    },
    async assistantCount() {
      const msgs = (await readSessionMessages(cli, SID)) ?? [];
      return msgs.filter((m) => msgRoleOf(m) === 'assistant').length;
    },
    /** 最后一次 user 消息是 `marker` 的那次请求里的工具名（工具面读数）。 */
    async toolsOfTurn(marker) {
      const reqs = await llm.requests();
      for (const r of reqs) {
        const users = (r.body.messages ?? []).filter((m) => m.role === 'user');
        if (String(users.at(-1)?.content ?? '').includes(marker)) {
          return (r.body.tools ?? []).map((t) => t.function?.name).filter(Boolean);
        }
      }
      return null;
    },
    stop() {
      cli.stop();
      llm.stop();
    },
  };
  return side;
}

/** 超时现场：两侧各自的转写、请求数、stderr 尾部——三种「没触发」长得一样。 */
async function diagnose(enabled, disabled) {
  const rows = [];
  for (const s of [enabled, disabled]) {
    const texts = await s.userTexts().catch((e) => `读取失败: ${e?.message ?? e}`);
    const reqs = await s.llm.requests().then((r) => r.length).catch(() => -1);
    const hbLines =
      s.cli.out.stderr
        .split(/\r?\n/)
        .filter((l) => /Heartbeat/i.test(l))
        .slice(-4)
        .join('\n') || '（无 Heartbeat 行）';
    rows.push(
      `  ${s.label}｜user 发言=${JSON.stringify(texts)}｜LLM 请求=${reqs}`,
      `  ${s.label} stderr（含 Heartbeat 的末 4 条）：`,
      hbLines,
      `  ${s.label} stderr 末 10 行：`,
      s.cli.out.stderr.split(/\r?\n/).slice(-10).join('\n') || '（空）',
    );
  }
  return ['', '--- T90 现场 ---', ...rows].join('\n');
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export default defineCase(
  'T90 主动开口是插件能力：停用 heartbeat 后工具面消失、一个触发窗内不产生提示词事实，系统完整运行',
  async () => {
    const enabled = await bootSide('on', { heartbeatEnabled: true });
    const disabled = await bootSide('off', { heartbeatEnabled: false });
    try {
      await Promise.all([
        enabled.cli.waitGatewayReady(20_000, 'T90 启用侧'),
        disabled.cli.waitGatewayReady(20_000, 'T90 停用侧'),
      ]);

      // ── 建会话（两侧都带同一份心跳配置），各跑一轮普通对话 ─────────────
      const addrOn = await enabled.createSession();
      const addrOff = await disabled.createSession();
      await Promise.all([enabled.say(addrOn, MSG), disabled.say(addrOff, MSG)]);

      // 普通轮真的跑完：两侧转写里各有一条 assistant 回复。
      for (const s of [enabled, disabled]) {
        await waitFor(async () => (await s.assistantCount()) >= 1, {
          what: `${s.label} 的普通轮跑完`,
          timeoutMs: 30_000,
        });
      }

      // 空闲计时起点：两侧普通轮都结束之后。相位相同，到点时刻只差这段起点。
      const startedAt = Date.now();

      // ── 工具面：这是「能力在不在」最直接的一条读数 ─────────────────────
      const onTools = await enabled.toolsOfTurn(MSG);
      const offTools = await disabled.toolsOfTurn(MSG);
      assert(onTools !== null, '启用侧应留下普通轮的请求记录（工具面读数）');
      assert(offTools !== null, '停用侧应留下普通轮的请求记录（工具面读数）');
      assert(
        onTools.includes('heartbeat'),
        `启用 heartbeat 时工具面应含 heartbeat（实际: ${onTools.join(',')}）`,
      );
      assert(
        !offTools.includes('heartbeat'),
        `停用 heartbeat 后工具面不该含 heartbeat（实际: ${offTools.join(',')}）`,
      );
      // 负面读数要有下限：工具面整体变空会让「不含」白绿，而那意味着别的插件也丢了。
      assert(offTools.length > 0, '停用 heartbeat 只该拿掉它自己的工具，其余插件的能力应仍在');

      // ── 启用侧：等到心跳真的落进转写（「到点」的实测时刻）──────────────
      try {
        await waitFor(async () => (await enabled.userTexts()).some((t) => t.includes(PROMPT)), {
          what: '启用侧的心跳提示词落进转写（到点触发）',
          timeoutMs: FIRE_TIMEOUT_MS,
        });
      } catch (e) {
        throw new Error(`${e.message}${await diagnose(enabled, disabled)}`);
      }
      const firedAt = Date.now();

      // ── 停用侧：等过「同窗 + 余量」再断言什么都没有 ─────────────────────
      //
      // 两侧同 id、同时起步 ⇒ 停用侧的到点时刻与启用侧相同。这里不猜超时，而是用
      // 启用侧**实测**的等待时间加一点余量当窗口：窗口之内若有能力，它必然已经触发过
      // （启用侧就是在这个窗口里触发的，两侧唯一的差别是有没有那个插件）。
      await sleep(Math.max(0, firedAt - startedAt) + 5_000);

      const offTexts = await disabled.userTexts();
      assert(
        !offTexts.some((t) => t.includes(PROMPT)),
        `停用 heartbeat 后一个完整触发窗内不得产生提示词事实（实际 user 发言: ${JSON.stringify(offTexts)}）`,
      );
      // 第二个读数口：LLM 侧也从没收到过那条提示词——转写没写不等于没发出去。
      const offSawPrompt = (await disabled.llm.requests()).some((r) =>
        JSON.stringify(r.body.messages ?? []).includes(PROMPT),
      );
      assert(!offSawPrompt, '停用 heartbeat 后模型侧也不该收到提示词（转写与请求两处都要空）');

      // ── 系统完整运行：窗口之后停用侧还能照常跑一轮 ─────────────────────
      const before = await disabled.assistantCount();
      await disabled.say(addrOff, MSG_AGAIN);
      await waitFor(async () => (await disabled.assistantCount()) > before, {
        what: '停用侧在触发窗之后仍能跑完一轮普通对话（系统完整运行）',
        timeoutMs: 30_000,
      });
      assertEq(disabled.cli.proc.exitCode, null, '停用侧进程在整条用例里应始终在跑（不是崩溃后被判绿）');

      // ── 反向对照：停用侧确实**有**心跳数据，缺的是执行它的插件 ─────────
      //
      // 上面三条读数合起来排除了「没配置所以没触发」：配置在、插件不在 ⇒ 能力不在。
      const stat = await disabled.cli.invoke('vdfs/stat', { path: addrOff });
      const hb = stat.body?.data?.metadata?.heartbeat;
      assertEq(stat.status, 200, `vdfs/stat 应受理（${JSON.stringify(stat.body)?.slice(0, 200)}）`);
      assertEq(hb?.prompt, PROMPT, '停用侧会话里的心跳配置必须在（缺的只是执行它的插件）');
      assertEq(hb?.enabled, true, '停用侧的心跳配置必须是启用态（否则「没触发」另有原因）');
    } finally {
      enabled.stop();
      disabled.stop();
      cleanupHomedir(enabled.hd);
      cleanupHomedir(disabled.hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
