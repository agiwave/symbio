// 用例自执行引导：`node e2e/cases/xxx.mjs` 时，用例文件先引入本模块（副作用
// import），defineCase 登记的用例在事件循环空闲后由这里执行并按结果退出。
//
// 仅在自执行标记（E2E_CASE_SELF=1）下生效——runner 或直接运行用例文件时都会
// 设置它；作为普通模块被 import（如未来的聚合工具）不会触发执行。
/**
 * 以 `code` 退出，但**让事件循环自然排空**而不是直接 `process.exit(code)`。
 *
 * ## 为什么不能直接 exit
 *
 * Windows 上 libuv 的 `uv__async_send` 会断言 `!(handle->flags & UV_HANDLE_CLOSING)`，
 * 崩在 `src\win\async.c:76`，退出码 `0xC0000409`（abort）。触发条件很窄但**完全确定**：
 * 用例跑完时**句柄还在拆除**，此时强制退出会把异步唤醒句柄打进「正在关闭」状态。
 *
 * 关键点：**与被测系统无关**。最小复现只要「起 mock-llm → 打 ≥2 次管控面 HTTP →
 * 定时器里 `process.exit(0)`」，不起 CLI、不建 homedir 也必崩；换成
 * `setImmediate` 同样崩，只有「不 exit、让循环自己排空」不崩。
 * 所以它一直藏在只有一次取数的用例底下，直到某条用例打了第二次
 * `llm.requests()` 才露头——**症状落在那条用例身上，病因在收尾路径上**。
 *
 * （`mock-llm.mjs` 里服务端早就有 `connection: close` 的注释说这是
 * 「Windows 上 Node fetch keep-alive 的 libuv 退出断言噪音」——那次只补了
 * 服务端那一半，客户端的强制退出没动。）
 *
 * ## 兜底为什么还需要
 *
 * 自然排空的前提是**没有残留句柄**。用例若忘了停自己的 mock / 长驻 CLI，
 * 进程会被吊住。所以留一个**不保活**的定时器：还有别的句柄撑着循环时它照常触发
 * （强制退，不让一个漏停的用例把整轮门禁挂死）；没有残留句柄时它自己不保活，
 * 循环排空即退出，**不给每条用例加固定延迟**。
 */
function exitAfterDrain(code) {
  process.exitCode = code;
  const force = setTimeout(() => process.exit(code), EXIT_DRAIN_GRACE_MS);
  force.unref?.();
}

/** 自然排空给多久：够短（不让漏停用例把门禁吊住）、够长（正常收尾通常 <10ms）。 */
const EXIT_DRAIN_GRACE_MS = 3_000;

export async function runRegisteredCase() {
  const cases = globalThis.__E2E_CASES ?? [];
  if (cases.length !== 1) {
    console.error(`[e2e] 自执行模式要求文件恰好定义一个用例（实际 ${cases.length}）`);
    process.exit(2);
  }
  const { name, fn } = cases[0];
  const t0 = Date.now();
  try {
    await fn();
    console.log(`  ✓ ${name} (${Date.now() - t0}ms)`);
    exitAfterDrain(0);
  } catch (e) {
    console.error(`  ✗ ${name} (${Date.now() - t0}ms)`);
    console.error(`      ${String(e.message ?? e)}`);
    if (process.env.E2E_DEBUG) console.error(e.stack);
    exitAfterDrain(1);
  }
}

if (process.env.E2E_CASE_SELF === '1') {
  setTimeout(() => void runRegisteredCase(), 0);
}
