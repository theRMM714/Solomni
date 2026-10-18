//! 目的：共享区**通用资产**的纯逻辑：一次遍历算指纹、大小、文本/二进制判定与受限预览，
//!   以及按扩展名给出的媒体类型提示与单文件上限。
//! 管：`AssetScanner`、`FileVersion`（三方视图的单项）、`media_hint`、`MAX_ASSET_BYTES`。
//! 不管：真正的文件读写（在 `detail` 的 `WorkStore` 实现里）；三方比较与冲突（在 `workstore`）。
//! 联动：`workstore` 的 `VersionView` 引用 `FileVersion`；`service` 用扫描结果拼三方视图与回执。

use crate::capabilities::workspace::domain::hash::Sha256;

/// 目的：单文件进入共享区的上限（超出整次拒绝，不做存储级分块）。
/// 约束：策略常量；上传、提交与回执共用这一个数。
pub const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;

/// 目的：单次文本预览上限（超出如实标注截断，不把大文件整块灌进上下文）。
pub const VERSION_TEXT_LIMIT: usize = 16 * 1024;

/// 目的：内容 IO 的分块大小（流式读写不按整份分配）。
pub const ASSET_CHUNK_BYTES: usize = 64 * 1024;

/// 目的：按扩展名给出的媒体类型提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaHint {
    /// 目的：MIME 风格的提示值（不保证与内容一致）。
    pub mime: &'static str,
    /// 目的：这个扩展名是否按二进制资产处理（不读内容、不给文本预览）。
    pub binary: bool,
}

/// 目的：按扩展名给媒体类型提示；扩展名大小写不敏感，未知 = None。
/// 约束：只是提示，内容仍以 hash / size 为准；不用于安全判定。
pub fn media_hint(path: &str) -> Option<MediaHint> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let hint = match ext.as_str() {
        "png" => ("image/png", true),
        "jpg" | "jpeg" => ("image/jpeg", true),
        "gif" => ("image/gif", true),
        "webp" => ("image/webp", true),
        "bmp" => ("image/bmp", true),
        "tif" | "tiff" => ("image/tiff", true),
        "svg" => ("image/svg+xml", true),
        "mp4" | "m4v" => ("video/mp4", true),
        "webm" => ("video/webm", true),
        "mov" => ("video/quicktime", true),
        "mkv" => ("video/x-matroska", true),
        "avi" => ("video/x-msvideo", true),
        "mp3" => ("audio/mpeg", true),
        "wav" => ("audio/wav", true),
        "ogg" => ("audio/ogg", true),
        "flac" => ("audio/flac", true),
        "m4a" | "aac" => ("audio/aac", true),
        "pdf" => ("application/pdf", true),
        "zip" => ("application/zip", true),
        "gz" | "tgz" => ("application/gzip", true),
        "tar" => ("application/x-tar", true),
        "7z" => ("application/x-7z-compressed", true),
        "rar" => ("application/vnd.rar", true),
        "bz2" => ("application/x-bzip2", true),
        "xz" => ("application/x-xz", true),
        "txt" => ("text/plain", false),
        "md" | "markdown" => ("text/markdown", false),
        "json" => ("application/json", false),
        "jsonl" => ("application/x-ndjson", false),
        "csv" => ("text/csv", false),
        "tsv" => ("text/tab-separated-values", false),
        "html" | "htm" => ("text/html", false),
        "xml" => ("application/xml", false),
        "yaml" | "yml" => ("application/yaml", false),
        "toml" => ("application/toml", false),
        "log" => ("text/plain", false),
        _ => return None,
    };
    Some(MediaHint {
        mime: hint.0,
        binary: hint.1,
    })
}

/// 目的：一次扫描的结果：指纹、大小、可选文本预览与截断标注。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    /// 目的：全文内容指纹。
    pub hash: String,
    /// 目的：全文字节数。
    pub size: u64,
    /// 目的：文本预览；二进制 / 非 UTF-8 / 未要求预览时为 None。
    pub text: Option<String>,
    /// 目的：文本预览是否因超过上限被截断。
    pub truncated: bool,
}

