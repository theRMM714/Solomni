//! 路径的**对外书写形式**：一律用 `/`。
//! 为什么在 kernel：这是跨平台机制（Windows 的反斜杠在 JSON 字符串里是转义符，`D:\a` 会解析失败），
//! 与任何业务无关；core 各处在拼提示词、回执与 API 时都要它。

/// 路径的**书写形式**（给模型看、进提示词与 JSON 的）：一律用 / 分隔。
/// Windows 的反斜杠在 JSON 字符串里是转义符（"D:\a" 会解析失败），所以对外只给 /——两边系统都认。
pub fn slash(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
