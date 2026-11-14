//! 目的：按安装方式落盘——归档、单文件、AppImage、工具链与 guest 磁盘。
//! 管：解归档（含 strip）、复制单文件、AppImage 自解包、Zig 包装脚本、guest 磁盘与画像。
//! 不管：下载与校验（在 fetch / service）、镜像内部构造（在 guest_image）、台账（在 ledger）。
//! 联动：service 按清单的 kind 选一个方法调用；本文件直接碰宿主文件系统与外部工具。

use std::fs;
use std::path::{Path, PathBuf};

use crate::capabilities::provision::detail::host::write_exe;
use crate::capabilities::provision::domain::Dep;
use crate::capabilities::provision::ports::Installers;

/// 目的：宿主安装器的实现——每个方法对应清单里的一种 `kind`。
#[derive(Debug, Default)]
pub struct HostInstallers;

impl Installers for HostInstallers {
    fn extract(&self, archive: &Path, dest: &Path, strip: usize) -> Result<(), String> {
        crate::kernel::detail::unpack::extract_into(archive, dest, strip)
    }

    fn file(&self, cached: &Path, dest: &Path, probe: &str) -> Result<(), String> {
        install_file(cached, dest, probe)
    }

    fn appimage(
        &self,
        cached: &Path,
        dest: &Path,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        install_appimage(cached, dest, on_line)
    }

    fn toolchain(
        &self,
        cached: &Path,
        dest: &Path,
        dep: &Dep,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        install_toolchain(cached, dest, dep, on_line)
    }

    fn guest_disk(
        &self,
        cached: &Path,
        dest: &Path,
        probe: &str,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        install_guest_disk(cached, dest, probe, on_line)
    }
}

/// 目的：基础工具链条目——解归档到安装目录；Zig 包再写 cc / gcc / g++ / clang 包装。
pub(crate) fn install_toolchain(
    cached: &Path,
    dest: &Path,
    dep: &Dep,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let spec = dep
        .toolchain
        .as_ref()
        .ok_or_else(|| "kind: toolchain 缺 toolchain 段".to_string())?;
    crate::kernel::detail::unpack::extract_into(cached, dest, dep.strip)?;
    if spec.wrappers {
        write_zig_wrappers(dest)?;
        on_line("已写 cc / gcc / g++ / clang 包装（Zig 提供）");
    }
    Ok(())
}

/// 目的：在安装目录里找 Zig 可执行文件（包根目录名带平台与版本，不写死）。
pub(crate) fn find_zig(root: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        let mut subdirs = Vec::new();
        for e in fs::read_dir(dir).ok()? {
            let e = e.ok()?;
            let p = e.path();
            if e.file_type().ok()?.is_dir() {
                subdirs.push(p);
            } else {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == "zig" || name == "zig.exe" {
                    return Some(p);
                }
            }
        }
        subdirs.into_iter().find_map(|d| walk(&d, depth + 1))
    }
    walk(root, 0)
}

/// 目的：把 `zig cc` / `zig c++` 包成常规编译器名（模块构建按 `gcc` / `g++` 这类能力名声明）。
/// 约束：包装用相对路径（`$DIR/../…`），产品根整体搬移后仍可用；unix 上给执行位（其它平台清单不用 Zig）。
pub(crate) fn write_zig_wrappers(install: &Path) -> Result<(), String> {
    let zig = find_zig(install)
        .ok_or_else(|| format!("包里找不到 zig 可执行文件：{}", install.display()))?;
    let rel = zig
        .strip_prefix(install)
        .map_err(|_| format!("zig 不在安装目录里：{}", zig.display()))?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    let bin = install.join("bin");
    fs::create_dir_all(&bin).map_err(|e| format!("建 {} 失败：{}", bin.display(), e))?;
    // Zig 默认把全局缓存写到 ~/.cache/zig（项目外）；钉进安装目录，项目根整体搬移后仍成立。
    let zig_body = format!(
        r#"#!/bin/sh
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export ZIG_GLOBAL_CACHE_DIR="${{ZIG_GLOBAL_CACHE_DIR:-$DIR/../cache/global}}"
export ZIG_LOCAL_CACHE_DIR="${{ZIG_LOCAL_CACHE_DIR:-$DIR/../cache/local}}"
exec "$DIR/../{}" "$@"
"#,
        rel
    );
    write_exe(&bin.join("zig"), &zig_body)?;
    for (name, sub) in [
        ("cc", "cc"),
        ("gcc", "cc"),
        ("clang", "cc"),
        ("c++", "c++"),
        ("g++", "c++"),
        ("clang++", "c++"),
    ] {
        // cc-rs 把 clang 形编译器当跨平台 clang，传 Rust 三元组（x86_64-unknown-linux-gnu）；
        // Zig 不认 `unknown-`，不翻译时 ring 等 C 依赖直接构建失败。
        let body = format!(
            r#"#!/bin/sh
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export ZIG_GLOBAL_CACHE_DIR="${{ZIG_GLOBAL_CACHE_DIR:-$DIR/../cache/global}}"
export ZIG_LOCAL_CACHE_DIR="${{ZIG_LOCAL_CACHE_DIR:-$DIR/../cache/local}}"
n=$#
i=0
while [ "$i" -lt "$n" ]; do
  a=$1
  shift
  case "$a" in
    --target=*unknown-*) a=$(printf '%s\n' "$a" | sed 's/^--target=\(.*\)-unknown-/--target=\1-/') ;;
    --target=x86_64-apple-darwin) a=--target=x86_64-macos ;;
    --target=aarch64-apple-darwin) a=--target=aarch64-macos ;;
  esac
  set -- "$@" "$a"
  i=$((i + 1))
