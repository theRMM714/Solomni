//! 目的：随包只读内容的入站面——按文档读文本、枚举提示词册；调用方拿到的是文本，不是宿主路径。
//! 管：`Assets` 两个读法、`Doc` 文档清单、`PromptDoc` 一份提示词文档。
//! 不管：文件形态与解码（消费方的 `detail`）；落点怎么拼（`domain::paths`）。
//! 联动：`prompt` / `tools` / `conversation` 的装配输入适配器持有 `Arc<dyn Assets>`；契约见 docs/assets/README.md。

use std::path::Path;

pub use crate::units::resource::assets::domain::doc::Doc;
pub use crate::units::resource::assets::domain::paths::AssetPaths;

/// 目的：按安装根建落点派生器（只给组合根与探针用）。
/// 参数：`install` = 产品安装根。
/// 返回：`AssetPaths`。
/// 约束：今天没有生产读者——各业务只经 `Assets` 拿文本；留着是为了别处不必再拼介质名。
#[allow(dead_code)]
pub fn paths(install: &Path) -> AssetPaths {
    AssetPaths::at(install)
}

/// 目的：提示词册里的一份文档。
pub struct PromptDoc {
    /// 目的：文件全文，交给提示词单元的 `merge_book` 解释。
    pub text: String,
}

/// 目的：随包只读内容的入站契约。
pub trait Assets: Send + Sync {
    /// 目的：读 `systools/` 下的一份固定文档。
    /// 参数：`doc` = 要读哪一份。
    /// 返回：文件全文。
    /// 错误：读不了（文件缺失、权限、解码不是 UTF-8）时给出原因；文案由调用方补文档名。
    fn read(&self, doc: Doc) -> Result<String, String>;

    /// 目的：递归枚举 `prompts/` 下的全部 `.yaml` 并按路径排序后逐份读出。
    /// 返回：按 `name` 升序的文档列表（装配必须确定：同一份代码在任何机器上装配出同一册子）。
    /// 错误：目录读不了、一份都没有、某份读不了。
    fn prompt_docs(&self) -> Result<Vec<PromptDoc>, String>;
}
