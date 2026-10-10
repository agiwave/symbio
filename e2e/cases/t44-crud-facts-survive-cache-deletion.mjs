import './_selfrun.mjs';
// T44 变更**入格**后缓存仍可重建（S2.7-b1 判据，[ADR-053]）：
// 跑两轮 ⇒ **编辑**第一问 ⇒ **截断**第二问 ⇒ 删掉 `messages.json` ⇒
// 会话仍能开，且历史 = **变更之后**的那份（编辑生效、被截的那段没复活）。
//
// ## 本用例钉的是什么
//
// T43 证明了「只有追加时，缓存删掉后历史能重建」。但主读路径整体切到重建之前有一个
// **硬阻塞**：编辑与截断此前只重写缓存、**没进事实源**。于是：
//
// | 变更 | 没入格的后果（切源后） |
// |---|---|
// | 编辑 | 内容**回退**成编辑前 |
// | 截断 | 被删的消息**复活** |
//
// 这不是性能问题，是**正确性**问题。本用例是这条阻塞的判据：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 编辑进了事实源 | 删缓存后重建出的正文是**改过**的那句 |
// | 截断进了事实源 | 删缓存后重建出的历史里**没有**被截掉的那一轮 |
// | 缓存仍是读侧真源（b1 零行为变更） | 删缓存**前**读到的历史已经就是变更后的那份 |
// | 数据来自 WAL 而非偷偷重建缓存 | 读完之后 `messages.json` **仍不存在** |
//
// ## 为什么不比 `seq` / `timestamp`
//
// 与 T43 同一条理由（ADR-049：切源后 `seq` 分配属呈现细节）。而且本用例里被编辑的
// 那条**刻意不重发号**——位置锚在编辑前后逐字相同，这正是「编辑不动位置」的判据。
import { join } from 'node:path';
import { rmSync, existsSync } from 'node:fs';
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
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t44';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** `messages.json`（与 `plugins/session/store/mod.rs::MESSAGES_FILE` 同一常量口径） */
const MESSAGES_JSON = 'messages.json';

/** 两轮的触发串（mock 场景 `match` 与用户正文共用同一个词）。 */
const Q1 = '第一问';
const Q2 = '第二问';

/** 编辑后的第一问（与 `Q1` 不同，否则「编辑生效」与「没编辑」分不开）。 */
const Q1_EDITED = '改过的第一问';

/** 取消息的正文文本（与 helpers 的 `textOf` 同口径，就地复算以保持用例自洽）。 */
function contentText(m) {
  const c = m.content;
  if (c == null) return '';
  if (typeof c === 'string') return c;
  if (Array.isArray(c)) return c.map((p) => p.text ?? '').join('');
  return JSON.stringify(c);
}

/** 消息的**可比形状**（剔除呈现细节 `seq` / `timestamp`，与 T43 同口径）。 */
function shapeOf(msgs) {
  return msgs.map((m) =>
    [
      m.id ?? '',
      m.parent_id ?? '',
      m.role ?? '',
      m.type ?? m.msg_type ?? '',
      m.status ?? '',
      m.name ?? '',
      m.tool_call_id ?? '',
      contentText(m),
    ].join('\u0001'),
  );
}

