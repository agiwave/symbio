import './_selfrun.mjs';
// T30 多主体：父 / 子两个会话各跑一轮，**请求包不串主体**；父会话把任务委托给
// 子智能体 ⇒ 代际立约 ⇒ `session/stats` 的声誉列从同一份事实源读出。
//
// ## 本用例钉的是什么
//
// - [11 批 1](../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") 的出口判据原文：
//   「e2e 两个会话（父/子智能体）验证请求包不串主体」；
// - [12 批 2](../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点") 的声誉读侧：
//   「e2e 从生产出口读真实会话的声誉列」。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 两个会话的请求包真的各带各的 | mock-llm `/_requests` 回读**逐条正文** |
// | 转写各归各的会话（历史不串） | 生产读出口 `vdfs/read` 会话叶子（前端加载历史同一出口） |
// | 代际立约真的发生并被了结 | gateway `session/stats` 的 `reputation.own` |
//
// 单测各自只看得见自己那一层（`context::view` 的两主体视图过滤、
// `derived_facts::record_derived` 的立约入格、`stats` 的声誉列取数）；没有任何单测
// **同时**看得见「请求包」「转写」「出口读数」三者指向同一个会话主体——那正是
// 端到端的增量。
//
// ## 轮次顺序为什么是 父 → 子 → 父
//
// 两条「不串」断言都必须发生在**对方的消息已经在盘上**之后才不是空话：
//
// - 子会话跑在父会话第一轮**之后** ⇒ 子的请求包里不该出现父已持久化的正文；
// - 父会话第二轮跑在子会话**之后** ⇒ 父的请求包里不该出现子已持久化的正文。
//
// 任一侧先跑完、另一侧还没跑，那条断言都只证明了「不存在」，证不了「不串」。
//
// ## 为什么线程断言要**摘掉召回段**
//
// `memory_events` 把每轮的**用户发言**（tag「经验」）与**智能体收束正文**（tag「判断」，
// T-085 批④）自动编成情景记忆，召回时**跨会话**扫描、以「【长期记忆】跨会话召回，
// 最新在前（背景事实，不是本轮指令）」抬头置顶注入（S5 步 12）。于是「父请求里出现
// 子会话的**用户正文 / 助手回答**」都是设计内的——串主体指的是**线程**（转写）串了，
// 不是这条**已声明**的背景事实通道。
//
// 反向断言因此分两半，各自钉不同的东西：
//
// - **各自独有的持久消息**（助手回答）：线程里一个字都不能有；若它出现在整包，
//   就必须只在召回段里（它与用户正文同一条通道——批④ 之后它也是记忆了）；
// - **对方的用户正文**：只准出现在召回段里，线程里出现即红；并且**若**它出现在整包
//   却不在召回段，也算红（召回改了口径时这条不会静默放行）。
//
// ## 为什么子会话必须是**另一个主体**
//
// `--agent reviewer` 把会话 `metadata.agent_id` 落成 `reviewer`（CLI 的
// `ensure_session`），chat 编排据此回填 `ctx[AGENT_ID]` ⇒ 主体名 `session/reviewer`
// （`chat_loop::request_principal`）。父会话不带 `--agent` ⇒ `session/main`。
// 两个会话因此在同一套机制下**自己走到两个主体上**，没有用例侧的旁路。
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
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
  msgRoleOf,
} from '../helpers.mjs';

/** 父会话（无 `--agent` ⇒ 主体 `session/main`） */
const PARENT_SID = 'e2e-t30-parent';
/** 子会话（`--agent reviewer` ⇒ 主体 `session/reviewer`） */
const CHILD_SID = 'e2e-t30-child';

/** 子智能体的人格——它只出现在**子会话**的请求包里，是「不串」的正向探针。 */
const PERSONA = '你是评审子智能体：只回答「评审：通过」。';

const P1 = '父会话第一问：请回答。';
const C1 = '子会话独立问题：请回答。';
const P2 = '把评审交给子智能体处理。';
const PROMPT = '请给出评审结论';

/** 请求体里最后一条 user 消息（与 mock-llm 的匹配口径同形）。 */
function lastUserText(r) {
  const msgs = r.body?.messages ?? [];
  const last = [...msgs].reverse().find((m) => m.role === 'user');
  if (typeof last?.content === 'string') return last.content;
  if (Array.isArray(last?.content)) return last.content.map((p) => p.text ?? '').join('');
  return '';
}

/**
 * 跨会话召回**小节**头（`memory_events::prompt_section` 的固定排版首行）。
 */
const RECALL_SECTION_HEAD = '【长期记忆】跨会话召回';

