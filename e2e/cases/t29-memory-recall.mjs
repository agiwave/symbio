import './_selfrun.mjs';
// T29 记忆三段（S5 步 11–13）：编码 → 置顶注入 → 巩固，端到端各判各的。
//
// ## 本用例钉的是什么
//
// [plan/04 §3.1 批⑦](../../docs/plan/PL-004-工程落地.md "PL-004-工程落地") 的接入判据：一轮用户发言 →
// 记忆入事实源；**下一轮起**请求视图置顶注入记忆段；活记忆到 4 条 ⇒ 巩固；
// **新会话首轮**也能召回到别的会话的记忆（跨会话）。
//
// 为什么必须端到端：单测（`v2_memory.test.rs` / `v2_facts.test.rs`）把三段各自的
// 出口判据钉死了，但它们证不了三件只有这条链路能看见的事：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 记忆段真的进了**发给大模型的请求包** | mock-llm `/_requests` 回读（`messages` 开头） |
// | 注入是**请求级**的：不落库、不顶掉用户的问题 | 读出口读回的转写 + 请求包末条 |
// | 巩固 / 检索真的发生在**真实轮次**上 | 真实 CLI 轮次 + 请求包里注入段的增减 |
//
// ## 判据分列（一个规则一个判定方）
//
// - **编码与同文去重**：同一句话再说一遍 ⇒ 注入段里那句仍只出现一次
//   （不编码第二条 ⇒ 没有第二条可注入）；
// - **巩固放行**：第 4 条记忆到齐 ⇒ 最早两条合并成一条（注入段里它带着两条原句），
//   被并掉的两条不再单独成行；
// - **注入位置**：抬头 `【长期记忆】` 只能出现在**开头**，且请求包末条仍是用户自己
//   的问题——置尾会把「最后一条 user 消息」换成记忆，mock 场景匹配与轮次窗口都会
//   读错；进系统提示词又撞上「唯一真源 = 注册段」的纪律（`plugins/session/README.md`）。
//
// ## 跨会话召回怎么判（本用例的落点）
//
// B 会话首轮的两面各自独立取证：**请求包**的注入段里带着 A 写下的那些记忆原句
// （读的是真实请求体，不是用例写死的字符串），**B 的首轮回答**里也含那些原句
// （转写经读出口读回）。两面合起来 = 「B 看得见 A 的记忆，且说出来了」。
//
// ## 为什么 A 会话要跑 5 轮
//
// 4 条**互不相同**的记忆才凑得出巩固门槛，中间那 1 轮把 T1 的原话再说一遍：它是
// 「同文去重」的反例轮。轮序即断言序：编码 → 注入 → 去重 → 凑齐 → 巩固。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  startReadbackCli,
  readSessionMessages,
  nextPort,
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

/** 注入段抬头（与 `v2_memory.rs::RECALL_SECTION_HEAD` 同一常量口径） */
const HEAD = '【长期记忆】';

/** A 会话的四句互不相同的发言 + 一次重复 + B 会话的一句。 */
const T_COFFEE = '我喜欢喝不加糖的咖啡';
const T_MEET = '每周三下午不开会';
const T_REPORT = '周四要发周报摘要';
const T_WATER = '喝水要喝温的';
const T_HELLO = '你好，今天天气如何';

/** B 会话首轮的答话：把跨会话召回得到的四句原话都说出来（对话内容面的召回判据）。 */
const B_ANSWER = `你好。我都记得：${T_COFFEE}；${T_MEET}；${T_REPORT}；${T_WATER}。`;

/** 请求包里一条消息的正文（string / 分片数组两种形态都认）。 */
function textOf(m) {
  if (typeof m?.content === 'string') return m.content;
  if (Array.isArray(m?.content)) return m.content.map((p) => p?.text ?? '').join('');
  return '';
}

