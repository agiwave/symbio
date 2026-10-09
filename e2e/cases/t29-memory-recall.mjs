import './_selfrun.mjs';
// T29 记忆三段（S5 步 11–13）：编码 → 置顶注入 → 巩固，端到端各判各的。
//
// ## 本用例钉的是什么
//
// [plan/04 §3.1 批⑦](../../docs/plan/04-工程落地.md) 的接入判据：一轮用户发言 →
// `memory.encoded` 入事实源；**下一轮起**请求视图置顶注入记忆段；活记忆到 4 条 ⇒
// `memory.consolidated` + 两条 `memory.forgotten`；**新会话首轮**也能召回到别的
// 会话的记忆（跨会话）。
//
// 为什么必须端到端：单测（`v2_memory.test.rs` / `v2_bridge.test.rs`）把三段各自的
// 出口判据钉死了，但它们证不了三件只有这条链路能看见的事：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 记忆段真的进了**发给大模型的请求包** | mock-llm `/_requests` 回读（`messages` 开头） |
// | 注入是**请求级**的：不落库、不顶掉用户的问题 | 同一份 `messages.json` + 请求包末条 |
// | 巩固 / 检索真的发生在**真实轮次**上 | 真实 CLI 轮次写出的 `v2-events.wal` |
//
// ## 判据分列（一个规则一个判定方）
//
// - **编码与同文去重**：判定方是 core 的 `RecallView::contains_content`，本用例只
//   数条数（重复说一遍同一句话 ⇒ `memory.encoded` 不涨）；
// - **巩固放行**：判定方是 core 的 `consolidate::accept`（代数上界 + 保真度下界），
//   本用例只看它**放行之后的落格**（generation / fidelity / sources / 两条 forgotten）；
// - **注入位置**：抬头 `【长期记忆】` 只能出现在**开头**，且请求包末条仍是用户自己
//   的问题——置尾会把「最后一条 user 消息」换成记忆，mock 场景匹配与轮次窗口都会
//   读错；进系统提示词又撞上「唯一真源 = 注册段」的纪律（`plugins/session/README.md`）。
//
// ## 为什么 A 会话要跑 5 轮
//
// 4 条**互不相同**的记忆才凑得出巩固门槛，中间那 1 轮把 T1 的原话再说一遍：它是
// 「同文去重」的反例轮——不产生第 5 条记忆，只产生一次检索。轮序即断言序：
// 编码 → 注入 → 去重 → 凑齐 → 巩固。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  readFileSyncSafe,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** A / B 两个会话共用一个 homedir ⇒ 共用一个会话存储根，跨会话扫才扫得到。 */
const SID_A = 'e2e-t29-a';
const SID_B = 'e2e-t29-b';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 注入段抬头（与 `v2_memory.rs::RECALL_SECTION_HEAD` 同一常量口径） */
const HEAD = '【长期记忆】';

/** A 会话的四句互不相同的发言 + 一次重复 + B 会话的一句。 */
const T_COFFEE = '我喜欢喝不加糖的咖啡';
const T_MEET = '每周三下午不开会';
const T_REPORT = '周四要发周报摘要';
const T_WATER = '喝水要喝温的';
const T_HELLO = '你好，今天天气如何';

