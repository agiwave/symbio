import './_selfrun.mjs';
// T34 忙窗插话：忙窗里入队一条插话 ⇒ 不抢占，插话轮照常收束、历史完整。
//
// ## 本用例钉的是什么
//
// 今天**没有抢占**：判定面已删（`D-388`），忙窗里插话一律排队到轮末，任务留在
// 就绪集里不被挂起。文件名沿用历史场景名；改名会牵动矩阵引用，不值当。
//
// 切主后（D-049）e2e 只读用户可见面（读通道禁令），本用例判据落在：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 任务**真的在跑** | 转写（生产读出口）里 `todo_write` 的往返 + `session/stats` 就绪集有 t1 |
// | 插话**真的被取走处理** | 后续某一轮 **LLM 请求包**里出现插话正文（前端说的每句话模型都看到） |
// | 插话轮**完整收束** | 历史里有插话的问答、三格用户发言齐 |
// | 任务**不被挂起** | `session/stats` 就绪集里 t1 始终在（`task.held` 无人写） |
//
// `task.held` / `res-*` 的 `task.progress` 等事实白盒不再有生产写方；本用例只证
// 「忙窗那一刻没有判定者、插话那条真的走了消费链」——那需要**真进程**、**真在跑的一轮**、
// **真的往收件箱写**三者同时成立。
//
// ## 为什么先要有「在跑的任务」
//
// 任务格由 `todo_write` 声明在**收束**时落（[T31](t31-task-table.mjs) 同源），
// 因此第一步必须真的跑一轮带 `todo_write` 的对话。
//
// ## 为什么插话要赶在下一轮 final 之前
//
// 收束的落笔顺序是**先 final 再派生事实**（`derived_facts` 的注释）；插话在下一轮
// 开跑前入队，才能被那一轮取走，否则它只是排在队里等下一条。
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  startLongLivedCli,
  waitFor,
  readSessionMessages,
  textOf,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  msgRoleOf,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t34';

/** 声明任务的触发串（与 mock 场景 `match` 共用同一个词） */
const DECLARE = '登记长任务';

/** 插话正文：它进的是**收件箱**，与 REPL 那条走同一条消费链 */
const INTERJECT = '插话：先回答我';

/** 单条任务声明：只有一个在跑任务 ⇒ 判定结论的 `task_id` 无歧义。 */
const TODOS = [{ id: 't1', content: '长任务', status: 'in_progress', priority: 'high' }];

/** 就绪集里的任务 id（用户可见面：`session/stats` 的 readyset 投影） */
async function readysetTaskIds(api) {
  const inv = await api.invoke('session/stats', {}, { session_id: SID });
  assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
  const stats = inv.body?.data;
  assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
  return (stats.readyset?.ready ?? []).map((t) => t.task_id);
}

export default defineCase(
  'T34 忙窗抢占判定：忙窗里入队插话 ⇒ 插话轮照常收束、历史完整',
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
        // 抢占的写方挂在**收束落格**与收件箱上。
        ...dialogFaceOff(),
      },
    });

    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: SID,
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });

    try {
      await cli.waitGatewayReady();
      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const inboxAddr = `${root.replace(/\/+$/, '')}/session/${SID}/inbox`;

      // ── 第 1 轮：登记一个在跑任务（就绪集 = 用户可见的任务在场）──────────
      cli.send(`请${DECLARE}`);
      await waitFor(async () => (await readysetTaskIds(cli)).includes('t1'), {
        what: '任务进就绪集（`todo_write` 经收束落成 task.opened）',
        timeoutMs: 40_000,
      });
      assertEq(
        (await readysetTaskIds(cli)).filter((id) => id === 't1').length,
        1,
        '只声明了一条任务',
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

      // ── 证据 ①：插话**真的被取走处理**了（请求包 = 模型看到的同一份）─────
      await waitFor(
        async () => (await llm.requests()).some((r) => JSON.stringify(r.body).includes(INTERJECT)),
        { what: '插话进了某一轮 LLM 请求（忙窗里入队的插话被消费）', timeoutMs: 30_000 },
      );

      // ── 证据 ②：插话轮**完整收束**，历史里三格发言齐 ────────────────────
      await waitFor(async () => {
        const msgs = await readSessionMessages(cli, SID);
        const finals = (msgs ?? []).filter((m) => msgRoleOf(m) === 'assistant').map((m) => textOf(m));
        return finals.some((t) => t.includes('插话已处理'));
      }, { what: '插话轮收束发言落进历史', timeoutMs: 30_000 });

      const msgs = await readSessionMessages(cli, SID);
      assert(msgs, '历史应读得到（生产读出口）');
      const userTexts = msgs.filter((m) => msgRoleOf(m) === 'user').map((m) => textOf(m));
      assertEq(
        userTexts.length,
        3,
        `三格用户发言（声明 / 长任务 / 插话）：实际 ${JSON.stringify(userTexts)}`,
      );
      assert(
        userTexts.some((t) => t.includes(INTERJECT)),
        `插话应成为一条用户发言（实际: ${JSON.stringify(userTexts)}）`,
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
