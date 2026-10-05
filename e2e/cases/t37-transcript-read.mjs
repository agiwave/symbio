import './_selfrun.mjs';
// T37 转写读列（**对话面读侧**，[plan/12 批 2](../../docs/plan/12-价值验收与基线埋点.md)）：
// 两轮真实对话（bridge 档 ⇒ 收束转写进 `<会话目录>/v2-events.wal`）⇒ 读出口
// `session/stats` 的 `transcript` 列，与**同一份事实源**逐条对账；再对事实源做手术、
// 验读侧闸。
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
// | 转写读的是**这一轮真实写下**的事实 | 磁盘上的 `v2-events.wal`（由真实 LLM 流量产生） |
// | 读方真的每次重读文件 | 对文件动手术后**再调一次**，条目跟着变 |
// | 别人的对话不从你的读数里漏出去 | `principal: agent:ghost` ⇒ `{ entries: [] }` + `has_wal: true` |
//
// ## 对账口径
//
// 用例只做**取值与计数**：把事实源里的三格（`user.message` / `chat.assistant.final` /
// `chat.assistant.fallback`）按投影的口径摊成 `role:text` 序列，与出口逐条比——
// 口径属于 `transcript` 投影，出口与用例都只准调它，谁都不许另写一份。
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  readFileSyncSafe,
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t37';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 两轮的触发串（mock 场景 `match` 与用户正文共用同一个词）。 */
const Q1 = '第一问';
const Q2 = '第二问';

/** 读文件里的全部事件（出口读数的对账基准）。 */
function readEvents(walPath) {
  const raw = readFileSyncSafe(walPath);
  assert(raw.length > 0, `v2 事实源应存在且非空：${walPath}`);
  return raw
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

/**
 * 按 `transcript` 投影的口径把事实源摊成 `role:text` 序列（**用例侧复算**）。
 *
 * 三格 → 两角色：`user.message` ⇒ `user`（载荷 `text`）；`chat.assistant.final` ⇒
 * `assistant`（载荷 `text`）；`chat.assistant.fallback` ⇒ `assistant`（载荷 `why`——
 * 兜底话术是用户实际看到的回复）。只认 `entity == Turn` 的那三格，与投影同款。
 */
function transcriptOf(events) {
  const rows = [];
  for (const e of events) {
    if (e.entity !== 'Turn') continue;
    if (e.kind === 'user.message') rows.push(`user:${e.payload?.text ?? ''}`);
    else if (e.kind === 'chat.assistant.final') rows.push(`assistant:${e.payload?.text ?? ''}`);
    else if (e.kind === 'chat.assistant.fallback') rows.push(`assistant:${e.payload?.why ?? ''}`);
  }
  return rows;
}

/** 出口那一列摊成同一形状（比 `role:text` 序列，绕开 JSON 键序的假红）。 */
function transcriptRows(stats) {
  return (stats.transcript?.entries ?? []).map((e) => `${e.role}:${e.text}`);
}

export default defineCase(
  'T37 转写读列：出口 session/stats 的 transcript 与事实源逐条对账，且随事实源变动',
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
        // `bridge` 档：每轮收束转写进 `<会话目录>/v2-events.wal`——转写列的事实源。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'bridge' },
      },
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };

    try {
      // ── 跑两轮成功对话（同一会话 ⇒ 同一份事实源）────────────────────────────
      const r1 = runCli({ ...base, session: SID, message: Q1 });
      assertEq(r1.code, 0, `第 1 轮退出码（stderr: ${r1.stderr.slice(0, 400)}）`);
      const r2 = runCli({ ...base, session: SID, message: Q2 });
      assertEq(r2.code, 0, `第 2 轮退出码（stderr: ${r2.stderr.slice(0, 400)}）`);

      const walPath = join(hd.homedir, 'session', SID, V2_WAL);
      const events = readEvents(walPath);
      const expected = transcriptOf(events);

      // 前置：事实源里确实有三格（两轮各一开轮 + 一收束）⇒ 转写应有 4 条。
      assertEq(
        expected.length,
        4,
        `两轮对话 ⇒ 事实源应有 4 条转写条目（实际 ${expected.length}: ${JSON.stringify(expected)}）`,
      );
      assertEq(
        expected.filter((r) => r.startsWith('user:')).length,
        2,
        `两轮各一格 user.message（实际: ${JSON.stringify(expected)}）`,
      );

      // ── 证据 ①：从**生产路由**读出口（gateway HTTP 边界，与前端同构）────────
      const api = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await api.waitGatewayReady();

        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
        assert(stats.has_wal, '跑过两轮 ⇒ 事实源应在');

        // ── 证据 ②：转写列与**同一时刻的文件**逐条对账 ────────────────────────
        const gotRows = transcriptRows(stats);
        assertEq(
          gotRows.join('\n'),
          expected.join('\n'),
          `转写列应与事实源的三格逐条对账：出口 ${JSON.stringify(gotRows)} / 文件 ${JSON.stringify(expected)}`,
        );
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

        // ── 证据 ②′：读侧闸——别人的对话不从你的读数里漏出去 ──────────────────
        // 不声明 `principal` = 本机默认（上面 ②）；声明属主 = 逐字相同；矩阵外主体 ⇒ 空表。
        const asOwner = (
          await api.invoke('session/stats', { principal: 'user' }, { session_id: SID })
        ).body.data;
        assertEq(
          transcriptRows(asOwner).join('\n'),
          expected.join('\n'),
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

        // ── 证据 ③：对事实源动手术 ⇒ 转写必须跟着变（不是常数）───────────────
        // 删掉**第一轮**的收束格 ⇒ 那一句必须从转写里消失（条目 4 → 3）。
        const dropTurn0Final = () => {
          const lines = readFileSyncSafe(walPath).split('\n').filter(Boolean);
          const kept = [];
          let dropped = 0;
          for (const line of lines) {
            const e = JSON.parse(line);
            if (e.kind === 'chat.assistant.final' && e.turn === 0) {
              dropped += 1;
              continue;
            }
            kept.push(line);
          }
          assert(dropped > 0, '手术刀必须真的删掉行，否则测不出变化');
          writeFileSync(walPath, `${kept.join('\n')}\n`, 'utf8');
        };
        dropTurn0Final();
        const after = (await api.invoke('session/stats', {}, { session_id: SID })).body.data;
        const afterRows = transcriptRows(after);
        assertEq(
          afterRows.length,
          3,
          `删一条 final ⇒ 转写 4 → 3（实际 ${afterRows.length}: ${JSON.stringify(afterRows)}）`,
        );
        assert(
          !afterRows.some((r) => r.includes('第一答。')),
          `被删的那句不得再出现在转写里（实际: ${JSON.stringify(afterRows)}）`,
        );
        assert(
          afterRows.some((r) => r.includes('第二答。')),
          '未被删的那一轮不受影响（读数真的在读文件，不是整体清空）',
        );
        assert(after.has_wal, '手术没有破坏事实源本身');
      } finally {
        api.stop();
      }

      // ── 转写不变量 ─────────────────────────────────────────────────────────
      const msgs = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs, 'T37');
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
