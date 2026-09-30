// 用例自执行引导：`node e2e/cases/xxx.mjs` 时，用例文件先引入本模块（副作用
// import），defineCase 登记的用例在事件循环空闲后由这里执行并按结果退出。
//
// 仅在自执行标记（E2E_CASE_SELF=1）下生效——runner 或直接运行用例文件时都会
// 设置它；作为普通模块被 import（如未来的聚合工具）不会触发执行。
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
    await settleBeforeExit();
    process.exit(0);
  } catch (e) {
    console.error(`  ✗ ${name} (${Date.now() - t0}ms)`);
    console.error(`      ${String(e.message ?? e)}`);
    if (process.env.E2E_DEBUG) console.error(e.stack);
    await settleBeforeExit();
    process.exit(1);
  }
}

/**
 * 退出前的事件循环收尾窗口（Windows 专属噪声的兜底）。
 *
 * 症状：用例断言**全部通过**（`✓` 已打印），退出码却是 1，stderr 只有一行
 * `Assertion failed: !(handle->flags & UV_HANDLE_CLOSING), file src\win\async.c`。
 * 成因：用例末尾刚杀掉 mock / 长驻 CLI 子进程，或刚完成若干 `fetch`，
 * 这些句柄仍在异步关闭中；此刻 `process.exit()` 会在 libuv 里撞断言。
 *
 * 为什么放在这里：**本文件是用例进程唯一的退出点**——放一处即覆盖全部用例，
 * 不必每个 `finally` 各写一遍。用固定一拍的代价换掉"红的其实是退出噪声"，
 * 同 `mock-llm.mjs`「connection: close 避开 libuv 退出断言」的同类处理。
 * 250ms 为实测校准值（200ms 已足够，留一档余量）。
 */
function settleBeforeExit() {
  return new Promise((r) => setTimeout(r, 250));
}

if (process.env.E2E_CASE_SELF === '1') {
  setTimeout(() => void runRegisteredCase(), 0);
}
