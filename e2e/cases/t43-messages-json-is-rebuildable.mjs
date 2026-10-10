import './_selfrun.mjs';
// T43 `messages.json` 是可重建的缓存（**S2.7 判据**，[ADR-049]）：
// 跑两轮真实对话 ⇒ 从生产读出口取一次会话叶子（基线）⇒ **删掉 `messages.json`** ⇒
// 再取一次 —— 会话仍能开，且历史逐字不变。
//
// ## 本用例钉的是什么
//
// 轨 S 的目标态是「`v2-events.wal` 是会话消息的**唯一事实源**，`messages.json` 降为
// 可重建的缓存」。这件事只有一种证明方式：**把缓存删掉，看会话还在不在**。判据是
// ADR-049 逐字写下的那一句——「删掉 `messages.json` 后会话仍能开且历史完整」。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 缓存删掉后会话**仍能开** | 生产读出口 `vdfs/read(<根>/session/<id>)` 仍回 200 |
// | 历史**完整**（不是"能开但空"） | 两轮的问答都在，且与删除前逐条对账 |
// | 数据真的来自 WAL，不是偷偷重建了缓存 | 读完之后 `messages.json` **仍不存在**（读路径无写副作用） |
// | 重建是**纯函数**（同事实源 ⇒ 同结果） | 连读两次，逐字相同 |
//
// ## 为什么不比 `seq` / `timestamp`
//
// ADR-049 已写明：切源后 `seq` 分配属**呈现细节**，重建按发射顺序另发一套号；时间戳
// 重建取事件自身的 `ts`（构造器取「此刻」会破坏纯函数性）。这两格因此**刻意不同**，
// 用例比的是其余每一格（id / parent_id / role / type / status / name / tool_call_id /
// 正文）——那些才是「历史」本身。id 相同是硬要求：前端按 id 合并节点，id 一变同一句
// 话在实时面与重建面就是两个身份。
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
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t43';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** `messages.json`（与 `plugins/session/store/mod.rs::MESSAGES_FILE` 同一常量口径） */
const MESSAGES_JSON = 'messages.json';

/** 两轮的触发串（mock 场景 `match` 与用户正文共用同一个词）。 */
const Q1 = '第一问';
const Q2 = '第二问';

/** 取消息的正文文本（与 helpers 的 `textOf` 同口径，就地复算以保持用例自洽）。 */
function contentText(m) {
  const c = m.content;
  if (c == null) return '';
  if (typeof c === 'string') return c;
  if (Array.isArray(c)) return c.map((p) => p.text ?? '').join('');
  return JSON.stringify(c);
}

/**
 * 消息的**可比形状**（剔除 ADR-049 判定的呈现细节 `seq` / `timestamp`）。
 *
 * 逐字段摊成一行——绕开 JSON 键序的假红，也让失败信息一眼看得出是哪一格变了。
 */
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
  'T43 缓存可重建：删掉 messages.json 后会话仍能开，历史与删除前逐字一致（ADR-049 判据）',
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
        // 生产读出口就打不到（与 T37 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 每轮收束把事实写进 `<会话目录>/v2-events.wal`——重建的事实源。
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

      // 前置：两样都在——缓存（本轮要删的）与事实源（删后顶上的）。
      assert(existsSync(messagesPath), `跑过两轮 ⇒ 缓存应在：${messagesPath}`);
      assert(existsSync(walPath), `跑过两轮 ⇒ 事实源应在：${walPath}`);
      const walEvents = readFileSyncSafe(walPath).split('\n').filter(Boolean).map((l) => JSON.parse(l));
      assert(walEvents.length > 0, '事实源应非空');
      const cachedBefore = readMessagesJson(hd.homedir, SID);
      assert(Array.isArray(cachedBefore) && cachedBefore.length > 0, '缓存里应有两轮的消息');

      // ── 证据 ①：删缓存前，从**生产读出口**取会话叶子（基线）────────────────
      const cli = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await cli.waitGatewayReady();

        /** 取根地址（运行期数据，不写死） */
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

        const before = await readLeaf('删缓存前读会话叶子');
        assertEq(before.id, SID, '叶子应自述会话 id');
        const beforeShape = shapeOf(before.messages);
        // 前置：基线里两轮问答都在（否则「删后仍完整」无从谈起）。
        assert(
          beforeShape.some((r) => r.includes(Q1)) &&
            beforeShape.some((r) => r.includes(Q2)) &&
            beforeShape.some((r) => r.includes('第一答。')) &&
            beforeShape.some((r) => r.includes('第二答。')),
          `基线应含两轮问答（实际: ${JSON.stringify(beforeShape)}）`,
        );

        // ── 证据 ②：**删掉缓存** ⇒ 会话仍能开，历史逐字不变 ────────────────────
        rmSync(messagesPath, { force: true });
        assert(!existsSync(messagesPath), '手术刀必须真的删掉缓存文件');

        const after = await readLeaf('删缓存后读会话叶子');
        const afterShape = shapeOf(after.messages);
        assertEq(
          afterShape.join('\n'),
          beforeShape.join('\n'),
          `删缓存后历史应与删除前逐字一致：\n  删除前 ${JSON.stringify(beforeShape)}\n  删除后 ${JSON.stringify(afterShape)}`,
        );
        // 两轮的正文真的都在（不是"能开但空表"）。
        assert(
          afterShape.some((r) => r.includes('第一答。')) && afterShape.some((r) => r.includes('第二答。')),
          `重建后两轮助手正文都应还在（实际: ${JSON.stringify(afterShape)}）`,
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
          afterShape.join('\n'),
          '同一事实源连读两次应逐字相同（重建必须是纯函数）',
        );

        // 转写不变量：重建结果也要满足全仓共享的落盘不变量（seq 严格递增、id 唯一、
        // 有调用必有结果）——「缓存」与「重建」在这一点上必须是同一份契约。
        assertTranscriptInvariants(after.messages, 'T43 重建');
      } finally {
        cli.stop();
      }

      // 收尾复核：整条读路径跑完，缓存**始终没有被重建出来**——这是「数据来自 WAL」
      // 的反证（若读路径偷偷写回缓存，这里就会又出现一个文件）。
      assert(
        !existsSync(messagesPath),
        '读完之后缓存仍不应存在（本用例的读全程走 WAL 重建，读路径无写副作用）',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t37 同一约定：Windows 上带活子进程退出会撞
      // libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
