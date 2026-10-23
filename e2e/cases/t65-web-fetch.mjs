import './_selfrun.mjs';
// T65 Web 外取：`web_fetch` 与 `http_request` 共用同一套出站守卫 ·
// 非法 scheme 被拒 · SSRF 拒绝发生在发请求之前。
//
// ## 这条用例补的是什么
//
// RE-005 的「Web 外取」行三格全空（面审计发现， 批）。两个工具的入站可见行为
// 都没有端到端锚：一个只校验 scheme 就能带着模型给的地址打内网，另一个明确拒绝——
// 同一类操作两种答案。现在两者共用 `net_guard`，本用例把「同一个本地地址、两个工具
// 都拒绝」钉成可判的读数。
//
// ## 靶子为什么是本地 mock（`e2e/mock-http.mjs`）
//
// 真外网在 CI 上不存在、内容会变、对方会限流——三类失败都**不在被测系统里**，
// 却会被记成产品缺陷。本用例不需要靶子回包：本地 mock 的价值是**请求留痕**——
// 守卫拒绝之后，靶子上应当一条请求都没有。错误文案可以照抄，留痕不能。
//
// ## 为什么会话要 `risk_level: 'high'`
//
// `http_request` 自声明 `risk: High`，而审批闸门的默认阈值是 `Medium`
// （`risk_level_from_ctx` 的缺省）——**默认档下这个工具根本跑不到出站校验**：
// auto 模式会直接回「权限不足」错误（`confirm_prompt_payload` 的 auto 分支）。
// 要验守卫，就得把阈值提到 `high`，让它真的进到 `validate_url`。
//
// ## 判据怎么取
//
// 全部取**模型实际收到的那段文本**（`role=tool` 消息正文）与**靶子的请求留痕**：
// 前者是「用户看得见的结果」在模型侧的唯一投影，后者是「外部世界发生了什么」。
// 不读被测系统的任何内部存储。
import {
  MockLlm,
  MockHttp,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  startLongLivedCli,
  subscribeSessionRealtime,
  waitFor,
  readSessionMessages,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

const SID = 'e2e-t65';

/** gateway 入站配置（与 t16 / t27 同一约定：读出口与实时面都经它）。 */
function gatewayConfig(port) {
  return {
    inbound_enabled: true,
    inbound_protocol: 'http',
    inbound_bind: '127.0.0.1',
    inbound_port: port,
    inbound_token: '',
    inbound_readonly: false,
  };
}

export default defineCase(
  'T65 Web 外取：web_fetch 与 http_request 对本地目标的 SSRF 拒绝（拒绝先于发请求）· 非法 scheme 被拒',
  async () => {
    // 靶子先起：它的端口与随机标记要写进 mock 的工具参数与断言。
    const http = await new MockHttp({ port: nextPort() }).start();
    const PAGE = http.url('/page');
    const ECHO = http.url('/echo');

    const llm = await new MockLlm([
      // 启动器带唯一 `match`（夹具陷阱 10）。
      {
        id: 'fetch-local',
        match: '取一下那个页面',
        once: true,
        toolCalls: [{ id: 'c1', name: 'web_fetch', arguments: { url: PAGE } }],
      },
      // 以下三轮全是工具结果轮 ⇒ 每条都带 `once`（夹具陷阱 14：不带就是无限工具循环）。
      {
        id: 'fetch-badscheme',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'c2', name: 'web_fetch', arguments: { url: 'file:///etc/passwd' } }],
      },
      {
        id: 'http-ssrf',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'c3', name: 'http_request', arguments: { url: ECHO } }],
      },
      { id: 'final', afterTool: true, content: '外取面跑完了。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        session: { ...DIALOG_FACE_OFF },
      },
    });

    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'unused-t65',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });

    let rt = null;
    try {
      await cli.waitGatewayReady();
      rt = await subscribeSessionRealtime(cli, { sessionId: SID, gatewayPort: GATEWAY_PORT });

      // 会话先建出来（收件箱是会话内部的集合），并把风险档提到 `high`。
      const rootResp = await cli.invoke('vdfs/root', {});
      const root = rootResp.body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${SID}`;
      const inboxAddr = `${sessionAddr}/inbox`;
      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({
          metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'high' },
        }),
      });
      assertEq(created.status, 200, `创建会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      // 发一轮，等这一轮真正跑完（水位线不可省：首帧空闲态会让条件在开跑前成立）。
      const mark = rt.sessionChanges().length;
      const w = await cli.invoke('vdfs/write', { path: inboxAddr, text: '取一下那个页面' });
      assertEq(w.status, 200, `写收件箱：${JSON.stringify(w.body)?.slice(0, 300)}`);
      await waitFor(
        () => {
          const sc = rt.sessionChanges().slice(mark);
          return (
            sc.length > 0 &&
            sc.some((c) => c.data?.status === 'working') &&
            sc[sc.length - 1]?.data?.status !== 'working'
          );
        },
        { what: '本轮跑完（实时面离开 working）', timeoutMs: 90_000 },
      );

      // ── 证据①：四次请求 = 用户轮 + 三轮工具结果回灌（轮次账对得上）
      const reqs = await llm.requests();
      assertEq(reqs.length, 4, `应四次请求（1 用户轮 + 3 工具结果轮），实际 ${reqs.length}`);
      const wireNames = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      for (const name of ['web_fetch', 'http_request']) {
        assert(
          wireNames.includes(name),
          `${name} 应注册进模型可见工具面（实际: ${wireNames.join(',')}）`,
        );
      }

      // ── 证据②：web_fetch 对本地目标被拒——与 http_request 同一判据
      const fetchText = toolResultText(reqs[1]);
      assert(
        fetchText.includes('127.0.0.1') && /阻止|私有|本地/.test(fetchText),
        `web_fetch 打本地主机应被出站守卫拒绝并指名主机（实际: ${fetchText.slice(0, 400)}）`,
      );

      // ── 证据③：非法 scheme 被拒（只允许 http/https）
      const schemeText = toolResultText(reqs[2]);
      assert(
        /http|https/i.test(schemeText) && /只支持|协议|scheme|invalid/i.test(schemeText),
        `file:// 应被拒并说明只支持 HTTP/HTTPS（实际: ${schemeText.slice(0, 400)}）`,
      );

      // ── 证据④：http_request 对本地主机被同一守卫拒绝
      const ssrfText = toolResultText(reqs[3]);
      assert(
        ssrfText.includes('127.0.0.1') && /阻止|私有|本地/.test(ssrfText),
        `http_request 打本地主机应被出站守卫拒绝并指名主机（实际: ${ssrfText.slice(0, 400)}）`,
      );

      // ── 证据⑤：**拒绝发生在发请求之前**——靶子上一条业务请求都没有
      //
      // 这是本条最关键的判据：「返回了错误」与「拦在了门内」是两回事。若某次重构
      // 把校验挪到 `send()` 之后，错误文案可以一字不变，而请求已经打到内网了。
      // `/_health` / `/_requests` 是 mock 的控制面，不算业务请求。
      const seen = (await http.requests()).filter((r) => !r.path.startsWith('/_'));
      assertEq(
        seen.length,
        0,
        `被守卫拒绝的请求不得打到靶子（实际: ${seen.map((r) => r.path).join(',')}）`,
      );

      // ── 证据⑥：转写落格，三个 tool_call 都终态化
      const msgs = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs, 'T65');
      const calls = msgs.filter((m) => msgTypeOf(m) === 'tool_call');
      assertEq(
        calls.map((c) => c.meta?.tool_name).join(','),
        'web_fetch,web_fetch,http_request',
        '工具名与顺序应保持',
      );
      for (const c of calls) {
        assertEq(c.status, 'completed', `tool_call(${c.meta?.tool_name}) 终态应为 completed`);
      }
    } finally {
      rt?.close();
      cli.stop();
      llm.stop();
      http.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm / mock-http 子进程退出',
      });
    }
  },
);

/**
 * 取「本轮工具结果回灌」的那条 `role=tool` 消息正文。
 *
 * ⚠️ **取最后一条，不是第一条**（夹具陷阱：历史里带着前面每一轮的工具结果）。
 */
function toolResultText(req) {
  const msgs = req?.body?.messages ?? [];
  const tool = [...msgs].reverse().find((m) => msgRoleOf(m) === 'tool');
  if (tool) return typeof tool.content === 'string' ? tool.content : JSON.stringify(tool.content);
  return '';
}
