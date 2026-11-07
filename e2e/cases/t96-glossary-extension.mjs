import './_selfrun.mjs';
// T96 横向扩展验收：`glossary`（术语表）是一个**新**能力，从零只经 plugin-api 接入。
//
// ## 这条用例补的是什么
//
// T89 证的是扩展面本身（`notes` 是夹具）；本用例证的是**同一条通路能接下一个项目里
// 原本不存在的能力**，且不需要改 core / kernel / loop / session / plugin-api：
//
// | 面 | 提交点 | 本用例的读数 |
// |---|---|---|
// | 工具 | `traverse` + `CapabilityRow::tool` | 模型请求的 `tools` 里出现 `glossary_define` |
// | 领域事实 | `submit_declarations!`（`Domain × Opened`） | 工具轮结束后 `glossary.defined` 真的落格 |
// | 效果 | `submit_behavior!` + `CapabilityMeta.behaviors` | 下一轮上下文里出现那条术语 |
// | 前置判断 | `ToolBehavior::before` | 同键同释义再定义一次，工具结果带回 `Refused: …` |
// | 上下文 | `submit_context_provider!` | 下一轮 prompt 里出现 `项目术语表（本会话已定义）` 段，同一个词只渲染最新释义 |
//
// 判据取模型请求里的**原文**（与用户看到的是同一条链路），不读 WAL 或内部投影。
// `项目术语表（本会话已定义）` 是 provider 自己的开场白，历史里不存在；用它当判据，
// 才能把「glossary provider 贡献了这一段」与「历史里恰好有用户说过这个术语」分开。
//
// ## 停用侧取什么
//
// 与 T89 同一口径：同一套夹具、同一类消息，只翻 `glossary` 的装配位。运行期停用拿掉
// 的是插件实例与它注册的工具；声明 / 行为 / provider 是链接期贡献，因此这里断言的
// 是「工具面消失 + 会话照常跑完 + 事实源没有新格」。
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

const TERM = '债';
const OLD_DEF = '义务环里尚未闭合的一笔义务';
const NEW_DEF = '必须由一条外部事实来清的一笔义务';
/** provider 的开场白（`glossary_context` 段的首行）——历史里不会出现这串字 */
const CONTEXT_HEAD = '项目术语表（本会话已定义）';

/**
 * mock 场景表：五轮共用一份。
 *
 * `times: 1` 不可省：后续轮次的请求带着历史原文（含「记住这个术语」），名额耗尽后
 * 它才会落到下一幕——否则同一幕会在下一轮再触发一次工具调用。
 */
function scenarios() {
  return [
    {
      id: 'g-define',
      match: '记住这个术语',
      times: 1,
      toolCalls: [
        {
          id: 'call_g1',
          name: 'glossary_define',
          arguments: { term: TERM, definition: OLD_DEF },
        },
      ],
    },
    { id: 'g-final', afterTool: true, times: 1, content: '术语已记下。' },
    { id: 'g-ask', match: '是什么意思', times: 1, content: '债是尚未闭合的一笔义务。' },
    {
      id: 'g-redefine',
      match: '改一下术语',
      times: 1,
      toolCalls: [
        {
          id: 'call_g2',
          name: 'glossary_define',
          arguments: { term: TERM, definition: NEW_DEF },
        },
      ],
    },
    { id: 'g-redefine-final', afterTool: true, times: 1, content: '释义已更新。' },
    { id: 'g-ask2', match: '现在的释义', times: 1, content: `现在的释义是：${NEW_DEF}` },
    {
      id: 'g-dup',
      match: '再定义一遍',
      times: 1,
      toolCalls: [
        {
          id: 'call_g3',
          name: 'glossary_define',
          arguments: { term: TERM, definition: NEW_DEF },
        },
      ],
    },
    { id: 'g-dup-final', afterTool: true, times: 1, content: '这个释义已经记过了。' },
  ];
}

