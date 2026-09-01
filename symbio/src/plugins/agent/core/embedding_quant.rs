//! Q8 (per-vector affine) 量化模块
//!
//! 把 `Vec<f32>` embedding 压缩为 `Vec<u8> + f32 scale + f32 zero_point`，
//! 内存节省 ~3.5×（N×4B → N+8B），误差 ≤ 1 LSB ≈ `scale`。
//!
//! ## 算法
//!
//! 1. 对每条 embedding，统计 `min` / `max`
//! 2. `scale = (max - min) / 255.0`
//! 3. `zero_point = clamp(round(-min / scale), 0, 255)`
//! 4. `q_i = clamp(round(v_i / scale + zero_point), 0, 255)`
//!
//! 反量化：`v_i' = (q_i - zero_point) * scale`
//!
//! ## 与 EmbeddingStore 桶码的关系
//!
//! 本模块是**存储层**量化（用于压缩 `_ext_embedding`），与
//! `embedding_store::quantize_embedding` 的 **1-bit mean 桶码**（用于 ANN 检索）
//! 是两个独立维度。流程：
//!
//! ```text
//! f32 原始向量 ── Q8 量化 ──> u8 存储（_ext_embedding）
//!        │
//!        └── 反量化 ──> f32 内存向量 ──> 1-bit 桶码（bucket_index）
//! ```
//!
//! ## 兼容
//!
//! `_ext_embedding` 历史数据可能为 `Value::Array<Number>`（直接 f32 数组）。
//! 读取时 `dequantize_from_value` 自动识别两种格式并返回 f32 向量：
//! - **Q8 格式**：`Value::Object` 且含 `q8` 字段 → Q8 反量化
//! - **Legacy 格式**：`Value::Array<Number>` → 直接读为 f32（**不做 Q8 转换**）
//!
//! 写入统一为 Q8 v2 格式（`Value::Object { q8, scale, zero_point, _format_version: 2 }`）。
//!
//! ## ⚠ v19 静默数据丢失修复
//!
//! v16-v18 的 `value_to_q8` 把 legacy `Value::Array<f32>` 误当作 Q8 字节读取：
//! ```ignore
//! // v18 错误代码（已删除）
//! let data: Vec<u8> = arr.iter()
//!     .filter_map(|x| x.as_f64().map(|f| f.round().clamp(0.0, 255.0) as u8))
//!     .collect();
//! ```
//! 对 `[-0.3, 0.5, 1.2]` 这类典型 embedding，clamp 会把负值置 0、小于 0.5 的值四舍五入到 0，
//! **静默丢失一半数据**。v19 修复后 legacy 路径直接读为 f32 数组，不再走 Q8 转换。

use serde_json::{Map, Value};

/// Q8 格式版本（静默数据丢失）
///
/// 每次修改 Q8 序列化/反量化逻辑时**必须**递增此值。
/// 读取时若 `_format_version` 不匹配，按错误处理（不要静默兼容）。
///
/// ## 修订记录
/// - v1（v16 引入）：`{ q8, scale, zero_point }`
/// - v2（v19 引入）：v1 + `_format_version: 2` 字段
pub const Q8_FORMAT_VERSION_V2: u32 = 2;

/// Q8 量化后的存储结构
#[derive(Debug, Clone, PartialEq)]
pub struct Q8Embedding {
    /// 量化后的字节（每元素 ∈ [0, 255]）
    pub data: Vec<u8>,
    /// `(max - min) / 255.0`（零向量时为 1.0）
    pub scale: f32,
    /// 反量化偏移：反量化值 = `(q - zero_point) * scale`
    pub zero_point: f32,
}

impl Q8Embedding {
    /// 估算内存占用（含 scale/zero_point）
    ///
    /// 仅测试使用：用于验证 Q8 量化确实能显著降低存储体积。
    /// 生产代码无调用方，避免污染公共 API。
    #[cfg(test)]
    pub fn approx_bytes(&self) -> usize {
        self.data.len() + 8
    }
}

