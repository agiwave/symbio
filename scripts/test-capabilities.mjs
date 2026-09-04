#!/usr/bin/env node
/**
 * 能力模块测试脚本（由 scripts/test_capabilities.ps1 迁移）
 *
 * 使用 CLI 工具直接测试，通过分析输出验证能力调用情况。
 *
 * 用法：
 *   node scripts/test-capabilities.mjs
 *
 * 环境变量：
 *   CLI_PATH  CLI 可执行文件路径（默认 <repoRoot>/symbio/target/release/cli[.exe]）
 *   WORKDIR   CLI 运行工作目录（默认仓库根）
 *
 * 说明：原 PowerShell 版把 CLI 路径与工作目录硬编码为 c:\Bing\agiwave\... ，
 * 现在改为从仓库根推导（可用上述环境变量覆盖）。输出直接由子进程捕获，
 * 不再落 temp_output.txt / temp_error.txt 临时文件。
 */

import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

const CLI_PATH =
  process.env.CLI_PATH ||
  path.join(repoRoot, 'symbio', 'target', 'release', process.platform === 'win32' ? 'cli.exe' : 'cli')
const WORKDIR = process.env.WORKDIR || repoRoot

/** 正则转义，用于把关键字当字面量匹配（等价于 PowerShell 的 [regex]::Escape） */
const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// ── 测试场景（与原 .ps1 保持一致）──────────────────────────────────────
const testScenarios = [
  {
    name: '知识查询 (agent_query)',
    description: '测试知识检索能力',
    prompt: '请查询并告诉我关于日出的知识',
    verification: {
      toolCallKeyword: 'agent_query',
      expectedOutputKeywords: ['sun_rise', '太阳', '升起'],
    },
  },
  {
    name: '知识存储 (agent_store)',
    description: '测试知识存储能力',
    prompt: '请帮我存储一条新知识：地球是圆的',
    verification: {
      toolCallKeyword: 'agent_store',
      expectedOutputKeywords: ['存储', '保存', 'knowledge'],
    },
  },
  {
    name: '因果推理 (agent_reasoning)',
    description: '测试因果推理能力',
    prompt: '分析一下降雨和地面变湿之间的因果关系',
    verification: {
      toolCallKeyword: 'agent_reasoning',
      expectedOutputKeywords: ['因果', 'cause', 'effect', '关系'],
    },
  },
  {
    name: '类比推理 (agent_analogy)',
    description: '测试类比推理能力',
    prompt: '用水循环和能量流动做一个类比分析',
    verification: {
      toolCallKeyword: 'agent_analogy',
      expectedOutputKeywords: ['类比', 'analogy', '相似', 'similar'],
    },
  },
  {
    name: '目标规划 (agent_goal_planner)',
    description: '测试目标规划能力',
    prompt: '帮我规划一个完成Rust项目的详细计划',
    verification: {
      toolCallKeyword: 'agent_goal_planner',
      expectedOutputKeywords: ['计划', 'plan', '步骤', 'task'],
    },
  },
  {
    name: '元认知 (agent_metacognition)',
    description: '测试元认知能力',
    prompt: '反思一下你是如何回答问题的，有什么可以改进的地方',
    verification: {
      toolCallKeyword: 'agent_metacognition',
      expectedOutputKeywords: ['反思', 'reflection', '改进', 'improve'],
    },
  },
  {
    name: '知识提取 (agent_learn)',
    description: '测试知识提取能力',
    prompt: '从这句话中提取并存储知识：Rust的所有权系统可以防止内存泄漏',
    verification: {
      toolCallKeyword: 'agent_learn',
      expectedOutputKeywords: ['提取', 'extract', 'learn', '学习'],
    },
  },
  {
    name: '记忆管理 (agent_memory_manage)',
    description: '测试记忆管理能力',
    prompt: '分析一下你的知识库，看看有什么可以优化的地方',
    verification: {
      toolCallKeyword: 'agent_memory_manage',
      expectedOutputKeywords: ['记忆', 'memory', '优化', 'optimize'],
    },
  },
  {
    name: '符号推理 (agent_symbolic_reasoner)',
    description: '测试符号推理能力',
    prompt: '验证这个逻辑：所有人都会死，苏格拉底是人，所以苏格拉底会死',
    verification: {
      toolCallKeyword: 'agent_symbolic_reasoner',
      expectedOutputKeywords: ['逻辑', 'logic', '推理', 'syllogism'],
    },
  },
  {
    name: '知识演化 (agent_knowledge_evolution)',
    description: '测试知识演化能力',
    prompt: '检查一下你的知识库中有没有冲突的知识',
    verification: {
      toolCallKeyword: 'agent_knowledge_evolution',
      expectedOutputKeywords: ['冲突', 'conflict', '演化', 'evolution'],
    },
  },
]

