//! 目的：模块入口的判定——模块工具声明的相对入口在目标环境里在不在场、是不是那个平台能直接跑的形式。
//! 管：`EntryProblem` / `MissingEntry` 两个形状、`missing_entries_for` 挑入口、`runs_on_target` 判文件形式。
//! 不管：包选择诊断（`catalog` 的 `packages` 子业务）、模块清单本身的校验（`domain/module.rs`）、怎么把入口复制进 guest（`vm`）。
//! 联动：`catalog` 的 `packages` 把入口问题并进执行计划；`work` 与 `maintenance` 在建/改/构建前预检。

use crate::units::business::catalog::modules::domain::module::{
    Module, entry_candidates, local_entry,
};
use serde::Serialize;
use std::path::Path;

/// 目的：一条**模块入口**诊断：结构化事实（界面文案由呈现层渲染；模型侧文案在提示词册）。
/// 约束：包选择问题归 `catalog` 的 `packages` 子业务的 `Problem`，不混进这里。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum EntryProblem {
    /// 模块工具的入口（模块内产物）不在场——命令是相对路径，模块根下却没有这个能直接跑的文件。
    MissingEntry {
        module: String,
        tool: String,
        command: String,
        /// 目的：这个模块声明了 `build:`（系统可以照它在目标环境里编出来）——拒绝理由据此指路。
        buildable: bool,
    },
}

/// 目的：一条"模块工具的入口不在场"的**事实**（谁、哪个工具、命令里写的那个入口）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingEntry {
    pub module: String,
    pub tool: String,
    /// 目的：命令里写的相对入口（如 `build/indexer`）。
    pub command: String,
    /// 目的：这个模块声明了 `build:`（见 MODULE_SPEC）——拒绝理由里就能给出"怎么编"的路。
    pub buildable: bool,
}

/// 目的：挑出"入口不在场"的模块工具——相对路径入口必须在**目标环境**里存在且能直接执行。
/// 参数：os = 目标系统（本机档 = 宿主，虚拟机档 = guest 的 os）；is_executable = "这个路径在目标环境里能不能直接跑"的事实。
/// 约束：裸名（`python` / `node` / `indexer`）走 PATH 与运行能力，不在本判据里；绝对路径同样不在模块目录里找。
/// 约束：名字按目标环境的壳解析（见 `entry_candidates`），跨平台时调用方还要按文件形式筛（见 `runs_on_target`）。
pub fn missing_entries_for(
    modules: &[Module],
    os: &str,
    is_executable: &dyn Fn(&Path) -> bool,
) -> Vec<MissingEntry> {
    let mut out = Vec::new();
    for m in modules {
        for (tool, decl) in &m.manifest.tools {
            let Some(entry) = local_entry(&decl.command) else {
                continue;
            };
            // 壳怎么解析这个名字，就按它的候选去验（Windows 上 `build/indexer` 实际落在 `.exe`）。
            let found = entry_candidates(entry, os)
                .iter()
                .any(|c| is_executable(&m.root.join(c)));
            if !found {
                out.push(MissingEntry {
                    module: m.manifest.id.clone(),
                    tool: tool.clone(),
                    command: entry.to_string(),
                    buildable: m.manifest.build.is_some(),
                });
            }
        }
    }
    out
}

/// 目的：这个文件在**目标平台**上能直接跑吗——看形式（ELF / Mach-O / PE / 脚本头 / Windows 的批处理扩展名）。
/// 参数：path = 模块目录里的入口文件；target_os = 目标平台（`std::env::consts::OS` 的取值）。
/// 约束：名字层面的解析由 `entry_candidates` 管，这里只管**内容**；跨平台时"宿主编出来的产物"与目标格式不符，
///   必须在预检就判否，而不是等 guest 里报 `not found` / `Exec format error`。
pub fn runs_on_target(path: &Path, target_os: &str) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 4];
    let n = f.read(&mut head).unwrap_or(0);
    let head = &head[..n];
    let elf = head == b"\x7fELF";
    let pe = head.starts_with(b"MZ");
    let script = head.starts_with(b"#!");
    let mach = matches!(
        head,
        [0xfe, 0xed, 0xfa, 0xce]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
    );
    match target_os {
        // Windows 的壳按扩展名认批处理与命令脚本；可执行体是 PE。
        "windows" => {
            pe || script
                || path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| matches!(e.to_ascii_lowercase().as_str(), "cmd" | "bat" | "com"))
                    .unwrap_or(false)
        }
        "macos" => mach || script,
        _ => elf || script,
    }
}
