//! 从 PNG 文本块中取出内嵌的角色卡 JSON。
//!
//! 依据 CCv3 规范（docs/13 §3.1【已核实】）：
//! - V2 卡放在名为 `chara` 的 `tEXt` 块里，V3 卡放在 `ccv3` 里；
//! - 块内是 JSON 字符串经 UTF-8 → base64 编码；
//! - 两者同时存在时**优先 `ccv3`**。
//!
//! 只读文本块，不解码图像，也不执行卡中的任何脚本。

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use std::io::Read;

/// PNG 文件签名。
pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// V3 卡的文本块名（优先级更高）。
pub const CHUNK_CCV3: &str = "ccv3";
/// V2 卡的文本块名。
pub const CHUNK_CHARA: &str = "chara";

/// 一个文本块的解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChunk {
    pub keyword: String,
    pub text: String,
}

pub fn looks_like_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&PNG_SIGNATURE)
}

/// 依次读出 PNG 中的所有 `tEXt` / `zTXt` / `iTXt` 文本块。
///
/// 不校验 CRC：卡片文本块的 CRC 错误不构成安全风险，而校验会让损坏的
/// 卡直接无法导入。chunk 长度越界按「文件损坏」直接报错。
pub fn text_chunks(bytes: &[u8]) -> Result<Vec<TextChunk>, String> {
    if !looks_like_png(bytes) {
        return Err("不是 PNG 文件：签名不匹配".into());
    }
    let mut chunks = Vec::new();
    let mut pos = PNG_SIGNATURE.len();
    while pos + 8 <= bytes.len() {
        let length = u32::from_be_bytes(
            bytes[pos..pos + 4]
                .try_into()
                .map_err(|_| "PNG chunk 长度字段读取失败".to_owned())?,
        ) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| "PNG chunk 长度溢出".to_owned())?;
        // 数据之后还有 4 字节 CRC。
        if data_end + 4 > bytes.len() {
            return Err("PNG chunk 越界：文件可能被截断或损坏".into());
        }
        let data = &bytes[data_start..data_end];
        match kind {
            b"tEXt" => chunks.push(parse_text_chunk(data, "tEXt")?),
            b"zTXt" => chunks.push(parse_ztxt_chunk(data)?),
            b"iTXt" => chunks.push(parse_itxt_chunk(data)?),
            b"IEND" => break,
            _ => {}
        }
        pos = data_end + 4;
    }
    Ok(chunks)
}

/// 从 PNG 中找到角色卡 JSON，返回 `(json, 块名)`。
///
/// 同时存在 `ccv3` 与 `chara` 时按规范取 `ccv3`。
pub fn card_json_from_png(bytes: &[u8]) -> Result<(String, String), String> {
    let chunks = text_chunks(bytes)?;
    let pick = [CHUNK_CCV3, CHUNK_CHARA]
        .iter()
        .find_map(|name| chunks.iter().find(|chunk| chunk.keyword.eq_ignore_ascii_case(name)));
    let chunk = pick.ok_or_else(|| {
        let found: Vec<&str> = chunks.iter().map(|c| c.keyword.as_str()).collect();
        if found.is_empty() {
            "PNG 中没有找到角色卡数据块（chara / ccv3）".into()
        } else {
            format!("PNG 中没有 chara / ccv3 数据块，只找到：{}", found.join("、"))
        }
    })?;
    let json = decode_base64_json(&chunk.text, &chunk.keyword)?;
    Ok((json, chunk.keyword.clone()))
}

/// base64 → 原始字节。容忍换行等空白（部分导出会在块内折行）。
pub fn decode_base64(encoded: &str) -> Result<Vec<u8>, String> {
    let compact: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err("导入数据为空".into());
    }
    STANDARD
        .decode(compact.as_bytes())
        .map_err(|e| format!("导入数据不是合法的 base64：{e}"))
}

/// base64 → UTF-8 文本，用于卡片文本块。
pub fn decode_base64_json(encoded: &str, what: &str) -> Result<String, String> {
    let decoded = decode_base64(encoded).map_err(|e| format!("{what} 数据块{e}"))?;
    String::from_utf8(decoded).map_err(|e| format!("{what} 数据块不是 UTF-8 文本：{e}"))
}

/// `tEXt`：`keyword\0text`，文本为 Latin-1。base64 只需 ASCII，按 UTF-8 无损读取。
fn parse_text_chunk(data: &[u8], kind: &str) -> Result<TextChunk, String> {
    let (keyword, text) = split_keyword(data, kind)?;
    Ok(TextChunk {
        keyword,
        text: String::from_utf8_lossy(text).into_owned(),
    })
}

/// `zTXt`：`keyword\0compression_method(1)compressed_text`（zlib）。
fn parse_ztxt_chunk(data: &[u8]) -> Result<TextChunk, String> {
    let (keyword, rest) = split_keyword(data, "zTXt")?;
    let (method, compressed) = rest.split_first().ok_or("zTXt 块缺少压缩方法字节")?;
    if *method != 0 {
        return Err(format!("zTXt 使用了不支持的压缩方法 {method}"));
    }
    let text = inflate(compressed, "zTXt")?;
    Ok(TextChunk { keyword, text })
}

