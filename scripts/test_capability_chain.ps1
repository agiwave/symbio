# 能力链测试脚本
# 验证：LLM调用 → 能力执行 → 能力返回正确结果 → LLM正确使用结果

$ErrorActionPreference = "Stop"

$CLI_PATH = "c:\Bing\agiwave\symbio\symbio\target\release\cli.exe"
$WORKDIR = "c:\Bing\agiwave\symbio"

# 测试场景 - 专注于验证能力的实际工作效果
$testScenarios = @(
    @{
        Name = "1. 知识查询 (agent_query) 测试"
        Description = "验证LLM能正确调用查询能力，且查询能力返回正确结果"
        Prompt = "请使用 agent_query 查询 ID 为 'sun_rise' 的知识，然后告诉我你找到了什么"
        Expected = @{
            ShouldCallTool = $true
            ToolName = "agent_query"
            ShouldFindContent = @("sun_rise", "太阳", "升起", "降雨", "地面变湿")
            ShouldUseResult = $true
        }
    },
    @{
        Name = "2. 知识存储 (agent_store) 测试"
        Description = "验证LLM能正确调用存储能力"
        Prompt = "请使用 agent_store 存储一条新知识，ID 为 'test_fact_1'，类型为 'fact'，描述为 '这是一条测试知识'"
        Expected = @{
            ShouldCallTool = $true
            ToolName = "agent_store"
            ShouldUseResult = $true
        }
    },
    @{
        Name = "3. 验证存储结果"
        Description = "验证刚才存储的知识能被正确查询到"
        Prompt = "请查询一下是否有 ID 为 'test_fact_1' 的知识"
        Expected = @{
            ShouldCallTool = $true
            ToolName = "agent_query"
            ShouldFindContent = @("test_fact_1")
        }
    }
)

Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host "  能力链完整测试" -ForegroundColor Cyan
Write-Host "  验证：LLM调用 → 能力执行 → 能力返回正确结果 → LLM正确使用结果" -ForegroundColor Cyan
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host ""

$results = @()
$sessionCounter = 0

