//! 稳定哈希：视图指纹与命运骰子共用的确定性原语。
//!
//! 两处都必须满足同一条性质：**同样的输入永远得到同样的输出**，跨进程、跨机器、
//! 跨世界线都一样。所以这里不做任何随机化（不加盐、不用 `RandomState`），
//! 算法本身固定为 SHA-256。
//!
//! docs/03 §8 允许「固定的哈希算法（如 BLAKE3）」；这里选 SHA-256，因为它在
//! `Cargo.lock` 里已经存在（作为传递依赖），不需要新增下载。

use serde::Serialize;
use sha2::{Digest, Sha256};

/// 视图指纹前缀。改动它等于改动判定记录（`Judgment.view.hash`）的语义。
pub const VIEW_HASH_PREFIX: &str = "sha256:";

/// 把若干段字节拼起来求摘要，返回 `sha256:` 前缀的十六进制串。
pub fn digest_hex(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        // 长度前缀，避免 `["ab","c"]` 与 `["a","bc"]` 撞到一起。
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    format!("{VIEW_HASH_PREFIX}{}", hex(&digest))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 取摘要的前 53 位映射到 `[0, 1)`。
///
/// 53 位是 `f64` 尾数的精度：再多的位也塞不进一个 `f64`，反而让「同一颗骰子」
/// 的比较在同一世界的不同机器上失去意义。
pub fn digest_unit(parts: &[&[u8]]) -> f64 {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let head = u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]);
    (head >> 11) as f64 / (1u64 << 53) as f64
}

/// 视图状态指纹（docs/07 §5）。同一投影 + 同一视图参数必须得到同一个 hash。
///
/// 序列化用 `serde_json` 的默认 `Map`（内部是 `BTreeMap`），键序天然固定，
/// 不依赖字段声明顺序。
pub fn view_hash<T: Serialize>(state: &T) -> String {
    match serde_json::to_vec(state) {
        Ok(bytes) => digest_hex(&[b"if.view.v1", &bytes]),
        // 序列化失败说明状态类型自己出了问题，不该悄悄给一个「看起来正常」的指纹。
        Err(error) => digest_hex(&[b"if.view.v1.error", error.to_string().as_bytes()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_input_yields_same_digest() {
        assert_eq!(digest_hex(&[b"a", b"b"]), digest_hex(&[b"a", b"b"]));
    }

    #[test]
    fn part_boundaries_are_not_ambiguous() {
        assert_ne!(digest_hex(&[b"ab", b"c"]), digest_hex(&[b"a", b"bc"]));
    }

    #[test]
    fn unit_lands_in_half_open_range() {
        for i in 0..200 {
            let key = format!("cand_{i}");
            let u = digest_unit(&[key.as_bytes()]);
            assert!((0.0..1.0).contains(&u), "{u}");
        }
    }

    /// 粗糙的均匀性检查：1000 个键应当铺满各个桶。
    #[test]
    fn unit_is_spread_across_buckets() {
        let mut buckets = [0usize; 10];
        for i in 0..1000 {
            let u = digest_unit(&[format!("k{i}").as_bytes()]);
            buckets[(u * 10.0) as usize] += 1;
        }
        assert!(buckets.iter().all(|n| *n > 50), "{buckets:?}");
    }

    #[test]
    fn view_hash_is_prefixed_and_stable() {
        let state = serde_json::json!({"b": 1, "a": 2});
        let h = view_hash(&state);
        assert!(h.starts_with(VIEW_HASH_PREFIX));
        assert_eq!(h, view_hash(&state));
        // 键序不该影响指纹：json! 宏给的是排序 Map。
        assert_eq!(h, view_hash(&serde_json::json!({"a": 2, "b": 1})));
    }
}
