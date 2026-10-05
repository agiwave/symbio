import './_selfrun.mjs';
// T34 忙窗抢占判定（`PreemptionDecider`，S8 第 19 步，04 §2.1–2.2）：
// 忙窗里入队一条插话 ⇒ 判定者判出**挂起** ⇒ 挂起 / 控制 / 恢复三条事实依次落格。
//
// ## 本用例钉的是什么
//
// [11 批 2 ③](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的接线判据：
// 「每件接线一条 e2e（**接线前后行为可见地不同**，否则等于没接）」——这里钉的是
// `PreemptionDecider`：`transcript/inbox.rs` 的忙窗分支在队列非空时判一次，
// 结论跨过忙窗、在空闲分支落成 `task.held` + `task.controlled{interrupt-suspend}`，
// 插话轮结束后再落 `task.progress`（恢复）。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 判定**真的在忙窗发生** | 事实源里 `held` / `controlled` 的**相对位置**（都在插话轮之前） |
// | 结论**真的跨过忙窗** | `held` 落在忙窗**之后**（判定只判定不落格，见 `judge_preemption` 边界 3） |
// | 挂起**真的被收** | 插话轮之后有一条 `res-*` 的 `task.progress`（只挂不收 = 任务永久消失） |
//
// 单测（`inbox` 的用例）直接调 `decide` / `commit_preemption`，证不了「忙窗那一刻
// 真的有人在判」——那需要**真进程**、**真在跑的一轮**、**真的往收件箱写**三者同时成立。
//
// ## 为什么先要有「在跑的任务」
//
// `decide` 的判定顺序（04 §2.1）第一句就是「无在跑任务 → 放行」。在跑任务 =
// 最新一个 `task.opened`、且未被 `task.asserted` 收掉、也未被 `task.held` 挂起。
// 任务格由 `todo_write` 声明在**收束转写**时落（[T31](../cases/t31-task-table.mjs) 同源），
// 因此第一步必须真的跑一轮带 `todo_write` 的对话。
//
// ## 为什么 `final` 不能落在任务之后
//
// 判定顺序的第二句：任务开启后若已有收束发言 ⇒ 排队（已发出的发言不可撤回）。
// 而收束转写的落笔顺序是**先 final 再派生事实**（`v2_bridge::record` 的注释）——
// 所以任务开格的 seq 天然大于本轮 final 的 seq，判定落在「挂起」这一支。
// 这也是为什么插话必须赶在**下一轮的 final 之前**：那一轮的 final 一落，
// 同一个任务就变成「排队」了。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  startLongLivedCli,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t34';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 声明任务的触发串（与 mock 场景 `match` 共用同一个词） */
const DECLARE = '登记长任务';

/** 插话正文：它进的是**收件箱**，与 REPL 那条走同一条消费链 */
const INTERJECT = '插话：先回答我';

/** 单条任务声明：只有一个在跑任务 ⇒ 判定结论的 `task_id` 无歧义。 */
const TODOS = [{ id: 't1', content: '长任务', status: 'in_progress', priority: 'high' }];

