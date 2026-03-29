# T010: RNA-seq 分析流程模板

## 基本信息

| 属性 | 值 |
|------|-----|
| 任务ID | T010 |
| 标题 | RNA-seq 分析流程模板 |
| 阶段 | Phase 1: 核心能力 (MVP) |
| 优先级 | P0 |
| 预估工时 | 24h |
| 状态 | pending |
| 依赖 | T005, T009 |

## 任务描述

创建完整的 RNA-seq 差异表达分析流程模板，用户可以选择模板后一键执行整个分析流程。

## 验收标准

- [ ] 模板包含完整的分析步骤
- [ ] 每个步骤有清晰的说明
- [ ] 参数可配置
- [ ] 执行结果可追溯
- [ ] 模板可保存复用

## 技术要求

### 模板结构

```typescript
interface AnalysisTemplate {
  id: string;
  name: string;
  description: string;
  category: 'rna-seq' | 'chip-seq' | 'scRNA-seq';
  
  // 分析步骤
  steps: AnalysisStep[];
  
  // 默认参数
  defaultParams: Record<string, any>;
  
  // 输入要求
  inputRequirements: InputRequirement[];
}

interface AnalysisStep {
  id: string;
  name: string;
  description: string;
  
  // 代码模板
  codeTemplate: string;
  
  // 参数定义
  params: ParamDefinition[];
  
  // 预期输出
  expectedOutputs: OutputDefinition[];
  
  // 验证规则
  validationRules: ValidationRule[];
}
```

### RNA-seq 流程步骤

```
RNA-seq 差异表达分析流程
│
├── Step 1: 数据质控
│   ├── FastQC 质控
│   └── MultiQC 汇总
│
├── Step 2: 序列比对
│   ├── 建立索引
│   └── HISAT2 比对
│
├── Step 3: 表达定量
│   └── featureCounts 定量
│
├── Step 4: 差异分析
│   ├── 数据导入
│   ├── 标准化
│   └── DESeq2 分析
│
└── Step 5: 功能注释
    ├── GO 富集
    └── KEGG 通路
```

### 模板文件示例

```markdown
# RNA-seq 差异表达分析

## 参数配置

| 参数 | 默认值 | 说明 |
|------|--------|------|
| reference_genome | hg38 | 参考基因组 |
| paired_end | true | 是否双端测序 |
| fdr_threshold | 0.05 | FDR 阈值 |
| fc_threshold | 1.0 | Fold Change 阈值 |

---

## Step 1: 数据质控

### 1.1 FastQC 质控

\`\`\`bash run
fastqc *.fastq.gz -o qc_results
\`\`\`

### 1.2 MultiQC 汇总

\`\`\`bash run
multiqc qc_results -o multiqc_report
\`\`\`

---

## Step 2: 序列比对

### 2.1 建立索引

\`\`\`bash run
hisat2-build {{reference_genome}}.fa {{reference_genome}}_index
\`\`\`

### 2.2 比对

\`\`\`bash run
hisat2 -x {{reference_genome}}_index \
       -1 {{sample}}_R1.fastq.gz \
       -2 {{sample}}_R2.fastq.gz \
       -S {{sample}}.sam
\`\`\`\`

<!-- 后续步骤... -->
```

## 子任务

1. **模板数据结构设计** (4h)
   - 定义模板接口
   - 设计参数系统
   - 设计验证规则

2. **质控步骤模板** (4h)
   - FastQC 代码
   - MultiQC 代码
   - 参数说明

3. **比对步骤模板** (4h)
   - HISAT2 代码
   - 参数配置
   - 结果验证

4. **定量步骤模板** (4h)
   - featureCounts 代码
   - 结果格式处理

5. **差异分析模板** (4h)
   - DESeq2 R 脚本
   - 可视化代码
   - 结果导出

6. **功能注释模板** (2h)
   - GO 富集代码
   - KEGG 通路代码

7. **模板测试** (2h)
   - 端到端测试
   - 参数验证测试

## 依赖

- T005: 代码块执行引擎
- T009: 数据文件识别

## 输出物

- RNA-seq 模板文件
- 模板解析器
- 参数配置 UI

## 风险

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 参考数据下载慢 | 中 | 提供本地缓存机制 |
| 参数配置复杂 | 中 | 提供智能默认值 |

## 备注

模板是用户学习的核心载体，需要注重可读性和可配置性。
