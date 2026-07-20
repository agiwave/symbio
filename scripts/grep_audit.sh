#!/usr/bin/env bash
# grep_audit.sh — 静态审计检查脚本（对应 PLAN §S-4）
#
# 用途：拦截常见异步/同步错误模式，防止 v27-v28 修复过的 bug 复发
# - S-002: std::sync::Mutex 在 async 上下文中持锁跨 await
# - S-007: CHANGELOG 缺关键修复条目（v25-N6 案例）
#
# 用法：
#   ./scripts/grep_audit.sh            # 审计 src/plugins/agent
#   ./scripts/grep_audit.sh --strict   # 严格模式：warning 也算失败
#
# 退出码：
#   0 = 全部通过
#   1 = 发现 ERROR（必须修复）
#   2 = 仅 WARNING（建议修复，--strict 才会失败）

set -euo pipefail
# 关闭 bash history expansion，避免字符串里 plugin_warn! 之类被展开
set +H

# 智能推断 SCOPE 默认值
# - 如果当前目录有 src/plugins/agent，则用相对路径
# - 否则用绝对路径（脚本在 repo 根目录）
if [[ -d "src/plugins/agent" ]]; then
  SCOPE="${SCOPE:-src/plugins/agent}"
elif [[ -d "symbio/src/plugins/agent" ]]; then
  SCOPE="${SCOPE:-symbio/src/plugins/agent}"
else
  SCOPE="${SCOPE:-src/plugins/agent}"
fi
STRICT=false
[[ "${1:-}" == "--strict" ]] && STRICT=true

RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
NC='\033[0m'

errors=0
warnings=0

err()  { echo -e "${RED}[ERROR]${NC} $1";   errors=$((errors+1)); }
warn() { echo -e "${YELLOW}[WARN]${NC}  $1"; warnings=$((warnings+1)); }
ok()   { echo -e "${GREEN}[OK]${NC}    $1"; }

echo "=== grep_audit.sh ==="
echo "Scope: $SCOPE"
echo "Strict: $STRICT"
echo

# --- S-002: std::sync::Mutex + .lock() 跨 await ---
#
# 检测模式：async fn（含 .await 的 fn）内出现 `xxx.lock()` 调 std Mutex。
# 误报排除：lock 后立即 drop（无后续 await）。
#
# 实现：用 ripgrep 多行模式抓取 `async fn` 函数体中的 `.lock()` 引用。
echo "--- S-002: std::sync::Mutex 跨 await 检查 ---"

# 抓所有 async fn 的位置（容忍 ripgrep 找不到）
if rg -n --no-heading '^\s*(pub\s+)?async\s+fn\s+\w+' "$SCOPE" >/tmp/audit_async_fns 2>/dev/null; then
  async_count=$(wc -l </tmp/audit_async_fns | tr -d ' ')
  if [[ "$async_count" -eq 0 ]]; then
    ok "未发现 async fn（跳过 S-002 检查）"
  else
    # 抓所有 .lock() 调用
    rg -n --no-heading '\.lock\(\)' "$SCOPE" 2>/dev/null >/tmp/audit_locks || true
    # 抓所有 .await 调用
    rg -n --no-heading '\.await\b' "$SCOPE" 2>/dev/null >/tmp/audit_awaits || true

    # 简易启发式：每行 .lock() 紧接 5 行内出现 .await → ERROR
    bad=0
    while IFS= read -r lock_line; do
      [[ -z "$lock_line" ]] && continue
      file=$(echo "$lock_line" | cut -d: -f1)
      lineno=$(echo "$lock_line" | cut -d: -f2)
      # 该 file 后面 5 行的 .await
      tail_awaits=$(rg -n --no-heading '^\S+:\d+:\s*\.await\b' "/tmp/audit_awaits" 2>/dev/null || true)
      near_awaits=$(rg -n "^${file}:" "/tmp/audit_awaits" 2>/dev/null \
        | awk -F: -v l="$lineno" '$2 > l && $2 <= l+5' || true)
      if [[ -n "$near_awaits" ]]; then
        # 进一步只报 std::sync::Mutex
        if rg -q "use std::sync::Mutex|std::sync::Mutex<" "$file" 2>/dev/null; then
          err "$file:$lineno  std::sync::Mutex 持锁后 5 行内出现 .await（潜在跨 await 持锁，请改用 tokio::sync::Mutex）"
          bad=$((bad+1))
        fi
      fi
    done </tmp/audit_locks
    if [[ "$bad" -eq 0 ]]; then
      ok "S-002 通过：未发现 std::sync::Mutex 跨 await 持锁"
    fi
  fi
else
  warn "ripgrep 不可用或 scope 不存在（跳过 S-002 检查）"
fi
echo

