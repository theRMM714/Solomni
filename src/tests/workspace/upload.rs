//! 目的：上传载荷解码——换行、缺填充与非法字符。
//! 管：`base64_decode` 的容忍与拒绝边界。
//! 不管：单文件上限与落盘（上限在 `workspace/domain/asset.rs`，写文件在 `service/`）。
//! 联动：`src/capabilities/workspace/domain/upload.rs`。

use crate::capabilities::workspace::api::base64_decode;

/// 目的：容忍换行、空白与缺填充——上传的 base64 常按行折，末段可能没补齐。
#[test]
pub(crate) fn base64_decode_tolerates_line_breaks_and_missing_padding() {
    assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
    assert_eq!(base64_decode("aGVs\nbG8=").unwrap(), b"hello", "换行被跳过");
    assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello", "缺填充照样解");
    assert_eq!(
        base64_decode(" aGVs\tbG8= ").unwrap(),
        b"hello",
        "空格与制表也跳过"
    );
    assert!(
        base64_decode("").unwrap().is_empty(),
        "空串解出零字节，不报错"
    );
}

/// 目的：标准字母表之外的字符一律拒绝，并给固定文案。
#[test]
pub(crate) fn base64_decode_rejects_illegal_characters() {
    let err = base64_decode("aGVs*bG8=").unwrap_err();
    assert_eq!(err, "base64 含非法字符");
    assert!(
        base64_decode("aGVsbG8_").is_err(),
        "URL-safe 的 _ 不在标准字母表里"
    );
}