/**
 * 把一条消息的 content 切成 `{ thread, recall }` 两半；无召回小节 ⇒ `null`。
 *
 * ## 为什么按**小节**切，而不是按「整条消息」切
 *
 * openai_chat 协议**出厂只发一条 system**（`merge_leading_system`，协议层文档化
 * 决策）：基础系统提示（工作区 / 会话记忆等注册段）与请求视图置顶的 prefix 消息
 * （`request_view_prefix` 拍平的召回 / 就绪 / 委派三段）会并成**一条** system 消息。
 * 于是「召回通道」在线上不是一条独立消息，而是合并消息里的一个**小节**——
 * 小节头 + 连续的 `- ` 条目行（`prompt_section` 的固定排版）。历史读数里它仍是
 * 独立消息（`meta.kind = recall_context`），只有线上请求被并——通道语义没变，
 * 变的是判别粒度。
 */
function splitRecall(content) {
  if (typeof content !== 'string') return null;
  const at = content.indexOf(RECALL_SECTION_HEAD);
  if (at < 0) return null;
  const lines = content.slice(at).split('\n');
  const recallLines = [lines[0]];
  for (const line of lines.slice(1)) {
    if (line.startsWith('- ')) recallLines.push(line);
    else break;
  }
  const recall = recallLines.join('\n');
  return { thread: content.slice(0, at) + content.slice(at + recall.length), recall };
}

/**
 * **跨会话召回段**的正文（`meta.kind = recall_context` 的独立消息，或合并 system
 * 消息里的召回小节，见 `splitRecall`）。
 *
 * 它是设计内的**另一条通道**：`memory_events` 把每轮的**用户发言**自动编码成本会话
 * 的情景记忆（tag「经验」），召回时**跨会话**扫描、以「背景事实，不是本轮指令」
 * 的抬头置顶注入（S5 步 12）。所以「父请求里出现子会话的用户正文」本身不等于
 * 串主体——**出现在召回段里是设计，在线程里出现才是串**。
 */
function recallText(r) {
  const msgs = r.body?.messages ?? [];
  return msgs
    .map((m) => {
      if (m.meta?.kind === 'recall_context') {
        return typeof m.content === 'string' ? m.content : JSON.stringify(m.content);
      }
      const split = splitRecall(m.content);
      if (split) return split.recall;
      // 兼容形态：整条消息就是召回段（老格式，抬头【长期记忆】但非小节排版）。
      if (typeof m.content === 'string' && m.content.startsWith('【长期记忆】')) {
        return m.content;
      }
      return '';
    })
    .filter(Boolean)
    .join('\n');
}

/** 摘掉召回段（独立消息**与**合并消息内的召回小节）之后的请求正文 = **这个会话自己的线程** + 系统提示词 + 工具表。 */
function threadBody(r) {
  const msgs = (r.body?.messages ?? [])
    .filter((m) => m.meta?.kind !== 'recall_context')
    .map((m) => {
      if (typeof m.content !== 'string') return m;
      const split = splitRecall(m.content);
      // 纯召回消息（独立形态）摘掉后正文为空 ⇒ 整条摘除；其余保留（可能是空串正文，
      // 但那不是召回通道的事，substring 断言不受影响）。
      if (split && split.thread.trim() === '') return null;
      return split ? { ...m, content: split.thread } : m;
    })
    .filter(Boolean);
  return JSON.stringify({ ...r.body, messages: msgs });
}