/// 目的：分块喂入的资产扫描器：hash + size + NUL/UTF-8 判定 + 受限预览。
/// 约束：内存只保留 64 字节 SHA 块、不超过预览上限的预览与最多 3 字节的 UTF-8 尾。
pub struct AssetScanner {
    hasher: Sha256,
    size: u64,
    preview_limit: Option<usize>,
    preview: Vec<u8>,
    has_nul: bool,
    utf8: Utf8Checker,
}

impl AssetScanner {
    /// 目的：新建扫描器；`preview_limit = None` 只算 hash 与大小，不做文本判定。
    pub fn new(preview_limit: Option<usize>) -> AssetScanner {
        AssetScanner {
            hasher: Sha256::new(),
            size: 0,
            preview_limit,
            preview: Vec::new(),
            has_nul: false,
            utf8: Utf8Checker::new(),
        }
    }

    /// 目的：喂入一块内容；块大小任意。
    pub fn feed(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
        self.size = self.size.wrapping_add(chunk.len() as u64);
        let Some(limit) = self.preview_limit else {
            return;
        };
        if chunk.contains(&0) {
            self.has_nul = true;
        }
        self.utf8.feed(chunk);
        if self.preview.len() < limit {
            let take = (limit - self.preview.len()).min(chunk.len());
            self.preview.extend_from_slice(&chunk[..take]);
        }
    }

    /// 目的：收尾并给出扫描结果。
    pub fn finish(self) -> Scanned {
        let hash = self.hasher.finish_hex();
        let size = self.size;
        let text = if self.preview_limit.is_some() && !self.has_nul && self.utf8.finish() {
            let mut end = self.preview.len();
            while end > 0 && std::str::from_utf8(&self.preview[..end]).is_err() {
                end -= 1;
            }
            String::from_utf8(self.preview[..end].to_vec()).ok()
        } else {
            None
        };
        let truncated = text
            .as_ref()
            .map(|t| size > t.len() as u64)
            .unwrap_or(false);
        Scanned {
            hash,
            size,
            text,
            truncated,
        }
    }
}

/// 目的：一条路径某一方的版本视图（work_versions 用）。
/// 约束：文本只给有上限的预览；二进制/非 UTF-8 只给指纹与大小，不冒充文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    /// 目的：内容指纹。
    pub hash: String,
    /// 目的：内容字节数。
    pub size: u64,
    /// 目的：文本预览；二进制 / 非 UTF-8 为 None。
    pub text: Option<String>,
    /// 目的：文本预览是否因超过上限被截断。
    pub truncated: bool,
    /// 目的：是否超过单文件上限（只给指纹、大小与类型，不预览）。
    pub too_large: bool,
}

/// 目的：增量 UTF-8 校验：逐字节保留最多 4 字节的未完成序列。
/// 约束：按首字节定长，凑满后用 `from_utf8` 验完整序列（过长短、代理区、越界都在那里挡）。
struct Utf8Checker {
    pending: Vec<u8>,
    need: usize,
    valid: bool,
}

