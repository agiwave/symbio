# 能力模块测试脚本
# 使用CLI工具直接测试，通过分析输出验证能力调用情况

$ErrorActionPreference = "Stop"

# 测试配置
$CLI_PATH = "c:\Bing\agiwave\symbio\symbio\target\release\cli.exe"
$WORKDIR = "c:\Bing\agiwave\symbio"

# 测试场景定义
$testScenarios = @(
    @{
        Name = "知识查询 (agent_query)"
        Description = "测试知识检索能力"
        Prompt = "请查询并告诉我关于日出的知识"
        Verification = @{
            ToolCallKeyword = "agent_query"
            ExpectedOutputKeywords = @("sun_rise", "太阳", "升起")
        }
    },
    @{
        Name = "知识存储 (agent_store)"
        Description = "测试知识存储能力"
        Prompt = "请帮我存储一条新知识：地球是圆的"
        Verification = @{
            ToolCallKeyword = "agent_store"
            ExpectedOutputKeywords = @("存储", "保存", "knowledge")
        }
    },
    @{
        Name = "因果推理 (agent_reasoning)"
        Description = "测试因果推理能力"
        Prompt = "分析一下降雨和地面变湿之间的因果关系"
        Verification = @{
            ToolCallKeyword = "agent_reasoning"
            ExpectedOutputKeywords = @("因果", "cause", "effect", "关系")
        }
    },
    @{
        Name = "类比推理 (agent_analogy)"
        Description = "测试类比推理能力"
        Prompt = "用水循环和能量流动做一个类比分析"
        Verification = @{
            ToolCallKeyword = "agent_analogy"
            ExpectedOutputKeywords = @("类比", "analogy", "相似", "similar")
        }
    },
    @{
        Name = "目标规划 (agent_goal_planner)"
        Description = "测试目标规划能力"
        Prompt = "帮我规划一个完成Rust项目的详细计划"
        Verification = @{
            ToolCallKeyword = "agent_goal_planner"
            ExpectedOutputKeywords = @("计划", "plan", "步骤", "task")
        }
    },
    @{
        Name = "元认知 (agent_metacognition)"
        Description = "测试元认知能力"
        Prompt = "反思一下你是如何回答问题的，有什么可以改进的地方"
        Verification = @{
            ToolCallKeyword = "agent_metacognition"
            ExpectedOutputKeywords = @("反思", "reflection", "改进", "improve")
        }
    },
    @{
        Name = "知识提取 (agent_learn)"
        Description = "测试知识提取能力"
        Prompt = "从这句话中提取并存储知识：Rust的所有权系统可以防止内存泄漏"
        Verification = @{
            ToolCallKeyword = "agent_learn"
            ExpectedOutputKeywords = @("提取", "extract", "learn", "学习")
        }
    },
    @{
        Name = "记忆管理 (agent_memory_manage)"
        Description = "测试记忆管理能力"
        Prompt = "分析一下你的知识库，看看有什么可以优化的地方"
        Verification = @{
            ToolCallKeyword = "agent_memory_manage"
            ExpectedOutputKeywords = @("记忆", "memory", "优化", "optimize")
        }
    },
    @{
        Name = "符号推理 (agent_symbolic_reasoner)"
        Description = "测试符号推理能力"
        Prompt = "验证这个逻辑：所有人都会死，苏格拉底是人，所以苏格拉底会死"
        Verification = @{
            ToolCallKeyword = "agent_symbolic_reasoner"
            ExpectedOutputKeywords = @("逻辑", "logic", "推理", "syllogism")
        }
    },
    @{
        Name = "知识演化 (agent_knowledge_evolution)"
        Description = "测试知识演化能力"
        Prompt = "检查一下你的知识库中有没有冲突的知识"
        Verification = @{
            ToolCallKeyword = "agent_knowledge_evolution"
            ExpectedOutputKeywords = @("冲突", "conflict", "演化", "evolution")
        }
    }
)

# 结果数组
$results = @()
$sessionCounter = 0

Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host "  Agent 能力模块测试" -ForegroundColor Cyan
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host ""