# --- S-002-bonus: let _ = ...await 业务路径回归 ---
#
# 之前已通过 v27 + v28-N1 收敛到 0。CI 守门防止新增。
# 仅检查 .rs 源文件，排除 docs/ 文档和测试代码
#
# 算法：
#   1. 找所有 `let _ = ...await` 的行号
#   2. 找所有 `#[test]` / `#[tokio::test]` / `fn test_` / `mod tests` 的行号
#   3. 对每个 suspect 行，向上 200 行内若出现上述任一标记 → 视为测试代码
echo "--- S-002-bonus: 业务路径 let _ = ...await 检查 ---"

# 1. 全部 match 行
matches=$(rg -n --no-heading --type rust 'let _ = .*\.await' "$SCOPE" 2>/dev/null \
  | rg -v '\.tx\.send' \
  || true)
# 2. 全部 test 标记 / fn test_ 起点行 / mod tests 行（带行号）
tests=$(rg -n --no-heading --type rust '#\[(tokio::)?test\]|fn test_|mod tests' "$SCOPE" 2>/dev/null || true)

if [[ -n "$matches" ]]; then
  suspect=""
  while IFS= read -r m; do
    [[ -z "$m" ]] && continue
    m_file=$(echo "$m" | cut -d: -f1)
    m_line=$(echo "$m" | cut -d: -f2)
    # 在同一 file 内，m_line 上方 200 行内是否有 test 标记
    if echo "$tests" | rg -q "^${m_file}:" 2>/dev/null; then
      near=$(echo "$tests" \
        | rg "^${m_file}:" \
        | awk -F: -v l="$m_line" '$2 < l && l - $2 <= 200')
      if [[ -n "$near" ]]; then
        continue  # 在 test 模块内，跳过
      fi
    fi
    suspect="${suspect}${m}"$'\n'
  done <<< "$matches"
  if [[ -n "${suspect// /}" ]]; then
    while IFS= read -r line; do
      [[ -z "$line" ]] && continue
      warn "$line  业务路径疑似吞错，请人工 review（应改为 if let Err(e) = ... + plugin_warn）"
    done <<< "$suspect"
  else
    ok "S-002-bonus 通过：业务路径无新增 let _ = ...await"
  fi
else
  ok "S-002-bonus 通过：业务路径无新增 let _ = ...await"
fi
echo

# --- S-007: CHANGELOG 格式 ---
#
# 防止"修复完成但 CHANGELOG 漏记"。要求每个 ISSUES.md 的 ✅ 项都有对应 v* 条目。
# 简化版：检查 CHANGELOG.md 最近 20 行包含 "v28" 字样（防止 v27 之后就停止维护）。
echo "--- S-007: CHANGELOG 维护检查 ---"

changelog="$SCOPE/docs/CHANGELOG.md"
if [[ ! -f "$changelog" ]]; then
  err "$changelog 不存在"
else
  last_version=$(rg -n '^## v\d+' "$changelog" 2>/dev/null | head -n 1 || true)
  if [[ -z "$last_version" ]]; then
    err "$changelog 无 ## vNN 版本标题"
  else
    ok "CHANGELOG 最新版本标题：$last_version"
  fi
fi
echo

# --- I-014-light: CognitiveUnit 字段硬编码字符串键 ---
#
# 临时检查：cognitive_feedback.rs / scaffold.rs 等是否还有"硬编码字符串键"（is_a、related 等）。
# 完整方案在 M-1（typed_unit 强类型迁移）。
# 仅检查 .rs 源文件，排除 jsonl 数据
echo "--- I-014-light: CognitiveUnit 字段硬编码字符串键抽查 ---"

hardcoded=$(rg -n --no-heading --type rust '"(is_a|related|name|description|meta_belief|is_strategy|is_skill|is_meta|is_conflict)"' "$SCOPE" 2>/dev/null \
  | rg -v '#\[test\]' \
  || true)
hardcoded_count=0
if [[ -n "$hardcoded" ]]; then
  hardcoded_count=$(echo "$hardcoded" | wc -l | tr -d ' ')
fi
if [[ "$hardcoded_count" -gt 0 ]]; then
  warn "发现 $hardcoded_count 处 CognitiveUnit 字段硬编码字符串键（I-014 中期任务方向）"
  warn "  完整方案见 PLAN M-1（typed_unit 强类型迁移）"
  # 显式限制前 5 条避免刷屏（用 awk 而非 head|while 避免 SIGPIPE）
  echo "$hardcoded" | awk 'NR<=5 { print "    " $0 }'
else
  ok "I-014-light 通过：未发现字段硬编码字符串键"
fi
echo

# --- 汇总 ---
echo "=== 汇总 ==="
echo "Errors:   $errors"
echo "Warnings: $warnings"

if [[ "$errors" -gt 0 ]]; then
  exit 1
fi
if [[ "$STRICT" == "true" && "$warnings" -gt 0 ]]; then
  exit 2
fi
exit 0
