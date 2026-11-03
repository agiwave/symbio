import './_selfrun.mjs';
// T97 平凡值等价：同一夹具下，「已装但本幕无效果贡献」与「未安装」逐字等价（`D-293`）。
//
// ## 这条用例补的是什么
//
// T89 把「出现」与「停用后消失」钉住了，t96 把「重复投递只落一次」钉住了；本用例
// 补最后一格 **平凡值**：能力包装着、但本幕一次效果草稿都没交时，宿主不会调用写口，
// 会话的**稳定事件序列**与它根本没装时逐项相等——效果机制的引入不改变无效果会话。
//
// ## 两次运行差在哪、为什么这样比
//
// | | 装配 | 本幕 |
// |---|---|---|
// | A 无效果贡献 | notes / skill 有 `PLUGIN.yml`（真的被装载） | 不调便签工具、无技能可召回 |
// | B 未安装 | 没有这两个 `PLUGIN.yml` | 同一条发言 |
//
// A 的工具面里确实有插件注册的工具（模型看得到），但请求正文不进 WAL，工具面也不落
// 事实——所以「装了插件」本身不该在事件序列里留下痕迹。两次用**同一个 session id**、
// 同一份 provider 配置，差的只有两个装配位。
//
// 逐项比的是**稳定投影**：`kind` / `entity` / `verb` / `turn` / `payload`，顺序原样。
// 不比的只有两类随运行变化的读数：顶层 `seq` / 时钟，以及载荷里由运行期生成的节点
// 身份（`id` / `*_id`，哈希随机）。除这两类外的任何一格差异都是红——id 的随机性
// 不是「等价」的例外，而是等价判据必须剥离的噪声。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';
import { join } from 'node:path';

const SID = 'e2e-t97';

/**
 * 把运行期生成的身份与**计时读数**抹成占位符（`id` / `*_id` / `cost_ms`），其余逐字保留。
 *
 * `cost_ms` 是 `std::time::Instant` 的实测毫秒，两侧各跑一次 CLI 时会被调度噪声
 * 拉出 1ms 差——它不是「值等价」判据的一部分（判据看的是载荷里的语义字段）。
 */
const VOLATILE_KEYS = new Set(['cost_ms']);

function maskIds(value) {
  if (Array.isArray(value)) return value.map(maskIds);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(
    Object.entries(value).map(([k, v]) => [
      k,
      k === 'id' || k.endsWith('_id')
        ? '<id>'
        : VOLATILE_KEYS.has(k)
          ? '<volatile>'
          : maskIds(v),
    ]),
  );
}

/** 稳定投影：语义字段 + 掩码后的载荷；顺序即事件顺序。 */
const stable = (e) =>
  JSON.stringify({
    kind: e.kind,
    entity: e.entity,
    verb: e.verb,
    turn: e.turn,
    payload: maskIds(e.payload),
  });

async function runTurn({ installed }) {
  const llm = await new MockLlm([{ id: 'final', content: '收到。' }]).start();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: {
      session: { ...DIALOG_FACE_OFF },
      // 「已装」= 装配位存在（`PLUGIN.yml` 由 makeHomedir 写出）；效果面开着，
      // 但本幕不产生草稿。缺省这一项即「未安装」：没有装配位可装载。
      ...(installed ? { notes: {}, skill: {} } : {}),
    },
  });
  try {
    const r = runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      message: '你好',
      provider: PROVIDER_ID,
      session: SID,
    });
    assertEq(r.code, 0, `${installed ? '已装' : '未安装'}侧退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    const events = readFileSyncSafe(join(hd.homedir, 'session', SID, 'v2-events.wal'))
      .split('\n')
      .filter(Boolean)
      .map((l) => JSON.parse(l));
    return { stdout: r.stdout, events };
  } finally {
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: 'CLI 退出',
    });
  }
}

export default defineCase(
  'T97 平凡值：已装但无效果贡献 ≡ 未安装（stdout 与稳定事件序列逐项相等，D-293）',
  async () => {
    const installed = await runTurn({ installed: true });
    const absent = await runTurn({ installed: false });

    // 读数 ①：对用户可见的输出逐字相同。
    assertEq(installed.stdout, absent.stdout, '两侧 stdout 应逐字相同');

    // 读数 ②：序列非空且逐项相等（先立「确有会话事件」，再立等价）。
    const a = installed.events.map(stable);
    const b = absent.events.map(stable);
    assert(a.length > 0, '已装侧应有会话事件（空序列上的相等不算判据）');
    assertEq(a.length, b.length, `stable 事件条数（已装 ${a.length} / 未安装 ${b.length}）`);
    for (let i = 0; i < a.length; i++) {
      assertEq(a[i], b[i], `第 ${i} 条 stable 事件（装与不装只在装配位上有差）`);
    }

    // 读数 ③：「无效果贡献」是这次等价的**前提**，不是巧合——三条效果读数都不在。
    const kinds = installed.events.map((e) => e.kind);
    assert(!kinds.includes('notes.indexed'), `已装侧本幕不该有行落格（实际 kinds: ${JSON.stringify(kinds)}）`);
    assert(
      !installed.events.some((e) => e.kind === 'message.added'),
      `已装侧本幕不该有对话面草稿（实际 kinds: ${JSON.stringify(kinds)}）`,
    );
    assert(
      !installed.events.some((e) => e.kind === 'turn.stage' && e.payload?.stage === 'skill.recall'),
      `已装侧本幕不该有阶段效果（实际 stages: ${JSON.stringify(installed.events.filter((e) => e.kind === 'turn.stage').map((e) => e.payload?.stage))}）`,
    );
  },
);