/// 对 `Vec<f32>` 做 per-vector affine Q8 量化
///
/// 边界处理：
/// - 空向量 → 全部 0，scale=1.0, zero_point=0
/// - 全常数向量（min==max）→ scale 强制 1.0 避免除零
/// - NaN/Inf 输入 → 替换为 0
pub fn quantize_q8(input: &[f32]) -> Q8Embedding {
    if input.is_empty() {
        return Q8Embedding {
            data: Vec::new(),
            scale: 1.0,
            zero_point: 0.0,
        };
    }

    // NaN/Inf → 0（与 CognitiveUnit::set_embedding 行为一致）
    let safe: Vec<f32> = input
        .iter()
        .map(|v| if v.is_finite() { *v } else { 0.0 })
        .collect();

    // 1) 找 min / max
    let mut min = safe[0];
    let mut max = safe[0];
    for &v in &safe {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }

    // 2) 全部相等（min==max）→ 退化为单位映射，scale=1.0
    let range = max - min;
    let (scale, zero_point) = if range <= f32::EPSILON {
        (1.0_f32, 0.0_f32)
    } else {
        let s = range / 255.0;
        let zp = (-min / s).round().clamp(0.0, 255.0);
        (s, zp)
    };

    // 3) 量化
    let inv_scale = if scale > f32::EPSILON {
        1.0 / scale
    } else {
        0.0
    };
    let data: Vec<u8> = safe
        .iter()
        .map(|&v| {
            let q = (v * inv_scale + zero_point).round().clamp(0.0, 255.0);
            q as u8
        })
        .collect();

    Q8Embedding {
        data,
        scale,
        zero_point,
    }
}

/// Q8 → f32 反量化
///
/// 计算：`v_i' = (q_i as f32 - zero_point) * scale`
pub fn dequantize_q8(q: &Q8Embedding) -> Vec<f32> {
    q.data
        .iter()
        .map(|&b| (b as f32 - q.zero_point) * q.scale)
        .collect()
}

/// 把 Q8 结构序列化为 `Value::Object`（v2 格式：含 `_format_version`）
///
/// 输出 JSON：
/// ```json
/// { "q8": [u8, ...], "scale": f32, "zero_point": f32, "_format_version": 2 }
/// ```
///
/// ## NaN/Inf 处理（v19 修复）
/// `scale` / `zero_point` 若为 NaN/Inf 会被替换为 0 并 `plugin_warn!` 上报，
/// 避免反量化时产生 NaN 污染整个语义搜索结果。
pub fn q8_to_value(q: &Q8Embedding) -> Value {
    // v19：拒绝 NaN/Inf（替换为 0 + warn），否则反量化会产生 NaN 污染
    let (scale, zero_point) = sanitize_q8_params(q.scale, q.zero_point);

    let mut obj = Map::with_capacity(4);
    obj.insert(
        "q8".to_string(),
        Value::Array(
            q.data
                .iter()
                .map(|&b| Value::Number(serde_json::Number::from(b)))
                .collect(),
        ),
    );
    obj.insert(
        "scale".to_string(),
        Value::Number(
            serde_json::Number::from_f64(scale as f64)
                .unwrap_or_else(|| serde_json::Number::from(0)),
        ),
    );
    obj.insert(
        "zero_point".to_string(),
        Value::Number(
            serde_json::Number::from_f64(zero_point as f64)
                .unwrap_or_else(|| serde_json::Number::from(0)),
        ),
    );
    obj.insert(
        "_format_version".to_string(),
        Value::Number(serde_json::Number::from(Q8_FORMAT_VERSION_V2)),
    );
    Value::Object(obj)
}

/// 把 NaN/Inf scale/zero_point 替换为 0 并 `plugin_warn!` 上报
fn sanitize_q8_params(scale: f32, zero_point: f32) -> (f32, f32) {
    let mut s = scale;
    let mut zp = zero_point;
    let mut dirty = false;
    if !s.is_finite() {
        crate::plugin_warn!(
            "agent",
            "[Q8] scale is not finite ({:?}), replaced with 0.0 — this indicates a bug in the producer",
            s
        );
        s = 0.0;
        dirty = true;
    }
    if !zp.is_finite() {
        crate::plugin_warn!(
            "agent",
            "[Q8] zero_point is not finite ({:?}), replaced with 0.0 — this indicates a bug in the producer",
            zp
        );
        zp = 0.0;
        dirty = true;
    }
    let _ = dirty; // 显式忽略 lint
    (s, zp)
}

