import './_selfrun.mjs';
// T76 召回跨主体读侧预过滤（[T-117](../../docs/009-task/117-召回跨主体读侧预过滤批.md "T-117-召回跨主体读侧预过滤批")）：
// 召回段（【长期记忆】跨会话召回）按**内容侧可见域 + 本轮主体**收放——
//
// | 断言 | 判据来自 |
// |---|---|
// | A 的判断记忆（thread_private）不进 B 的召回段；A 的经验（shared）进 | `memory_events.test.rs`（单测钉判定函数，这里钉生产链） |
// | shared 的记忆对矩阵内主体可见（B = `agent:main`） | 同上 + `stats.test.rs`（同一份 `can_see` 判据） |
// | shared 的记忆对矩阵外主体不可见（C = `agent:ghost`，未登记） | 同 t73 的读侧口径 |
//
// ## 为什么矩阵外主体要真跑一轮
//
// 单测里 viewer 是个字符串参数；生产里它是 `request_principal(ctx)`——**本轮主体**。
// 只有真跑一轮才能证明接线取的是「谁在跑这一轮」而不是又一处硬编码：C 绑
// `--agent ghost`（agent 目录只是准入，授权表无登记行 ⇒ 矩阵外），它的召回段
// 必须看不见任何他人的 shared 记忆，**但看得见自己会话的**（own 判据与可见域
// 是两次独立收放）——所以 C 跑两轮：第一轮留下自己的经验，第二轮的召回段
// 「有自己的、没别人的」才是预过滤生效的完整形状。
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import { join } from 'node:path';

/** 会话 A：正常一轮 ⇒ 经验（shared）+ 判断（thread_private）各一条。 */
const SID_A = 'e2e-t76-a';
/** 会话 B：`agent:main`（矩阵内）⇒ 读得到 A 的经验、读不到 A 的判断。 */
const SID_B = 'e2e-t76-b';
/** 会话 C：`agent:ghost`（矩阵外）⇒ 连 A 的经验也读不到，自己的照旧。 */
const SID_C = 'e2e-t76-c';

/** A 的用户发言（⇒ 经验记忆，shared）。 */
const Q_A = '跨会话记忆提问';
/** A 的收束发言（⇒ 判断记忆，thread_private）——B / C 的召回段都不得出现。 */
const R_A = '这是 A 会话的收束结论，属本部署私有。';
/** B / C 各自的发言。 */
const Q_B = 'B 会话想看看邻居记了什么';
const Q_C1 = 'ghost 自身第一问';
const Q_C2 = 'ghost 自身第二问';
const R_C1 = 'ghost 第一轮的收束。';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径）。 */
const V2_WAL = 'v2-events.wal';
/** 注入段抬头（与 `memory_events.rs::RECALL_SECTION_HEAD` 同一常量口径）。 */
const HEAD = '【长期记忆】';

const textOf = (m) => (typeof m?.content === 'string' ? m.content : JSON.stringify(m?.content ?? ''));

/**
 * 请求包里**记忆段那一截**的文本（按抬头起到空行止）。
 *
 * 切段的理由与 t35 同名函数同源：注入段与对话转写各出现一次同样的内容，
 * 拿整条消息 `includes` 恒真；段内只有 `\n- ` 行，首个空行即段尾。
 * 没有注入段（无可召回）⇒ `''`。
 */
function recallSection(req) {
  const all = textOf((req?.body?.messages ?? []).find((m) => textOf(m).includes(HEAD)));
  const at = all.indexOf(HEAD);
  if (at < 0) return '';
  const blank = all.indexOf('\n\n', at);
  return blank < 0 ? all.slice(at) : all.slice(at, blank);
}