foreach ($scenario in $testScenarios) {
    $sessionCounter++
    $sessionId = "test_cap_$sessionCounter"
    
    Write-Host "[$($scenario.Name)]" -ForegroundColor Yellow
    Write-Host "  描述: $($scenario.Description)"
    Write-Host "  提示: $($scenario.Prompt)"
    Write-Host "  会话: $sessionId"
    Write-Host ""
    
    $toolCalled = $false
    $outputMatched = $false
    $issues = @()
    $fullOutput = ""
    
    try {
        Write-Host "  执行中..." -ForegroundColor Gray
        
        $process = Start-Process -FilePath $CLI_PATH -ArgumentList @(
            "--agent", "tester",
            "--session", $sessionId,
            $scenario.Prompt
        ) -WorkingDirectory $WORKDIR -NoNewWindow -Wait -RedirectStandardOutput "temp_output.txt" -RedirectStandardError "temp_error.txt"
        
        $exitCode = $process.ExitCode
        $fullOutput = Get-Content "temp_output.txt" -Raw
        
        Write-Host "  退出码: $exitCode"
        
        # 分析输出
        if ($fullOutput -match [regex]::Escape($scenario.Verification.ToolCallKeyword)) {
            $toolCalled = $true
            Write-Host "  ✓ 检测到能力调用: $($scenario.Verification.ToolCallKeyword)" -ForegroundColor Green
        } else {
            Write-Host "  ✗ 未检测到能力调用: $($scenario.Verification.ToolCallKeyword)" -ForegroundColor Red
            $issues += "未检测到能力调用"
        }
        
        # 检查输出内容
        $matchedKeywords = @()
        foreach ($keyword in $scenario.Verification.ExpectedOutputKeywords) {
            if ($fullOutput -match [regex]::Escape($keyword)) {
                $matchedKeywords += $keyword
            }
        }
        
        if ($matchedKeywords.Count -gt 0) {
            $outputMatched = $true
            Write-Host "  ✓ 检测到相关输出关键词: $($matchedKeywords -join ', ')" -ForegroundColor Green
        } else {
            Write-Host "  ✗ 未检测到相关输出关键词" -ForegroundColor Red
            $issues += "未检测到相关输出"
        }
        
        $success = $toolCalled -or $outputMatched
        $confidence = if ($toolCalled -and $outputMatched) { 1.0 }
                      elseif ($toolCalled -or $outputMatched) { 0.5 }
                      else { 0.0 }
        
        $result = [PSCustomObject]@{
            ScenarioName = $scenario.Name
            ToolCalled = $toolCalled
            OutputMatched = $outputMatched
            Success = $success
            Confidence = $confidence
            Issues = $issues -join "; "
            ExitCode = $exitCode
        }
        
        $results += $result
        
        Write-Host ""
        Write-Host "  结果: $(if ($result.Success) { "✓ 通过" } else { "✗ 失败" })" -ForegroundColor $(if ($result.Success) { "Green" } else { "Red" })
        Write-Host "  置信度: $([math]::Round($result.Confidence * 100, 0))%"
        
    } catch {
        Write-Host "  错误: $_" -ForegroundColor Red
        
        $result = [PSCustomObject]@{
            ScenarioName = $scenario.Name
            ToolCalled = $false
            OutputMatched = $false
            Success = $false
            Confidence = 0.0
            Issues = "执行错误: $_"
            ExitCode = -1
        }
        
        $results += $result
    }
    
    Write-Host ""
    Write-Host "-" * 60 -ForegroundColor Gray
    Write-Host ""
}

# 清理临时文件
Remove-Item "temp_output.txt" -ErrorAction SilentlyContinue
Remove-Item "temp_error.txt" -ErrorAction SilentlyContinue

# 生成报告
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host "  测试报告" -ForegroundColor Cyan
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host ""

$total = $results.Count
$passed = ($results | Where-Object { $_.Success }).Count
$toolCalledCount = ($results | Where-Object { $_.ToolCalled }).Count
$outputMatchedCount = ($results | Where-Object { $_.OutputMatched }).Count
$avgConfidence = ($results | Measure-Object -Property Confidence -Average).Average

Write-Host "测试统计:" -ForegroundColor Yellow
Write-Host "  总测试数: $total"
Write-Host "  通过数: $passed"
Write-Host "  失败数: $($total - $passed)"
Write-Host "  能力调用检测: $toolCalledCount/$total"
Write-Host "  输出匹配检测: $outputMatchedCount/$total"
Write-Host "  通过率: $([math]::Round(($passed / $total) * 100, 1))%"
Write-Host "  平均置信度: $([math]::Round($avgConfidence * 100, 0))%"
Write-Host ""

Write-Host "详细结果:" -ForegroundColor Yellow
foreach ($result in $results) {
    $status = if ($result.Success) { "✓" } else { "✗" }
    $statusColor = if ($result.Success) { "Green" } else { "Red" }
    Write-Host "  $status $($result.ScenarioName)" -ForegroundColor $statusColor
    Write-Host "    能力调用: $(if ($result.ToolCalled) { "✓" } else { "✗" }), 输出匹配: $(if ($result.OutputMatched) { "✓" } else { "✗" }), 置信度: $([math]::Round($result.Confidence * 100, 0))%"
    if ($result.Issues -ne "") {
        Write-Host "    问题: $($result.Issues)" -ForegroundColor Yellow
    }
}

Write-Host ""
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host "  测试完成" -ForegroundColor Cyan
Write-Host "=" * 80 -ForegroundColor Cyan