/// `iTXt`：`keyword\0 compression_flag(1) compression_method(1) language\0 translated\0 text`。
fn parse_itxt_chunk(data: &[u8]) -> Result<TextChunk, String> {
    let (keyword, rest) = split_keyword(data, "iTXt")?;
    let (&compressed, rest) = rest
        .split_first()
        .ok_or("iTXt 块缺少压缩标志")?;
    let (_, rest) = rest.split_first().ok_or("iTXt 块缺少压缩方法")?;
    let (_, rest) = split_keyword(rest, "iTXt 语言标签")?;
    let (_, text) = split_keyword(rest, "iTXt 翻译关键词")?;
    let text = if compressed == 1 {
        inflate(text, "iTXt")?
    } else {
        String::from_utf8_lossy(text).into_owned()
    };
    Ok(TextChunk { keyword, text })
}

/// 按第一个 NUL 切开 `keyword` 与其余数据。
fn split_keyword<'a>(data: &'a [u8], kind: &str) -> Result<(String, &'a [u8]), String> {
    let end = data
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| format!("{kind} 块缺少关键词分隔符"))?;
    if end == 0 {
        return Err(format!("{kind} 块关键词为空"));
    }
    Ok((
        String::from_utf8_lossy(&data[..end]).into_owned(),
        &data[end + 1..],
    ))
}

fn inflate(data: &[u8], kind: &str) -> Result<String, String> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("{kind} 块解压失败：{e}"))?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    /// 按 PNG 规范拼一个只含文本块的最小文件，用于测试解析器本身。
    pub(crate) fn png_with_text_chunks(chunks: &[(&str, &str, &str)]) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        for (kind, keyword, text) in chunks {
            let mut data = keyword.as_bytes().to_vec();
            data.push(0);
            data.extend_from_slice(text.as_bytes());
            push_chunk(&mut out, kind.as_bytes().try_into().unwrap(), &data);
        }
        push_chunk(&mut out, *b"IEND", &[]);
        out
    }

    fn push_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC 不校验，占位即可
    }

    fn b64_payload(json: &str) -> String {
        STANDARD.encode(json.as_bytes())
    }

    #[test]
    fn reads_chara_chunk_and_decodes_base64_json() {
        let json = r#"{"spec":"chara_card_v2","data":{"name":"裴聿"}}"#;
        let png = png_with_text_chunks(&[("tEXt", CHUNK_CHARA, &b64_payload(json))]);
        let (decoded, chunk) = card_json_from_png(&png).unwrap();
        assert_eq!(chunk, CHUNK_CHARA);
        assert!(decoded.contains("裴聿"));
    }

    #[test]
    fn prefers_ccv3_over_chara() {
        let v2 = r#"{"spec":"chara_card_v2","data":{"name":"旧"}}"#;
        let v3 = r#"{"spec":"chara_card_v3","data":{"name":"新"}}"#;
        let png = png_with_text_chunks(&[
            ("tEXt", CHUNK_CHARA, &b64_payload(v2)),
            ("tEXt", CHUNK_CCV3, &b64_payload(v3)),
        ]);
        let (decoded, chunk) = card_json_from_png(&png).unwrap();
        assert_eq!(chunk, CHUNK_CCV3);
        assert!(decoded.contains("新"));
    }

    #[test]
    fn tolerates_line_breaks_inside_base64() {
        let json = r#"{"spec":"chara_card_v2","data":{"name":"折行"}}"#;
        let wrapped = b64_payload(json)
            .as_bytes()
            .chunks(16)
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        let png = png_with_text_chunks(&[("tEXt", CHUNK_CHARA, &wrapped)]);
        let (decoded, _) = card_json_from_png(&png).unwrap();
        assert!(decoded.contains("折行"));
    }

    #[test]
    fn reports_missing_card_chunk() {
        let png = png_with_text_chunks(&[("tEXt", "Comment", "hello")]);
        let error = card_json_from_png(&png).unwrap_err();
        assert!(error.contains("chara / ccv3"), "{error}");
    }

    #[test]
    fn rejects_non_png_and_broken_chunks() {
        assert!(card_json_from_png(b"not a png").is_err());
        let mut broken = PNG_SIGNATURE.to_vec();
        broken.extend_from_slice(&9999u32.to_be_bytes());
        broken.extend_from_slice(b"tEXt");
        broken.extend_from_slice(&[0, 0, 0, 0]);
        assert!(card_json_from_png(&broken).unwrap_err().contains("越界"));
    }

    #[test]
    fn rejects_invalid_base64() {
        let png = png_with_text_chunks(&[("tEXt", CHUNK_CHARA, "!!!not base64!!!")]);
        assert!(card_json_from_png(&png).unwrap_err().contains("base64"));
    }
}
