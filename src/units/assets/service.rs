//! 目的：随包只读内容的用例——把 `Doc` 与 `prompts/` 目录翻译成端口调用，补上文案与排序。
//! 管：`Assets` 的实现；提示词册的相对名与读失败文案。
//! 不管：读盘（在 `detail` 的端口实现里）；文档内容的解释（消费方）。
//! 联动：组合根构造；`prompt` / `tools` / `conversation` 的适配器持有它。契约见 docs/assets/README.md。

use crate::units::assets::api::{Assets, Doc, PromptDoc};
use crate::units::assets::domain::paths::AssetPaths;
use crate::units::assets::ports::AssetFiles;
use std::path::Path;

/// 目的：随包只读内容的资源单元实现。
pub struct AssetsService {
    paths: AssetPaths,
    fs: Box<dyn AssetFiles + Send + Sync>,
}

impl AssetsService {
    /// 目的：按产品安装根与文件系统端口装配。
    /// 参数：`install` = 产品安装根；`fs` = 文件系统端口。
    /// 返回：可用的 `Assets`。
    pub fn new(install: &Path, fs: Box<dyn AssetFiles + Send + Sync>) -> AssetsService {
        AssetsService {
            paths: AssetPaths::at(install),
            fs,
        }
    }
}

impl Assets for AssetsService {
    fn read(&self, doc: Doc) -> Result<String, String> {
        self.fs.read(&self.paths.systools().join(doc.file_name()))
    }

    fn prompt_docs(&self) -> Result<Vec<PromptDoc>, String> {
        let dir = self.paths.prompts();
        let mut files = self
            .fs
            .yaml_tree(&dir)
            .map_err(|e| format!("提示词册目录读不了（prompts/）：{}", e))?;
        if files.is_empty() {
            return Err("提示词册目录里一个 .yaml 都没有（prompts/）".to_string());
        }
        files.sort();
        let mut docs = Vec::with_capacity(files.len());
        for file in &files {
            let name = doc_name(&dir, file);
            let text = self
                .fs
                .read(file)
                .map_err(|e| format!("提示词册文件读不了（prompts/{}）：{}", name, e))?;
            docs.push(PromptDoc { text });
        }
        Ok(docs)
    }
}

/// 目的：把文件路径折成相对 `prompts/` 的 `/` 路径，给合并与错误文案用。
fn doc_name(dir: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(dir).unwrap_or(file);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 目的：假的文件系统端口——按固定表回话，不碰真盘。
    struct FakeFs {
        files: Vec<(&'static str, &'static str)>,
    }

    impl AssetFiles for FakeFs {
        fn read(&self, path: &Path) -> Result<String, String> {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            self.files
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, t)| t.to_string())
                .ok_or_else(|| "没有这份文件".to_string())
        }

        fn yaml_tree(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
            Ok(self.files.iter().map(|(n, _)| dir.join(n)).collect())
        }
    }

    #[test]
    fn prompt_docs_are_sorted_and_named_relative_to_prompts() {
        let fs = FakeFs {
            files: vec![("b.yaml", "b: 1"), ("a.yaml", "a: 1")],
        };
        let assets = AssetsService::new(Path::new("product"), Box::new(fs));
        let docs = assets.prompt_docs().expect("提示词册应当读得出来");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].text, "a: 1", "排序按路径名，a 在前");
        assert_eq!(docs[1].text, "b: 1");
    }

    #[test]
    fn an_empty_prompts_dir_is_an_error() {
        let fs = FakeFs { files: vec![] };
        let assets = AssetsService::new(Path::new("product"), Box::new(fs));
        assert!(assets.prompt_docs().is_err());
    }

    #[test]
    fn missing_document_is_an_error() {
        let fs = FakeFs { files: vec![] };
        let assets = AssetsService::new(Path::new("product"), Box::new(fs));
        assert!(assets.read(Doc::SystoolsTools).is_err());
    }
}
