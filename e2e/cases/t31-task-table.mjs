import './_selfrun.mjs';
// T31 任务表（S7 步 16–18，[04 §3.1 批⑨](../../docs/plan/04-工程落地.md)）：
// 模型调 `todo_write` 声明一张**带依赖的任务图** ⇒ 收束转写把声明落成 `task.*` 事件 ⇒
// 读出口 `session/stats` 的 `readyset` 列与请求视图的**调度段**都从同一份事实源算出来；
// 对事实源做手术，两者跟着变。
//
// ## 本用例钉的是什么
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 模型给的名字后端认得回来 | 第一次请求体里的 `tools[]` 清单 + 工具真的执行了 |
// | `depends_on` 真的穿过 schema → 存储 → 事件 | `v2-events.wal` 的 `task.opened` 载荷 |
// | 就绪集**不是常数**（真在按依赖算） | 手术前 `[t1, t3]` → 把 t2/t3 造成环后 `[t1]` |
// | 调度段真的进了**下一次请求** | mock-llm `/_requests` 回读第二次对话的请求体 |
// | 读侧闸与各列共用同一次判定 | `principal: agent:ghost` ⇒ `{ready: []}` + `has_wal: true` |
// | 断言真的在跑 | 手术后 `invariants` 从 0 条变非 0 条（含「依赖环」） |
//
// 单测各自只看得见一层：`v2_tasks.test.rs` 用假事件验状态推进与返工，
// `stats.test.rs` 验列的复算，`view.test.rs` 验调度段的插入位置。
// 没有任何单测**同时**看得见「模型给的名字」「落盘的依赖」「进请求的段落」
// 「出口读的列」四者指向同一张任务图——那正是端到端的增量。
//
// ## 三轮各自干什么
//
// 1. **声明**（`请登记任务`）：模型一次吐三条——`t1` 进行中（无依赖）、
//    `t2` 未开始（依赖 `t1`）、`t3` 未开始（无依赖）。此刻就绪的只有 `t1`/`t3`。
// 2. **下一轮**（`继续推进任务`）：请求体里必须出现【任务调度】段且点名 `t1、t3`；
//    反向：声明之前那两发请求里一个字都没有（`tool_rounds > 0` 也不再取段）。
// 3. **手术**：把 `t2`/`t3` 的 `depends_on` 互相指回去造一个环 ⇒
//    `readyset` 掉到 `[t1]`、`invariants` 报「依赖环」——两处都读同一份切片，
//    一处变一处不变就是常数。
//
// ## 为什么调度段要在**第二轮**才断言
//
// 调度段按 `tool_rounds == 0` 取一次（`chat_loop::inputs`），而 `readyset` 要等
// 第一轮**收束转写**落完格才有数据。首轮请求发出去的那一刻，任务还只是模型嘴里
// 的一段 JSON——把断言放在那里等于断言「还没有发生的事没发生」。
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  readFileSyncSafe,
  nextPort,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  textOf,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t31';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 声明这一轮的触发串（mock 场景 `match` 与用户正文共用同一个词） */
const DECLARE = '登记任务';

/** 任务图：`t2` 依赖 `t1`；`t1`/`t3` 无依赖 ⇒ 首轮就绪集 = `[t1, t3]`。 */
const TODOS = [
  { id: 't1', content: '分析架构', status: 'in_progress', priority: 'high' },
  { id: 't2', content: '落地实现', status: 'pending', priority: 'medium', depends_on: ['t1'] },
  { id: 't3', content: '写文档', status: 'pending', priority: 'low' },
];

