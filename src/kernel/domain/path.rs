//! 目的：路径的对外书写形式——一律用 `/`。
//! 管：把路径拼成跨平台可读的字符串，供提示词、回执与 API 使用。
//! 不管：真实文件系统访问；把用户输入解析回路径。
//! 联动：各处拼对外路径时调用它；与 R9（跨平台与路径）同源。

/// 目的：把路径写成对外形式——一律用 `/` 分隔（给模型看、进提示词与 JSON）。
/// 约束：Windows 的反斜杠在 JSON 字符串里是转义符（"D:\a" 会解析失败），所以对外只给 `/`——两边系统都认。
pub fn slash(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 目的：把宿主真实路径还原成用户能认的写法——Windows 去掉 `\\?\` 扩展前缀。
/// 约束：只影响显示 / 绑定存储；真正访问文件系统仍可用扩展前缀。
pub fn native(p: &std::path::Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        if let Some(rest) = p.to_string_lossy().strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(rest);
        }
    }
    p.to_path_buf()
}