// ── 前置检查 ───────────────────────────────────────────────────────────
if (!fs.existsSync(CLI_PATH)) {
  console.error(`[FATAL] 找不到 CLI 可执行文件：${CLI_PATH}`)
  console.error('请先构建（cargo build --release），或用 CLI_PATH 环境变量指定路径。')
  process.exit(1)
}

const results = []

console.log('='.repeat(80))
console.log('  Agent 能力模块测试')
console.log('='.repeat(80))
console.log('')

testScenarios.forEach((scenario, idx) => {
  const sessionId = `test_cap_${idx + 1}`

  console.log(`[${scenario.name}]`)
  console.log(`  描述: ${scenario.description}`)
  console.log(`  提示: ${scenario.prompt}`)
  console.log(`  会话: ${sessionId}`)
  console.log('')

  let result
  try {
    console.log('  执行中...')

    const proc = spawnSync(
      CLI_PATH,
      ['--agent', 'tester', '--session', sessionId, scenario.prompt],
      { cwd: WORKDIR, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, shell: false },
    )

    const exitCode = proc.status ?? -1
    const fullOutput = `${proc.stdout ?? ''}\n${proc.stderr ?? ''}`

    console.log(`  退出码: ${exitCode}`)

    const issues = []

    // 1) 是否检测到能力调用
    const toolCalled = new RegExp(escapeRe(scenario.verification.toolCallKeyword)).test(fullOutput)
    if (toolCalled) {
      console.log(`  ✓ 检测到能力调用: ${scenario.verification.toolCallKeyword}`)
    } else {
      console.log(`  ✗ 未检测到能力调用: ${scenario.verification.toolCallKeyword}`)
      issues.push('未检测到能力调用')
    }

    // 2) 是否命中预期输出关键词
    const matchedKeywords = scenario.verification.expectedOutputKeywords.filter((kw) =>
      new RegExp(escapeRe(kw)).test(fullOutput),
    )
    const outputMatched = matchedKeywords.length > 0
    if (outputMatched) {
      console.log(`  ✓ 检测到相关输出关键词: ${matchedKeywords.join(', ')}`)
    } else {
      console.log('  ✗ 未检测到相关输出关键词')
      issues.push('未检测到相关输出')
    }

    const success = toolCalled || outputMatched
    const confidence = toolCalled && outputMatched ? 1.0 : success ? 0.5 : 0.0

    result = {
      scenarioName: scenario.name,
      toolCalled,
      outputMatched,
      success,
      confidence,
      issues: issues.join('; '),
      exitCode,
    }

    console.log('')
    console.log(`  结果: ${result.success ? '✓ 通过' : '✗ 失败'}`)
    console.log(`  置信度: ${Math.round(result.confidence * 100)}%`)
  } catch (e) {
    console.log(`  错误: ${e?.message ?? e}`)
    result = {
      scenarioName: scenario.name,
      toolCalled: false,
      outputMatched: false,
      success: false,
      confidence: 0.0,
      issues: `执行错误: ${e?.message ?? e}`,
      exitCode: -1,
    }
  }

  results.push(result)

  console.log('')
  console.log('-'.repeat(60))
  console.log('')
})

// ── 报告 ───────────────────────────────────────────────────────────────
console.log('='.repeat(80))
console.log('  测试报告')
console.log('='.repeat(80))
console.log('')

const total = results.length
const passed = results.filter((r) => r.success).length
const toolCalledCount = results.filter((r) => r.toolCalled).length
const outputMatchedCount = results.filter((r) => r.outputMatched).length
const avgConfidence = total === 0 ? 0 : results.reduce((s, r) => s + r.confidence, 0) / total

console.log('测试统计:')
console.log(`  总测试数: ${total}`)
console.log(`  通过数: ${passed}`)
console.log(`  失败数: ${total - passed}`)
console.log(`  能力调用检测: ${toolCalledCount}/${total}`)
console.log(`  输出匹配检测: ${outputMatchedCount}/${total}`)
console.log(`  通过率: ${total === 0 ? '0.0' : ((passed / total) * 100).toFixed(1)}%`)
console.log(`  平均置信度: ${Math.round(avgConfidence * 100)}%`)
console.log('')

console.log('详细结果:')
for (const r of results) {
  const mark = r.success ? '✓' : '✗'
  console.log(`  ${mark} ${r.scenarioName}`)
  console.log(
    `    能力调用: ${r.toolCalled ? '✓' : '✗'}, 输出匹配: ${r.outputMatched ? '✓' : '✗'}, 置信度: ${Math.round(r.confidence * 100)}%`,
  )
  if (r.issues) console.log(`    问题: ${r.issues}`)
}

console.log('')
console.log('='.repeat(80))
console.log('  测试完成')
console.log('='.repeat(80))
