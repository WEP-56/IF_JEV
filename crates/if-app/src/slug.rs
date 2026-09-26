//! 名字 → 文件名 / ID 片段。
//!
//! 世界名与角色名基本都是中文，所以这里**保留所有字母与数字（含 CJK）**，只把空格、标点、
//! emoji 折成 `-`。早先只保留 ASCII 的写法会把「裴聿」「雨城」全部折成 `world` 或空串——
//! 能跑（时间戳还顶着唯一性），但日志和文件名里等于没有名字。
//!
//! 两个消费方：
//! - `world_worker::new_world_path` 用它拼 `.ifworld` 的文件名；
//! - `seed` 用它拼主体 ID（`c_裴聿`）。

/// 保留字母与数字（含 CJK），其余字符折成单个 `-`；首尾不补 `-`。
///
/// 大小写统一折成小写，这样 `Rain` 与 `rain` 不会变成两个不同的片段。
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() {
            if pending && !out.is_empty() {
                out.push('-');
            }
            pending = false;
            out.extend(ch.to_lowercase());
        } else {
            pending = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_cjk_and_folds_everything_else() {
        assert_eq!(slug("裴聿"), "裴聿");
        assert_eq!(slug("雨城 · 第一卷"), "雨城-第一卷");
        assert_eq!(slug("  Rain City  "), "rain-city");
        assert_eq!(slug("a//b__c"), "a-b-c");
        // 首尾的标点不产生多余的 `-`
        assert_eq!(slug("··裴聿··"), "裴聿");
    }

    #[test]
    fn pure_punctuation_yields_nothing() {
        // 调用方负责兜底（文件名退到 `world`，ID 退到序号）
        assert_eq!(slug("···"), "");
        assert_eq!(slug("   "), "");
        assert_eq!(slug(""), "");
    }
}
