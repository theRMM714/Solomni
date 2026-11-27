//! 目的：会话形状声明的装载——两份 yaml 读成 `SessionDecls`，装配期规则在这里收口。
//! 管：`YamlDecls::load` 的读取、解析与 `validate` 调用。
//! 不管：形状本身（在 `domain/mechanism.rs` 与 `domain/profile.rs`）；读盘（`resource::assets` 端口）；消费（`Flows` / `Core`）。
//! 联动：组合根装载它并注入；装载失败就是装配失败（`main.rs` 打印并退出）。

use std::sync::Arc;

use crate::units::business::session_model::domain::decls::SessionDecls;
use crate::units::business::session_model::domain::mechanism::Loops;
use crate::units::business::session_model::domain::profile::Profiles;
use crate::units::resource::assets::api::{Assets, Doc};

/// 目的：从随包只读内容里装载会话形状声明。
pub struct YamlDecls {
    assets: Arc<dyn Assets>,
}

impl YamlDecls {
    /// 目的：按只读内容端口装配。
    /// 参数：`assets` = 随包只读内容端口。
    /// 返回：可装载的适配器。
    pub fn new(assets: Arc<dyn Assets>) -> YamlDecls {
        YamlDecls { assets }
    }

    /// 目的：读两份声明、解析成形状并做装配期校验。
    /// 返回：装好的 `SessionDecls`。
    /// 错误：读不了、不是合法 yaml、或校验不过（文案点名文件与出问题的那一项）。
    pub fn load(&self) -> Result<SessionDecls, String> {
        let loops: Loops = self.read(Doc::Loops, "流程段表")?;
        let profiles: Profiles = self.read(Doc::Profiles, "会话类型表")?;
        let decls = SessionDecls::new(loops, profiles);
        decls.validate()?;
        Ok(decls)
    }

    /// 目的：读一份声明 yaml 并解析成它的形状。
    fn read<T: serde::de::DeserializeOwned>(&self, doc: Doc, what: &str) -> Result<T, String> {
        let text = self
            .assets
            .read(doc)
            .map_err(|e| format!("{}读不了（{}）：{}", what, doc.label(), e))?;
        yaml_serde::from_str(&text).map_err(|e| format!("{}非法（{}）：{}", what, doc.label(), e))
    }
}