impl Utf8Checker {
    fn new() -> Utf8Checker {
        Utf8Checker {
            pending: Vec::new(),
            need: 0,
            valid: true,
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if !self.valid {
            return;
        }
        for &b in chunk {
            if self.pending.is_empty() {
                let need = match b {
                    0x00..=0x7f => 1,
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => {
                        self.valid = false;
                        return;
                    }
                };
                if need == 1 {
                    continue;
                }
                self.pending.push(b);
                self.need = need;
                continue;
            }
            self.pending.push(b);
            if self.pending.len() == self.need {
                if std::str::from_utf8(&self.pending).is_ok() {
                    self.pending.clear();
                    self.need = 0;
                } else {
                    self.valid = false;
                    return;
                }
            }
        }
    }

    fn finish(&self) -> bool {
        self.valid && self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::workspace::domain::hash::content_hash;

    fn version(hash: &str, bytes: &[u8]) -> FileVersion {
        let mut scanner = AssetScanner::new(Some(VERSION_TEXT_LIMIT));
        scanner.feed(bytes);
        let scanned = scanner.finish();
        FileVersion {
            hash: hash.to_string(),
            size: scanned.size,
            text: scanned.text,
            truncated: scanned.truncated,
            too_large: false,
        }
    }

    #[test]
    fn media_hints_are_advisory_and_case_insensitive() {
        assert_eq!(
            media_hint("a/b/photo.PNG").map(|h| h.mime),
            Some("image/png")
        );
        assert!(media_hint("clip.mp4").map(|h| h.binary).unwrap_or(false));
        assert_eq!(media_hint("note.md").map(|h| h.mime), Some("text/markdown"));
        assert!(media_hint("blob").is_none());
        assert!(media_hint("a.bin").is_none());
    }

    #[test]
    fn scanner_hashes_chunked_input_like_one_shot() {
        let data: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
        let want = content_hash(&data);
        for chunk in [1usize, 3, 7, 63, 64, 65, 1024] {
            let mut s = AssetScanner::new(None);
            for part in data.chunks(chunk) {
                s.feed(part);
            }
            let got = s.finish();
            assert_eq!(got.hash, want, "chunk {}", chunk);
            assert_eq!(got.size, data.len() as u64);
            assert!(got.text.is_none());
        }
    }

    #[test]
    fn scanner_keeps_utf8_across_chunk_boundaries() {
        let text = "你好，世界🙂";
        let bytes = text.as_bytes();
        for chunk in [1usize, 2, 3, 4, 5] {
            let mut s = AssetScanner::new(Some(VERSION_TEXT_LIMIT));
            for part in bytes.chunks(chunk) {
                s.feed(part);
            }
            let got = s.finish();
            assert_eq!(got.text.as_deref(), Some(text), "chunk {}", chunk);
            assert!(!got.truncated);
        }
    }

    #[test]
    fn scanner_marks_binary_on_nul_or_invalid_utf8() {
        let mut s = AssetScanner::new(Some(VERSION_TEXT_LIMIT));
        s.feed(b"abc\0def");
        assert!(s.finish().text.is_none(), "NUL 是二进制");

        let mut s = AssetScanner::new(Some(VERSION_TEXT_LIMIT));
        s.feed(&[0x41, 0xff, 0xfe, 0x42]);
        assert!(s.finish().text.is_none(), "非法 UTF-8 是二进制");
    }

    #[test]
    fn scanner_truncates_text_preview_without_splitting_a_char() {
        let limit = 5;
        let mut s = AssetScanner::new(Some(limit));
        s.feed("你好世界".as_bytes());
        let got = s.finish();
        assert_eq!(got.text.as_deref(), Some("你"), "不能在多字节字符中间切");
        assert!(got.truncated);
        assert_eq!(got.size, "你好世界".len() as u64);
    }

    #[test]
    fn empty_file_is_text_with_empty_preview() {
        let got = version("h", b"");
        assert_eq!(got.hash, "h");
        assert_eq!(got.size, 0);
        assert_eq!(got.text.as_deref(), Some(""));
        assert!(!got.truncated);
        assert!(!got.too_large);
    }

    #[test]
    fn scanned_version_previews_text_and_never_pretends_binary_is_text() {
        let text = version("h1", "你好".as_bytes());
        assert_eq!(text.text.as_deref(), Some("你好"));
        assert!(!text.truncated);

        let binary = version("h2", &[0, 159, 146, 150]);
        assert!(binary.text.is_none(), "含 NUL 的二进制不能给文本");

        let long = "a".repeat(VERSION_TEXT_LIMIT + 10);
        let cut = version("h3", long.as_bytes());
        assert!(cut.truncated);
        assert_eq!(cut.text.as_ref().map(String::len), Some(VERSION_TEXT_LIMIT));
    }
}
