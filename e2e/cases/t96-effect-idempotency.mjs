import './_selfrun.mjs';
// T96 效果面重复投递：同一幂等键投两次只落一次（`D-293` 第三类判据）。
//
// ## 这条用例补的是什么
//
// T89 已把「效果出现」与「停用后效果面消失」钉在 notes 的对话面卡上。这里补的是
// **重复投递**那一格：能力行的动作按债触发，同一份阶段草稿可能被交两次；宿主的
// 唯一写口必须按幂等键把它落成**一格**事实。
//
// ## 为什么要两次技能行
//
// `skill.recall` 的债键是 `(recall, stage)`——本幕**每条**属于技能的召回条目各欠一笔。
// 连着两轮成功收束 ⇒ 编译出两条技能（t35 已证同一 trigger 至多一条，不同 trigger
// 两条），第三轮召回把两条都带进来 ⇒ 同一幕里这一行跑两次。两次都交
// `StageDraft{key:"st-t{turn}-skill.recall"}`（锚是宿主的轮号词干，不含贷键里的
// recall）⇒ 第二次撞同一事件 id。这正是生产里**真实存在**的重复投递，不是夹具伪造。
//
// ## 为什么读 WAL
//
// 阶段格是引擎内部事实，没有任何用户可见的读口能数「同一幕落了几格」。能读到的
// 只有两件事：本能力自己的观测格（`memory.calibrated` / `skill.answered` 各两条
// ⇒ 行确实跑了两次）与阶段格（一条 ⇒ 第二次被幂等键吸收）。判据因此落在
// 「行跑两次、格落一次」这对读数上；与 t89 停用侧的 WAL 断言同一口径。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  waitFor,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';
import { join } from 'node:path';

const SID = 'e2e-t96';

// 三条发言各编一条技能：前两条成功收束后被编译，第三条把两条都召回。
const TRIGGER_A = '把周报整理成摘要';
const REPLY_A = '先列要点；再合并同类项；最后按重要度排序。';
const TRIGGER_B = '把月报归档到共享盘';
const REPLY_B = '按月份建目录，再写索引。';
const TRIGGER_C = '帮我看看明天的安排';

export default defineCase(
  'T96 效果面重复投递：同一幂等键只落一格阶段事实（D-293）',
  async () => {
    const llm = await new MockLlm([
      // 顺序即优先级：后轮的 prompt 里含前轮原文（转写与召回注入），更具体的先排。
      { id: 'third', match: TRIGGER_C, content: '好的，我来看看。' },
      { id: 'second', match: TRIGGER_B, content: REPLY_B },
      { id: 'first', match: TRIGGER_A, content: REPLY_A },
      { id: 'rest', content: '收到。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 编译显式打开（默认 off，DE-041 §4 的平凡值）；快路不开 ⇒ 两条技能都走
        // 未命中侧，各交一格阶段草稿。
        session: { ...DIALOG_FACE_OFF, skill_compile_enabled: true },
      },
    });

    const turn = (message) => {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r.code, 0, `「${message}」退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    };

    try {
      turn(TRIGGER_A);
      turn(TRIGGER_B);
      turn(TRIGGER_C);

      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));
      // 轮号是「已有用户格数」⇒ 终点轮就是最后一轮的号，不靠文件名猜。
      const lastTurn = Math.max(...events.map((e) => e.scope ?? 0));
      const inTurn = (turnNo, kind) =>
        events.filter((e) => e.scope === turnNo && e.kind === kind);

      // ── 前置：第二次投递确实是**新的一笔债** ─────────────────────────────
      // 倒数第二轮只有前一条技能可召回 ⇒ 一行、一格。
      assertEq(
        inTurn(lastTurn - 1, 'memory.calibrated').length,
        1,
        '倒数第二轮只召回一条技能（否则「两条」不是新债，判据失效）',
      );

      // ── 读数 ①：这一行在本幕跑了两次 ────────────────────────────────────
      assertEq(
        inTurn(lastTurn, 'memory.calibrated').length,
        2,
        `最后一轮应召回并判过两条技能（实际 kinds: ${JSON.stringify(inTurn(lastTurn, 'memory.calibrated').map((e) => e.payload))}）`,
      );
      assertEq(inTurn(lastTurn, 'skill.answered').length, 2, '两条技能各落一条判定事实');

      // ── 读数 ②：两次投递只落一格阶段事实 ────────────────────────────────
      const stages = events.filter(
        (e) => e.scope === lastTurn && e.kind === 'turn.stage' && e.payload?.stage === 'skill.recall',
      );
      assertEq(
        stages.length,
        1,
        `同一幂等键只落一次（实际 ${stages.length} 格: ${JSON.stringify(stages.map((e) => e.event_id))}）`,
      );
      // 身份就是幂等键本身：第二次投递撞的正是这条 id。
      assertEq(stages[0].event_id, `st-t${lastTurn}-skill.recall`, '阶段格的幂等键由宿主编排');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
