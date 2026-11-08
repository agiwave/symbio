import './_selfrun.mjs';
// T82 定稿答话轮（快路）的记忆闭包：
//
// | 断言 | 缺陷形态（修复前） |
// |---|---|
// | 答话轮写下的经验能被新会话召回 | 快路直接返回，一轮记忆都不写 |
// | 新会话的完整轮请求包置顶注入这条记忆 | 没编码 ⇒ 没得召回 |
//
// ## 为什么必须端到端
//
// 单测只能钉 `record_learning` 本身；「`classify` 判 `direct` 的轮会不会走到它」
// 只有真实请求序列看得见。修复前实测（CLI a10）：用户说「请记住…」，默认配置下
// `classify` 判答话 ⇒ 快路直接返回，下一个会话的召回段是空的。
//
// ## 场景与请求数的对应
//
// A 会话一句「请记住…」：分类请求（`direct`）+ 措辞生成请求 = 2 次；
// B 会话一句提问：分类请求（`work`）+ 完整工具轮请求 = 2 次。共 4 次，
// 顺序固定（每轮先分类后干活），断言按请求下标取证。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

const SID_A = 'e2e-t82-a';
const SID_B = 'e2e-t82-b';

/** A 的发言：显式要求记住一件事（正是被误判成答话的那类输入）。 */
const T_MEMORY = '请记住：我的幸运数字是 7';
/** A 的答话（措辞产线产出，也是判断记忆的内容）。 */
const REPLY = '好的，我记住了：你的幸运数字是 7。';
/** B 的提问：当前对话里没有答案 ⇒ 分类必须落 `work`，完整轮才有召回段。 */
const T_RECALL = '我的幸运数字是多少？';

const HEAD = '【长期记忆】';

const textOf = (m) =>
  typeof m?.content === 'string' ? m.content : JSON.stringify(m?.content ?? '');

export default defineCase(
  'T82 定稿答话轮（快路）记忆闭包：经验可召回，跨会话复现',
  async () => {
    // 顺序 + `once`：分类请求先于措辞 / 完整轮请求，末条 user 都是同一句话。
    const llm = await new MockLlm([
      { id: 'cls-direct', match: T_MEMORY, content: 'direct', once: true },
      { id: 'compose-direct', match: T_MEMORY, content: REPLY },
      { id: 'cls-work', match: T_RECALL, content: 'work', once: true },
      { id: 'answer-recall', match: T_RECALL, content: '你的幸运数字是 7。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      // 对话面**显式开启**（本用例验的就是默认产品行为）；汇报关掉，避免多一次请求。
      pluginConfigs: {
        session: {
          classify_enabled: true,
          compose_enabled: true,
          progress_enabled: false,
        },
      },
    });

    try {
      const turn = async (message, session) => {
        const r = await runCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          message,
          provider: PROVIDER_ID,
          session,
        });
        assertEq(r.code, 0, `${session}「${message}」退出码（stderr: ${r.stderr.slice(0, 500)}）`);
        return r;
      };

      // ── ① A 会话：快路答话轮（写下的记忆能否召回，由 ② 的注入段判）──────────
      await turn(T_MEMORY, SID_A);

      // ── ② B 会话：完整轮，召回段应带 A 的经验 ────────────────────────────
      await turn(T_RECALL, SID_B);
      const reqs = await llm.requests();
      assertEq(
        reqs.length,
        4,
        `A（分类 + 措辞）+ B（分类 + 完整轮）= 4 次请求，实际 ${reqs.length}`,
      );

      const bMessages = reqs[3]?.body?.messages ?? [];
      const section = bMessages.find((m) => textOf(m).includes(HEAD));
      assert(
        section,
        `B 的完整轮请求应置顶注入记忆段（实际首条: ${JSON.stringify(textOf(bMessages[0]))}）`,
      );
      assert(
        textOf(section).includes(T_MEMORY),
        `召回段应带 A 写下的原话「${T_MEMORY}」（实际: ${textOf(section)}）`,
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
