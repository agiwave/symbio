import './_selfrun.mjs';
// T77 长度截断：被 max_tokens 掐断的半段思考不得当答话交付（T-142 判据）。
//
// ## 本用例钉的是什么
//
// 「思考即正文」这条机制（`T-095`）说：正文通道空、只有 `reasoning_content` 时，那段
// 思考**就是**模型要说的话。机制没错，缺的是**长度截断这一格**——模型在思考中途被
// `max_tokens` 掐掉时，那段文字既不是答案、也不是写完的思考。
//
// 实测的失败形状（本机 24B 模型，`max_tokens: 4096`，任务=产出质量报告）：
//
// ```
// 流正常结束 (耗时 348s, 文本 0 字符, 推理 13045 字符, finish=Length)
// stdout = 13067 字节 —— 整段草稿；退出码 0
// ```
//
// 于是这条判据在 CLI 边界上落成两问：**stdout 里有没有草稿**、**退出码说不说是失败**。
//
// | 线 | 响应形状 | 期望 |
// |---|---|---|
// | A 截断 | 正文 0 字符 + 只有思考 + `finish=length` | 退出码非零；stdout **查无草稿**；stderr 说出「长度」；草稿作为**思考事实**入格 |
// | B 未截断 | 正文 0 字符 + 只有思考 + `finish=stop` | 退出码 0；stdout = 那句思考（`T-095` 照旧成立） |
//
// ## 为什么必须有 B 线
//
// A 线单独存在会被一种改法蒙过去：把「思考即正文」整条关掉。B 线钉住另一半——
// **写完的思考仍然算答案**。两条一起，判据才是「区分截断与完成」，而不是「交付不交付思考」。
//
// ## 为什么在 CLI 边界而不是只看插件返回值
//
// 缺陷的现场就是 stdout 被草稿灌满，而 stdout 是**要交给下游程序的**。插件内部返回什么
// 字符串都不构成判据；「下游拿到的字节里那句草稿消失了」才是。单元侧的对偶判据在
// `turn_runner.test.rs::length_truncated_reasoning_is_not_delivered_as_answer`。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  nextPort,
  startReadbackCli,
  readSessionMessages,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgTypeOf,
} from '../helpers.mjs';

/** A 线的草稿（模型「打算怎么写」，被掐断时它全部就在 `reasoning_content` 里） */
const DRAFT = '先读三处，再逐段列数字，然后检查字数。还没写完，接下来才要动笔。';
/** B 线的答话（同样是「只有思考」的形状，但 provider 说它说完了） */
const ANSWER = '答案是 42，依据是本轮读到的两处数字。';

const SID_TRUNCATED = 'e2e-t77-length';
const SID_COMPLETE = 'e2e-t77-stop';

export default defineCase(
  'T77 长度截断：只有思考且 finish=length 时草稿不进 stdout，且以失败退出（未截断时照旧交付）',
  async () => {
    const llm = await new MockLlm([
      // A 线：正文 0 字符（不写 `content`）+ 只有思考 + provider 明说被长度掐断。
      { id: 'draft-length', match: '写一份报告', reasoning: [DRAFT], finishReason: 'length' },
      // B 线：同形，只是**没有**被掐断（缺省 `stop`）⇒ 那段思考就是答话。
      { id: 'think-stop', match: '只凭思考回答', reasoning: [ANSWER] },
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
        // 两条线都要走到**worker 的末轮收束**——那是缺陷现场。判决与措辞开着时，
        // 「写一份报告」这句可能被判成直接答话，走的就不是同一条收束路径了。
        session: { classify_enabled: false, compose_enabled: false },
      },
    });

    let api = null;
    try {
      // ── A 截断：草稿不进 stdout，退出码说失败 ──
      const a = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '写一份报告',
        provider: PROVIDER_ID,
        session: SID_TRUNCATED,
        timeoutMs: 60_000,
      });

      assert(a.code !== 0, `被截断的轮次必须以失败退出（实得 code=${a.code}）`);
      assert(
        !a.stdout.includes(DRAFT),
        `stdout 里不得出现被截断的草稿——它要交给下游程序（实得 stdout=${JSON.stringify(a.stdout.slice(0, 300))}）`,
      );
      assert(
        a.stderr.includes('长度'),
        `stderr 要说出是长度截断，不是一句「空文本」（实得 stderr=${JSON.stringify(a.stderr.slice(-400))}）`,
      );

      // ── B 未截断：「思考即正文」照旧成立 ──
      const b = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '只凭思考回答',
        provider: PROVIDER_ID,
        session: SID_COMPLETE,
        timeoutMs: 60_000,
      });
      assertEq(b.code, 0, '正文空但思考写完的那一轮是回答，不是失败（T-095）');
      assert(
        b.stdout.includes(ANSWER),
        `未截断的思考必须作为答话交付（实得 stdout=${JSON.stringify(b.stdout.slice(0, 300))}）`,
      );

      // ── 事实源：草稿没被丢掉，只是不再冒充答话 ──
      api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID_TRUNCATED,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      const msgs = await readSessionMessages(api, SID_TRUNCATED);
      assertTranscriptInvariants(msgs, 'T77-A');
      assert(
        !msgs.some((m) => msgTypeOf(m) === 'text' && (m.content ?? '').includes(DRAFT)),
        '草稿不得以正文节点入格（那是「答了」的形状）',
      );
      // 草稿**仍然是事实**：它在实时面上流过，收束格把它作为思考记下来。判据要的是
      // 「不当答案」，不是「当没发生过」——否则这 13K 字符的线索就永久消失了。
      await waitFor(
        async () => {
          const fresh = await readSessionMessages(api, SID_TRUNCATED);
          return (fresh ?? []).some(
            (m) => msgTypeOf(m) === 'reasoning' && (m.content ?? '').includes(DRAFT),
          );
        },
        { what: '被截断的思考仍作为思考事实入格', timeoutMs: 20_000 },
      );

      const done = await readSessionMessages(api, SID_COMPLETE);
      assertTranscriptInvariants(done, 'T77-B');
      assert(
        done.some((m) => msgTypeOf(m) === 'text' && (m.content ?? '').includes(ANSWER)),
        '未截断那一轮的答话以正文节点入格',
      );
    } finally {
      api?.stop();
      llm.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '读回 CLI / mock-llm 子进程退出',
      });
    }
  },
);