/**
 * 请求包里带记忆抬头的那条（没有 ⇒ null）。
 *
 * ⚠️ **不能只认「以抬头开头的那一条消息」**——记忆段不独占一条消息：
 * `chat_loop::request_view_prefix` 把它与就绪段 / 委派者段拼进置顶的同一条前缀消息
 * （空行分隔），抬头在消息里的位置没有被钉死。
 *
 * 所以判据是「找**含** HEAD 的消息」——`startsWith` 要求那条消息**整个**以抬头开头，
 * 一旦渲染前面多出任何东西（例如系统提示被并进来、或抬头前面加了别的段）就整条
 * 找不到，症状是「记忆段没注入」，指向却像记忆链路坏了。
 */
function recallMessage(req) {
  return (req?.body?.messages ?? []).find((m) => textOf(m).includes(HEAD)) ?? null;
}

/**
 * 请求包里**记忆段那一截**的文本（按 `HEAD` 起到下一个空行为止）。
 *
 * ## 为什么必须切段，不能拿整条消息断言
 *
 * 发给模型的请求里，`chat_loop::request_view_prefix` 把请求视图层
 * 置顶的三段拼成**一条** `system` 消息（排在消息数组最前），其后是 `Reasoner::render_messages`
 * 投影出的对话转写与本轮发言。**转写里必然重放着同一批内容**（上一轮的收束与发言）——
 * 拿整个请求去数 `- ` 行或判「某条在不在」，数到的是合集、与注入段无关（实测 3 条记忆 +
 * 6 行能力目录 = 9）。所以必须先切出注入段那**一条**消息，再按空行分段。
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

/** 注入段里的记忆条目行（`- ` 开头；合并产物占一行，其两行原句跟在后面不带条目头） */
function memoryLines(req) {
  return recallSectionText(req)
    .split('\n')
    .filter((l) => l.startsWith('- '))
    .map((l) => l.slice(2));
}

