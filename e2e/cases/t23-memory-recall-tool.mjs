import './_selfrun.mjs';
// T23 记忆召回消费口（B5）：默认装配 → memory_recall 进模型工具清单 → 跨会话召回
// 真实落盘 → 停用即从清单消失（J2）。
//
// ## 这条用例锁什么
//
// B1–B4 只有**生产侧**（事实 / 投影 / Actor 行）与诊断路由，模型看不见——「可查而
// 不可用」。B5 补上**消费侧**，本用例锁它的四条出口：
//
// | 性质 | 判据 | 为什么单测证明不了 |
// |---|---|---|
// | **能力到达用户** | 空 homedir（夹具不建 retrieval 目录）跑一轮后 `retrieval/PLUGIN.yml` 已被装配出来 | 装配方行为（`home::SYSTEM_EXTRA_PLUGINS` → REQUIRED_PLUGINS 补目录），单测里没有装配方 |
// | **模型可见** | mock 收到的首轮请求 `tools` 含 `memory_recall` | 跨进程：traverse 注册 → collect → model 请求组装，全链才成立 |
// | **跨会话召回** | 会话 A 钉住的 `MEMORY.md` 行出现在会话 B 的工具结果里 | 需要真实两个落盘会话 + 真实工具执行 |
// | **平凡值（J2）** | `plugin_enabled: false` ⇒ 工具从清单消失，**会话照常** | 停用是装配期行为 |
//
// ## 为什么用真实对话而不是直接 invoke 路由
//
// B5 的价值就是「聊天里能用」——只有让 mock LLM 真的发出 tool_call、会话真的执行
// 工具、结果真的落进转写，才证明消费口接通了（单测证明的是逻辑，不是接线）。
import { writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  assert,
  assertEq,
  assertTranscriptInvariants,
  textOf,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

const SESSION_A = 'e2e-t23-a';
const SESSION_B = 'e2e-t23-b';
const SESSION_C = 'e2e-t23-c';
/// 会话 A 钉住的**辨识行**——B 的工具结果里必须出现它
const SECRET_LINE = 'e2e-t23 秘密口令：紫金十三钗';

export default defineCase(
  'T23 记忆召回消费口（B5）：默认装配 · 模型可见 · 跨会话召回 · 停用即退化',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'recall',
        match: '之前记过什么',
        toolCalls: [{ id: 'call_recall', name: 'memory_recall', arguments: {} }],
      },
      { id: 'after-recall', afterTool: true, content: '我翻到了你钉住的记忆。' },
      { id: 'generic', content: '好的。' },
    ]).start();

    // 夹具**只给 model provider**：retrieval 等四插件的目录必须由装配方补出来
    const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
    try {
      // ⓪ 反证前提：夹具没有预建 retrieval 目录
      assert(
        !existsSync(join(hd.homedir, 'retrieval')),
        '夹具不应预建 retrieval 目录（否则“默认装配”断言失去意义）',
      );

      // ① 会话 A：一轮真实对话，随后钉住一条记忆（MEMORY.md 是公开约定落位）
      const rA = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我记一笔',
        provider: PROVIDER_ID,
        session: SESSION_A,
      });
      assertEq(rA.code, 0, `会话 A 退出码（stderr: ${rA.stderr.slice(0, 400)}）`);
      writeFileSync(
        join(hd.homedir, 'session', SESSION_A, 'MEMORY.md'),
        `${SECRET_LINE}\n`,
        'utf8',
      );

      // ② 默认装配：目录与 manifest 是这次运行被 REQUIRED_PLUGINS 补出来的
      const ymlPath = join(hd.homedir, 'retrieval', 'PLUGIN.yml');
      assert(existsSync(ymlPath), `retrieval 应被默认装配（缺 ${ymlPath}）`);

      // ③ 会话 B：模型可见工具 → 真的调用 → 跨会话召回落进转写
      await llm.reset();
      const rB = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '之前记过什么',
        provider: PROVIDER_ID,
        session: SESSION_B,
      });
      assertEq(rB.code, 0, `会话 B 退出码（stderr: ${rB.stderr.slice(0, 400)}）`);

      const reqsB = await llm.requests();
      assert(reqsB.length >= 2, `应有工具轮+收尾轮（实得 ${reqsB.length}）`);
      const toolNames = (reqsB[0].body.tools ?? []).map((t) => t.function?.name);
      assert(
        toolNames.includes('memory_recall'),
        `首轮 tools 应含 memory_recall（实际: ${toolNames.join(',')}）`,
      );

      const msgsB = readMessagesJson(hd.homedir, SESSION_B);
      assertTranscriptInvariants(msgsB, 'T23-B');
      const tc = msgsB.find((m) => m.type === 'tool_call');
      assert(tc, '应有 tool_call 节点（主会话转写保留工作记录）');
      assertEq(tc.name, 'memory_recall', 'tool_call 的工具名');
      assertEq(tc.status, 'completed', 'tool_call 终态应为 completed');

      const toolMsg = msgsB.find((m) => m.role === 'tool');
      assert(toolMsg, '应有工具结果消息');
      const resultText = textOf(toolMsg);
      assert(
        resultText.includes(SECRET_LINE),
        `工具结果应含会话 A 钉住的记忆行（实得: ${resultText.slice(0, 300)}）`,
      );
      assert(
        resultText.includes(SESSION_A),
        '工具结果应标明记忆来自哪个会话（跨会话召回）',
      );
      const finalB = msgsB.find((m) => m.role === 'assistant' && m.type === 'text');
      assert(finalB, '应有 assistant 终态文本');
      assert(
        textOf(finalB).includes('我翻到了你钉住的记忆'),
        '收尾轮应基于工具结果产出终答',
      );

      // ④ J2：停用 ⇒ 工具从模型清单消失，会话照常
      writeFileSync(ymlPath, `${readFileSync(ymlPath, 'utf8')}plugin_enabled: false\n`, 'utf8');
      await llm.reset();
      const rC = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '随便说句什么',
        provider: PROVIDER_ID,
        session: SESSION_C,
      });
      assertEq(rC.code, 0, `停用后退出码（stderr: ${rC.stderr.slice(0, 400)}）`);

      const reqsC = await llm.requests();
      const toolNamesC = (reqsC[0].body.tools ?? []).map((t) => t.function?.name);
      assert(
        !toolNamesC.includes('memory_recall'),
        `停用后 memory_recall 应从清单消失（实际: ${toolNamesC.join(',')}）`,
      );
      const msgsC = readMessagesJson(hd.homedir, SESSION_C);
      assert(Array.isArray(msgsC) && msgsC.length > 0, '停用后会话照常落盘');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);