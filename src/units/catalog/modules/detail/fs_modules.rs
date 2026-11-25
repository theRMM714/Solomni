//! 目的：模块清单来源——扫描 `modules/` 目录（实现 `ModuleSource` 端口）。
//! 管：目录遍历、yaml 解析、拒收原因如实呈现。
//! 不管：清单契约与校验（`domain/module.rs`）、重扫策略（`service.rs`）、私有区的落点与确保存在（`content` 单元，本文件只问它要）。
//! 联动：由本单元 `service.rs` 持有；见 docs/modules/README.md。

use crate::units::catalog::modules::api::{Module, ModuleManifest, Roster};
use crate::units::catalog::modules::ports::ModuleSource;
use crate::units::content::api::Content;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct FsModules {
    /// **工具名保留表**（内置工具名）：由组合根在装配期问一次 `tools` 交进来——
    /// 清单校验归本单元，名字空间归工具单元，两边不互相依赖。
    tool_reserved: Vec<String>,
    /// **模块 id 保留表**：产品命令名/别名与 `names.yaml` 的 `modules` 块。
    /// 模块 id 会成为动作 id 前缀 `module.<模块>.<工具>`，所以不能与命令词同形。
    module_reserved: Vec<String>,
    dir: PathBuf,
    /// **内容盘**：模块私有区（`data/modules/<id>/userdata`）的落点与确保存在归它。
    content: Arc<dyn Content + Send + Sync>,
}

impl FsModules {
    pub fn new(
        dir: PathBuf,
        tool_reserved: Vec<String>,
        module_reserved: Vec<String>,
        content: Arc<dyn Content + Send + Sync>,
    ) -> FsModules {
        FsModules {
            dir,
            tool_reserved,
            module_reserved,
            content,
        }
    }
}

impl ModuleSource for FsModules {
    fn scan(&self) -> Roster {
        scan_dir(
            &self.dir,
            &self.tool_reserved,
            &self.module_reserved,
            self.content.as_ref(),
        )
    }
}

/// 列出 modules/ 下每个含合法 module.yaml 的文件夹。
/// 放入即出现，移出即消失；id 与文件夹名不一致 = 非法，拒收并说明原因。
fn scan_dir(
    modules_dir: &Path,
    reserved: &[String],
    module_reserved: &[String],
    content: &dyn Content,
) -> Roster {
    let mut modules = Vec::new();
    let mut rejected = Vec::new();
    let entries = match std::fs::read_dir(modules_dir) {
        Ok(e) => e,
        Err(_) => return Roster { modules, rejected },
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let yaml_path = path.join("module.yaml");
        let yaml_text = match std::fs::read_to_string(&yaml_path) {
            Ok(t) => t,
            Err(_) => {
                rejected.push(format!(
                    "{}: 缺少 module.yaml",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                continue;
            }
        };
        match yaml_serde::from_str::<ModuleManifest>(&yaml_text) {
            Ok(m) => {
                let dir_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if m.id != dir_name {
                    rejected.push(format!("{}: id '{}' 与文件夹名不一致", dir_name, m.id));
                    continue;
                }
                if let Err(why) =
                    crate::units::catalog::modules::api::check_module_id(&m.id, module_reserved)
                {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::units::catalog::modules::api::check_version(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::units::catalog::modules::api::check_runtimes(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::units::catalog::modules::api::check_build(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::units::catalog::modules::api::check_tools(&m, reserved) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::units::catalog::modules::api::check_services(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                if let Err(why) = crate::units::catalog::modules::api::check_secrets(&m) {
                    rejected.push(format!("{}: {}", dir_name, why));
                    continue;
                }
                // 私有区在 data/modules/<id>/userdata，不在源码树里（源码随包只读）；载入即确保它存在
                // （产品唯一的自动写盘，幂等）。建不了不阻断加载（只读挂载、权限不足）：如实记一条，事实字段给"能不能用"。
                let usable = content.ensure_userdata(&m.id);
                if !usable {
                    eprintln!(
                        "[模块] {} 的私有区建不了（data/modules/{}/userdata）：这一席拿不到私有可写落点",
                        dir_name, m.id
                    );
                }
                let private = usable.then(|| content.area(&m.id).userdata());
                modules.push(Module {
                    manifest: m,
                    root: path.clone(),
                    private,
                });
            }
            Err(e) => rejected.push(format!(
                "{}: module.yaml 非法（{}）",
                path.file_name().unwrap_or_default().to_string_lossy(),
                e
            )),
        }
    }
    modules.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    Roster { modules, rejected }
}