/// 把 JSON Value 反序列化为 Q8 结构（**仅 Q8 格式**，不再兜底 legacy）
///
/// legacy `Value::Array<f32>` 格式**不再**通过本函数处理，
/// 改由 `dequantize_from_value` 直接读为 f32 向量（避免静默数据丢失）。
///
/// 支持的 Q8 格式：
/// - **v2**（当前）：`{ q8, scale, zero_point, _format_version: 2 }`
/// - **v1**（v16 兼容）：`{ q8, scale, zero_point }`（无版本字段；自动按 v1 解码）
///
/// 未知 / 不匹配返回 `None`，由 `dequantize_from_value` 进一步尝试 legacy 格式。
pub fn value_to_q8(v: &Value) -> Option<Q8Embedding> {
    let obj = v.as_object()?;

    // v2 格式：检查 _format_version
    if let Some(ver) = obj.get("_format_version").and_then(|x| x.as_u64()) {
        if ver != Q8_FORMAT_VERSION_V2 as u64 {
            crate::plugin_warn!(
                "agent",
                "[Q8] Unknown _format_version={}, falling back to None — data needs migration",
                ver
            );
            return None;
        }
    }
    // v1 格式：无 _format_version 字段，按 v1 解码（v16 那段时间的数据）

    let data_arr = obj.get("q8")?.as_array()?;
    let scale = obj.get("scale")?.as_f64()? as f32;
    let zero_point = obj.get("zero_point")?.as_f64()? as f32;

    // v19：拒绝 NaN/Inf（避免反量化产生 NaN 污染）
    if !scale.is_finite() || !zero_point.is_finite() {
        crate::plugin_warn!(
            "agent",
            "[Q8] Stored scale/zero_point contains non-finite value (scale={}, zp={}); returning None",
            scale, zero_point
        );
        return None;
    }

    let data: Vec<u8> = data_arr
        .iter()
        .filter_map(|x| {
            x.as_u64()
                .and_then(|n| if n <= 255 { Some(n as u8) } else { None })
        })
        .collect();

    // v19：如果过滤后长度与原始数组长度不一致，说明有元素 > 255，标记为可疑
    if data.len() != data_arr.len() {
        crate::plugin_warn!(
            "agent",
            "[Q8] Some q8 array elements > 255 (got {} valid out of {}), potential corruption",
            data.len(),
            data_arr.len()
        );
    }

    Some(Q8Embedding {
        data,
        scale,
        zero_point,
    })
}

/// 直接从 JSON Value 取得 f32 embedding
///
/// 自动识别两种格式：
/// - **Q8 格式**（v1/v2）：`{ q8, scale, zero_point, ... }` → Q8 反量化
/// - **Legacy 格式**：直接 `[f32, f32, ...]` → 原样返回为 f32 向量
///
/// 返回 `None` 当 `_ext_embedding` 字段缺失或格式无法识别。
///
/// ## v19 修复要点
/// legacy 格式**不再**走 Q8 转换（避免 `.round().clamp(0, 255)` 把负值置 0、
/// 小数置 0 的静默数据丢失）。负数 / 小数 f32 值会被**完整保留**。
pub fn dequantize_from_value(v: &Value) -> Option<Vec<f32>> {
    match v {
        // Q8 格式：Value::Object 且含 `q8` 字段
        Value::Object(obj) if obj.contains_key("q8") => {
            let q = value_to_q8(v)?;
            Some(dequantize_q8(&q))
        }
        // Legacy 格式：直接 f32 数组（**不做 Q8 转换**）
        Value::Array(arr) => {
            let vec: Vec<f32> = arr
                .iter()
                .filter_map(|x| x.as_f64().map(|f| f as f32))
                .collect();
            if vec.is_empty() && !arr.is_empty() {
                // 全部元素都不是 number（如全是 string），返 None
                None
            } else {
                Some(vec)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "embedding_quant_tests.rs"]
mod tests;
