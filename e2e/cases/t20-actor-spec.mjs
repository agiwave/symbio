import './_selfrun.mjs';
// T20 Actor 行（B3）：真实会话跑一轮 → Actor 表含 reasoner 行 → 判定者行由**独立插件**
// 登记 → 停用该插件即退化（J2）且会话照常。
//
// ## 这条用例锁什么
//
// B3 交付两半：**core 的 actor 域**（单测覆盖四字段 / 覆盖语义 / 越层登记 A5）与
// **可选的 `actor` 插件**（本用例覆盖）。插件那半有三条性质，只有真跑一整轮才成立：
//
// | 性质 | 判据 | 为什么单测证明不了 |
// |---|---|---|
// | **reasoner 行真被登记** | 跑一轮真实对话后 `actor/list` 含 `session.reasoner` 行 | 登记发生在 `run_chat_loop` 入口——单测里没有真实主循环 |
// | **新 Actor 零改 chat_loop 接入** | 判定者行（`session.decider`）由 `actor` 插件登记，`actor/list` 看得到 | 这是"扩展不改架构"的验收，只有两个插件都装上、同一进程里跑才成立 |
// | **平凡值（J2）** | 停用 `actor` ⇒ 判定者行不登记、路由不可达，**会话照常**（vdfs/root + 真实对话仍可用） | 装配方行为，单测里没有装配方 |
//
// ## 为什么整轮对话必须在**同一个长驻进程**里发
//
// Actor 表是**进程级**的（`inventory` + `OnceLock`），登记行不落盘、不跨进程。
// 因此不能用"`runCli` 跑一轮（进程 A 退出）→ 再起长驻 CLI（进程 B）查表"——
// 进程 A 登记的 reasoner 行随它一起消失，进程 B 只会看到自己登记的（`actor` 插件
// 在 `build` 期登记，故 decider 行在）。
//
// 这恰好是一次**自然实验**：`session.reasoner` 只能由"真的跑过一轮"产生，
// 而 decider 行在 `build` 期就有——两者出现在同一张表里，**同时**证明了
// 「主循环入口登记」与「插件构造期登记」两条路径。
//
// ## 为什么用 reasoner 的 `principal` 作证据
//
// reasoner 行在**登记期** `principal` 是空占位符（登记发生在任何会话之前），
// 运行期才被真实 `session_id` 覆盖。e2e 里跑的是指定会话 `e2e-t20`，
// 因此**只要该行的 principal 非空且等于该会话 id**，就证明"运行期解析"确实发生过。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

function gatewayConfig(port) {
  return {
    inbound_enabled: true,
    inbound_protocol: 'http',
    inbound_bind: '127.0.0.1',
    inbound_port: port,
    inbound_token: '',
    inbound_readonly: false,
  };
}