foreach ($scenario in $testScenarios) {
    $sessionCounter++
    $sessionId = "test_chain_$sessionCounter"
    
    Write-Host "[$($scenario.Name)]" -ForegroundColor Yellow
    Write-Host "  描述: $($scenario.Description)"
    Write-Host "  提示: $($scenario.Prompt)"
    Write-Host ""
    
    $outputFile = "test_output_$sessionCounter.txt"
    
    try {
        Write-Host "  执行测试..." -ForegroundColor Gray
        
        # 运行CLI并捕获输出
        $process = Start-Process -FilePath $CLI_PATH -ArgumentList @(
            "--agent", "tester",
            "--session", $sessionId,
            $scenario.Prompt
        ) -WorkingDirectory $WORKDIR -NoNewWindow -Wait -RedirectStandardOutput $outputFile
        
        $exitCode = $process.ExitCode
        $fullOutput = Get-Content $outputFile -Raw
        
        Write-Host "  退出码: $exitCode"
        Write-Host ""
        
        # 分析输出
        $toolCalled = $false
        $toolNameMatched = $false
        $contentFound = $false
        $resultUsed = $false
        $issues = @()
        
        # 1. 检查能力是否被调用
        if ($fullOutput -match "\[\s*Tool\s*\]\s*Execution\s*started") {
            $toolCalled = $true
            Write-Host "  ✓ 检测到能力调用" -ForegroundColor Green
        } else {
            Write-Host "  ✗ 未检测到能力调用" -ForegroundColor Red
            $issues += "未调用能力"
        }
        
        # 2. 检查是否调用了正确的能力
        if ($scenario.Expected.ToolName -and $fullOutput -match [regex]::Escape($scenario.Expected.ToolName)) {
            $toolNameMatched = $true
            Write-Host "  ✓ 检测到目标能力: $($scenario.Expected.ToolName)" -ForegroundColor Green
        } elseif ($scenario.Expected.ToolName) {
            Write-Host "  ✗ 未检测到目标能力: $($scenario.Expected.ToolName)" -ForegroundColor Red
            $issues += "未调用正确的能力"
        }
        
        # 3. 检查是否找到了预期的内容
        if ($scenario.Expected.ShouldFindContent) {
            $foundKeywords = @()
            foreach ($keyword in $scenario.Expected.ShouldFindContent) {
                if ($fullOutput -match [regex]::Escape($keyword)) {
                    $foundKeywords += $keyword
                }
            }
            if ($foundKeywords.Count -gt 0) {
                $contentFound = $true
                Write-Host "  ✓ 找到预期内容: $($foundKeywords -join ', ')" -ForegroundColor Green
            } else {
                Write-Host "  ✗ 未找到预期内容" -ForegroundColor Red
                $issues += "未找到预期内容"
            }
        }
        
        # 4. 检查LLM是否使用了能力返回的结果
        if ($scenario.Expected.ShouldUseResult -and $toolCalled) {
            # 简单判断：如果有工具调用且回答有实质内容，认为使用了结果
            $answerLines = ($fullOutput -split "`n" | Where-Object { $_ -match "^\s*[^#\[\s]" } | Select-Object -Last 10)
            if ($answerLines.Count -gt 2) {
                $resultUsed = $true
                Write-Host "  ✓ LLM基于能力结果给出了回答" -ForegroundColor Green
            }
        }
        
        $success = $toolCalled -and 
                   (-not $scenario.Expected.ToolName -or $toolNameMatched) -and 
                   (-not $scenario.Expected.ShouldFindContent -or $contentFound)
        
        $result = [PSCustomObject]@{
            Scenario = $scenario.Name
            ToolCalled = $toolCalled
            CorrectTool = $toolNameMatched
            ContentFound = $contentFound
            ResultUsed = $resultUsed
            Success = $success
            Issues = $issues -join "; "
            ExitCode = $exitCode
        }
        
        $results += $result
        
        Write-Host ""
        Write-Host "  结果: $(if ($result.Success) { "✓ 通过" } else { "✗ 失败" })" -ForegroundColor $(if ($result.Success) { "Green" } else { "Red" })
        
    } catch {
        Write-Host "  错误: $_" -ForegroundColor Red
        
        $result = [PSCustomObject]@{
            Scenario = $scenario.Name
            ToolCalled = $false
            CorrectTool = $false
            ContentFound = $false
            ResultUsed = $false
            Success = $false
            Issues = "执行错误: $_"
            ExitCode = -1
        }
        
        $results += $result
    }
    
    Write-Host ""
    Write-Host "-" * 80 -ForegroundColor Gray
    Write-Host ""
}

# 生成报告
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host "  测试报告" -ForegroundColor Cyan
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host ""

$total = $results.Count
$passed = ($results | Where-Object { $_.Success }).Count
$toolCalledRate = ($results | Where-Object { $_.ToolCalled }).Count

Write-Host "统计:" -ForegroundColor Yellow
Write-Host "  总测试数: $total"
Write-Host "  通过数: $passed"
Write-Host "  能力调用率: $toolCalledRate/$total"
Write-Host "  通过率: $([math]::Round(($passed / $total) * 100, 1))%"
Write-Host ""

Write-Host "详细结果:" -ForegroundColor Yellow
foreach ($r in $results) {
    $status = if ($r.Success) { "✓" } else { "✗" }
    Write-Host "  $status $($r.Scenario)"
    Write-Host "    能力调用: $(if ($r.ToolCalled) { "✓" } else { "✗" }), 正确能力: $(if ($r.CorrectTool) { "✓" } else { "✗" }), 内容找到: $(if ($r.ContentFound) { "✓" } else { "✗" })"
    if ($r.Issues) {
        Write-Host "    问题: $($r.Issues)" -ForegroundColor Yellow
    }
}

Write-Host ""
Write-Host "=" * 80 -ForegroundColor Cyan
Write-Host "  测试完成" -ForegroundColor Cyan
Write-Host "=" * 80 -ForegroundColor Cyan

# 清理临时文件
Get-ChildItem -Path . -Filter "test_output_*.txt" -ErrorAction SilentlyContinue | Remove-Item
