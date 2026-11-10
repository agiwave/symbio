import './_selfrun.mjs';
// T87 分支切换：重试另开一支 → 两支都在树里 → 「读哪一支」是一条事实。
//
// ## 本用例钉的是什么
//
// [D-173](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的端到端半边。它把
// 「删除式重试」换成了「并存几支 + 一个选中态」，而这件事只有跑真轮次才看得见：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 重试**不删**旧支，两支都还在转写里 | 生产读出口 `vdfs/read(<会话叶子>)` |
// | 「此刻读哪一支」真的进了**模型**的请求包 | mock-llm 的 `/_requests` 逐轮请求体 |
// | 切换不开轮、不删历史 | 切换后的转写：骨架条数不变、两支答复都在 |
// | 切读法**不开轮**（用户不重挂载会话） | 同一地址再读一次：轮锚（用户消息节点）条数不变、成员 id 不变 |
//
// 口径本身（并组、组内后到胜出、`flat` 档升根的子孙也一起藏）已在 core 的
// `projection/branch.test.rs` 与会话层的 `vdfs_provider.test.rs` 钉死；本用例不复刻
// 判据，只对**真轮次产出的形状**取值。
//
// ## D-335：轮不是独立可见节点
//
// 一轮的可见锚是**用户消息节点**（其 id 即轮分组 id，`Skeleton{run}` 的 `meta.context`
// 并入它）。所以首轮的判据是「恰有一个用户消息节点」，重试/切换的成员根也是它。
// 没有独立 Turn 骨架，不再按 `type = skeleton` 数轮。
//
// ## 为什么重试与切换都经 gateway 节点动作，而不是 CLI stdin
//
// 两者的落点都是**那一支的轮骨架自身**（`<sid>/message/<root>`），与前端点箭头走的
// 同一条 `vdfs/action`（[D-066] 之后没有第二条恢复入口）。CLI 的 stdin 只能发用户
// 发言，发不出「对这条消息做点什么」。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 本用例逐轮读 mock 的**请求账本**（第 N 个请求体 = 第 N 幕），对话面会给同一幕多加
// 一次静默请求，把索引整体挪走。与对话面正交 ⇒ 钉死（理由见该常量）。
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  nextPort,
  startRealtimeSession,
  sendAndSettle,
  readSessionMessages,
  resumeSession,
  waitFor,
  settledAfterWorking,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgTypeOf,
  msgRoleOf,
  textOf,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t87';

/** 首轮的问题：重试那一幕与后续各幕都带着它（它是两支的**共同祖先**，不该被藏）。 */
const QUESTION = '这道题怎么解';

/** 两支的答复文本：只在本用例与 mock 场景表之间流通，断言拿它认「读的是哪一支」。 */
const ANSWER_A = '甲的答案';
const ANSWER_B = '乙的答案';

/** 后续两幕的发言（各带一个独词，好让 mock 场景表按最后一句 user 命中）。 */
const AFTER_B = '先按乙答的这支往下问';
const AFTER_A = '再按甲答的这支往下问';

/** 一支 = 带 `meta.branch` 标注的成员根节点（D-335：锚是用户消息节点，不是 Turn 骨架）。 */
const runsOf = (msgs) => (msgs ?? []).filter((m) => m.meta?.branch);

/** 整份转写的正文串（认「哪一支的答复还在树里」用）。 */
const corpusOf = (msgs) => msgs.map((m) => textOf(m)).join('\n');

/**
 * 请求包里的正文串（进模型的那一批，只在这里看得见）——**只取对话那一段**。
 *
 * 为什么滤掉 `role === "system"`：那一条是**请求级前缀**（长期记忆召回段、能力目录、
 * 委派真源），不是转写投影的产出。分支的读法住在转写那一条路上
 * （`projection::transcript` 折叠 `turn.branch`）；召回段**不在它的管辖内**——
 * `memory.encoded` 的载荷里没有来源节点身份，一条记忆说不出自己是哪一支产出的，
 * 所以「切走一支之后，那一支产出的记忆仍然被召回」是另一条缺口（登记在
 * [DE-036-长期记忆与语义检索](../../docs/003-design/036-长期记忆与语义检索.md) §8）。
 * 把它算进本用例的断言，钉的就不再是「读哪一支」而是「记忆有没有跟着分支一起失效」——
 * 那是另一件事、另一个期限，而且失败时报的是同一句「读到旧支」，会把人引到错的层去修。
 */
const promptOf = (body) =>
  (body?.messages ?? [])
    .filter((m) => m.role !== 'system')
    .map((m) => (typeof m.content === 'string' ? m.content : ''))
    .join('\n');

/**
 * 逐幕请求的形状（失败时的诊断行）。
 *
 * 「第 N 个请求体 = 第 N 幕」是本用例的索引前提，而它一旦不成立（对话面没钉死、
 * 跨轮重发、压缩多打了一次），失败会表现为「读错了某一支」——那是**假故障**。
 * 把每幕读到哪些标记词摊在报错里，一眼就能分辨是索引错位还是读法错了。
 */
