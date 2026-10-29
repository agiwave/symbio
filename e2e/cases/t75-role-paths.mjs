import './_selfrun.mjs';
// T75 角色主体执行路径（相应批次，S02/S03）：
// `session/worker` 经**委派链真跑一轮**——产物格的作者是工人自己；同一轮里它调
// `todo_write` 被**任务表写侧闸**诚实拒止（纯产出主体不立任务，C15 的精神）。
//
// ## 本用例钉的是什么
//
// 的 exit ①/④：worker 行**与驱动路径同批**——不是「表里有一行、生产里没人」。
// 四格各只有 e2e 能看见**接在一起**：
//
// | 格 | 单测钉在哪 | 这里钉什么 |
// |---|---|---|
// | 委派 ⇒ 子会话拿登记窄行 | `chat_loop/state.test.rs`（五字段真值） | 子会话真跑一轮，`ActorSpec` 的后果落在格子上 |
// | 产物收束（作者 = 工人） | `turn_runner.test.rs`（收束闸分种类） | 真实轮次的 `artifact.added` actor = `session/worker`、无发言格 |
// | 收窄生效（vs 派生整套） | `authz.test.rs`（行窄于派生） | 未登记的 `worker` 会落 final 格；登记的**不落** |
// | 任务表闸拒纯产出主体 | `tool_executor.test.rs`（闸的接线） | 拒因回灌进模型下一发请求，且**没有任务格**落盘 |
//
// ## 场景（两个轮次）
//
// - **轮 1**（正对照）：`session/main` 调 `todo_write` ⇒ 照常执行（持 `define.work`），
//   任务格作者 = `session/main`。缺了这一格，「闸恒拒」也能全绿。
// - **轮 2**：`agent_run` 委派 `worker` ⇒ 子会话三发请求：真调 `vdfs_read`（产物格）、
//   调 `todo_write`（被拒）、收尾正文（发言闸拒 ⇒ 不落 final 格，产物收束已足）。
//
// ## 为什么断言「无 final 格」
//
// worker 行只持 `produce.artifact`——若哪天有人把它改回派生（未登记 `agent:<id>` ⇒
// 主智能体整套含 `reply.*`），子会话会**多出一格** `chat.assistant.final`。那一格就是
// 「工人以主控身份发言」的静默面，本用例把它钉成断言。
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  readSessionMessages,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
} from '../helpers.mjs';
import { join } from 'node:path';

/** 主会话：两轮（轮 1 立任务；轮 2 委派工人）。 */
const SID = 'e2e-t75-roles';

/** 轮 1 触发串：声明任务图（`todo_write`）。 */
const Q1 = '登记任务';
/** 轮 2 触发串：委派工人（`agent_run`）。 */
const Q2 = '派工人干活';

/** 委托出去的那句话（= `agent_run` 的 `prompt`，也是子会话的触发串）。 */
const WORKER_PROMPT = '把 README 读一遍并汇报';

/** 任务图（同 t31/t42 形状）。 */
const TODOS = [
  { id: 't1', content: '分析架构', status: 'in_progress', priority: 'high' },
  { id: 't2', content: '落地实现', status: 'pending', priority: 'medium', depends_on: ['t1'] },
];

/** v2 事实源文件名（与 `crates/symbio-plugin-session/src/paths.rs::V2_WAL_FILE` 同一常量口径）。 */
const V2_WAL = 'v2-events.wal';

