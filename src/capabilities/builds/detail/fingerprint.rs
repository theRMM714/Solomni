//! 目的：构建缓存的机制——源指纹与 `.vm/build-state/<模块>.json` 的读写。
//! 管：算源指纹（构建命令 + 能力 + 网络 + 全部源文件）、读/写构建状态、本机档按次家目录（`BuildStore`）。
//! 不管：什么时候该重编（在 `service.rs`，按入口在场与目标比对）。
//! 联动：端口见 `ports.rs`；落点与字段见 MODULE_SPEC 的「构建·缓存」。

use crate::capabilities::builds::ports::{BuildHome, BuildStates, BuiltRecord, SourceDigest};
use crate::capabilities::modules::api::{BuildDecl, Module};
use std::path::{Path, PathBuf};

/// 目的：构建状态的落点（`.vm/build-state/<模块>.json`）——本机档与各虚拟根共用一份，靠 `target` 区分。
/// 参数：`root` = 产品根；`id` = 模块 id。
/// 返回：状态文件路径。
pub(crate) fn state_path(root: &Path, id: &str) -> PathBuf {
    root.join(".vm")
        .join("build-state")
        .join(format!("{}.json", id))
}

/// 目的：构建状态（指纹一致且入口在场就跳过重编）。
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct BuildState {
    /// 目的：模块 id（排障时认人）。
    pub module: String,
    /// 目的：目标（`host` / 虚拟根名）。
    pub target: String,
    /// 目的：这个目标上那次构建的源指纹。
    pub fingerprint: String,
}

/// 目的：读构建状态；读不到（首次）或格式不认识都给 None（那就重编）。
/// 参数：`path` = 状态文件。
/// 返回：读得出的状态；读不出给 None。
pub(crate) fn read_state(path: &Path) -> Option<BuildState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 目的：落一次构建状态（父目录自动建）；写不成如实报错。
/// 参数：`path` = 状态文件；`state` = 这次的事实。
/// 错误：序列化或写盘失败。
pub(crate) fn write_state(path: &Path, state: &BuildState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| format!("状态序列化失败：{}", e))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("建状态目录失败（{}）：{}", dir.display(), e))?;
    }
    std::fs::write(path, format!("{}\n", text))
        .map_err(|e| format!("写状态失败（{}）：{}", path.display(), e))
}

/// 目的：模块的源指纹（构建命令 + 能力 + 网络 + 全部源文件的相对路径与内容）——变了才重编。
/// 参数：`module_root` = 模块根；`decl` = 构建声明；`providers` = 这次构建期能力由谁提供（`能力=提供者`）。
/// 返回：指纹（内容哈希）。
/// 错误：读源文件失败。
/// 约束：跳过 `build/`（产物本身）与 `userdata/`（跨任务私有状态，不是源）。
pub(crate) fn source(
    module_root: &Path,
    decl: &BuildDecl,
    providers: &[String],
) -> Result<String, String> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(decl.command.as_bytes());
    buf.push(0);
    buf.extend_from_slice(decl.needs.join(",").as_bytes());
    buf.push(0);
    buf.extend_from_slice(providers.join(",").as_bytes());
    buf.push(0);
    buf.push(if decl.net { b'1' } else { b'0' });
    buf.push(0);
    collect_sources(module_root, module_root, &mut buf)?;
    Ok(crate::capabilities::workspace::api::content_hash(&buf))
}

/// 目的：端口 `SourceDigest` 与 `BuildStates` 的落盘实现——状态文件与按次家目录都在产品根的 `.vm/build-state/` 下。
/// 约束：由组合根构造并交给 `BuildsService`（端口只有一个持有者，R12）。
pub(crate) struct BuildStore {
    root: PathBuf,
}

impl BuildStore {
    /// 目的：按产品根建适配器。
    /// 参数：`root` = 产品根。
    pub(crate) fn new(root: PathBuf) -> BuildStore {
        BuildStore { root }
    }
}

impl SourceDigest for BuildStore {
    fn digest(
        &self,
        module: &Module,
        decl: &BuildDecl,
        providers: &[String],
    ) -> Result<String, String> {
        source(&module.root, decl, providers)
    }
}

impl BuildStates for BuildStore {
    fn record(&self, id: &str) -> Option<BuiltRecord> {
        read_state(&state_path(&self.root, id)).map(|prev| BuiltRecord {
            target: prev.target,
            fingerprint: prev.fingerprint,
        })
    }

    fn record_built(&self, id: &str, record: &BuiltRecord) -> Result<(), String> {
        write_state(
            &state_path(&self.root, id),
            &BuildState {
                module: id.to_string(),
                target: record.target.clone(),
                fingerprint: record.fingerprint.clone(),
            },
        )
    }

    fn prepare_home(&self, id: &str) -> Result<BuildHome, String> {
        let state = self.root.join(".vm").join("build-state").join(id);
        let home = state.join("home");
        std::fs::create_dir_all(&home).map_err(|e| format!("建状态目录失败：{}", e))?;
        Ok(BuildHome { state, home })
    }
}

/// 目的：把模块根下的源文件按相对路径 + 内容收进指纹缓冲。
/// 参数：`root` = 模块根；`dir` = 当前目录；`buf` = 指纹缓冲。
/// 错误：读目录或读文件失败。
fn collect_sources(root: &Path, dir: &Path, buf: &mut Vec<u8>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("读 {} 失败：{}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let top = rel.split(['/', '\\']).next().unwrap_or("");
        if top == "build" || top == "userdata" {
            continue;
        }
        // 覆盖率插桩会把 .profraw 落在子进程的当前目录（模块根），它不是源文件；
        // 算进指纹会让"第二次构建命中缓存"在覆盖率运行下失真。
        if rel.ends_with(".profraw") || rel.ends_with(".profdata") {
            continue;
        }
        if path.is_dir() {
            collect_sources(root, &path, buf)?;
            continue;
        }
        buf.extend_from_slice(rel.as_bytes());
        buf.push(0);
        buf.extend_from_slice(
            &std::fs::read(&path).map_err(|e| format!("读 {} 失败：{}", path.display(), e))?,
        );
        buf.push(0);
    }
    Ok(())
}