function dataOf(resp, what) {
  assertEq(resp.status, 200, `${what} 应受理（${resp.status}）`);
  const d = resp.body?.data;
  assert(d != null, `${what} 应返回 Data 载荷（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
  return d;
}

/** 表名（与 `session/actors.rs` / `actor/plugin.rs` 的登记点一致） */
const NAME_REASONER = 'session.reasoner';
const NAME_DECIDER = 'session.decider';

/** 在 `actor/list` 的返回里按名字找一行 */
function rowOf(list, name) {
  const rows = list?.actors;
  assert(
    Array.isArray(rows),
    `actor/list 应返回 { actors: [...] }（实得 ${JSON.stringify(list)?.slice(0, 200)}）`,
  );
  return rows.find((r) => r.name === name);
}

export default defineCase(
  'T20 Actor 行：reasoner 登记（现行行为显式化）· 判定者由独立插件登记 · 停用即退化',
  async () => {
    // 一轮带工具调用的对话：真实走完 `run_chat_loop`（reasoner 行在入口登记）
    const llm = await new MockLlm([
      {
        id: 'write-note',
        match: '记一笔',
        toolCalls: [
          {
            id: 'call_note',
            name: 'vdfs_write',
            arguments: { path: 'note-t20.md', text: 'T20 Actor 行' },
          },
        ],
      },
      { id: 'after-tool', afterTool: true, content: '已记录。' },
      // 停用 actor 后的第二轮（会话 id 不同，匹配靠 afterTool 之后的兜底）
      { id: 'generic', content: '好的。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        // `actor` 目录存在 + provider 合格 ⇒ 装配方构造它（可选插件：
        // 不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里，只有目录在才挂载）。
        actor: { enabled: true },
      },
    });

    const SESSION = 'e2e-t20';
    let cli = null;
    try {
      // ① 起长驻 CLI（带 gateway + actor 插件）——**整轮对话都在这个进程里跑**
      cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SESSION,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      await cli.waitGatewayReady();

      // ② 跑一轮真实对话（REPL stdin）→ `run_chat_loop` 入口登记本层 reasoner 行
      cli.send('帮我记一笔');
      await waitFor(
        async () => {
          const msgs = readMessagesJson(hd.homedir, SESSION) ?? [];
          const settled = msgs.some((m) => m.type === 'turn' && ['completed', 'failed'].includes(m.status));
          return msgs.length > 0 && settled;
        },
        { what: '首轮对话收敛（turn 落库终态）', timeoutMs: 30_000 },
      );
      const msgs = readMessagesJson(hd.homedir, SESSION);
      assert(Array.isArray(msgs) && msgs.length > 0, '会话应落盘（有消息）');

      // ③ `actor/list`：两行都在 —— reasoner 由 session 登记、decider 由 actor 插件登记
      const list = dataOf(await cli.invoke('actor/list', {}), 'actor/list');

      const reasoner = rowOf(list, NAME_REASONER);
      assert(reasoner != null, `应含 reasoner 行（实得: ${JSON.stringify(list)}）`);
      assertEq(reasoner.pattern, 'reasoner', 'reasoner 行的 pattern');
      assertEq(reasoner.budget_ms, 0, 'reasoner 行的 budget_ms 应为 0（现行无墙钟上限）');
      assertEq(reasoner.scope, 'root', 'reasoner 行的 scope 应为 root（root 会话）');
      assertEq(
        reasoner.principal,
        SESSION,
        '运行期应用真实会话 id 覆盖登记期占位符',
      );

      const decider = rowOf(list, NAME_DECIDER);
      assert(
        decider != null,
        `应含 decider 行（由 actor 插件登记，实得: ${JSON.stringify(list)}）`,
      );
      assertEq(decider.pattern, 'decider', 'decider 行的 pattern');
      assertEq(decider.budget_ms, 80, 'decider 行的 budget_ms 应为 80（抢占判定者）');
      assertEq(decider.scope, 'root', 'decider 行的 scope 应为 root');
      assertEq(decider.principal, '', 'decider 行未被本轮解析——它以登记期占位符留在表里');

      // ④ 四字段齐备（键名与结构体一致——这是"表里真的是这些值"的证据）
      for (const key of ['name', 'principal', 'pattern', 'budget_ms', 'scope']) {
        assert(key in reasoner, `reasoner 行应含字段 ${key}`);
        assert(key in decider, `decider 行应含字段 ${key}`);
      }

      // ⑤ 停用 `actor` ⇒ 路由不可达（插件没被构造）
      cli.stop();
      cli = null;
      const { writeFileSync, readFileSync } = await import('node:fs');
      const { join } = await import('node:path');
      const ymlPath = join(hd.homedir, 'actor', 'PLUGIN.yml');
      writeFileSync(ymlPath, `${readFileSync(ymlPath, 'utf8')}plugin_enabled: false\n`);

      const cli2 = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'e2e-t20-b',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli2.waitGatewayReady();

        // 路由不可达
        const disabled = await cli2.invoke('actor/list', {});
        assert(disabled.status !== 200, `停用后 actor/list 应不可达（实得 ${disabled.status}）`);

        // 反证：其余系统照常
        const root = await cli2.invoke('vdfs/root', {});
        assertEq(root.status, 200, '停用 actor 不应影响 vdfs/root');

        // ⑥ **会话照常**：停用 actor 后再跑一轮真实对话，仍须跑通 —— reasoner 行由
        //    session 自己登记（与 actor 插件无关）；即便连 reasoner 行都不在，
        //    执行器也回落到内置默认行（J2 的机制基础）。
        cli2.send('随便说一句');
        await waitFor(
          async () => {
            const m = readMessagesJson(hd.homedir, 'e2e-t20-b') ?? [];
            const settled = m.some((x) => x.type === 'turn' && ['completed', 'failed'].includes(x.status));
            return m.length > 0 && settled;
          },
          { what: '停用 actor 后会话仍收敛', timeoutMs: 30_000 },
        );
        const msgs2 = readMessagesJson(hd.homedir, 'e2e-t20-b');
        assert(Array.isArray(msgs2) && msgs2.length > 0, '停用 actor 后会话仍应落盘');
      } finally {
        cli2.stop();
      }
    } finally {
      cli?.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