/** 读一个会话的全部事实（出口读数的对账基准）。 */
function readEvents(homedir, sid) {
  const raw = readFileSyncSafe(join(homedir, 'session', sid, V2_WAL));
  assert(raw.length > 0, `${sid} 的事实源应存在且非空`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

/** 请求包里一条消息的正文（string / 分片数组两种形态都认）。 */
function textOf(m) {
  if (typeof m?.content === 'string') return m.content;
  if (Array.isArray(m?.content)) return m.content.map((p) => p?.text ?? '').join('');
  return '';
}

/**
 * 请求包里带记忆抬头的那条（没有 ⇒ null）。
 *
 * ⚠️ **不能只认「以抬头开头的那一条消息」**——两种 prompt 形状下它落在不同的地方：
 *
 * - v1：记忆段是**独立的一条** user 消息，排在请求包**最前** ⇒ `startsWith(HEAD)` 成立；
 * - v2（`full` 档，出厂）：`ProviderLlmAdapter::generate_turn` 把整段请求渲染成
 *   **一条** user 消息（`transcript` 投影 + 就地追加的 exchange），记忆段被 `build_request_view`
 *   拼在这条消息的**开头** ⇒ 消息**本身**仍以 `HEAD` 开头，但它是**唯一**那条。
 *
 * 所以判据从「找以 HEAD 开头的消息」放宽成「找**含** HEAD 的消息」——两种形状都命中，
 * 而 `startsWith` 在 v2 下要求整条消息以 HEAD 开头，一旦渲染前面多出任何东西
 * （例如系统提示被并进来、或抬头前面加了别的段）就整条找不到，症状是
 * 「记忆段没注入」，指向却像记忆链路坏了。
 */
function recallMessage(req) {
  return (req?.body?.messages ?? []).find((m) => textOf(m).includes(HEAD)) ?? null;
}

/**
 * 请求包里**记忆段那一截**的文本（按 `HEAD` 起到下一个空行为止）。
 *
 * ## 为什么必须切段，不能拿整条消息断言
 *
 * 出厂档位（`full`）下发给模型的是**一条** user 消息：`chat_loop::request_view_prefix`
 * 把请求视图层置顶的三段拼成前缀，`Reasoner::render_prompt` 的对话转写接在它后面。
 * 同一条消息里因此同时有「记忆段 / 能力目录 / 对话历史 / 本轮发言」，而**对话历史
 * 里必然重放着同一批内容**（上一轮的收束与发言）——拿整条消息去数 `- ` 行或判
 * 「某条在不在」，数到的是合集、与注入段无关（实测 3 条记忆 + 6 行能力目录 = 9）。
 *
 * ## 切法：空行
 *
 * 三段之间由 `request_view_prefix` 用**空行**分隔（`parts.join("\n\n")`），而
 * `v2_memory::prompt_section` 产出的段内只有 `\n- ` 行、没有空行 ⇒ 首个空行就是本段
 * 结束。比按「下一个抬头」切更稳：抬头是可改的展示文案，空行是分隔约定。
 */
function recallSectionText(req) {
  const all = textOf(recallMessage(req));
  const at = all.indexOf(HEAD);
  if (at < 0) return '';
  const blank = all.indexOf('\n\n', at);
  return blank < 0 ? all.slice(at) : all.slice(at, blank);
}

export default defineCase(
  'T29 记忆三段：编码入格 → 置顶注入请求视图 → 跨会话召回与巩固',
  async () => {
    // 场景按**最后一条 user 消息**匹配（`mock-llm.mjs` 的规则）——记忆段置顶在
    // 开头，正因如此它不会把匹配读歪；末条兜底保证任何一轮都有收束。
    const llm = await new MockLlm([
      { id: 'a1', match: T_COFFEE, content: '好的，记住了。' },
      { id: 'a2', match: T_MEET, content: '明白，周三下午不安排会议。' },
      { id: 'a3', match: T_REPORT, content: '周报摘要会准时发出。' },
      { id: 'a4', match: T_WATER, content: '好，记得喝温水。' },
      { id: 'b1', match: T_HELLO, content: '你好，今天阳光不错。' },
      { id: 'rest', content: '收到。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // `bridge` 档：每轮收束转写进 `<会话目录>/v2-events.wal`——记忆写方只挂在这
        // 条路上（full 档的记忆写随 full 档启用，见 `v2_facts::record` 的注记）。
        // 对话面钉死：本用例按精确请求数下标断言，主题与对话面正交（见 `DIALOG_FACE_OFF`）。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'full' },
      },
    });

    try {
      const turn = (message, session) => {
        const r = runCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          message,
          provider: PROVIDER_ID,
          session,
        });
        assertEq(r.code, 0, `${session}「${message}」退出码（stderr: ${r.stderr.slice(0, 500)}）`);
      };

      // ── 轮 1：首轮没有可召回的记忆 ⇒ 只编码，不注入 ──────────────────────
      turn(T_COFFEE, SID_A);
      let a = readEvents(hd.homedir, SID_A);
      let encoded = a.filter((e) => e.kind === 'memory.encoded');
      assertEq(encoded.length, 1, '首轮 ⇒ 恰好一条记忆');
      const m1 = encoded[0];
      assertEq(m1.entity, 'Memory', '记忆格实体坐标');
      assertEq(m1.verb, 'Opened', '记忆格动词坐标（memory × opened）');
      assertEq(m1.payload.content, T_COFFEE, '记忆存的是本轮用户发言');
      assertEq(m1.payload.tag, '经验', '标签是七类认知内容之一（经验）——巩固按它分组');
      assertEq(m1.payload.generation, 0, '情景记忆从第 0 代起');
      assert(
        Array.isArray(m1.payload.vec) && m1.payload.vec.length === 0,
        `本批不写 embedding ⇒ vec 恒空（实际 ${JSON.stringify(m1.payload.vec)}）`,
      );
      assert(m1.produced_by != null, '记忆必带溯源（I2：覆盖 100%）');
      assert(m1.ts > 0, '记忆事件填真实编码时刻（RecallEntry::ts 的契约，跨会话新近度靠它）');

      // ── 轮 2：有了可召回的 ⇒ 置顶注入 + 检索事实入格 ─────────────────────
      turn(T_MEET, SID_A);
      a = readEvents(hd.homedir, SID_A);
      encoded = a.filter((e) => e.kind === 'memory.encoded');
      assertEq(
        encoded.length,
        2,
        `第二条互不相同的记忆（实际事实源: ${a
          .map((e) => `${e.event_id}=${e.payload?.text ?? e.payload?.content ?? ''}`)
          .join(' | ')}）`,
      );
      const recalled = a.find((e) => e.kind === 'memory.recalled');
      assert(recalled, '检索发生了 ⇒ 必须落成一条事实（不落 = 静默检索）');
      assertEq(recalled.entity, 'Memory', '检索事实实体坐标');
      assertEq(recalled.verb, 'Asserted', '检索事实动词坐标（memory × asserted）');
      const u2 = a.find((e) => e.kind === 'user.message' && e.turn === 1);
      assert(u2, '本轮用户格应在');
      assertEq(
        recalled.produced_by,
        u2.seq,
        '溯源锚 = 触发本次检索的那格用户发言（Translator 出事实，不造信息）',
      );
      assertEq(recalled.payload.found, 1, '本轮只召回得到轮 1 那一条');
      assert(
        String(recalled.payload.top ?? '').includes(T_COFFEE),
        `载荷 top 取自视图首条（实际 ${JSON.stringify(recalled.payload)}）`,
      );

      // ── 轮 3：把轮 1 的原话再说一遍 ⇒ 同文去重，记忆不涨，检索照常 ─────────
      turn(T_COFFEE, SID_A);
      a = readEvents(hd.homedir, SID_A);
      assertEq(
        a.filter((e) => e.kind === 'memory.encoded').length,
        2,
        '同一句话不再编码一条（判定方 = core 的 contains_content）',
      );
      assertEq(
        a.filter((e) => e.kind === 'memory.recalled').length,
        2,
        '去重只管写方：这轮照样检索过、照样入了事实',
      );

      // ── 轮 4 / 轮 5：第 4 条到齐 ⇒ 巩固自动触发（合并最旧两条）─────────────
      turn(T_REPORT, SID_A);
      a = readEvents(hd.homedir, SID_A);
      assertEq(
        a.filter((e) => e.kind === 'memory.encoded').length,
        3,
        '第 3 条记忆（3 条仍不足巩固门槛）',
      );
      assertEq(
        a.filter((e) => e.kind === 'memory.consolidated').length,
        0,
        '不到 4 条不触发（CONSOLIDATE_MIN_ENTRIES）',
      );

      turn(T_WATER, SID_A);
      a = readEvents(hd.homedir, SID_A);
      encoded = a.filter((e) => e.kind === 'memory.encoded');
      assertEq(encoded.length, 4, '第 4 条记忆到位');

      const consolidated = a.find((e) => e.kind === 'memory.consolidated');
      assert(consolidated, '第 4 条到齐 ⇒ 巩固必须触发（不触发 = 门槛没接上）');
      assertEq(consolidated.entity, 'Memory', '合并产物实体坐标');
      assertEq(consolidated.verb, 'Progressed', '合并产物动词坐标（memory × progressed）');
      assertEq(consolidated.payload.generation, 1, '首次合并代数 = 1');
      assertEq(consolidated.payload.fidelity, 1, '四条短句子装得下 ⇒ 保真度 1（过 0.7 下界）');
      assertEq(consolidated.payload.tag, '经验', '合并产物同标签（巩固按标签分组）');
      assertEq(
        (consolidated.payload.sources ?? []).length,
        2,
        '合并的是两条源记忆（可审计）',
      );
      assert(consolidated.produced_by != null, '合并产物带溯源（指向最旧那条源）');

      const forgotten = a.filter((e) => e.kind === 'memory.forgotten');
      assertEq(forgotten.length, 2, '两条源记忆各一条排除式遗忘格（Log 不删）');
      for (const f of forgotten) {
        assertEq(f.verb, 'Closed', '遗忘格动词坐标（memory × closed）');
        assertEq(f.payload.why, 'consolidated', '遗忘原因记账');
        assertEq(
          f.payload.into,
          consolidated.seq,
          '记下并进哪一条（可审计，失败方向不丢记忆）',
        );
        assert(f.produced_by != null, '遗忘格溯源到被遗忘的那条');
      }
      const forgottenContents = forgotten
        .map((f) => a.find((e) => e.seq === f.produced_by)?.payload?.content)
        .sort();
      assertEq(
        forgottenContents,
        [T_COFFEE, T_MEET].sort(),
        '合并的是**最旧两条**（轮 1 / 轮 2），不是随便两条',
      );

      // ── B 会话首轮：跨会话召回（读的全是 A 写下的记忆）────────────────────
      turn(T_HELLO, SID_B);
      const b = readEvents(hd.homedir, SID_B);
      assertEq(
        b.filter((e) => e.kind === 'memory.encoded').length,
        1,
        'B 会话自己也编码了首轮发言（写方只写自己的事实源）',
      );
      const bRecalled = b.find((e) => e.kind === 'memory.recalled');
      assert(bRecalled, 'B 首轮就该召回得到（跨会话扫 <会话存储根>/*/v2-events.wal）');
      assertEq(
        bRecalled.payload.found,
        3,
        'A 的三条活记忆：4 条 − 2 条被遗忘 + 1 条合并产物',
      );
      assertEq(
        b.filter((e) => e.kind === 'memory.consolidated').length,
        0,
        'B 只有 1 条记忆 ⇒ 不触发巩固',
      );

      // ── 请求回读：记忆段真的进了发给大模型的包，且不落库 ──────────────────
      const reqs = await llm.requests();
      assertEq(reqs.length, 6, 'A 五轮 + B 一轮，每轮一次请求（对话面已关，注入不加请求）');

      // 轮 1：没有可召回的 ⇒ 整段省略（不印空壳标题）。
      const r0 = recallMessage(reqs[0]);
      assert(
        r0 === null,
        `首轮不该有记忆段（实际: ${JSON.stringify(textOf(reqs[0]?.body?.messages?.[0]))}）`,
      );

      // 轮 2 起：记忆段**置顶**，且末条仍是用户自己的问题。
      for (const i of [1, 2, 3, 4]) {
        const head = recallMessage(reqs[i]);
        assert(head, `第 ${i + 1} 轮请求包应以记忆段开头`);
        // 请求级三段是**系统注入的读视图**，不是用户说的话 ⇒ `role: system`。
        //
        // 标成 `user` 有实测代价：e2e 的 mock 按「最后一条 user = 本轮用户发言」
        // 选场景，于是 prefix 顶掉用户原话 ⇒ 心跳轮（本来没有用户发言）场景匹配
        // 全落空 ⇒ t36 报「恰好一次触发，实得 2 次」。
        assertEq(
          head.role,
          'system',
          '记忆段是 role=system 的请求级消息——它是系统给的背景，不是用户说的话',
        );
        const msgs = reqs[i].body.messages;
        const roles = msgs.map((m) => `${m.role}:${textOf(m).slice(0, 12)}`).join(' | ');
        // 记忆段必须排在**第一条对话消息（user/assistant/tool）之前**。
        //
        // 判据钉的是「相对顺序」，不是下标：协议层会插一条真的 `system_prompt`、
        // 请求级 prefix 又是 `system`，所以「下标 0」这种绝对位置是无关细节。
        // 而「排在所有对话消息之前」正是本用例要防的东西——置尾会让 prefix 顶掉
        // 「最后一条 user 消息」，mock 的场景匹配与轮次窗口都会随之失真。
        const firstDialogAt = msgs.findIndex((m) =>
          ['user', 'assistant', 'tool'].includes(m.role),
        );
        assert(
          msgs.indexOf(head) < firstDialogAt,
          `记忆段必须排在所有对话消息之前（置尾会顶掉「最后一条 user 消息」），roles=${roles}`,
        );
        const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
        assert(
          textOf(lastUser).includes([T_COFFEE, T_MEET, T_COFFEE, T_REPORT, T_WATER][i]),
          `第 ${i + 1} 轮末条 user 仍是用户自己的问题（实际: ${textOf(lastUser)}）`,
        );
      }
      assert(
        textOf(recallMessage(reqs[1])).includes(T_COFFEE),
        '注入段带上召回得到的那条记忆',
      );

      // B 会话首轮：跨会话的三条活记忆排成三行，被遗忘的两条不再出现。
      const bHead = recallMessage(reqs[5]);
      assert(
        bHead,
        `B 首轮请求包应以记忆段开头（实际: ${JSON.stringify(reqs[5]?.body?.messages)}）`,
      );
      const section = textOf(bHead);
      // 只数**记忆段自己**的那些 `- ` 行：请求包在 `full` 档（出厂）下是**一条**
      // user 消息，同一条里还并列着委派者真源段（`【能力目录】` 下面也全是 `- ` 行）。
      // 所以必须按抬头切出记忆段那一截再数，否则数到的是「记忆 + 能力目录」的合集
      // （实测 3 条记忆 + 6 行能力目录 = 9）。这不是断言过时——记忆段照旧三条，
      // 只是它不再独占一条消息。
      const memSection = recallSectionText(reqs[5]);
      const lines = memSection.split('\n').filter((l) => l.startsWith('- '));
      assertEq(lines.length, 3, `跨会话召回三行（4 − 2 遗忘 + 1 合并），实际: ${memSection}`);
      assert(memSection.includes(T_WATER), `活记忆在段内（实际: ${memSection}）`);
      assert(memSection.includes(T_REPORT), `活记忆在段内（实际: ${memSection}）`);
      assert(
        section.includes(`${T_COFFEE}\n${T_MEET}`),
        `合并产物带两条源的原句（保真度 1 的落点，实际: ${section}）`,
      );

      // 请求级注入 ⇒ 存储里一个字都不该多（两份事实源就等于两个真源）。
      const msgsA = readMessagesJson(hd.homedir, SID_A);
      assert(
        !msgsA.some((m) => textOf(m).includes(HEAD)),
        '记忆段不得写进会话存储（请求级、与 nudge 同一机制）',
      );
      assertTranscriptInvariants(msgsA, 'T29(A)');
      assertTranscriptInvariants(readMessagesJson(hd.homedir, SID_B), 'T29(B)');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净：Windows 上带着活子进程 `process.exit()` 会撞
      // libuv 的 `uv_async_send` 断言（见 t28 同一段注释）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'mock-llm 子进程退出',
      });
    }
  },
);
