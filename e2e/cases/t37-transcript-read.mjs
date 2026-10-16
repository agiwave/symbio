import './_selfrun.mjs';
// T37 转写读列（**对话面读侧**，[plan/12 批 2](../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）：
// 两轮真实对话（每轮收束把事实写进事实源）⇒ 读出口 `session/stats` 的 `transcript`
// 列按**已知动作**钉死期望值；读侧闸验「别人的对话不从你的读数里漏出去」。
//
// ## 本用例钉的是什么
//
// `transcript` 是七个投影里最后一个接上**读数口**的：它早就喂着模型
// （`actors::Reasoner` 组 prompt 的输入，`actors/mod.rs`），但没有一条**生产读出口**
// 能让「对话能不能从事实源重建」这件事被验收——这正是 plan/12 §0 说的
// 「实测有了、判据没有」。本用例补的就是那一格的判据。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 出口真的挂上了生产路由 | gateway `POST /api/v1/invoke` → 容器 → `session/stats` |
// | 转写读的是**这一轮真实写下**的事实 | 真实 CLI 轮次产生的转写（读出口同样读得到） |
// | 别人的对话不从你的读数里漏出去 | `principal: agent:ghost` ⇒ `{ entries: [] }` + `has_wal: true` |
//
// ## 期望值为什么钉得死
//
// 两轮真实对话 ⇒ 转写恰四条（user/assistant 各两条）、角色按事件顺序交替、
// 两轮的问答正文都在里面。这些是**已知动作**的直接后果，不依赖任何文件对账；
// 口径属于 `transcript` 投影，出口与用例都只准调它，谁都不许另写一份。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  readSessionMessages,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t37';

/** 两轮的触发串（mock 场景 `match` 与用户正文共用同一个词）。 */
const Q1 = '第一问';
const Q2 = '第二问';

/** 出口那一列摊成同一形状（比 `role:text` 序列，绕开 JSON 键序的假红）。 */
function transcriptRows(stats) {
  return (stats.transcript?.entries ?? []).map((e) => `${e.role}:${e.text}`);
}

export default defineCase(
  'T37 转写读列：出口 session/stats 的 transcript 按两轮对话钉死期望值，读侧闸放行属主',
  async () => {
    const llm = await new MockLlm([
      { id: 'q1', match: Q1, content: '第一答。' },
      { id: 'q2', match: Q2, content: '第二答。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 无此文件时网关回退默认配置（`inbound_enabled: false`）——监听不会被拉起，
        // 出口就打不到（与 T28/T31 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 每轮收束把事实写进事实源——转写列的事实源。
        session: { ...DIALOG_FACE_OFF },
      },
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };

    try {
      // ── 跑两轮成功对话（同一会话 ⇒ 同一份事实源）────────────────────────────
      const r1 = runCli({ ...base, session: SID, message: Q1 });
      assertEq(r1.code, 0, `第 1 轮退出码（stderr: ${r1.stderr.slice(0, 400)}）`);
      const r2 = runCli({ ...base, session: SID, message: Q2 });
      assertEq(r2.code, 0, `第 2 轮退出码（stderr: ${r2.stderr.slice(0, 400)}）`);

      // ── 证据 ①：从**生产路由**读出口（gateway HTTP 边界，与前端同构）────────
      const api = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await api.waitGatewayReady();

        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
        assert(stats.has_wal, '跑过两轮 ⇒ 事实源应在');

        // ── 证据 ②：转写列 = **两轮对话**的直接后果（条目数 / 角色序 / 正文）──
        const gotRows = transcriptRows(stats);
        assertEq(stats.transcript.entries.length, 4, '两轮 ⇒ 四条（user/assistant 各两条）');
        assertEq(
          stats.transcript.entries.map((e) => e.role).join(','),
          'user,assistant,user,assistant',
          `角色应按事件顺序交替（实际: ${JSON.stringify(stats.transcript.entries.map((e) => e.role))}）`,
        );
        // 真实内容真的穿过了链路（不是空串占位）。
        const assistantTexts = stats.transcript.entries
          .filter((e) => e.role === 'assistant')
          .map((e) => e.text);
        assert(
          assistantTexts.some((t) => t.includes('第一答。')) &&
            assistantTexts.some((t) => t.includes('第二答。')),
          `两轮的助手正文都应出现在转写里（实际: ${JSON.stringify(assistantTexts)}）`,
        );
        const userTexts = stats.transcript.entries
          .filter((e) => e.role === 'user')
          .map((e) => e.text);
        assert(
          userTexts.some((t) => t.includes(Q1)) && userTexts.some((t) => t.includes(Q2)),
          `两轮的用户发言都应出现在转写里（实际: ${JSON.stringify(userTexts)}）`,
        );

        // ── 证据 ②′：读侧闸——别人的对话不从你的读数里漏出去 ──────────────────
        // 不声明 `principal` = 本机默认（上面 ②）；声明属主 = 逐字相同；矩阵外主体 ⇒ 空表。
        const asOwner = (
          await api.invoke('session/stats', { principal: 'person/local' }, { session_id: SID })
        ).body.data;
        assertEq(
          transcriptRows(asOwner).join('\n'),
          gotRows.join('\n'),
          '读方 = 属主 ⇒ 与本机默认读到同一份转写（闸不是为了把属主挡在外面）',
        );
        const denied = (
          await api.invoke('session/stats', { principal: 'agent:ghost' }, { session_id: SID })
        ).body.data;
        assertEq(
          denied.transcript,
          { entries: [] },
          `矩阵外主体 ⇒ 转写走四列形态（空切片 = {entries: []}，不是声誉的空对象）（实际: ${JSON.stringify(denied.transcript)}）`,
        );
        assertEq(denied.has_wal, true, '有源但不给你看——与「没有源」要能分辨');

        // ── 转写不变量（读出口读回：与前端 loadMessages 同一出口）────────────
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T37');
        assertEq(
          msgs.filter((m) => m.role === 'user').length,
          2,
          '两轮 ⇒ 读回的历史里两条 user 消息',
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28/t31 同一约定：Windows 上带活子进程退出会撞
      // libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