export default defineCase(
  'T44 变更入格：编辑 / 截断后删掉 messages.json，重建出的历史是**变更之后**的那份（ADR-053 判据）',
  async () => {
    const llm = await new MockLlm([
      { id: 'q1', match: Q1, content: '第一答。' },
      { id: 'q2', match: Q2, content: '第二答。' },
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
        session: { ...DIALOG_FACE_OFF },
      },
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const dir = join(hd.homedir, 'session', SID);
    const messagesPath = join(dir, MESSAGES_JSON);
    const walPath = join(dir, V2_WAL);

    try {
      // ── 跑两轮成功对话（同一会话 ⇒ 同一份事实源 + 一份缓存）────────────────
      const r1 = runCli({ ...base, session: SID, message: Q1 });
      assertEq(r1.code, 0, `第 1 轮退出码（stderr: ${r1.stderr.slice(0, 400)}）`);
      const r2 = runCli({ ...base, session: SID, message: Q2 });
      assertEq(r2.code, 0, `第 2 轮退出码（stderr: ${r2.stderr.slice(0, 400)}）`);

      assert(existsSync(messagesPath), `跑过两轮 ⇒ 缓存应在：${messagesPath}`);
      assert(existsSync(walPath), `跑过两轮 ⇒ 事实源应在：${walPath}`);
      const walBefore = readFileSyncSafe(walPath)
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const cli = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await cli.waitGatewayReady();

        const rootPath = (await cli.invoke('vdfs/root', {})).body?.data?.path;
        assert(typeof rootPath === 'string' && rootPath.length > 0, 'vdfs/root 应返回根地址');
        const sessionAddr = `${rootPath.replace(/\/+$/, '')}/session/${SID}`;

        /** 读会话叶子，解出 `{ id, messages, … }`（与前端 `loadMessages` 同一出口） */
        const readLeaf = async (what) => {
          const resp = await cli.invoke('vdfs/read', { path: sessionAddr });
          assertEq(resp.status, 200, `${what} 应成功（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
          const text = resp.body?.data?.text;
          assert(
            typeof text === 'string' && text.length > 0,
            `${what} 应带回文本正文（${JSON.stringify(resp.body)?.slice(0, 300)}）`,
          );
          return JSON.parse(text);
        };

        const beforeCrud = await readLeaf('变更前读会话叶子');
        const q1Node = beforeCrud.messages.find(
          (m) => m.role === 'user' && contentText(m).includes(Q1),
        );
        const q2Node = beforeCrud.messages.find(
          (m) => m.role === 'user' && contentText(m).includes(Q2),
        );
        assert(q1Node?.id, `应能找到第一问的节点（实际: ${JSON.stringify(shapeOf(beforeCrud.messages))}）`);
        assert(q2Node?.id, '应能找到第二问的节点');
        assert(q1Node.id !== q2Node.id, '两条用户消息必须是两个节点');

        // ── ① 编辑：`vdfs/write(<会话>/message/<mid>, { content })` ──────────
        const editResp = await cli.invoke('vdfs/write', {
          path: `${sessionAddr}/message/${q1Node.id}`,
          text: JSON.stringify({ content: Q1_EDITED }),
        });
        assertEq(
          editResp.status,
          200,
          `编辑应成功（${JSON.stringify(editResp.body)?.slice(0, 300)}）`,
        );

        // ── ② 截断：`vdfs/action(<会话>/message/<mid>, truncate)` ────────────
        const truncResp = await cli.invoke('vdfs/action', {
          path: `${sessionAddr}/message/${q2Node.id}`,
          action: 'truncate',
        });
        assertEq(
          truncResp.status,
          200,
          `截断应成功（${JSON.stringify(truncResp.body)?.slice(0, 300)}）`,
        );
        const deletedIds = truncResp.body?.data?.data;
        assert(
          Array.isArray(deletedIds) && deletedIds.length > 0,
          `截断回执应带被删 id 列表（实际: ${JSON.stringify(truncResp.body?.data)}）`,
        );
        assert(
          deletedIds.includes(q2Node.id),
          `第二问必须在被删列表里（实际: ${JSON.stringify(deletedIds)}）`,
        );

        // ── 前置：变更**真的进了事实源**（两条变更格）────────────────────────
        const walAfter = readFileSyncSafe(walPath)
          .split('\n')
          .filter(Boolean)
          .map((l) => JSON.parse(l));
        const added = walAfter.slice(walBefore.length);
        assert(
          added.some((e) => e.kind === 'message.edited'),
          `编辑必须落一条事实源事件（新增: ${JSON.stringify(added.map((e) => e.kind))}）`,
        );
        assert(
          added.some((e) => e.kind === 'message.removed'),
          '截断必须落一条事实源事件',
        );
        // 一次截断落**一条**（区间用 ids 表达），不是 n 条。
        assertEq(
          added.filter((e) => e.kind === 'message.removed').length,
          1,
          '一次截断只落一条事实',
        );
        const removedFact = added.find((e) => e.kind === 'message.removed');
        assertEq(
          removedFact.payload?.ids?.length,
          deletedIds.length,
          '事实里的 ids 与回执里的权威列表逐条一致',
        );

        // ── 证据 ①：删缓存**前**读到的历史 = 变更之后的那份 ──────────────────
        const afterCrud = await readLeaf('变更后读会话叶子（缓存仍在）');
        const afterShape = shapeOf(afterCrud.messages);
        assert(
          afterShape.some((r) => r.includes(Q1_EDITED)),
          `编辑后缓存里应是新正文（实际: ${JSON.stringify(afterShape)}）`,
        );
        assert(
          !afterShape.some((r) => r.includes(Q2)),
          `截断后缓存里不该还有第二问（实际: ${JSON.stringify(afterShape)}）`,
        );
        assert(
          !afterShape.some((r) => r.includes('第二答。')),
          `截断后缓存里不该还有第二答（实际: ${JSON.stringify(afterShape)}）`,
        );

        // ── 证据 ②：**删掉缓存** ⇒ 会话仍能开，且历史是变更后的那份 ────────────
        rmSync(messagesPath, { force: true });
        assert(!existsSync(messagesPath), '手术刀必须真的删掉缓存文件');

        const rebuilt = await readLeaf('删缓存后读会话叶子');
        const rebuiltShape = shapeOf(rebuilt.messages);
        assertEq(
          rebuiltShape.join('\n'),
          afterShape.join('\n'),
          `删缓存后重建的历史应与变更后的缓存逐字一致：\n  缓存 ${JSON.stringify(afterShape)}\n  重建 ${JSON.stringify(rebuiltShape)}`,
        );
        // 三条核心断言各自成条（逐字一致已覆盖，但分开写让失败信息直接指出**哪种**
        // 变更没入格——回退与复活是两种不同的病）。
        assert(
          rebuiltShape.some((r) => r.includes(Q1_EDITED)),
          `重建后第一问应是**改过**的那句（否则编辑回退了）：${JSON.stringify(rebuiltShape)}`,
        );
        assert(
          !rebuiltShape.some((r) => r.includes(Q1) && !r.includes(Q1_EDITED)),
          `重建后不该还有编辑前的原文（实际: ${JSON.stringify(rebuiltShape)}）`,
        );
        assert(
          !rebuiltShape.some((r) => r.includes(Q2)) &&
            !rebuiltShape.some((r) => r.includes('第二答。')),
          `重建后被截断的那一轮不得复活（实际: ${JSON.stringify(rebuiltShape)}）`,
        );

        // ── 证据 ③：数据来自 WAL，不是偷偷重建了缓存（读路径无写副作用）────────
        assert(
          !existsSync(messagesPath),
          '读出口不得重建缓存——缓存缺失时读的是 WAL 重建结果，读路径不得有写副作用',
        );

        // ── 证据 ④：重建是纯函数（同事实源 ⇒ 连读两次逐字相同）────────────────
        const again = await readLeaf('再读一次会话叶子');
        assertEq(
          shapeOf(again.messages).join('\n'),
          rebuiltShape.join('\n'),
          '同一事实源连读两次应逐字相同（重建必须是纯函数）',
        );

        assertTranscriptInvariants(rebuilt.messages, 'T44 重建');
      } finally {
        cli.stop();
      }

      // 收尾复核：整条读路径跑完，缓存**始终没有被重建出来**。
      assert(
        !existsSync(messagesPath),
        '读完之后缓存仍不应存在（本用例的读全程走 WAL 重建，读路径无写副作用）',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t37 / t43 同一约定：Windows 上带活子进程退出
      // 会撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