/** 读一份 WAL 文件的行（逐行 JSON）。 */
function walLines(path) {
  return readFileSyncSafe(path)
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

export default defineCase(
  'T76 召回可见域：判断记忆不出会话、shared 只给矩阵内主体（含矩阵外真跑一轮的反判据）',
  async () => {
    const llm = await new MockLlm([
      { id: 'a-final', match: Q_A, content: R_A },
      { id: 'b-final', match: Q_B, content: 'B 自己的收束。' },
      { id: 'c1-final', match: Q_C1, content: R_C1 },
      { id: 'c2-final', match: Q_C2, content: 'ghost 第二轮的收束。' },
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
    // ghost 智能体目录：`--agent` 绑定的目录准入（与 t73 的 reflex 同一硬约束）。
    // 目录只是准入；授权表没有 `agent:ghost` 的登记行 ⇒ 矩阵外主体。绑定登记
    // （ADR-100）只把**能力**随会话装配点绑来（ghost 轮照常跑），**可见域不登记**：
    // `sees_of` 仍为空 ⇒ `can_see` 对 shared 必拒 —— 这条正是本用例的反判据。
    makeAgentDir(hd.homedir, {
      id: 'ghost',
      name: '未登记智能体',
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const walA = join(hd.homedir, 'session', SID_A, V2_WAL);

    try {
      // ── ① 会话 A：一轮 ⇒ 经验（shared）+ 判断（thread_private）各一条 ──
      const a = runCli({ ...base, session: SID_A, message: Q_A });
      assertEq(a.code, 0, `会话 A 一轮退出码（stderr: ${a.stderr.slice(0, 400)}）`);

      // 产地核实（生产链第一格）：两条记忆真的带着各自的可见域落了盘。
      const encoded = walLines(walA).filter((e) => e.kind === 'memory.encoded');
      const scopeOf = (c) =>
        encoded.find((e) => e.payload?.content?.includes(c))?.payload?.vis_scope;
      assertEq(scopeOf(Q_A), 'shared', `用户发言 ⇒ 经验 ⇒ shared（实际: ${JSON.stringify(encoded)}）`);
      assertEq(scopeOf(R_A), 'thread_private', '收束发言 ⇒ 判断 ⇒ thread_private');

      // ── ② 会话 B（矩阵内）：召回段有 A 的经验、没有 A 的判断 ──
      const b = runCli({ ...base, session: SID_B, message: Q_B });
      assertEq(b.code, 0, `会话 B 一轮退出码（stderr: ${b.stderr.slice(0, 400)}）`);
      const reqsB = await llm.requests();
      const sectionB = recallSection(reqsB.at(-1));
      assert(sectionB.includes(HEAD), `B 的请求应有召回段（否则判据落空）：${sectionB}`);
      assert(
        sectionB.includes(Q_A),
        `矩阵内主体的召回段应含他人 shared 记忆：\n${sectionB}`,
      );
      assert(
        !sectionB.includes(R_A),
        `判断记忆不得跨会话可见（T-117 要消灭的泄漏）：\n${sectionB}`,
      );

      // ── ③ 会话 C（矩阵外）：两轮——第二轮的召回段有自己的、没别人的 ──
      const c1 = runCli({ ...base, session: SID_C, agent: 'ghost', message: Q_C1 });
      assertEq(c1.code, 0, `会话 C 第一轮退出码（stderr: ${c1.stderr.slice(0, 400)}）`);
      const c2 = runCli({ ...base, session: SID_C, agent: 'ghost', message: Q_C2 });
      assertEq(c2.code, 0, `会话 C 第二轮退出码（stderr: ${c2.stderr.slice(0, 400)}）`);
      const reqsC = await llm.requests();
      const sectionC = recallSection(reqsC.at(-1));
      assert(sectionC.includes(HEAD), `C 的请求应有召回段（自己的记忆在，判据才不是落空）：${sectionC}`);
      assert(
        sectionC.includes(Q_C1),
        `own 判据独立于可见域：本会话自己的经验照常召回（即使主体在矩阵外）：\n${sectionC}`,
      );
      assert(
        !sectionC.includes(Q_A),
        `矩阵外主体不得经召回读到他人 shared 记忆：\n${sectionC}`,
      );
      assert(
        !sectionC.includes(R_A),
        'A 的判断记忆对矩阵外主体同样不可见（两道收放叠加）',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