/** 读一次事实源（缺文件 / 空文件都是**用例前提不成立**，直接报而不是当零值算）。 */
function readEvents(path) {
  const raw = readFileSyncSafe(path);
  assert(raw.length > 0, `v2 事实源应存在且非空：${path}`);
  return raw
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

export default defineCase(
  'T31 任务表：todo_write 声明入格 → readyset 列与调度段同源 → 成环后两处一起变',
  async () => {
    const llm = await new MockLlm([
      // 第一次请求：模型一次性声明整张任务图（`once` 保证只烧一次，
      // 否则每一轮非工具请求都会再来一次工具调用 ⇒ 无限工具循环）。
      {
        id: 'todo-declare',
        match: DECLARE,
        once: true,
        content: '我先把任务登记进清单。',
        toolCalls: [
          { id: 'call_1', name: 'todo_write', arguments: { todos: TODOS } },
        ],
      },
      // 工具结果回灌（bridge 档是 v1 消息形态 ⇒ 最后一条 role = tool）。
      { id: 'after-todo', afterTool: true, content: '任务清单已登记。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 任务表挂在**收束转写**（`v2_bridge::record`）上 ⇒ 与 t30 同款的 bridge 档。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'bridge' },
      },
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };

    try {
      // ── 第 1 轮：声明任务图（工具真的执行）────────────────────────────────
      const r1 = runCli({ ...base, session: SID, message: `请${DECLARE}` });
      assertEq(r1.code, 0, `第 1 轮退出码（stderr: ${r1.stderr.slice(0, 600)}）`);
      assert(
        r1.stdout.includes('任务清单已登记'),
        `stdout 应为收尾正文（实际: ${JSON.stringify(r1.stdout)}）`,
      );

      // ── 第 2 轮：下一次对话的请求里要带上调度段 ───────────────────────────
      const r2 = runCli({ ...base, session: SID, message: '继续推进任务' });
      assertEq(r2.code, 0, `第 2 轮退出码（stderr: ${r2.stderr.slice(0, 600)}）`);

      // ── 证据 ①：模型看到的名字后端认得回来 ────────────────────────────────
      const reqs = await llm.requests();
      assertEq(
        reqs.length,
        3,
        `三发请求（声明 + 结果回灌 + 下一轮）：实际 ${reqs.length}\n` +
          `各发最后一条 user：${JSON.stringify(
            reqs.map((r) => {
              const ms = r.body.messages ?? [];
              const u = [...ms].reverse().find((m) => m.role === 'user');
              return typeof u?.content === 'string' ? u.content.slice(0, 40) : '(非字符串)';
            }),
          )}`,
      );
      const tools1 = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      assert(
        tools1.includes('todo_write'),
        `第一次请求的工具清单应含 \`todo_write\`（能力注册表名 = LLM 可见名）——实际: ${tools1.join(', ')}`,
      );

      // 调度段此刻**不该**出现：任务还没落格，且第二发是工具轮（`tool_rounds > 0`）。
      for (const [i, r] of reqs.slice(0, 2).entries()) {
        assert(
          !JSON.stringify(r.body).includes('【任务调度】'),
          `第 ${i + 1} 发请求（声明阶段）不该有任务调度段——就绪集还没有数据`,
        );
      }

      // ── 证据 ②：`depends_on` 穿过 schema → 存储 → 事件 ───────────────────
      const walPath = join(hd.homedir, 'session', SID, V2_WAL);
      const events = readEvents(walPath);
      const kinds = events.map((e) => e.kind).join(', ');
      const opened = events.filter((e) => e.kind === 'task.opened');
      const progress = events.filter((e) => e.kind === 'task.progress');
      const asserted = events.filter((e) => e.kind === 'task.asserted');
      const users = events.filter((e) => e.kind === 'user.message');
      assertEq(opened.length, 3, `三条任务各开一格（实际 ${opened.length}: ${kinds}）`);
      assertEq(progress.length, 1, `只有 t1 推进一格（实际 ${progress.length}: ${kinds}）`);
      assertEq(asserted.length, 0, `本轮没有验收（实际 ${asserted.length}: ${kinds}）`);
      assertEq(
        opened.find((e) => e.payload?.task_id === 't2')?.payload?.depends_on,
        ['t1'],
        `t2 的依赖必须原样落格（实际: ${JSON.stringify(opened.map((e) => e.payload))}）`,
      );
      assertEq(
        opened.find((e) => e.payload?.task_id === 't3')?.payload?.depends_on,
        [],
        '无依赖的任务落空表（不是缺席——读侧两种缺省算出同一个集合，但形状要一致）',
      );
      assertEq(progress[0].payload?.task_id, 't1', '推进的应是 t1');

      // I2：任务事件带溯源；锚是**本轮**用户格；事件号带 `{user_id}-a{attempt}` 前缀。
      assertEq(users.length, 2, `两轮各一格用户发言（实际 ${users.length}: ${kinds}）`);
      for (const e of [...opened, ...progress]) {
        assertEq(
          e.produced_by,
          users[0].seq,
          `任务事件溯源应锚在声明那一轮的用户格（${e.event_id} → ${e.produced_by}）`,
        );
        assert(
          /^v2t-.+-a\d+-\d+$/.test(e.event_id),
          `任务号须带 {user_id}-a{attempt} 前缀（调用编号只在一次响应内唯一，不加前缀会撞幂等键）：${e.event_id}`,
        );
        assertEq(e.actor, 'agent:main', '任务事件 actor = 收束 principal');
      }

      // ── 证据 ③：下一次请求带调度段，且点名的是**真的就绪**的那两条 ────────
      //
      // 断言只看**调度段自己那条消息**：请求体里本来就带着工具调用的参数历史
      // （`落地实现` 在 `todo_write` 的入参里），拿整包当判据等于在断言「历史里
      // 有历史」。
      const nextMsgs = reqs[2].body.messages ?? [];
      const sections = nextMsgs.filter(
        (m) => m.role === 'user' && textOf(m).includes('【任务调度】'),
      );
      assertEq(
        sections.length,
        1,
        `声明之后的下一次请求应恰有一条调度段消息（实际 ${sections.length}；首条 = ${JSON.stringify(
          textOf(nextMsgs[0]).slice(0, 80),
        )}）`,
      );
      const sectionText = textOf(sections[0]);
      assert(
        sectionText.includes('t1、t3'),
        `就绪集应是 [t1, t3]（t2 还等着 t1）——实际段落: ${JSON.stringify(sectionText)}`,
      );
      assert(
        !sectionText.includes('落地实现') && !sectionText.includes('分析架构'),
        `调度段只给候选集的 id 与一句判据，不复述清单正文——实际段落: ${JSON.stringify(sectionText)}`,
      );

      // ── 转写不变量 ─────────────────────────────────────────────────────────
      const msgs = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs, 'T31');

      // ── 证据 ④：出口读数与下一次请求**同源**（gateway `session/stats`）────
      const api = startLongLivedCli({
        ...base,
        session: SID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
        assertEq(
          stats.invariants.length,
          0,
          `声明那一轮已收束 ⇒ 断言全绿（实际: ${JSON.stringify(stats.invariants)}）`,
        );
        assertEq(
          stats.readyset.ready.map((t) => t.task_id),
          ['t1', 't3'],
          `出口读出的就绪集应与调度段点名的两条一致（实际: ${JSON.stringify(stats.readyset)}）`,
        );
        assertEq(
          stats.readyset.ready[1].depends_on,
          [],
          '就绪任务的依赖必然已全部终态（放进集合时就满足，不是断言时才校验）',
        );

        // 读侧闸：矩阵外主体 ⇒ 就绪集走**四列形态**（空切片 = `{ready: []}`），
        // 不是声誉那列的整列 `{}`——「没有任务」与「不给你看」由 `has_wal` 分辨。
        const denied = await api.invoke(
          'session/stats',
          { principal: 'agent:ghost' },
          { session_id: SID },
        );
        assertEq(
          denied.status,
          200,
          `被拒读也应是 200 + 空读数（实际: ${JSON.stringify(denied.body)}）`,
        );
        assert(denied.body?.data?.has_wal, '有源但不给你看——与「没有源」要能分辨');
        assertEq(
          denied.body?.data?.readyset,
          { ready: [] },
          '无读权限 ⇒ 就绪集为空表（形态与「确实没有就绪任务」一致，值域由 has_wal 分）',
        );

        // ── 证据 ⑤：对事实源做手术 ⇒ 就绪集与不变量**一起**变 ───────────────
        // 判据是「两处都读同一份切片」：一处跟着变、另一处纹丝不动就是常数。
        const lines = readFileSyncSafe(walPath)
          .split('\n')
          .filter(Boolean)
          .map((l) => JSON.parse(l));
        let patched = 0;
        for (const e of lines) {
          if (e.kind !== 'task.opened') continue;
          if (e.payload?.task_id === 't2') { e.payload.depends_on = ['t3']; patched += 1; }
          if (e.payload?.task_id === 't3') { e.payload.depends_on = ['t2']; patched += 1; }
        }
        assertEq(patched, 2, `手术应恰好改两条 opened（实际 ${patched}）`);
        writeFileSync(walPath, lines.map((e) => JSON.stringify(e)).join('\n') + '\n', 'utf8');

        const after = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(after.status, 200, `手术后读数应成功（实际: ${JSON.stringify(after.body)}）`);
        const afterStats = after.body?.data;
        assertEq(
          afterStats.readyset.ready.map((t) => t.task_id),
          ['t1'],
          `成环后就绪集只剩 t1（t2/t3 互为前置 = 依赖未闭合）（实际: ${JSON.stringify(afterStats.readyset)}）`,
        );
        assert(
          afterStats.invariants.length > 0,
          '成环必须让不变量清单变红——断言是会跑的，不是永远空的装饰',
        );
        assert(
          JSON.stringify(afterStats.invariants).includes('依赖环'),
          `违规条目应点名依赖环（实际: ${JSON.stringify(afterStats.invariants)}）`,
        );
        // 正向对照：手术只动了 `depends_on`，事实源本身还读得出来（`has_wal` 仍真）。
        assert(afterStats.has_wal, '手术没有破坏事实源本身');
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28/t30 同一约定：Windows 上带活子进程退出会撞
      // libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
