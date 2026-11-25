//! 目的：登记处资源的入站面——别的单元与组合根只准用这里。
//! 管：`Home`（读 / 整份写）与 `Doc`（允许出现的文档）；文档内容是**原样文本**，不在这里解释。
//! 不管：yaml 解码与校验（在各业务单元）；目录路径（由组合根经 `FsHome::new` 给）。
//! 联动：实现见 `service.rs`；catalog 的三个 yaml 适配器消费本面。

pub use crate::units::home::domain::doc::Doc;

/// 目的：登记处的唯一读写面。
/// 约束：只经值对话（R16）——入参是 `Doc`，出参是原样文本；不把宿主绝对路径交给调用方。
pub trait Home: Send + Sync {
    /// 目的：读一份文档；文件不存在 = `None`（首启没有设置不是错误），其余 IO 错误如实报。
    fn read(&self, doc: Doc) -> Result<Option<String>, String>;

    /// 目的：整份写回一份文档：建目录 → 临时文件 → rename（读者不会读到半份）；私密文档收紧权限。
    fn write(&self, doc: Doc, text: &str) -> Result<(), String>;
}
