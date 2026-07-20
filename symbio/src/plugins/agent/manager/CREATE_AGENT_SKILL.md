---
name: create_agent_skill
description: 智能体创建技能。当用户提到创建某个公众人物（如名人、专家、学者等）的智能体时，此技能会被激活。
allowedTools:
  - agent_create
  - agent_query
  - agent_store
  - web_search
  - web_fetch
  - read_file
  - write_file
  - file_edit
  - shell
when_to_use: 创建关于某个人的智能体、创建某个名人的分身、模仿某个公众人物
---

# Agent Creator Skill

## 🎯 触发条件

当用户请求创建某个**特定公众人物**的智能体时触发：
- "创建张雪峰的智能体"
- "创建一个模仿马斯克的助手"
- "我想创建一个名人分身"

## ⚡ 核心原则

1. **主动搜索**：不依赖用户提供的详细信息，主动从网络搜索公众人物的相关资料
2. **信息验证**：收集多来源信息，进行分析和验证
3. **七维认知**：按照七维认知模型构建智能体配置文件

## 📋 执行流程

### 步骤 1: 识别目标人物
从用户请求中提取公众人物的名称。

### 步骤 2: 主动信息收集
使用 `web_search` 搜索目标人物的背景信息：
- 搜索格式：`"[人物名称] 简介 背景 成就"`
- 收集维度：身份背景、专业领域、成就贡献、风格特点、公众评价等

### 步骤 3: 深入了解
使用 `web_fetch` 获取关键网页内容，了解更详细的信息。

### 步骤 4: 分析与提炼
根据收集到的信息，提炼出七维认知模型所需的各个单元：

| 维度 | 需要提炼的信息 |
|------|---------------|
| fact | 事实知识 |
| rule | 行为准则、处世原则、价值观 |
| skill | 专业能力、主要成就、擅长领域 |
| judgment | 判断标准、决策风格 |
| strategy | 思考方式、解决问题的方法论 |
| tone | 表达风格、语言特点、沟通方式 |
| knowledge | 专业知识体系、经验积累 |

### 步骤 5: 部署智能体
使用 `agent_create` 直接创建智能体：
- 参数: `{"id": "${person_name}", "is_global": false, "cognition_units": [...]}`

## 📝 配置文件格式模板

**注意**：调用 `agent_create` 工具时，`cognition_units` 字段需要传入 **JSON 数组格式**：

```json
{
  "id": "${person_name}",
  "is_global": false,
  "cognition_units": [
    {
      "id": "identity",
      "is_a": ["fact"],
      "name": "${人物名称}",
      "description": "${一句话描述人物的核心定位和公众形象}",
      "content": "${人物的性格特点和行为特征}"
    },
    {
      "id": "core_rules",
      "is_a": ["rule"],
      "level": "sys",
      "description": "${提炼的行为准则}"
    },
    {
      "id": "core_skills",
      "is_a": ["skill"],
      "level": "sys",
      "description": "${专业领域描述}",
      "content": "${技能列表}"
    },
    {
      "id": "core_judgment",
      "is_a": ["judgment"],
      "level": "sys",
      "description": "${人物的判断标准和决策风格}"
    },
    {
      "id": "thinking_strategy",
      "is_a": ["strategy"],
      "level": "sys",
      "description": "${人物的思考方式和解决问题的方法论}"
    },
    {
      "id": "communication_style",
      "is_a": ["tone"],
      "level": "sys",
      "description": "${人物的语言风格和表达特点}"
    },
    {
      "id": "knowledge::domain_knowledge",
      "is_a": ["knowledge"],
      "level": "msg",
      "description": "${专业知识体系和个人经验}"
    }
  ]
}
```

## 🔍 搜索策略示例

### 张雪峰
```
搜索: "张雪峰 高考志愿 教育专家 背景"
```

### 马斯克
```
搜索: "马斯克 特斯拉 SpaceX 埃隆·马斯克 简介"
```

### 其他公众人物
```
搜索: "[人物名称] 简介 个人履历 成就 风格"
```

## ⚠️ 注意事项

1. **智能体 ID**：使用人物名称的拼音或英文（如 `zhangxuefeng`、`elon_musk`）
2. **信息真实性**：多来源交叉验证，避免错误信息
3. **七维完整性**：尽量包含所有七个维度
4. **保持中立**：客观描述人物特点，不添加主观评价
5. **必需字段**：`cognition_units` 必须包含 `id` 为 `identity` 的认知单元，且该单元必须包含 `name` 字段
