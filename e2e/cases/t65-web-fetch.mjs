import './_selfrun.mjs';
// T65 Web 外取：`web_fetch` 真取本地靶子 · 非法 scheme / 超时 / 404 三种失败形态 ·
// `http_request` 的 SSRF 防护。
//
// ## 这条用例补的是什么
//
// RE-005 的「Web 外取」行三格全空（面审计发现，T-076 批）。三个工具都已实现、
// 都已注册进 LLM 可见面，却一条 e2e 都没有——**整个插件的用户可见行为没有端到端锚**。
//
// ## 靶子为什么是本地 mock（`e2e/mock-http.mjs`）
//
// 真外网在 CI 上不存在、内容会变、对方会限流——三类失败都**不在被测系统里**，
// 却会被记成产品缺陷。本地靶子把「网络」这一层换成确定性读数：状态码、正文，
// 以及 `requests()` 里的**请求留痕**。留痕在本用例里承担一条关键判据：SSRF 拒绝
// 必须发生在**发请求之前**——靶子上没有那次请求，就是「拦在了门内」的证据。
//
// ## 为什么会话要 `risk_level: 'high'`
//
// `http_request` 自声明 `risk: High`，而审批闸门的默认阈值是 `Medium`
// （`risk_level_from_ctx` 的缺省）——**默认档下这个工具根本跑不到 SSRF 校验**：
// auto 模式会直接回「权限不足」错误（`confirm_prompt_payload` 的 auto 分支）。
// 要验 SSRF 防护，就得把阈值提到 `high`，让它真的进到 `validate_url`。
// 这条路径本身也是产品行为（用户可以把会话风险档调高），此前同样零 e2e。
//
// ## 判据怎么取
//
// 全部取**模型实际收到的那段文本**（`role=tool` 消息正文）与**靶子的请求留痕**：
// 前者是「用户看得见的结果」在模型侧的唯一投影，后者是「外部世界发生了什么」。
// 不读被测系统的任何内部存储。
//
// ## 元数据头与错误分流（T-109 已闭，本用例钉的是收口后的形状）
//
// 登记过的两条缺口已清偿：①`extract_result` 的 `content` 分支只取正文 ⇒ 404 页与
// 200 页对模型同形——现在 `web_fetch` 把状态并进 `content` 首行元数据头
// （`[web_fetch] status=… · …`）；②超时与连不上在 reqwest 的 Display 里同形——
// 现在按 `is_timeout` / `is_connect` 分流成「请求超时」/「连接失败」。
// 本用例把两处都按**收口后**的形状断言（404 必须带 `status=404`、超时必须报「超时」）。
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

/** `/slow` 的延迟必须**大于** `web_timeout`（本用例取 3s vs 2s），否则超时分支不触发。 */
const SLOW_MS = 3000;
const WEB_TIMEOUT_SEC = 2;