done
exec "$DIR/zig" {} "$@"
"#,
            sub
        );
        write_exe(&bin.join(name), &body)?;
    }
    Ok(())
}

/// 目的：单文件依赖——把缓存里的文件按 probe 的文件名放进安装目录（如 guest 的 qcow2）。
pub(crate) fn install_file(cached: &Path, dest: &Path, probe: &str) -> Result<(), String> {
    let name = Path::new(probe)
        .file_name()
        .ok_or_else(|| format!("probe 没有文件名：{}", probe))?;
    let to = dest.join(name);
    fs::copy(cached, &to).map_err(|e| format!("复制到 {} 失败：{}", to.display(), e))?;
    Ok(())
}

/// 目的：AppImage 便携包——跑它自带的 `--appimage-extract`（不依赖 FUSE），把解出的 AppDir 内容搬进安装目录。
/// 约束：`bin/` 里的 applet 按 argv[0] 解析 `../shared/bin/<名>`，所以**保持原布局**、不做包装脚本。
pub(crate) fn install_appimage(
    archive: &Path,
    dest: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    crate::kernel::api::make_executable_checked(archive)
        .map_err(|e| format!("给 {} 加执行位失败：{}", archive.display(), e))?;
    // 解到安装目录里（同一文件系统，随后原地搬移，避免跨设备 rename）。
    let _ = fs::remove_dir_all(dest.join("AppDir"));
    let _ = fs::remove_file(dest.join("squashfs-root"));
    on_line("解包 AppImage（--appimage-extract，不依赖 FUSE）");
    let out = std::process::Command::new(archive)
        .arg("--appimage-extract")
        .current_dir(dest)
        .output()
        .map_err(|e| format!("跑不了 AppImage 解包（{}）：{}", archive.display(), e))?;
    if !out.status.success() {
        let msg: String = String::from_utf8_lossy(&out.stderr)
            .chars()
            .take(200)
            .collect();
        return Err(format!("AppImage 解包失败：{}", msg));
    }
    // pkgforge 的运行时解到 AppDir/（另留一个 squashfs-root 软链）；两种布局都认。
    let root = dest.join("AppDir");
    let root = if root.is_dir() {
        root
    } else {
        dest.join("squashfs-root")
    };
    if !root.is_dir() {
        return Err("AppImage 解包后没找到 AppDir / squashfs-root".to_string());
    }
    for entry in fs::read_dir(&root).map_err(|e| format!("读 {} 失败：{}", root.display(), e))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dest.join(entry.file_name());
        if to.exists() {
            continue; // 重复安装时幂等：已在场的不覆盖
        }
        fs::rename(entry.path(), &to)
            .map_err(|e| format!("搬 {} 失败：{}", entry.file_name().to_string_lossy(), e))?;
    }
    let _ = fs::remove_dir_all(dest.join("AppDir"));
    let _ = fs::remove_file(dest.join("squashfs-root"));
    Ok(())
}

/// 目的：Windows 宿主的 guest 磁盘——qcow2 单文件落盘，并写一份 guest.yaml 画像（命令通道 QGA）。
pub(crate) fn install_guest_disk(
    cached: &Path,
    dest: &Path,
    probe: &str,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    // 磁盘是**可变状态**（guest 里已经装过 agent / 运行包），不能拿缓存里的原始镜像覆盖它；
    // 只有不在场时才落盘。运行期编排（guest-runtime）负责把它编到可用。
    let name = Path::new(probe)
        .file_name()
        .ok_or_else(|| format!("probe 没有文件名：{}", probe))?;
    if dest.join(name).is_file() {
        on_line("guest 镜像已在场，不覆盖（运行期状态在里面）");
    } else {
        install_file(cached, dest, probe)?;
    }
    let yaml = dest.join("guest.yaml");
    if !yaml.is_file() {
        fs::write(
            &yaml,
            "# 由产品生成：命令通道 QGA；共享客户端由运行期编排装好 cifs-utils 后声明（见 guest-runtime）。\n\nid: debian-13-genericcloud-amd64\nos: linux\narch: amd64\nagent: qga\nclients: []\nruntimes: []\n",
        )
        .map_err(|e| format!("写 guest.yaml 失败：{}", e))?;
        on_line("写 guest.yaml（QGA 通道画像）");
    }
    Ok(())
}
