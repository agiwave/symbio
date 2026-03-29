//! Calculator Plugin

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct CalculatorPlugin {
    meta: PluginMeta,
}

impl CalculatorPlugin {
    pub fn new() -> Self {
        CalculatorPlugin {
            meta: PluginMeta {
                name: "calculator".to_string(),
                description: "简单计算器，执行基本数学运算".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "a": {
                            "type": "number",
                            "description": "第一个操作数",
                            "default": 0
                        },
                        "b": {
                            "type": "number",
                            "description": "第二个操作数",
                            "default": 0
                        },
                        "operation": {
                            "type": "string",
                            "description": "运算类型",
                            "default": "add",
                            "enum": ["add", "subtract", "multiply", "divide"]
                        }
                    },
                    "required": ["a", "b", "operation"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "result": {
                            "type": "number",
                            "description": "计算结果"
                        },
                        "expression": {
                            "type": "string",
                            "description": "运算表达式"
                        }
                    },
                    "required": ["result", "expression"]
                })),
                author: Some("Symbio Team".to_string()),
            },
        }
    }
}

#[async_trait::async_trait]
impl Plugin for CalculatorPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(self.meta.clone())
        } else {
            Err(PluginError::NotFound(format!("插件路径 '{}' 未找到", path)))
        }
    }
    
    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("插件路径 '{}' 未找到", path)));
        }
        
        let obj = input.as_object()
            .ok_or_else(|| PluginError::ValidationError("输入必须是对象".to_string()))?;
        
        let a = obj.get("a")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| PluginError::ValidationError("参数 'a' 必须是数字".to_string()))?;
        
        let b = obj.get("b")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| PluginError::ValidationError("参数 'b' 必须是数字".to_string()))?;
        
        let operation = obj.get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("add");
        
        let (result, op_symbol) = match operation {
            "add" => (a + b, "+"),
            "subtract" => (a - b, "-"),
            "multiply" => (a * b, "×"),
            "divide" => {
                if b == 0.0 {
                    return Err(PluginError::ValidationError("除数不能为零".to_string()));
                }
                (a / b, "÷")
            }
            _ => return Err(PluginError::ValidationError(format!("未知的运算：{}", operation))),
        };
        
        let result_value = Value::Object(serde_json::Map::from_iter([
            ("result".to_string(), Value::Number(serde_json::Number::from_f64(result).unwrap())),
            ("expression".to_string(), Value::String(format!("{} {} {} = {}", a, op_symbol, b, result))),
        ]));
        
        Ok(InvokeStream::single(result_value))
    }
}

impl Default for CalculatorPlugin {
    fn default() -> Self {
        Self::new()
    }
}