const shapeOf = async (llm) => {
  const reqs = await llm.requests();
  return reqs
    .map((r, i) => {
      const p = promptOf(r.body);
      const mark = (w) => (p.includes(w) ? '✓' : '·');
      return `#${i}[问${mark(QUESTION)} 甲${mark(ANSWER_A)} 乙${mark(ANSWER_B)}]`;
    })
    .join(' ');
};

export default defineCase(
  'T87 分支切换：重试另开一支、两支都在树里，读法由事实决定且切换不删历史',
  async () => {
    const llm = await new MockLlm([
      // 第一幕：命中 `once` ⇒ 第二幕（重试）落到下面的兜底场景。
      // ⚠ 重试那一幕的**最后一句 user 仍是 QUESTION**（恢复不带新用户格），所以第一幕
      // 的标必须 `once`，否则两支拿到同一段正文，断言就认不出「哪一支是谁」了。
      { id: 'first', match: QUESTION, once: true, content: ANSWER_A },
      { id: 'after-b', match: AFTER_B, once: true, content: '收到，按乙答这支继续。' },
      { id: 'after-a', match: AFTER_A, once: true, content: '收到，按甲答这支继续。' },
      // 兜底：重试那一幕（第二幕）走这里 ⇒ 新支的答复是 ANSWER_B。
      { id: 'rest', content: ANSWER_B },
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
        ...dialogFaceOff(),
      },
    });

    let rt = null;
    try {
      rt = await startRealtimeSession({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      const { cli, sub } = rt;

      // ── 第一幕：正常一问一答（此时树上只有一支，没有任何分支标注）──────────
      await sendAndSettle(cli, sub, QUESTION);
      const msgs0 = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs0, 'T87 首支');
      const users0 = (msgs0 ?? []).filter((m) => msgRoleOf(m) === 'user');
      assertEq(users0.length, 1, `首轮应恰有一个用户消息节点（实得 ${users0.length}）`);
      const rootA = users0[0].id;
      assert(
        users0[0].meta?.branch === undefined,
        '没有第二次尝试 ⇒ 后端不盖分支标注（画出来的 `1/1` 是假的两支）',
      );

      // ── 重试：落点在**那一支的轮锚（用户消息节点）自身**，载荷没有新内容 ───────
      // 水位线要在受理**之前**取：重试是 spawn 执行，它的「在跑」帧可能在 `resumeSession`
      // 返回前就到（见下方等它跑完的那段）。
      const retryMark = sub.sessionChanges().length;
      const retry = await resumeSession(cli, SID, rootA, 'retry_subtree', {}, { session_id: SID });
      assert(
        retry.status === 200 && retry.body?.data?.ok === true,
        `retry_subtree 应被受理（实际: ${JSON.stringify(retry.body)}）`,
      );
      // 恢复是 spawn 执行 ⇒ 等第二支真的开出来（转写里出现第二个轮骨架）。
      await waitFor(
        async () => runsOf(await readSessionMessages(cli, SID)).length >= 2,
        { what: '重试开出的第二支进入转写', timeoutMs: 30_000 },
      );
      // **轮骨架出现 ≠ 那一轮跑完**：骨架在轮首就落格，模型调用与落格还在后面。
      // `sendAndSettle` 的判据是「水位线之后见过 working、且最后一个状态帧已离开
      // working」（无状态词的中间帧不算收尾，见 helpers 的 `settledAfterWorking`）——
      // 若重试那一轮此刻还在跑，它的收尾帧就会被下一幕的等待当成**自己的**收尾，
      // 于是下一幕在模型还没被叫到时就返回（并发 6 下稳定复现：第三幕实得 2 个请求，
      // 第二幕的请求就是第 2 个）。所以这里按实时面把重试轮等干净，再进下一幕。
      let retryStatuses = '(等待未通过)';
      await waitFor(
        () => {
          const sc = sub.sessionChanges().slice(retryMark);
          const ok = settledAfterWorking(sc);
          if (ok) retryStatuses = sc.map((c) => c.data?.status ?? '?').join('→');
          return ok;
        },
        { what: '重试那一轮真的跑完（实时面离开 working）', timeoutMs: 60_000 },
      );

      // ── 证据 ①：两支**都在树里**，标注把它们成对说出来 ─────────────────────
      const msgs1 = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs1, 'T87 两支');
      const runs1 = runsOf(msgs1);
      assertEq(runs1.length, 2, `重试后有两个轮骨架：旧支没被删（实得 ${runs1.length}）`);
      const [nodeA, nodeB] = runs1;
      assertEq(nodeA.id, rootA, '旧支还是同一个节点（id 稳定 = 切换器的锚）');
      const rootB = nodeB.id;
      for (const [m, index, selected] of [
        [nodeA, 0, false],
        [nodeB, 1, true],
      ]) {
        const br = m.meta?.branch;
        assert(br, `${m.id} 应带 meta.branch 标注（它是一支）`);
        assertEq(br.total, 2, `${m.id}: 组里两支`);
        assertEq(br.index, index, `${m.id}: 组内下标`);
        assertEq(
          br.selected,
          selected,
          `${m.id}: 应为${selected ? '当前读的那一支' : '被切走的候选'}`,
        );
        assertEq(br.candidates, [rootA, rootB], `${m.id}: 候选按转写顺序`);
      }
      const corpus1 = corpusOf(msgs1);
      assert(corpus1.includes(ANSWER_A), '旧支的答复仍在转写里（删掉它就等于没有重试过）');
      assert(corpus1.includes(ANSWER_B), '新支的答复也在');

      // ── 证据 ②：重试那一幕不再读它要替换的那一支 ───────────────────────────
      // 分支格落在 `prepare-turn`，而本幕的上下文在那之前就读了 ⇒ 剪掉被替换的那一支
      // 是**轮侧**的一刀（`chat_loop.rs` 步骤 1）。不剪，模型读到的正是用户不要的那份
      // 答复，表现是「重试了但答案一模一样」。
      const reqs2 = await llm.requests();
      assert(reqs2.length >= 2, `重试应真的打了一次模型（实得 ${reqs2.length} 个请求）`);
      const retryPrompt = promptOf(reqs2[1].body);
      assert(
        !retryPrompt.includes(ANSWER_A),
        `重试那一幕不该读到被它替换的旧支答复（各幕形状: ${await shapeOf(llm)}｜stderr: ${cli.out.stderr.slice(-1200)}）`,
      );
      assert(retryPrompt.includes(QUESTION), '共同祖先（用户那句问题）不藏：新支要知道问的是什么');

      // ── 证据 ③：读法进了模型 —— 第三幕只读选中支（乙）───────────────────────
      const mark3 = sub.sessionChanges().length;
      await sendAndSettle(cli, sub, AFTER_B);
      const reqs3 = await llm.requests();
      const statuses3 = sub
        .sessionChanges()
        .slice(mark3)
        .map((c) => c.data?.status ?? '?')
        .join('→');
      assert(
        reqs3.length >= 3,
        `第三幕应打了一次模型（实得 ${reqs3.length} 个请求｜第三幕状态帧: ${statuses3}｜重试等待通过时的帧: ${retryStatuses}｜各幕形状: ${await shapeOf(llm)}）`,
      );
      const prompt3 = promptOf(reqs3[2].body);
      assert(prompt3.includes(ANSWER_B), '选中支（乙）在请求包里');
      assert(!prompt3.includes(ANSWER_A), '未选中支（甲）连同它的子树不进模型');

      // ── 证据 ④：切换不开轮、不删历史 ──────────────────────────────────────
      const runsBeforeSwitch = runsOf(await readSessionMessages(cli, SID));

      const switched = await resumeSession(
        cli,
        SID,
        rootB,
        'switch_branch',
        { to: rootA },
        { session_id: SID },
      );
      assert(
        switched.status === 200 && switched.body?.data?.ok === true,
        `switch_branch 应被受理（实际: ${JSON.stringify(switched.body)}）`,
      );

      const msgs2 = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs2, 'T87 切换后');
      const runs2 = runsOf(msgs2);
      assertEq(
        runs2.length,
        runsBeforeSwitch.length,
        '切换不开轮（骨架条数与切换前逐条相同 ⇒ 用户不必重挂载会话）',
      );
      assertEq(runs2.find((m) => m.id === rootA).meta?.branch?.selected, true, '甲回到「读的那一支」');
      assertEq(runs2.find((m) => m.id === rootB).meta?.branch?.selected, false, '乙降为候选');
      const corpus2 = corpusOf(msgs2);
      assert(
        corpus2.includes(ANSWER_A) && corpus2.includes(ANSWER_B),
        '两支的答复都还在转写里（切回来的只是读法，不是重建历史）',
      );

      // ── 证据 ⑤：换读法之后，模型读的确实是另一支 ────────────────────────────
      await sendAndSettle(cli, sub, AFTER_A);
      const reqs4 = await llm.requests();
      assert(reqs4.length >= 4, `第四幕应打了一次模型（实得 ${reqs4.length} 个请求）`);
      const prompt4 = promptOf(reqs4[3].body);
      assert(prompt4.includes(ANSWER_A), '切回旧支后，甲的答复回到请求包里');
      assert(!prompt4.includes(ANSWER_B), '被切走的那一支（乙）不进模型');
    } finally {
      rt?.sub?.close();
      rt?.cli?.stop();
      llm.stop();
      cleanupHomedir(hd);
      // Windows 上带活子进程退出会撞 libuv 断言（见 t28 收尾）：等进程真的退干净
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
