import './_selfrun.mjs';
// T57 system 装配契约（LC-002）：注册段 + 请求级前缀合成唯一一条 system 消息。
//
// ## 本用例钉的是什么
//
// 系统提示词的真源是「请求显式段 + 全部注册段」（`chat_loop/inputs.rs::
// resolve_system_prompt`），协议层再把请求级前缀（记忆召回 / 委派者段）并进
// **同一条** system 消息（`openai_chat.rs::merge_leading_system`）。这条链上
// 最安静的失效是「段注册了但没送达」——模型不再知道某件事存在，没有任何报错，
// 响应侧照常绿。
//
// 本用例用三重注册把每条段都摆到场，再逐条判「到场 + 序对 + 合成一条」：
// ① 智能体段（`--agent` 绑定 ⇒ agent 的 AGENTS.md 成为【智能体记忆】段正文）；
// ② 人格段（provider 配置的 `system_prompt`，model 插件注册期选定）；
// ③ 记忆段（memory 插件恒注册：agent 级三段 + 根级两段）；
// ④ 请求级前缀（第二轮起记忆召回段，必须排在注册段**之后**——置顶会改模型
//    读到的优先级，`request_view_prefix` 的顺序纪律）。
//
// ## 为什么按「地址标记」判段，不按顺序号
//
// 注册段的条数随挂载形态变（绑不绑 agent、挂几个 memory 实例），按下标判
// 一换形态就红；按**段自己的地址标记**判则与形态无关——段在不在、序对不对，
// 都是段内容自身的事实。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  makeAgentDir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import { assertSystemAssembly, assertLastUserIs } from '../llm-contract.mjs';

const PERSONA = '你是测试人格小P，说话简短。';
const AGENTS_BODY = '你是智能体小A，必须先复述用户问题再回答。';
const AGENTS_MD = `# 智能体指令\n${AGENTS_BODY}`;

const TURN1 = '第一句：记住我喜欢红茶';
const TURN2 = '第二句：我喜欢什么';

/** agent 级【智能体记忆】段的地址标记（段头自带，段内容 = AGENTS.md 正文） */
const AGENT_MEM_ADDR = '.vdfsv2/agent/e2e-t57-agent/memory/AGENTS.md';
/** 根级【工作区记忆】段地址标记 */
const ROOT_WS_ADDR = '.vdfsv2/memory/WORKSPACE.md';
/** 根级【会话记忆】段地址标记 */
const ROOT_SESS_ADDR = '.vdfsv2/session/e2e-t57/MEMORY.md';

export default defineCase(
  'T57 system 装配契约：三重注册段全部送达 + 前缀置后 + 合成唯一一条（LC-002）',
  async () => {
    const llm = await new MockLlm([
      { id: 't1', match: TURN1, content: '第一答：记住了。' },
      { id: 't2', match: TURN2, content: '第二答：红茶。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port, { system_prompt: PERSONA }) }],
      pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
    });
    makeAgentDir(hd.homedir, {
      id: 'e2e-t57-agent',
      name: 'T57 智能体',
      persona: AGENTS_MD,
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        agent: 'e2e-t57-agent',
        session: 'e2e-t57',
        stdinText: `${TURN1}\n${TURN2}\n`,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      assertEq(reqs.length, 2, '两轮各一次请求（对话面已关）');

      // ── ① 注册段全部送达（第一轮：尚无召回 ⇒ 前缀缺席是正确形态）────────
      assertSystemAssembly(
        reqs[0],
        {
          segments: [AGENT_MEM_ADDR, AGENTS_BODY, ROOT_WS_ADDR, PERSONA, ROOT_SESS_ADDR],
        },
        'T57① 注册段',
      );
      const sys1 = reqs[0].body.messages.find((m) => m.role === 'system')?.content ?? '';
      assert(
        !sys1.includes('【长期记忆】'),
        '第一轮没有可召回的记忆 ⇒ 不得印空壳召回段（不印空标题是既定设计）',
      );

      // ── ② 第二轮：召回段进前缀，且排在注册段之后 ─────────────────────────
      assertSystemAssembly(
        reqs[1],
        {
          segments: [AGENT_MEM_ADDR, AGENTS_BODY, ROOT_WS_ADDR, PERSONA, ROOT_SESS_ADDR],
          prefixMarkers: ['【长期记忆】'],
        },
        'T57② 前缀置后',
      );
      // 召回内容是本会话上一轮编码的记忆（t29 判内容面，这里只判「段到场且置后」）
      const sys2 = reqs[1].body.messages.find((m) => m.role === 'system')?.content ?? '';
      assert(
        sys2.includes(TURN1),
        `召回段应带上轮编码的记忆（实际 system 尾部: ${sys2.slice(-300)}）`,
      );

      // ── ③ 末条 user 仍是用户自己的话（前缀不得顶掉它）────────────────────
      assertLastUserIs(reqs[1], TURN2, 'T57②');

      // ── ④ AGENTS.md 正文必须**原样**在段里（不是摘录、不是改写）──────────
      assert(
        sys1.includes(AGENTS_BODY),
        '智能体 AGENTS.md 正文应原样出现在【智能体记忆】段',
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