/** 读一份 WAL 文件的行（逐行 JSON）——与 t71 / t74 同一条直读口径。 */
function walLines(path) {
  return readFileSyncSafe(path)
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

/** 请求体里最后一条 user 消息的正文（mock 匹配同口径）。 */
function lastUserText(r) {
  const msgs = r.body?.messages ?? [];
  const last = [...msgs].reverse().find((m) => msgRoleOf(m) === 'user');
  if (typeof last?.content === 'string') return last.content;
  if (Array.isArray(last?.content)) return last.content.map((p) => p.text ?? '').join('');
  return '';
}

export default defineCase(
  'T75 角色主体执行路径：worker 委派真跑一轮（产物格作者 = session/worker）+ 任务表闸拒纯产出主体',
  async () => {
    const llm = await new MockLlm([
      // ── 主会话轮 1：声明任务图（工具真的执行——正对照）────────────────────
      {
        id: 'r1-todo',
        match: Q1,
        once: true,
        toolCalls: [{ id: 'call_1', name: 'todo_write', arguments: { todos: TODOS } }],
      },
      { id: 'r1-final', match: Q1, afterTool: true, content: '任务清单已登记。' },
      // ── 主会话轮 2：委派工人（`agent_run` ⇒ 代际立约）──────────────────────
      {
        id: 'r2-delegate',
        match: Q2,
        once: true,
        toolCalls: [
          {
            id: 'call_2',
            name: 'agent_run',
            arguments: { agent_id: 'worker', prompt: WORKER_PROMPT },
          },
        ],
      },
      { id: 'r2-final', match: Q2, afterTool: true, content: '已收到工人的成果。' },
      // ── 工人子会话：第 1 发——真调一个工具（产物格的来源）─────────────────
      {
        id: 'w1-read',
        match: WORKER_PROMPT,
        once: true,
        toolCalls: [{ id: 'call_w1', name: 'vdfs_read', arguments: { path: 'README.md' } }],
      },
      // 工人子会话：第 2 发（工具结果轮）——调 `todo_write` ⇒ 被任务表闸拒
      {
        id: 'w2-todo',
        match: WORKER_PROMPT,
        afterTool: true,
        once: true,
        toolCalls: [
          {
            id: 'call_w2',
            name: 'todo_write',
            arguments: { todos: [{ id: 'w1', content: '工人自己记一笔', status: 'completed' }] },
          },
        ],
      },
      // 工人子会话：第 3 发（工具结果轮）——收尾正文（发言闸拒 ⇒ 不落 final 格）
      { id: 'w3-final', match: WORKER_PROMPT, afterTool: true, content: '工人活干完了。' },
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
        // 对话面三项按 `DIALOG_FACE_OFF` 钉死（与 t42 同一条理由）。
        session: { ...DIALOG_FACE_OFF },
      },
    });
    // 工人智能体目录：`agent_run` 的落点（t42 的 `reviewer` 同款）。
    makeAgentDir(hd.homedir, {
      id: 'worker',
      persona: '你是工人，只负责产生产物，不立任务也不发结论。',
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const mainWal = join(hd.homedir, 'session', SID, V2_WAL);

    try {
      // ── ① 轮 1：主会话立任务（正对照——闸不打断现有流）───────────────────
      const r1 = runCli({ ...base, session: SID, message: `请${Q1}` });
      assertEq(r1.code, 0, `轮 1 退出码（stderr: ${r1.stderr.slice(0, 600)}）`);
      assert(
        r1.stdout.includes('任务清单已登记'),
        `轮 1 stdout 应为收尾正文（实际: ${JSON.stringify(r1.stdout)}）`,
      );

      const r1Events = walLines(mainWal);
      const taskOpened = r1Events.find((e) => e.kind === 'task.opened');
      assert(taskOpened, `轮 1 应落任务格（实际 kinds: ${JSON.stringify(r1Events.map((e) => e.kind))}）`);
      assertEq(
        taskOpened.actor,
        'session/main',
        '主智能体持 define.work ⇒ 任务格照常落（作者 = 会话主体）',
      );

      // ── ② 轮 2：委派工人 ⇒ 子会话真跑一轮 ────────────────────────────────
      const r2 = runCli({ ...base, session: SID, message: `请${Q2}`, timeoutMs: 180_000 });
      assertEq(r2.code, 0, `轮 2 退出码（stderr: ${r2.stderr.slice(0, 600)}）`);
      assert(
        r2.stdout.includes('已收到工人的成果'),
        `轮 2 stdout 应为收尾正文（实际: ${JSON.stringify(r2.stdout)}）`,
      );

      // 子会话 id：`agent_run` 的结果正文带回 `[subagent_session_id: <id>]`
      // （父会话 WAL 里的产物格载着那条结果——读方与生产同一条路）。
      //
      // 子会话的目录是**嵌套**的：`<session>/<父>/sessions/<子>/`（子会话不住顶层，
      // 见 `store` 模块头「子会话嵌套存储」）——这正是部署级读侧（淘汰闸覆盖层 /
      // stats 实例聚合）必须下探一层的原因，本用例的路径按生产布局拼。
      const r2Events = walLines(mainWal);
      const delegateArtifact = r2Events.find(
        (e) => e.kind === 'artifact.added' && e.payload?.tool === 'agent_run',
      );
      assert(delegateArtifact, `轮 2 应有 agent_run 的产物格（实际 kinds: ${JSON.stringify(r2Events.map((e) => e.kind))}）`);
      const sidMatch = /\[subagent_session_id: ([0-9a-f-]+)\]/.exec(
        delegateArtifact.payload?.text ?? '',
      );
      assert(sidMatch, `agent_run 结果应带回子会话 id（实际: ${(delegateArtifact.payload?.text ?? '').slice(0, 300)}）`);
      const childSid = sidMatch[1];
      const childWal = join(hd.homedir, 'session', SID, 'sessions', childSid, V2_WAL);

      await waitFor(() => readFileSyncSafe(childWal).includes('artifact.added'), {
        what: '工人子会话的产物格入格',
        timeoutMs: 30_000,
      });

      // ── ③ 子会话事实源逐格断言 ───────────────────────────────────────────
      const childEvents = walLines(childWal);
      const kinds = childEvents.map((e) => e.kind);

      const userCell = childEvents.find((e) => e.kind === 'user.message');
      assert(userCell, `子会话应有开轮格（实际 kinds: ${JSON.stringify(kinds)}）`);

      // 产物收束（D-067）：工具结果入格，**作者 = 工人自己**。
      const artifact = childEvents.find((e) => e.kind === 'artifact.added');
      assert(artifact, `工人子会话必须落产物格（实际 kinds: ${JSON.stringify(kinds)}）`);
      assertEq(artifact.actor, 'session/worker', '产物格的作者 = 工人（委派链上的登记窄行主体）');
      assertEq(artifact.payload?.tool, 'vdfs_read', '产物来自工人真调的那个工具');
      assertEq(artifact.turn, userCell.turn, '产物格记在本轮');
      assertEq(artifact.produced_by, userCell.seq, '产物格溯源指向本轮开口格（I2）');

      // 收窄生效（vs 派生整套）：工人无发言权 ⇒ 不落发言格。未登记的 `worker`
      // 会派生主智能体整套（含 reply.*）⇒ 会多出一格 final——那一格就是
      // 「工人以主控身份发言」的静默面。
      assert(
        !kinds.includes('chat.assistant.final') && !kinds.includes('chat.assistant.fallback'),
        `工人无发言权 ⇒ 不落发言格（实际 kinds: ${JSON.stringify(kinds)}）`,
      );

      // 任务表闸：被拒的 `todo_write` **没有落任何任务格**（工具没跑，
      // `note_tasks` 只在跑完后记成功调用）。
      assert(
        !kinds.some((k) => k.startsWith('task.')),
        `纯产出主体不立任务 ⇒ 无任务格（实际 kinds: ${JSON.stringify(kinds)}）`,
      );

      // ── ④ 请求留痕：拒因真的回灌进了模型（用户与模型都看得见）────────────
      const reqs = await llm.requests();
      const workerReqs = reqs.filter((r) => lastUserText(r) === WORKER_PROMPT);
      assertEq(
        workerReqs.length,
        3,
        `工人子会话恰好三发请求（工具轮 ×2 + 收尾；实际 ${workerReqs.length}）`,
      );
      const thirdTools = (workerReqs[2].body?.messages ?? []).filter((m) => m.role === 'tool');
      assert(
        JSON.stringify(thirdTools).includes('Refused:'),
        `第三发应带回灌的拒因（实际: ${JSON.stringify(thirdTools).slice(0, 300)}）`,
      );
      assert(
        JSON.stringify(thirdTools).includes('define.work / assert.verification'),
        '拒因应点名两项资格能力（可审计）',
      );

      // ── ⑤ 生产读出口：子会话不变量为空（C4 认产物收束）+ 父会话承诺了结 ──
      const api = startLongLivedCli({
        ...base,
        session: SID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        const childStats = await api.invoke('session/stats', {}, { session_id: childSid });
        assertEq(
          childStats.status,
          200,
          `子会话 session/stats 应成功（实际: ${JSON.stringify(childStats.body)}）`,
        );
        assertEq(
          childStats.body?.data?.invariants.length,
          0,
          `产物收束 ⇒ 子会话不变量清单为空（C4 认产物格，D-067；实际: ${JSON.stringify(childStats.body?.data?.invariants)}）`,
        );

        // 声誉列（两个事实各断一面）：
        // ① own 列的主体 = **目标会话自己的**主体（子会话 metadata.agent_id=worker
        //    → session/worker），不是请求方（长驻 CLI 绑主会话）的主体——跨会话查数
        //    时两者不同，接错就会把「别人看我的声誉」读成「别人的声誉」。
        // ② 工人这一轮的权威动作是**产物**不是承诺（todo_write 已被闸拒）⇒
        //    它没立任何约 ⇒ by_principal 为空是正确事实，不是读数缺失。
        const rep = childStats.body?.data?.reputation;
        assertEq(rep?.own?.principal, 'session/worker', 'own 列的主体 = 目标会话自己的主体（工人）');
        assertEq(
          Object.keys(rep?.by_principal ?? {}).length,
          0,
          `工人没立任何约 ⇒ by_principal 空（实际: ${JSON.stringify(rep)}）`,
        );

        // 父会话的承诺族：委派成功 ⇒ opened + released。
        const offered = r2Events.find((e) => e.kind === 'commitment.opened');
        const released = r2Events.find((e) => e.kind === 'commitment.released');
        assert(offered, '委派先立约（代际立约，S08）');
        assert(released, '工人真跑完 ⇒ 守约了结（不是违约）');
        assertEq(offered.actor, 'session/main', '承诺方 = 父会话主体');

        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T75 主会话');
        const toolNodes = msgs.filter((m) => msgRoleOf(m) === 'tool');
        assert(
          toolNodes.length >= 2,
          `主会话至少两个工具结果节点（todo_write + agent_run；实际 ${toolNodes.length}）`,
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
