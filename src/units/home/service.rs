//! 目的：登记处资源的用例层——把 `Doc` 映射成文件名，其余交给文件端口。
//! 管：文档 → 文件名的映射、私密标记的传递；失败原样上抛（不吞、不兜底）。
//! 不管：文件机制（在 detail）；目录从哪来（组合根给）；内容语义（各业务）。
//! 联动：实现 `api::Home`；由 `src/entry/assembly.rs` 装配一次、三个 yaml 适配器共用。

use crate::units::home::api::{Doc, Home};
use crate::units::home::ports::HomeFiles;

/// 目的：登记处资源——持文件端口，按文档名读写。
pub struct HomeService {
    files: Box<dyn HomeFiles>,
}

impl HomeService {
    /// 目的：组合根专用——注入文件端口（真实实现是 `detail::FsHome`）。
    pub fn new(files: Box<dyn HomeFiles>) -> HomeService {
        HomeService { files }
    }
}

impl Home for HomeService {
    fn read(&self, doc: Doc) -> Result<Option<String>, String> {
        self.files.read(doc.file_name())
    }

    fn write(&self, doc: Doc, text: &str) -> Result<(), String> {
        self.files.write(doc.file_name(), text, doc.is_private())
    }
}