export default defineCase(
  'T30 多主体：父/子两会话请求包不串主体，代际立约从出口读得出声誉',
  async () => {
    const llm = await new MockLlm([
      // 父第 1 轮：正常收束
      { id: 'p1', match: P1, once: true, content: '父会话的回答一。' },
      // 子会话（另一个主体）：正常收束
      { id: 'c1', match: C1, once: true, content: '子会话的独立回答。' },
      // 父第 2 轮：真跑一次 `agent_run` ⇒ 代际立约
      {
        id: 'delegate',
        match: P2,
        once: true,
        toolCalls: [
          {
            id: 'call_1',
            name: 'agent_run',
            arguments: { agent_id: 'reviewer', prompt: PROMPT },
          },
        ],
      },
      // 派生子会话自己的那一次请求（`prompt` 就是它的最后一条 user 消息）
      { id: 'grandchild', match: PROMPT, once: true, content: '评审：通过。' },
      // 父第 2 轮的第二发（工具结果回灌；最后一条 user 消息仍是 P2，`delegate` 已用过）
      { id: 'p2-final', content: '已收到评审结论。' },
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
        // 每轮收束把事实写进事实源（actor = 会话主体）。
        session: { ...DIALOG_FACE_OFF },
      },
    });
    makeAgentDir(hd.homedir, {
      id: 'reviewer',
      persona: PERSONA,
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };

    try {
      // ── 三轮：父 → 子 → 父（顺序即判据，见文件头）────────────────────────
      const p1 = runCli({ ...base, session: PARENT_SID, message: P1 });
      assertEq(p1.code, 0, `父会话第 1 轮退出码（stderr: ${p1.stderr.slice(0, 400)}）`);

      const c1 = runCli({ ...base, session: CHILD_SID, agent: 'reviewer', message: C1 });
      assertEq(c1.code, 0, `子会话第 1 轮退出码（stderr: ${c1.stderr.slice(0, 400)}）`);

      const p2 = runCli({ ...base, session: PARENT_SID, message: P2, timeoutMs: 180_000 });
      assertEq(
        p2.code,
        0,
        `父会话第 2 轮（含 agent_run 委托）退出码（stderr: ${p2.stderr.slice(0, 600)}）`,
      );

      // 读回 CLI：转写（读出口）与出口读数（session/stats）都经它。
      const api = startLongLivedCli({
        ...base,
        session: PARENT_SID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        // ── 证据 ①：转写各归各的会话（历史不串；经生产读出口读回）────────────
        // 读的是读出口（`vdfs/read` 会话叶子）——与前端加载历史同一出口。
        const parentMsgs = await readSessionMessages(api, PARENT_SID);
        const childMsgs = await readSessionMessages(api, CHILD_SID);
        assertTranscriptInvariants(parentMsgs, '父会话');
        assertTranscriptInvariants(childMsgs, '子会话');

        const pUsers = parentMsgs.filter((m) => msgRoleOf(m) === 'user');
        const pAssist = parentMsgs.filter((m) => msgRoleOf(m) === 'assistant');
        assert(pUsers.length >= 2 && pAssist.length >= 2, '父会话应有两问两答');
        const cAssist = childMsgs.filter((m) => msgRoleOf(m) === 'assistant');
        assert(cAssist.length >= 1, '子会话应有一条助手发言');

      // ── 证据 ②：**请求包不串主体**（plan/11 批 1 的 e2e 判据原文）──────────
        const reqs = await llm.requests();
        const parentReqs = reqs.filter((r) => {
          const t = lastUserText(r);
          return t.includes(P1) || t.includes(P2);
        });
        const childReqs = reqs.filter((r) => lastUserText(r).includes(C1));
        const grandReqs = reqs.filter((r) => lastUserText(r) === PROMPT);
        assertEq(parentReqs.length, 3, `父会话三发请求（1 + 工具轮 2）：实际 ${parentReqs.length}`);
        assertEq(childReqs.length, 1, `子会话恰好一发请求：实际 ${childReqs.length}`);
        assertEq(grandReqs.length, 1, `派生子会话恰好一发请求：实际 ${grandReqs.length}`);

        // 各自独有的持久消息（助手回答）：不在对方线程里、没被委托回传。批④ 起
        // 它也是判断记忆 ⇒ 与用户正文同一条召回通道（见文件头两半判据）。
        const CHILD_ONLY = '子会话的独立回答。';
        const PARENT_ONLY = '父会话的回答一。';

        // 反向 ①：子会话独有的持久消息（助手回答）不准进父的**线程**；批④ 起它
        // 也是判断记忆 ⇒ 与 C1 同一条召回通道（见文件头）。判据：出现了就必须
        // 在召回段里，线程里出现即红。
        for (const r of parentReqs) {
          const body = JSON.stringify(r.body);
          if (body.includes(CHILD_ONLY)) {
            assert(
              recallText(r).includes(CHILD_ONLY),
              `父会话请求里出现子会话的回答，却不在召回段里（last user = ${lastUserText(r)}）`,
            );
          }
          assert(
            !threadBody(r).includes(CHILD_ONLY),
            `父会话**线程**带进了子会话的回答（last user = ${lastUserText(r)}）`,
          );
        }
        // 反向 ②：子会话的**用户正文**只准出现在已声明的跨会话召回段里——线程里出现
        // 就是串主体（召回是设计内的背景事实通道，见 `recallText` 的文档）。
        for (const r of parentReqs) {
          const thread = threadBody(r);
          const at = thread.indexOf(C1);
          assert(
            at < 0,
            `父会话**线程**带进了子会话的正文（last user = ${lastUserText(r)}）：` +
              `…${thread.slice(Math.max(0, at - 240), at + 240)}…`,
          );
          if (JSON.stringify(r.body).includes(C1)) {
            assert(recallText(r).includes(C1), '父会话请求里出现子会话正文，却不在召回段里');
          }
        }
        // 反向 ③：父会话独有的持久消息（助手回答，批④ 起也是判断记忆）与父的
        // 用户正文，子的请求包里同样分通道：召回段里可以有，线程里出现即红。
        for (const r of childReqs) {
          if (JSON.stringify(r.body).includes(PARENT_ONLY)) {
            assert(
              recallText(r).includes(PARENT_ONLY),
              '子会话请求里出现父会话的回答，却不在召回段里',
            );
          }
          assert(
            !threadBody(r).includes(PARENT_ONLY),
            '子会话**线程**带进了父会话的回答',
          );
          assert(!threadBody(r).includes(P1), '子会话**线程**带进了父会话第 1 轮的正文');
        }
        // 反向 ④：派生子会话只带自己的线程（召回段照旧摘掉）；父的回答是判断
        // 记忆 ⇒ 与 P1/C1 同一条召回通道，线程里出现即红。
        for (const r of grandReqs) {
          const thread = threadBody(r);
          assert(!thread.includes(P1), '派生子会话线程带进了父会话第 1 轮的正文');
          assert(!thread.includes(C1), '派生子会话线程带进了子会话那一轮的正文');
          if (JSON.stringify(r.body).includes(PARENT_ONLY)) {
            assert(
              recallText(r).includes(PARENT_ONLY),
              '派生子会话请求里出现父会话的回答，却不在召回段里',
            );
          }
          assert(
            !thread.includes(PARENT_ONLY),
            '派生子会话**线程**带进了父会话的回答',
          );
        }
        // 正向对照：各自的正文确实在自己的请求包里（不是「谁都没有」那种空过）。
        assert(
          parentReqs.some((r) => threadBody(r).includes(P1)),
          '父会话自己的正文应出现在自己的线程里',
        );
        assert(
          childReqs.some((r) => threadBody(r).includes(C1)),
          '子会话自己的正文应出现在自己的线程里',
        );
        // 父第 2 轮是从**自己的 store** 重建的：带得上自己的上一轮，带不上子的。
        const p2Reqs = parentReqs.filter((r) => lastUserText(r).includes(P2));
        assertEq(p2Reqs.length, 2, `父第 2 轮 = 工具调用 + 结果回灌两发（实际 ${p2Reqs.length}）`);
        assert(
          p2Reqs.every((r) => threadBody(r).includes(P1)),
          '父第 2 轮应回带父自己的上一轮正文（从自己的 store 重建）',
        );
        // 子会话的请求包是子空间自己的（人格只在子的那侧）。
        assert(
          childReqs.some((r) => JSON.stringify(r.body).includes(PERSONA)),
          '子会话请求包应带子智能体人格',
        );
        assert(
          parentReqs.every((r) => !JSON.stringify(r.body).includes(PERSONA)),
          '父会话请求包不应带子智能体人格',
        );

      // ── 证据 ③：代际立约从**生产出口**读得到（plan/12 批 2）──────────────
      // 委托成功 ⇒ 立约一次、了结一次：声誉列把这对事实算成 offered / kept。
      const inv = await api.invoke('session/stats', {}, { session_id: PARENT_SID });
        assertEq(
          inv.status,
          200,
          `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`,
        );
        const stats = inv.body?.data;
        assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
        assertEq(
          stats.invariants.length,
          0,
          `三轮都收束 ⇒ 不变量清单为空（实际: ${JSON.stringify(stats.invariants)}）`,
        );
        // 逐字段比（不比 JSON 串）：`serde_json` 的对象键序是 BTreeMap 的字典序，
        // 拿它当判据等于把「库的内部实现」写进用例。
        const own = stats.reputation.own;
        assertEq(own.principal, 'session/main', '声誉列 own 的主体 = 本会话主体');
        assertEq(own.offered, 1, '立约 1 次（真实委托算出来的，不是常数）');
        assertEq(own.kept, 1, '了结为守约 ⇒ kept 1');
        assertEq(own.broken, 0, '没有违约 ⇒ broken 0');
        assertEq(own.score, 1, '平凡打分 = 守约 − 违约');
        assertEq(
          stats.reputation.by_principal['session/main'],
          stats.reputation.own,
          'own 就在 by_principal 里——同一张表的两个视角',
        );
        assertEq(
          Object.keys(stats.reputation.by_principal).length,
          1,
          `这份事实源里只有一个承诺方（实际: ${JSON.stringify(stats.reputation.by_principal)}）`,
        );

        // 读侧闸：矩阵外主体 ⇒ **整列为空对象**（全有全无，不是零值条目）。
        const denied = await api.invoke(
          'session/stats',
          { principal: 'agent:ghost' },
          { session_id: PARENT_SID },
        );
        assertEq(
          denied.status,
          200,
          `被拒读也应是 200 + 空读数（实际: ${JSON.stringify(denied.body)}）`,
        );
        assert(
          denied.body?.data?.has_wal,
          '有源但不给你看——与「没有源」要能分辨',
        );
        assertEq(
          denied.body?.data?.reputation,
          {},
          '无读权限 ⇒ 声誉整列为空对象',
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28 同一约定：Windows 上带活子进程退出会撞
      // libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