function homedirFor(port, { glossaryEnabled }) {
  return makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
      session: { ...DIALOG_FACE_OFF },
      // 启用侧不写这一项：缺省即启用（`PluginDir::enabled` 的约定）。
      ...(glossaryEnabled ? {} : { glossary: { plugin_enabled: false } }),
    },
  });
}

const toolNames = (body) =>
  (body.tools ?? []).map((t) => t.function?.name).filter(Boolean);

/** 请求体全文：注入段拼在**系统提示词**里，只看最后一条 user 会读成「没有注入」。 */
function requestText(body) {
  return (body.messages ?? []).map((m) => String(m.content ?? '')).join('\n');
}

/** 术语表段的渲染行：`- 债：<释义>`。段外的用户发言里不会出现这个形状。 */
const renderedLine = (definition) => `- ${TERM}：${definition}`;

export default defineCase(
  'T96 横向扩展验收：新能力 glossary 只经 plugin-api 接入——工具 / 领域事实 / 效果 / 前置判断 / 上下文在真实链路生效',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port, { glossaryEnabled: true });
    try {
      // ── 第一轮：模型调工具 ⇒ 事实落格 ───────────────────────────────
      const first = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: `帮我记住这个术语：${TERM} = ${OLD_DEF}`,
        provider: PROVIDER_ID,
        session: 'e2e-t96',
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 400)}）`);
      assert(
        first.stdout.includes('术语已记下。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(first.stdout)}）`,
      );

      const afterFirst = await llm.requests();
      assertEq(afterFirst.length, 2, `工具轮 + 收尾轮两次请求，实际 ${afterFirst.length}`);
      const tools = toolNames(afterFirst[0].body);
      assert(
        tools.includes('glossary_define'),
        `新能力注册的工具应进模型工具面（实际: ${tools.join(',')}）`,
      );

      // ── 第二轮：provider 把落格的术语渲染进下一轮 prompt ─────────────
      const second = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: `「${TERM}」是什么意思？`,
        provider: PROVIDER_ID,
        session: 'e2e-t96',
      });
      assertEq(second.code, 0, `第二轮 CLI 退出码（stderr: ${second.stderr.slice(0, 400)}）`);
      const afterSecond = await llm.requests();
      assertEq(
        afterSecond.length,
        afterFirst.length + 1,
        `第二轮应只有一次模型请求（实际新增 ${afterSecond.length - afterFirst.length} 次）`,
      );
      const firstPrompt = requestText(afterSecond[afterSecond.length - 1].body);
      assert(
        firstPrompt.includes(CONTEXT_HEAD),
        `第二轮 prompt 应含术语表上下文段（实际: ${firstPrompt.slice(0, 800)}）`,
      );
      assert(
        firstPrompt.includes(renderedLine(OLD_DEF)),
        `第二轮 prompt 的术语表段应渲染出行为落下的那条释义（实际: ${firstPrompt.slice(0, 800)}）`,
      );
      assertEq(
        firstPrompt.split(renderedLine(OLD_DEF)).length - 1,
        1,
        `同一个词只渲染一条（实际: ${firstPrompt.slice(0, 800)}）`,
      );

      // ── 第三轮：同一个词换一条释义 ⇒ 放行，并覆盖旧值 ───────────────
      const third = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: `改一下术语：${TERM} = ${NEW_DEF}`,
        provider: PROVIDER_ID,
        session: 'e2e-t96',
      });
      assertEq(third.code, 0, `第三轮 CLI 退出码（stderr: ${third.stderr.slice(0, 400)}）`);
      assert(
        third.stdout.includes('释义已更新。'),
        `第三轮 stdout 应为收尾正文（实际: ${JSON.stringify(third.stdout)}）`,
      );

      // ── 第四轮：渲染的是最新释义，不是第一条 ─────────────────────────
      const fourth = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: `现在的释义是什么？`,
        provider: PROVIDER_ID,
        session: 'e2e-t96',
      });
      assertEq(fourth.code, 0, `第四轮 CLI 退出码（stderr: ${fourth.stderr.slice(0, 400)}）`);
      const afterFourth = await llm.requests();
      const secondPrompt = requestText(afterFourth[afterFourth.length - 1].body);
      assert(
        secondPrompt.includes(renderedLine(NEW_DEF)),
        `第四轮 prompt 应渲染最新释义（实际: ${secondPrompt.slice(0, 1200)}）`,
      );
      assertEq(
        secondPrompt.split(renderedLine(NEW_DEF)).length - 1,
        1,
        `同一个词仍只渲染一条（实际: ${secondPrompt.slice(0, 1200)}）`,
      );

      // ── 第五轮：同键同释义再定义 ⇒ before 在执行前拒 ─────────────────
      const fifth = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: `再定义一遍：${TERM} = ${NEW_DEF}`,
        provider: PROVIDER_ID,
        session: 'e2e-t96',
      });
      assertEq(fifth.code, 0, `第五轮 CLI 退出码（stderr: ${fifth.stderr.slice(0, 400)}）`);
      assert(
        fifth.stdout.includes('这个释义已经记过了。'),
        `第五轮 stdout 应为收尾正文（实际: ${JSON.stringify(fifth.stdout)}）`,
      );
      const afterFifth = await llm.requests();
      assertEq(
        afterFifth.length,
        afterFourth.length + 2,
        `第五轮应是工具轮 + 收尾轮两次请求（实际新增 ${afterFifth.length - afterFourth.length} 次）`,
      );
      const judged = requestText(afterFifth[afterFifth.length - 1].body);
      assert(
        judged.includes('Refused: 这个术语已经用同样的释义定义过'),
        `重复释义应在执行前被拒并回灌模型（实际: ${judged.slice(-800)}）`,
      );

      // ── 事实源读数：五轮里只有前两笔成功定义落格 ─────────────────────
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', 'e2e-t96', 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));
      const defined = events.filter((e) => e.kind === 'glossary.defined');
      assertEq(
        defined.length,
        2,
        `只有两次成功定义落格（第五轮被 before 拒）：${JSON.stringify(defined.map((e) => e.payload))}`,
      );
      assertEq(defined[0]?.payload?.term, TERM, '第一条定义的术语名');
      assertEq(defined[0]?.payload?.definition, OLD_DEF, '第一条定义是旧释义');
      assertEq(defined[1]?.payload?.definition, NEW_DEF, '第二条定义是新释义（覆盖）');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }

    // ── 停用侧：装配位翻掉后工具面没有它，会话照常跑完 ─────────────────
    const offLlm = await new MockLlm([{ id: 'final', content: '收到。' }]).start();
    const offHd = homedirFor(offLlm.port, { glossaryEnabled: false });
    try {
      const r = await runCli({
        homedir: offHd.homedir,
        workdir: offHd.workdir,
        message: '你好',
        provider: PROVIDER_ID,
        session: 'e2e-t96-off',
      });
      assertEq(r.code, 0, `停用 glossary 后 CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      assert(
        r.stdout.includes('收到。'),
        `停用 glossary 不该让会话装载失败（实际: ${JSON.stringify(r.stdout)}）`,
      );

      const reqs = await offLlm.requests();
      assertEq(reqs.length, 1, `一轮对话应只有一次模型请求，实际 ${reqs.length}`);
      const tools = toolNames(reqs[0].body);
      assert(
        !tools.includes('glossary_define'),
        `停用 glossary 后工具面不该含 glossary_define（实际: ${tools.join(',')}）`,
      );
      assert(tools.length > 0, '停用 glossary 只该拿掉它的工具，其余插件的能力应仍在');

      const offEvents = readFileSyncSafe(
        join(offHd.homedir, 'session', 'e2e-t96-off', 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));
      assert(
        !offEvents.some((e) => e.kind === 'glossary.defined'),
        `停用 glossary 后不该有本能力的领域事实（实际 kinds: ${JSON.stringify(offEvents.map((e) => e.kind))}）`,
      );
    } finally {
      offLlm.stop();
      cleanupHomedir(offHd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'CLI / mock-llm 子进程退出',
      });
    }
  },
);