/** 读事实源（缺文件 / 空文件都是用例前提不成立，直接报而不是当零值算）。 */
function readEvents(path) {
  const raw = readFileSyncSafe(path);
  assert(raw.length > 0, `v2 事实源应存在且非空：${path}`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

export default defineCase(
  'T34 忙窗抢占判定：忙窗里入队插话 ⇒ task.held + task.controlled{interrupt-suspend} + 恢复',
  async () => {
    const llm = await new MockLlm([
      // 第一轮：声明一个在跑任务（`once` 防工具循环，与 T31 同款）。
      {
        id: 'todo-declare',
        match: DECLARE,
        once: true,
        content: '我把长任务登记进清单。',
        toolCalls: [{ id: 'call_1', name: 'todo_write', arguments: { todos: TODOS } }],
      },
      { id: 'after-todo', afterTool: true, content: '清单已登记。' },
      // 第二轮：**慢吐**——拉长忙窗，给「忙窗里写插话」留出时序空间。
      {
        id: 'slow',
        match: '开始长任务',
        chunks: ['长', '任务', '正', '在', '跑', '…'],
        chunkDelayMs: 400,
      },
      // 插话轮：收件箱那条被取走后正常收束。
      { id: 'interject', match: '插话', content: '插话已处理。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 无此文件时网关回退默认配置（`inbound_enabled: false`）——收件箱要靠
        // gateway 的 `vdfs/write` 边界写进去（与 T15/T28 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 抢占的写方挂在**收束转写**（`v2_bridge::record`）与收件箱上 ⇒ bridge 档。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'bridge' },
      },
    });

    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: SID,
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const walPath = join(hd.homedir, 'session', SID, V2_WAL);
    const eventsOf = (kind) => readEvents(walPath).filter((e) => e.kind === kind);

    try {
      await cli.waitGatewayReady();
      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const inboxAddr = `${root.replace(/\/+$/, '')}/session/${SID}/inbox`;

      // ── 第 1 轮：登记一个在跑任务 ─────────────────────────────────────────
      cli.send(`请${DECLARE}`);
      await waitFor(() => eventsOf('task.opened').length >= 1, {
        what: '任务开格（`todo_write` 经收束转写落成 task.opened）',
        timeoutMs: 40_000,
      });
      const opened = eventsOf('task.opened');
      assertEq(opened.length, 1, `只声明了一条任务（实际 ${opened.length}）`);
      assertEq(opened[0].payload?.task_id, 't1', '任务 id 原样落格');
      assert(
        !eventsOf('task.held').length,
        '忙窗还没开始，不该有挂起格（挂起是**判出来的**，不是声明出来的）',
      );

      // ── 第 2 轮：慢吐轮 —— 忙窗里写插话 ──────────────────────────────────
      const beforeReq = (await llm.requests()).length;
      cli.send('开始长任务');
      await waitFor(async () => (await llm.requests()).length > beforeReq, {
        what: '第二轮请求到达（证明这一轮真的在跑 = 忙窗已开）',
        timeoutMs: 30_000,
      });

      const interjected = await cli.invoke('vdfs/write', { path: inboxAddr, text: INTERJECT });
      assertEq(
        interjected.status,
        200,
        `忙窗里写收件箱应成功（入队即返回）：${JSON.stringify(interjected.body)?.slice(0, 300)}`,
      );

      // ── 证据 ①：忙窗里判出的挂起 + 控制事实落格 ───────────────────────────
      await waitFor(() => eventsOf('task.held').length >= 1, {
        what: '挂起事实落格（task.held）',
        timeoutMs: 30_000,
      });
      const held = eventsOf('task.held');
      assertEq(held.length, 1, `应恰好一次挂起（实际 ${held.length}）`);
      assertEq(held[0].entity, 'Task', '挂起格实体坐标');
      assertEq(held[0].verb, 'Held', '挂起格动词坐标');
      assertEq(held[0].payload?.task_id, 't1', '挂起的正是在跑的那个任务');
      assertEq(
        held[0].payload?.as_of,
        opened[0].seq,
        '`as_of` = 任务开格 seq（判定用的 as-of 锚随事实落盘，不留在内存里）',
      );
      assert(held[0].produced_by != null, '挂起格必须带溯源（I2）');

      const controlled = eventsOf('task.controlled');
      assertEq(controlled.length, 1, `打断处置应落一格控制事实（实际 ${controlled.length}）`);
      assertEq(
        controlled[0].payload?.reason,
        'interrupt-suspend',
        '打断理由与熔断共用同一格，靠 reason 区分（不能与 budget-exhausted 混为一谈）',
      );

      // ── 证据 ②：插话轮结束后挂起被**收掉**（只挂不收 = 任务永久消失）──────
      //
      // 插话格的落笔在**插话轮收束**时（与其余收束转写同一处）⇒ 顺序判据要等恢复
      // 落格之后才比得全。
      await waitFor(
        () => readEvents(walPath).some((e) => String(e.event_id).startsWith('res-')),
        { what: '恢复事实落格（res-* 的 task.progress）', timeoutMs: 30_000 },
      );
      const settled = readEvents(walPath);
      const resumed = settled.filter((e) => String(e.event_id).startsWith('res-'));
      assertEq(resumed.length, 1, `应恰好一次恢复（实际 ${resumed.length}）`);
      assertEq(resumed[0].kind, 'task.progress', '恢复 = task × progressed');
      assertEq(resumed[0].payload?.task_id, 't1', '恢复的是被挂起的那个任务');

      // ── 证据 ③：三者的**相对位置**（顺序即判据）──────────────────────────
      const interjectUser = settled.find(
        (e) => e.kind === 'user.message' && String(e.payload?.text ?? '').includes('插话'),
      );
      assert(
        interjectUser,
        `插话应成为一条用户格（实际 kinds: ${settled.map((e) => e.kind).join(', ')}）`,
      );
      const heldSeq = settled.find((e) => e.kind === 'task.held').seq;
      assert(
        heldSeq < interjectUser.seq,
        `挂起必须先于插话轮落格（顺序反了就是"插话开跑时任务还在就绪集里"）：held=${heldSeq} 插话=${interjectUser.seq}`,
      );
      assert(
        interjectUser.seq < resumed[0].seq,
        `恢复必须晚于插话轮（挂起期间任务不进 readyset，不收就是永久挂起）：插话=${interjectUser.seq} 恢复=${resumed[0].seq}`,
      );

      // ── 证据 ④：插话轮自身完整收束（挂起/恢复都不开新轮、都不动队列）──────
      assert(
        readEvents(walPath).some(
          (e) => e.kind === 'chat.assistant.final' && String(e.payload?.text ?? '').includes('插话已处理'),
        ),
        '插话轮应有自己的收束发言',
      );
      const kinds = readEvents(walPath).map((e) => e.kind);
      assertEq(
        kinds.filter((k) => k === 'user.message').length,
        3,
        `三格用户发言（声明 / 长任务 / 插话）：实际 ${JSON.stringify(kinds)}`,
      );
    } finally {
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28/t31 同一约定：Windows 上带活子进程退出会
      // 撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