export default defineCase(
  'T29 记忆三段：编码 → 置顶注入请求视图 → 跨会话召回（请求包 + 对话内容两面）',
  async () => {
    // 场景按**最后一条 user 消息**匹配（`mock-llm.mjs` 的规则）——记忆段置顶在
    // 开头，正因如此它不会把匹配读歪；末条兜底保证任何一轮都有收束。
    const llm = await new MockLlm([
      { id: 'a1', match: T_COFFEE, content: '好的，记住了。' },
      { id: 'a2', match: T_MEET, content: '明白，周三下午不安排会议。' },
      { id: 'a3', match: T_REPORT, content: '周报摘要会准时发出。' },
      { id: 'a4', match: T_WATER, content: '好，记得喝温水。' },
      { id: 'b1', match: T_HELLO, content: B_ANSWER },
      { id: 'rest', content: '收到。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 无此文件时网关回退默认配置（`inbound_enabled: false`）——读出口就打不到
        // （与 T28/T31 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 对话面钉死：本用例按精确请求数下标断言，主题与对话面正交（见 `DIALOG_FACE_OFF`）。
        session: { ...DIALOG_FACE_OFF },
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

      // 轮 1：首轮没有可召回的记忆 ⇒ 只编码，不注入
      turn(T_COFFEE, SID_A);
      // 轮 2：有了可召回的 ⇒ 置顶注入
      turn(T_MEET, SID_A);
      // 轮 3：把轮 1 的原话再说一遍 ⇒ 同文去重（不编码第二条 ⇒ 注入段里仍只有一句）
      turn(T_COFFEE, SID_A);
      // 轮 4 / 轮 5：第 4 条到齐 ⇒ 巩固（合并最早两条）
      turn(T_REPORT, SID_A);
      turn(T_WATER, SID_A);
      // B 会话首轮：跨会话召回（读的全是 A 写下的记忆）
      turn(T_HELLO, SID_B);

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

      // ── 同文去重（轮 3 把轮 1 的原话再说一遍）─────────────────────────────
      // 编码发生在**轮末**（`record_learning`），所以某轮请求包里看得见的是**此前
      // 各轮**已编码的记忆。轮 4（`reqs[3]`）的请求只看得见轮 1–3 的产物
      // （咖啡 + 周三 = 2 条，咖啡去重后仍 1 行）；到**轮 5**（`reqs[4]`）请求时
      // 轮 4 的周报也已入格 ⇒ 咖啡 / 周三 / 周报 = 3 条。
      //
      // 判据用 `reqs[4]` 而不是 `reqs[3]`：后者在轮 4 自己还没编码时就去数，
      // 天然差一条。这不是"放宽"，是把"记忆在什么时候可见"这件事摆到正确的位置。
      const lines4 = memoryLines(reqs[4]);
      assertEq(
        lines4.filter((l) => l.includes(T_COFFEE)).length,
        1,
        `同一句话不重复编码 ⇒ 注入段里它只出现一次（实际: ${JSON.stringify(lines4)}）`,
      );
      assertEq(
        lines4.length,
        3,
        `轮 5 请求应看得到三条记忆（咖啡 / 周三 / 周报；实际: ${JSON.stringify(lines4)}）`,
      );

      // ── 巩固（第 4 条到齐 ⇒ 合并最早两条）────────────────────────────────
      // B 会话首轮的注入段：合并产物带着两条原句、被并掉的两条不再单独成行；
      // 另两条（周报 / 温水）原样在段里。「摘技能 / 清空整段」这类退化都会在这里红。
      const bHead = recallMessage(reqs[5]);
      assert(
        bHead,
        `B 首轮请求包应以记忆段开头（实际: ${JSON.stringify(reqs[5]?.body?.messages)}）`,
      );
      const section = textOf(bHead);
      // 只数**记忆段自己**的那些 `- ` 行：记忆段不独占一条消息，同一条前缀里还
      // 并列着委派者真源段（`【能力目录】` 下面也全是 `- ` 行）。
      // 所以必须按抬头切出记忆段那一截再数，否则数到的是「记忆 + 能力目录」的合集
      // （实测 3 条记忆 + 6 行能力目录 = 9）。记忆段照旧三条，只是它不独占一条消息。
      const memSection = recallSectionText(reqs[5]);
      const lines = memSection.split('\n').filter((l) => l.startsWith('- '));
      assertEq(lines.length, 3, `跨会话召回三行（合并 + 周报 + 温水），实际: ${memSection}`);
      assert(memSection.includes(T_WATER), `活记忆在段内（实际: ${memSection}）`);
      assert(memSection.includes(T_REPORT), `活记忆在段内（实际: ${memSection}）`);
      assert(
        section.includes(`${T_COFFEE}\n${T_MEET}`),
        `合并产物带两条源的原句（保真度 1 的落点，实际: ${section}）`,
      );

      // ── 跨会话召回的**对话内容面**：B 的首轮回答里含 A 写下的四句原话 ──────
      // （转写经生产读出口读回，与前端加载历史同一出口。回答的原句与请求包注入段
      // 的原句是两面：注入段取自真实请求体，回答取自转写，互不为证。）
      const api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID_B,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        const msgsB = await readSessionMessages(api, SID_B);
        assertTranscriptInvariants(msgsB, 'T29(B)');
        // 取**有正文**的助手节点：Turn 根也是 `role = assistant`（组合节点，无正文），
        // `find` 按角色取会先撞上它。答案正文挂在 Turn 之下的 text 子节点上。
        const bAnswer = msgsB.find((m) => m.role === 'assistant' && textOf(m).trim().length > 0);
        assert(bAnswer, 'B 会话应有一条带正文的助手回答');
        for (const original of [T_COFFEE, T_MEET, T_REPORT, T_WATER]) {
          assert(
            textOf(bAnswer).includes(original),
            `B 的首轮回答应含 A 的记忆原句「${original}」（跨会话召回的对话面，实际: ${textOf(bAnswer)}）`,
          );
        }

        // 请求级注入 ⇒ 两份转写里一个字都不该多（请求级注入不落库）。
        const msgsA = await readSessionMessages(api, SID_A);
        assert(
          !msgsA.some((m) => textOf(m).includes(HEAD)),
          '记忆段不得写进会话转写（请求级、与 nudge 同一机制）',
        );
        assertTranscriptInvariants(msgsA, 'T29(A)');
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净：Windows 上带着活子进程 `process.exit()` 会撞
      // libuv 的 `uv_async_send` 断言（见 t28 同一段注释）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '读回 CLI / mock-llm 子进程退出',
      });
    }
  },
);