export default defineCase(
  'T65 Web 外取：web_fetch 真取本地靶子（404 / 非法 scheme / 超时）· http_request 的 SSRF 拒绝',
  async () => {
    // 靶子先起：它的端口与随机标记要写进 mock 的工具参数与断言。
    const http = await new MockHttp({ port: nextPort(), slowMs: SLOW_MS }).start();
    const PAGE = http.url('/page');
    const SLOW = http.url('/slow');
    const MISSING = http.url('/missing');
    const ECHO = http.url('/echo');

    const llm = await new MockLlm([
      // 启动器带唯一 `match`（夹具陷阱 10）。
      {
        id: 'fetch-page',
        match: '取一下那个页面',
        once: true,
        toolCalls: [{ id: 'c1', name: 'web_fetch', arguments: { url: PAGE } }],
      },
      // 以下四轮全是工具结果轮 ⇒ 每条都带 `once`（夹具陷阱 14：不带就是无限工具循环）。
      {
        id: 'fetch-badscheme',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'c2', name: 'web_fetch', arguments: { url: 'file:///etc/passwd' } }],
      },
      {
        id: 'fetch-slow',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'c3', name: 'web_fetch', arguments: { url: SLOW } }],
      },
      {
        id: 'fetch-missing',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'c4', name: 'web_fetch', arguments: { url: MISSING } }],
      },
      {
        id: 'http-ssrf',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'c5', name: 'http_request', arguments: { url: ECHO } }],
      },
      { id: 'final', afterTool: true, content: '外取面跑完了。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        session: { ...DIALOG_FACE_OFF },
        // `web_timeout` 是 `web_fetch` 与 `http_request` 的**单一真源**（web_config.rs）。
        // 取 2s 是为了让 `/slow`（3s）必然超时——这一条同时把「面板上的值真的被读」
        // 变成可判的读数（该字段历史上曾「面板能改、代码不读」）。
        web: { web_timeout: WEB_TIMEOUT_SEC },
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

      // ── 证据①：六个请求 = 用户轮 + 五轮工具结果回灌（轮次账对得上）
      const reqs = await llm.requests();
      assertEq(reqs.length, 6, `应六次请求（1 用户轮 + 5 工具结果轮），实际 ${reqs.length}`);
      const wireNames = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      for (const name of ['web_fetch', 'http_request']) {
        assert(
          wireNames.includes(name),
          `${name} 应注册进模型可见工具面（实际: ${wireNames.join(',')}）`,
        );
      }

      // ── 证据②：web_fetch 正常面——本地靶子的正文原样回灌 + 元数据头（T-109 ①）
      const pageText = toolResultText(reqs[1]);
      assert(
        pageText.includes(http.marker),
        `web_fetch 应把靶子正文交给模型（实际: ${pageText.slice(0, 400)}）`,
      );
      assert(
        pageText.includes('mock 页面'),
        `web_fetch 交回的是**原始正文**（不剥 HTML 标签）（实际: ${pageText.slice(0, 400)}）`,
      );
      assert(
        pageText.includes('[web_fetch] status=200'),
        `回灌首行应带元数据头且 status=200（T-109 ①，实际: ${pageText.slice(0, 200)}）`,
      );

      // ── 证据③：非法 scheme 被拒（只允许 http/https）
      const schemeText = toolResultText(reqs[2]);
      assert(
        /http|https/i.test(schemeText) && /只支持|协议|scheme|invalid/i.test(schemeText),
        `file:// 应被拒并说明只支持 HTTP/HTTPS（实际: ${schemeText.slice(0, 400)}）`,
      );

      // ── 证据④：超时——`web_timeout` 真的被读到（2s < 靶子的 3s）+ 错误分流（T-109 ②）
      //
      // 判据要求回灌**报出「超时」**：reqwest 的 Display 把超时与连接失败收成同一句
      // `error sending request…`，那条同形缺口已闭——`web_fetch` 现按 `is_timeout`
      // 分流成「请求超时：…（可调大 web_timeout 后重试）」。若哪天又退回直接
      // `format!("请求失败: {e}")`，这条当场红。
      const slowText = toolResultText(reqs[3]);
      assert(
        !slowText.includes('slow 端点终于回了'),
        `web_timeout 未生效：3s 的靶子本该被 2s 的超时掐掉，却拿到了正文（实际: ${slowText.slice(0, 400)}）`,
      );
      assert(
        slowText.includes('请求超时') && slowText.includes('web_timeout'),
        `超时应报「请求超时」并指路 web_timeout（T-109 ②，实际: ${slowText.slice(0, 400)}）`,
      );

      // ── 证据⑤：404 的正文照回，且**带 status=404 元数据头**（T-109 ①）
      const missingText = toolResultText(reqs[4]);
      assert(
        missingText.includes('没有这个页面') && missingText.includes(http.marker),
        `404 页面的正文应回灌（实际: ${missingText.slice(0, 400)}）`,
      );
      assert(
        missingText.includes('[web_fetch] status=404'),
        `404 页面必须向模型声明 status=404（T-109 ①，实际: ${missingText.slice(0, 200)}）`,
      );

      // ── 证据⑥：http_request 对本地主机被 SSRF 防护拒绝
      const ssrfText = toolResultText(reqs[5]);
      assert(
        ssrfText.includes('127.0.0.1') && /阻止|私有|本地/.test(ssrfText),
        `http_request 打本地主机应被 SSRF 防护拒绝并指名主机（实际: ${ssrfText.slice(0, 400)}）`,
      );

      // ── 证据⑦：**拒绝发生在发请求之前**——靶子上没有那次请求
      //
      // 这是本条最关键的判据：「返回了错误」与「拦在了门内」是两回事。若某次重构
      // 把校验挪到 `send()` 之后，错误文案可以一字不变，而请求已经打到内网了。
      const seen = await http.requests();
      const paths = seen.map((r) => r.path);
      assert(paths.includes('/page'), `靶子应收到 /page（实际: ${paths.join(',')}）`);
      assert(paths.includes('/missing'), `靶子应收到 /missing（实际: ${paths.join(',')}）`);
      assert(paths.includes('/slow'), `靶子应收到 /slow（实际: ${paths.join(',')}）`);
      assert(
        !paths.includes('/echo'),
        `SSRF 被拒的请求**不得**打到靶子（实际: ${paths.join(',')}）`,
      );

      // ── 证据⑧：转写落格，五个 tool_call 都终态化
      const msgs = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs, 'T65');
      const calls = msgs.filter((m) => msgTypeOf(m) === 'tool_call');
      assertEq(
        calls.map((c) => c.meta?.tool_name).join(','),
        'web_fetch,web_fetch,web_fetch,web_fetch,http_request',
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
