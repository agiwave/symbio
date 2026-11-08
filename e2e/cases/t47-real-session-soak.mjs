import './_selfrun.mjs';
// T47 真实会话负载压出厂 `full` 路径：21 轮长会话（含一轮 45 步的工具长轮），判据是
// 「压过之后仍然成立的那些事」，不是「跑起来了」。
//
// ## 本用例钉的是什么
//
// [13 终点第 3 条](../../docs/archive/13-质量改进总纲.md)：「`full` 是出厂路径，且被
// 真实负载压过——不是『跑过 e2e』，是『跑过真实会话』」（登记条目，口径见
// [D-058](../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。此前的 43 例里最长的会话是 4 轮，
// 单轮最多 3 次工具调用；压缩、记忆召回各自有单点用例，但**没有任何一条
// 链路是在「会话已经很长」这个前提下一起跑过的**。本用例补的就是这个前提：
//
// | 真实负载的形状 | 本用例的取法 | 为什么这样取 |
// |---|---|---|
// | 轮次多到越过压缩水位线若干次 | 20 轮 ≈4.7K 字长回答 ＋ `max_context_tokens: 60000` | 触发线是**相对**的（0.7 × 窗口，`compact` 插件的 `COMPACT_AT`），而常驻量由请求视图的轮次窗口（出厂 `context_messages = 6`）决定——顶到水位线靠的是**最近几轮的体量**，真实会话正是这样（模型一次给几千字）。本用例让 mock 的用量按请求体估算（CJK ≈ 1 token/字），水位因此随会话变长而真的涨落：每 8~9 轮越线一次，整场压两次，最近一次之后仍留着尾巴（证据 ④ 要的后缀）。窗口大小不是判据，「触发过、触发后守住」才是（与 t8 / t18 同一取舍） |
// | 单轮里几十次工具调用 | 最后一轮连调 45 次工具 | 长 agentic 轮是 v2 的主用法。它同时钉住一个**实测形状**：交换**跨轮累加**，所以第 45 步的请求里 `role=tool` 有 45 条——模型看得见自己已经做过什么（这一条曾经是 1 条：交换每轮清空，长轮于是反复重读同一份材料，见 [t55](t55-multi-tool-round-memory.mjs)）。**长轮自身不设工具轮数上限**（[D-074](../../docs/003-design/014-单环一表.md "DE-014-单环一表")），这一格钉的是「累积」而不是「有界」 |
// | 工具结果有真实字节量 | 第 1 轮读一份 ≈14KB 的材料，长轮里读小文件 | 大结果**当时**完整落进库，之后从请求视图与 live 读面一起滑走——「裁的是带着跑的那一份，不是记过的那一份」这条契约只有两头都在才判得动 |
// | 会话会出故障 | 第 2 轮的应答按 `failTimes: 1` 打 500 | 真实会话不是一路成功；兜底也要给出一句可见的失败收束 |
// | 会话会被重新打开 | 全程跑完后**换进程**再读一遍 | 长跑会话第二天被打开是常态：事实源要还能整体读出，且读数不随重开漂移 |
//
// ## 这副负载压出来的两处缺陷（都修在 core，都有单点判据）
//
// 同一条边界在两个出口上各坏了一次：**压缩把本轮的发言原文留在快照载荷尾部，而那条
// 发言同时是本轮的开场格**（落在快照之后）。于是同一个节点身份有两条来源：
//
// | 出口 | 症状 | 修法 |
// |---|---|---|
// | `transcript` 投影（对话网格） | 每次压缩之后网格里多一条**重复的用户发言**（`rebuild` 那边只认一条——两个出口两种读法，且是静默的） | 换基边界按节点身份不重复产行（`projection/transcript.rs`，判据 `the_user_turn_a_snapshot_carries_is_not_emitted_twice`；本用例证据 ④ 的「一句不多」） |
// | `prompt_fidelity` 复核 | 同一条发言被**认领两次**，第二次找不到未被用掉的匹配 ⇒ 每次压缩后每轮一条「网格→prompt 丢事实」的**恒假警报**，真缺口淹在里面 | 同一条纪律：快照基线已占的节点身份不再提第二次主张（`projection/prompt_fidelity.rs`，判据 `the_user_turn_a_snapshot_carries_claims_only_once`；本用例证据 ⑥ 的零告警） |
//
// 两条都在**单点用例**里看不见：要真的压出一次压缩、且压的那一轮正好是带着发言继续跑的
// 一轮，边界才露头。这正是「跑过真实会话」相对「跑过 e2e」的增量。
//
// ## 本负载**没有**压到的：请求视图级的体积上界
//
// **那一层不存在**。请求组装期不改写历史正文——三层视图裁剪（内容淡化 / 轮次淡化 /
// 窗口骨架化）已按 [D-078](../../crates/symbio-plugin-session/README.md "crates/symbio-plugin-session/src/README") 整族退役，理由是它与「请求体 =
// 事实网格的纯投影」互斥（[D-048a](../../crates/symbio-plugin-session/README.md "crates/symbio-plugin-session/src/README")）。v2 唯一的体积界在
// **写入时**（L0 单条工具结果守卫，约 8K token/条）与**水位**（压缩 70%），两者都不在
// 请求视图里，所以本负载的字节量按设计由它们承担，不指望视图再收一道。
//
// 请求视图唯一会**变小**的时机是**换基**：压缩把一段前缀换成快照，模型请求从快照
// 起折（`session.consolidated` 的载荷就是那份整段消息列表）。于是证据 ⑤ 判的是换基的
// 后果（大结果不再出现在主请求里），**不判**任何条数上界——那会把「快照 + 本轮」
// 错当成窗口，也会拿摘要请求（它本来就要看全段）当主请求的样本。
//
// ## 为什么只有配置里的一项被改过
//
// `session` 侧只钉 `DIALOG_FACE_OFF`（本用例主题与对话面正交，理由见该常量的文档），
// **其余全取出厂默认**：`auto_compress` 等是真实会话跑的同一套旋钮。
// 负载要「真实」，配置就不能是为用例特调的。被改小的只有 provider 窗口——它是上面那格
// 说清的相对性取舍。
//
// ## 判据为什么读生产出口
//
// 七段断言全部走用户可见的出口：gateway `session/stats`（轮次账 / 事实账 / 不变量 / 转写）
// 与 `vdfs/read(<根>/session/<sid>)`（消息集合，前端加载历史走同一条地址），加 mock-llm
// 收到的**原样请求体**。会话的内部存储文件一个都不读——形状一改就假红的那种断言，
// 与「跑过真实会话」无关。
import { writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  nextPort,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  waitFor,
  startRealtimeSession,
  startReadbackCli,
  sendAndSettle,
  readSessionMessages,
  assertTranscriptInvariants,
  E2E_ROOT,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';
import { extractObservations } from '../../scripts/prompt-shape-audit.mjs';

const SID = 'e2e-t47';

/** 长轮的两段循环各跑多少步（内置工具 / MCP 工具交替，两种执行路径都要在）。 */
const LOOP_A_STEPS = 24;
const LOOP_B_STEPS = 20;
/** 最后一轮：单轮长工具轮。1（开始）+ 两段循环，45 步（v2 不设工具轮数上限） */
const DEEP_ROUNDS = 1 + LOOP_A_STEPS + LOOP_B_STEPS;
/** 正文轮数（第 1 轮带一次大工具结果，第 2 轮是故障轮） */
const NORMAL_TURNS = 20;
const TOTAL_TURNS = NORMAL_TURNS + 1;
/**
 * provider 窗口：水位线 = 0.7 × 本值。
 *
 * 取值口径（实测，不是猜的）：mock 的用量**按请求体估算**（`usageFromRequest`：
 * CJK ≈ 1 token/字），所以水位随会话变长真的涨落，而不是被固定的 `prompt_tokens: 64`
 * 钉死。请求视图的常驻量 = 系统提示 + 工具清单 + 最近 `context_messages = 6` 轮的正文；
 * 一轮长回答 ≈4.7K token，越过 0.7 × 60000 = 42000 需要约 9 轮 ⇒ 整场（20 个正文轮）
 * 压两次，且最近一次在收尾前若干轮——末尾几轮的发言因此留在网格里（证据 ④）。
 * 窗口值本身**不是**判据——判据是「压到了压缩、压过之后守住」（证据 ③）。
 */
const CONTEXT_WINDOW_TOKENS = 60000;

/** 第 1 轮读的那份材料：唯一首尾锚点用来证「整段原文进了存储」 */
const BIG_FILE = 'incident-notes.md';
const BIG_HEAD = '事故复盘〔T47-BIG-HEAD〕：网关侧 5xx 的退避节奏';
const BIG_TAIL = '事故复盘〔T47-BIG-TAIL〕：结论是视图裁剪、存储保留全文。';
/** 长轮里反复读的小文件（两条循环各读一份，参数静态 ⇒ 同轮里同一份内容重复进视图） */
const SMALL_FILES = ['ctx/loop-a.md', 'ctx/loop-b.json'];

/**
 * 正文轮的问题：中英混排、长度不一，含粘贴的日志与代码片段——真实会话的提问形状。
 *
 * **每条都必须是一行**（夹具陷阱，见 `e2e/README.md`）：REPL 的 stdin 是**按行**消费的，
 * 一条带 `\n` 的发言会被当成两轮，症状是「轮次账比设计的多出好几倍」——而这不是被测
 * 系统坏了。多行材料的字节量改由工具结果承担（`BIG_FILE`）。
 */
const TOPICS = [
  '帮我看下这段栈：panicked at indexer.rs:412，索引重建到第 3 万个条目时挂了，先查是不是越界。',
  '把 gateway 的入站配置写成一份注释齐全的 PLUGIN.yml 示例，默认只监听 127.0.0.1。',
  '下面这段日志里哪几条是退避重发？WARN retry 1/5 in 200ms / WARN retry 2/5 in 400ms / ERROR gave up after 5 attempts',
  'refactor 建议：`chat_loop` 里的收口点有三个，我想只留一个，说说会碰到哪些调用方。',
  '压缩阈值现在是 0.7，如果改成 0.6，对 128k 窗口的模型意味着多送多少 token 的摘要请求？',
  '解释一下 VDFS 的挂载序为什么必须稳定，给一个能被观测到的后果。',
  '这段 Rust 借用检查报错怎么破：cannot borrow `*turn` as mutable more than once。',
  '把「长轮里工具结果会累积进上下文」这件事写成三条给用户看的说明，别用内部术语。',
  '帮我核对一份清单：会话不变量八条分别是什么，每条会给用户造成什么故障。',
  '这个函数命名不合适：`process_data_v2_final`，给三个候选并说各自偏向。',
  '为什么 prompt 形状基线不该把字节数钉进判据？用一句话说清失效形态。',
  '写一段 200 字左右的中文说明：上下文压缩失败时为什么不能裁剪历史。',
];

/** 每轮的尾缀 marker：单行、唯一，既做证据 ④ 的出处锚点，也做 mock 场景的路由键。 */
const markerOf = (i) => `〔T47-A${String(i + 1).padStart(2, '0')}〕`;
const turnText = (i) => `${TOPICS[i % TOPICS.length]} ${markerOf(i)}`;

/**
 * 正文轮的回答：≈1000 字的一段真实长度回答。
 *
 * 负载的字节量必须落在**回答**上，这是由请求视图的形状决定的：轮次窗口只留最近
 * `context_messages = 6` 轮，历史轮的工具交换随整轮一起被窗口丢在外，窗口里常驻的是
 * 最近几轮的**正文**。于是「会话很长」要顶到水位线，靠的是最近几轮的正文体量——
 * 真实会话正是这样（模型一次给几千字）。短回答的会话压几十轮也只走到窗口的零头。
 * （长轮**自己**的交换是另一件事：它跨轮累加，45 步走到 45 条 `role=tool`，见证据 ⑤。）
 */
const LONG_ANSWER = Array.from(
  { length: 60 },
  (_, i) =>
    `${i + 1}、视图裁剪只发生在请求侧，存储恒为原文；这条契约的失效形态是模型看不见老结果而存储里还在，` +
    `于是同一会话的两次请求不再可比，压缩也无从判断自己压过了什么。`,
).join('');
const DEEP_TEXT = '把这两份材料逐份读完再给结论，中间不要停。〔T47-DEEP〕';

export default defineCase(
  'T47 真实会话负载：21 轮 + 单轮 45 次工具调用压出厂 full 路径，七段判据全部读生产出口',
  async () => {
    const llm = await new MockLlm([
      // 摘要请求单独一条：`matchSystem` 命中 compact 插件的摘要指令（对话线与主请求
      // 同形，只有 system 能区分两者），不占正文轮的场景名额（逻辑请求计数的口径见
      // `mock-llm.mjs` 文件头）。
      {
        id: 'snapshot',
        matchSystem: '你是上下文压缩器',
        content: [
          '<state_snapshot>',
          '  <completed_items>',
          '    - 已答：栈定位、配置示例、退避日志、收口点收敛。',
          '  </completed_items>',
          '  <in_progress_items>- 继续同一会话的后续提问。</in_progress_items>',
          '  <open_questions></open_questions>',
          '</state_snapshot>',
        ].join('\n'),
      },
      // 第 1 轮（按 marker 路由）：带一次大结果的工具调用。**路由必须按 marker 命中，
      // 不能靠「无 match 的排在前面」**——无 match 的场景是「先声明者先被消费」，于是
      // 循环启动器会被第一个正文轮吃掉（症状：长轮根本没跑，负载没压到机制）。
      {
        id: 'early-read',
        match: markerOf(0),
        content: '先把复盘材料读完整。',
        toolCalls: [{ id: 'call_big', name: 'vdfs_read', arguments: { path: BIG_FILE } }],
      },
      // 最后一轮：长工具轮的开始（同上，靠 marker 而不是靠顺序）。
      {
        id: 'deep-start',
        match: '〔T47-DEEP〕',
        content: '我逐份读完再答。',
        toolCalls: [{ id: 'call_d0', name: 'vdfs_read', arguments: { path: SMALL_FILES[0] } }],
      },
      // 正文轮的应答：没有任何 marker 命中的轮都走它；`failTimes: 1` ⇒ 第一个落到它
      // 上的逻辑请求（第 2 轮）失败 ⇒ 那一轮进兜底格。
      { id: 'soak-reply', failTimes: 1, content: LONG_ANSWER },
      // ── 工具结果轮（`afterTool` 池，先声明的先用）──
      { id: 'early-read-after', afterTool: true, once: true, content: '材料读完了，要点在三处。' },
      {
        id: 'deep-loop-a',
        afterTool: true,
        times: LOOP_A_STEPS,
        content: '继续。',
        toolCalls: [{ id: 'call_da', name: 'vdfs_read', arguments: { path: SMALL_FILES[0] } }],
      },
      {
        id: 'deep-loop-b',
        afterTool: true,
        times: 20,
        content: '再核一份。',
        // MCP 侧的调用：长轮里两种执行路径（内置 vdfs 工具 + 外部 MCP 工具）交替出现。
        toolCalls: [{ id: 'call_db', name: 'mcp__mockserv__echo', arguments: { text: 'loop-b' } }],
      },
      { id: 'deep-final', afterTool: true, content: '两份都读完了：差异只在裁剪时机。' },
    ], { usageFromRequest: true }).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [
        { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: CONTEXT_WINDOW_TOKENS }) },
      ],
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
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, 'mcp-record.ndjson')],
      enabled: true,
    });
    // 负载材料落在工作目录 = VDFS 根（`vdfs_read` 的相对路径按根解析，与 t3 同一地址口径）
    const bigBody = [
      BIG_HEAD,
      ...Array.from({ length: 90 }, (_, i) => `第 ${i + 1} 节：退避 200ms→400ms→800ms 后放弃，条目 ${'x'.repeat(64)}。`),
      BIG_TAIL,
    ].join('\n');
    writeFileSync(join(hd.workdir, BIG_FILE), bigBody, 'utf8');
    mkdirSync(join(hd.workdir, 'ctx'), { recursive: true });
    writeFileSync(join(hd.workdir, SMALL_FILES[0]), 'loop A：一次工具轮的材料。\n', 'utf8');
    writeFileSync(join(hd.workdir, SMALL_FILES[1]), '{"loop":"B","entries":[1,2,3]}\n', 'utf8');

    let rt = null;
    let reopened = null;
    try {
      rt = await startRealtimeSession({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      const { cli, sub } = rt;
      // 负载 CLI 的退出**事件**（证据 ⑦ 重开前要等它）。为什么不是 `proc.exitCode`：
      // Windows 下 `stop()` 走的是信号路径，退出码恒 `null`（信号在 `signalCode`），
      // 只盯 `exitCode` 会一直等到超时——进程其实当场就退了。
      let loadCliExited = false;
      cli.proc.once('exit', () => {
        loadCliExited = true;
      });

      // ── 跑负载：20 轮正文 + 1 轮 45 步工具轮 ─────────────────────────────
      for (let i = 0; i < NORMAL_TURNS; i++) {
        await sendAndSettle(cli, sub, turnText(i), { timeoutMs: 90_000 });
        // 每轮的用户可见收束：从生产读出口取「本轮用户消息之后」的节点——
        // 正常轮有一句完成正文，故障轮（第 2 轮）有失败节点。
        // 读出口可能比实时面晚一拍，waitFor 轮询到本轮自己的节点出现为止；
        // 以本轮用户消息为锚，避免上一轮的收束冒充本轮。
        await waitFor(
          async () => {
            const now = await readSessionMessages(cli, SID);
            const anchor = [...now]
              .reverse()
              .find(
                (m) =>
                  msgRoleOf(m) === 'user' &&
                  JSON.stringify(m).includes(turnText(i)),
              );
            if (!anchor) return false;
            const after = now.filter((m) => (m.seq ?? -1) > (anchor.seq ?? -1));
            return i === 1
              ? after.some((m) => m.status === 'failed')
              : after.some(
                  (m) =>
                    msgRoleOf(m) === 'assistant' &&
                    msgTypeOf(m) === 'text' &&
                    m.status === 'completed',
                );
          },
          { what: `第 ${i + 1} 轮的可见收束`, timeoutMs: 30_000 },
        );
        // 大结果**当时**完整进存储（不是 head/tail 之后的样子）——存储侧的全文锚点只在
        // 第 1 轮结束时取，后续压缩会归并消息集合，届时这一条已不代表「存储」。
        if (i === 0) {
          const leaf = await readSessionMessages(cli, SID);
          const joined = JSON.stringify(leaf ?? '');
          assert(joined.includes(BIG_HEAD) && joined.includes(BIG_TAIL), '大工具结果应整段进存储（首尾锚点都在）');
        }
      }
      await sendAndSettle(cli, sub, DEEP_TEXT, { timeoutMs: 120_000 });

      // ── 证据 ①：读数口的状态列（轮次明细不再数账，逐轮的可见收束见上面的负载循环）──
      const stats = (await cli.invoke('session/stats', {}, { session_id: SID })).body?.data;
      assert(stats, `session/stats 应带 data 载荷`);
      assert(stats.has_wal, `跑过 ${TOTAL_TURNS} 轮 ⇒ 事实源应在`);
      assertEq(
        stats.invariants.length,
        0,
        `负载下八条不变量全绿（实际: ${JSON.stringify(stats.invariants).slice(0, 600)}）`,
      );
      assertEq(stats.tiers.length, 1, '一档到底（本用例不换档）');

      // ── 证据 ②：事实账（累积量与动作逐一对得上）──────────────────────────
      const kinds = stats.checkpoint.kind_counts ?? {};
      // 产物格与动作逐一对得上：1 次早读 ＋ 长轮的 45 次工具调用都超出内联上限 ⇒ 各落一条。
      // 钉死而非 `>= 1`：这是「事实账随负载累积」唯一能核对的量，松成下界就退回收据。
      assertEq(
        Number(kinds['artifact.added'] ?? 0),
        1 + DEEP_ROUNDS,
        `产物格数 = 早读 1 次 + 长轮 ${DEEP_ROUNDS} 次（实际: ${kinds['artifact.added']}）`,
      );
      assert(
        stats.checkpoint.event_count > TOTAL_TURNS * 2,
        `事实总量应随负载累积（实际: ${stats.checkpoint.event_count}）`,
      );

      // ── 证据 ③：压缩真的被这副负载触发了多次 ─────────────────────────────
      //
      // 压缩的现行形态是**落格**（`session.consolidated`，`message × progressed`），
      // 不是实时节点——实时面上没有 compression 帧（t11 钉的是同一件事）。所以这里数
      // 事实账里的格数，再从重建历史里核对最近一次快照的形状：长跑里「压过、但压出来的
      // 东西读不回来」才是真的坏。
      const compactions = Number(stats.checkpoint.kind_counts?.['session.consolidated'] ?? 0);
      assert(
        compactions >= 2,
        `正文轮应至少越过水位线两次（session.consolidated 落格 ${compactions} 条；窗口=${CONTEXT_WINDOW_TOKENS}）`,
      );
      const leafForSnapshot = await readSessionMessages(cli, SID);
      const compNode = leafForSnapshot.find((m) => msgTypeOf(m) === 'compression');
      assert(
        compNode,
        `重建历史里应有最近一次压缩的快照节点（现有类型: ${leafForSnapshot.map((m) => msgTypeOf(m)).join(',')}）`,
      );
      assertEq(compNode.status, 'completed', '压缩快照节点应收在终态');
      assert(
        String(compNode.content ?? '').includes('state_snapshot'),
        `压缩快照正文应是摘要（实际首 60 字: ${String(compNode.content ?? '').slice(0, 60)}）`,
      );

      // ── 证据 ④：发言的**出处账**（终点第 1 条在负载下的正确形态）──────────
      //
      // 单点用例判「每一句用户发言都在网格里」。本用例的负载**必然**触发压缩，而压缩
      // 的语义就是把一段前缀换成一条快照（D-053 的「换基」）——在这里逐句点名会把正确
      // 行为判成故障。换成三条能对账的：
      //   a) **换基的痕迹在**：快照落格且重建可读（证据 ③）——对话网格只承载还能逐句
      //      对账的后缀，快照摘要节点是 `Notice`、不进网格（转写投影的规则见
      //      `symbio-sdk/src/projection/mutations.rs` 的 `snapshot_entries`）；
      //   b) **换基只吃前缀**：网格里剩下的发言是轮次序列的一段**连续后缀**——中间少一句
      //      就是真丢（后缀之外的那些在快照里，无从逐句核对，那是压缩的语义而不是缺陷）；
      //   c) **一句不多**：每个 marker 至多出现一次。换基边界的两条来源（快照载荷与
      //      紧随其后的轮次格）对同一个节点身份各产一行，投影里就会多一条重复发言。
      //      这一条只在真的压过压缩、且压过之后还有轮次时才有靶子，是 ④ 里唯一
      //      「不跑够轮次就看不见」的判据（边界去重见
      //      `symbio-sdk/src/projection/transcript.rs` 的 `superseded`）。
      const gridEntries = stats.transcript?.entries ?? [];
      assert(gridEntries.length > 0, '转写列应读出内容（对话面读侧）');
      const userGrid = gridEntries.filter((e) => e.role === 'user').map((e) => e.text ?? '');
      const hits = Array.from(
        { length: NORMAL_TURNS },
        (_, i) => userGrid.filter((t) => t.includes(markerOf(i))).length,
      );
      for (const [i, n] of hits.entries()) {
        assert(
          n <= 1,
          `第 ${i + 1} 轮的发言在网格里出现 ${n} 次——同一次发言两个出处，换基边界被产了两遍`,
        );
      }
      const firstKept = hits.findIndex((n) => n === 1);
      assert(firstKept >= 0, '网格里的用户格一条 marker 都不认得（读侧与写侧的正文不同源？）');
      const hole = hits.slice(firstKept).findIndex((n) => n === 0);
      assert(
        hole < 0,
        `被换基的应当是**前缀**，中间不该有洞：第 ${firstKept + hole + 1} 轮的发言两头都找不到`,
      );
      assert(
        userGrid.some((t) => t.includes('〔T47-DEEP〕')),
        '最新一轮（长轮）的发言必须在网格里——任何一次换基都不该吃掉它',
      );
      const leaf = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(leaf, 'T47 收尾');

      // ── 证据 ⑤：换基之后大结果不进请求、也不在读面，而存储一侧整段原文还在 ──
      const reqs = await llm.requests();
      // **不判体积上界**（这条曾经在这里，被删掉是对的，且有两层原因）：
      //   · v2 没有请求视图级的窗口上界（D-078 把「请求视图三层淡化」整族退役，见文件头
      //     「没有压到的」一节 / `D-318 B2`）：请求体只按事实网格投影走，能把它变小的只有
      //     **换基**（压缩把前缀换成快照），不是某个轮的条数上限。
      //   · 取 `max` 还会混进**摘要请求**：它本来就要看全段（那是压缩的输入），拿它的条数
      //     去断言「视图被按住」是拿错了样本（摘要请求的 system 就是压缩指令）。
      // 因此这里只判换基的**后果**：主请求里读不到那条大结果的原文，读面也一样。
      const isSummaryReq = (q) => {
        const system = (q.body?.messages ?? []).find((m) => m.role === 'system');
        return typeof system?.content === 'string' && system.content.includes('你是上下文压缩器');
      };
      const mainReqs = reqs.filter((q) => !isSummaryReq(q));
      const peak = Math.max(...extractObservations(reqs).map((o) => o.request_bytes));
      // 长轮**真的跑了几十步**：每次工具调用都会产生一个「视图末条是工具结果」的请求，
      // 于是请求序列就是步数账。事实网格在这里帮不上——它给工具轮立的是产物格（证据 ②）。
      const toolRoundReqs = mainReqs.filter(
        (q) => (q.body?.messages ?? []).at(-1)?.role === 'tool',
      ).length;
      assertEq(
        toolRoundReqs,
        1 + DEEP_ROUNDS,
        '工具结果回灌请求数 = 早读 1 次 + 长轮 45 次（少一次就是长轮没跑完）',
      );
      // 而长轮**自己的交换全带着**：45 步的第 45 个请求里有 45 条 `role=tool`。
      // 这条判据钉的是「模型看得见自己做过什么」——曾经是 1 条（交换每轮清空），
      // 症状就是长轮反复重读同一份材料（缺陷与单点判据见 [t55](t55-multi-tool-round-memory.mjs)）。
      const maxToolMsgs = Math.max(
        ...mainReqs.map((q) => (q.body?.messages ?? []).filter((m) => m.role === 'tool').length),
      );
      assert(
        maxToolMsgs >= DEEP_ROUNDS,
        `长轮的工具结果应累积到 ${DEEP_ROUNDS} 条（实测峰值 ${maxToolMsgs}）；少一条就是交换被清空、模型重新开局`,
      );
      // 大结果换基之后：请求里读不到它的原文。另一半在读面上（下面那条）。
      const lastView = JSON.stringify(mainReqs.at(-1)?.body?.messages ?? '');
      assert(
        !lastView.includes(BIG_TAIL),
        '第 1 轮的大结果应已随换基离开请求视图（还带着就是压缩只落了格、没有被请求侧消费）',
      );
      // 同一条契约在读面上的另一半：压过缩之后 live 段也该换基到快照，不再携带那条被
      // 取代的大结果（`consolidate_into_wal` 的归档）。留着它 = 「一次压缩在 prompt 面上
      // 等于没发生过」——那是本仓给 `transcript` 换基写的那条反面判据。
      const stored = await readSessionMessages(cli, SID);
      assert(
        !JSON.stringify(stored ?? '').includes(BIG_TAIL),
        `压缩后 live 读面应换基（工具结果 ${(stored ?? []).filter((m) => msgRoleOf(m) === 'tool').length} 条里还留着第 1 轮那条 = 换基没落到读面）`,
      );
      // 而**事实**一侧一条没少：产物格数在证据 ② 已钉死，重开后的 `event_count` 在证据 ⑦
      // 钉死。裁掉的是「带着跑的那一份」，不是「记过的那一份」。

      // ── 证据 ⑥：整场负载零保真告警 ───────────────────────────────────────
      //
      // 这条判据在负载下才现形：单点用例里压缩与复核各判各的都对，而**同一条发言被
      // 两条来源各认领一次**（快照载荷尾部 + 本轮开场格）只有跑够轮次、真的压过缩才
      // 触得到——症状是每次压缩之后每轮一条「网格→prompt 丢事实」的恒假警报。
      // 修在 `prompt_fidelity::verify` 的换基边界，与 ④ 钉的那条同源。
      const warns = (cli.out.stderr.match(/\[prompt-fidelity\]/g) ?? []).length;
      assertEq(warns, 0, `${TOTAL_TURNS} 轮里不得出现 prompt 保真告警（实际 ${warns} 条）`);

      // ── 证据 ⑦：换进程重开——长跑会话的读数不随重开漂移 ──────────────────
      const eventCountBefore = stats.checkpoint.event_count;
      cli.stop();
      // 等的是**这一个**子进程退出，不是「全局没有 ProcessWrap」：此刻 mock-llm（以及 CLI
      // 自己拉起的 mock-mcp）还在跑，全局探针永远非空——那是夹具的等待写错了对象，
      // 不是产品没释放 WAL 写者令牌。收尾（`finally`）才用全局那条，因为那时都该退了。
      //
      // 判据挂在 **exit 事件**上，不是 `proc.exitCode`：Windows 下 `kill()` 走信号路径，
      // 退出码恒为 `null`（信号在 `signalCode`），只盯 `exitCode` 会一直等到超时——
      // 进程其实当场就退了。
      await waitFor(() => loadCliExited, {
        what: '负载 CLI 退出（重开前要让 WAL 写者令牌释放）',
        timeoutMs: 30_000,
      });
      reopened = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        const stats2 = (await reopened.invoke('session/stats', {}, { session_id: SID })).body?.data;
        assertEq(stats2.checkpoint.event_count, eventCountBefore, '重开不新增也不丢事实');
        assertEq(stats2.invariants.length, 0, '重开后八条不变量仍全绿');
        const leaf2 = await readSessionMessages(reopened, SID);
        assertTranscriptInvariants(leaf2, 'T47 重开');
      } finally {
        reopened.stop();
      }

      // 观测行（不参与判定）：规模是负载的读数，两次运行之差才有意义。这里报的是
      // **本负载实际压到的形状**——主请求里的 user 发言数（换基之后回落，压缩的读数）
      // 与工具结果条数（长轮内累积）。两者都不经任何请求视图裁剪，见文件头。
      const viewUsers = Math.max(
        ...mainReqs.map((q) => (q.body?.messages ?? []).filter((m) => m.role === 'user').length),
      );
      console.log(
        `  负载观测：${TOTAL_TURNS} 轮 · ${reqs.length} 次模型请求 · 峰值请求体 ${(peak / 1024).toFixed(1)}KB` +
          ` · 压缩 ${compactions} 次 · 长轮 ${toolRoundReqs} 步` +
          ` · 视图峰值 ${viewUsers} 条发言 / ${maxToolMsgs} 条工具结果`,
      );
    } finally {
      // 三个句柄全在这里收（⑦ 里已经 stop 过的那次是幂等的）：漏掉哪一个，症状都是
      // 收尾等待超时——而失败信息会说「子进程没退」，看不出是哪一个。
      rt?.sub?.close?.();
      rt?.cli?.stop();
      reopened?.stop();
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm / mock-mcp 子进程退出',
      });
    }
  },
);
