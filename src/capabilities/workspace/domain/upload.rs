//! 目的：上传载荷的纯解码——把动作参数里的 base64 还原成字节。
//! 管：`base64_decode` 一条：标准字母表，容忍换行与缺失填充。
//! 不管：单文件上限与落盘（上限常量在 `asset.rs`，写文件在 `service/`）。
//! 联动：由 `workspace/api.rs` 重导出；conductor 的上传动作与测试都经能力面调用。

/// 目的：极简 base64 解码（上传动作的参数是 base64）。
/// 参数：`s` = 待解码的文本；空白（换行 / 回车 / 空格 / 制表）与 `=` 跳过。
/// 返回：解出的字节；缺填充按已有位补足，不额外报错。
/// 错误：出现标准字母表之外的字符时给固定文案。
pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return Err("base64 含非法字符".to_string()),
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}
