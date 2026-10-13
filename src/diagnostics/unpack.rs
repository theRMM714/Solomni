//! 目的：归档解包的机制——跨平台、纯 Rust，不依赖用户装了什么解压工具。
//! 管：按后缀认容器与压缩（tar / tar.gz / tgz / tar.zst / tzst），解到目标目录；软/硬链接建不了就退化成复制。
//! 不管：下载与网络（在 setup-vm-*.js）、归档的来源与布局（调用方定）。
//! 联动：入口层的隐藏模式 --unpack 走它（src/diagnostics/mod.rs）。

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// 目的：把归档解到目录（按后缀认 tar / tar.gz(tgz) / tar.zst(tzst)）。
/// 返回：成功给 Ok；后缀不认识或解码失败给一句为什么。
pub fn extract(archive: &Path, dest: &Path) -> Result<(), String> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let file =
        File::open(archive).map_err(|e| format!("打不开归档 {}：{}", archive.display(), e))?;
    fs::create_dir_all(dest).map_err(|e| format!("建不了目标目录 {}：{}", dest.display(), e))?;
    if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
        let mut file = file;
        let dec = ruzstd::decoding::StreamingDecoder::new(&mut file)
            .map_err(|e| format!("不是有效的 zstd（{}）：{}", archive.display(), e))?;
        unpack_tar(dec, dest)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        unpack_tar(flate2::read::GzDecoder::new(file), dest)
    } else if name.ends_with(".tar") {
        unpack_tar(file, dest)
    } else {
        Err(format!(
            "不认识的归档后缀：{}（认 .tar / .tar.gz / .tgz / .tar.zst / .tzst）",
            name
        ))
    }
}

/// 目的：解 tar 本体——**宽容链接**：宿主建不了链接（Windows 没建链特权）就退化成复制；
/// 链接目标还没解出来就先记账、解完再补（MSYS2 包里大量同目录软链）。
fn unpack_tar<R: Read>(reader: R, dest: &Path) -> Result<(), String> {
    let mut ar = tar::Archive::new(reader);
    let entries = ar.entries().map_err(|e| format!("读 tar 失败：{}", e))?;
    let mut pending: Vec<(PathBuf, PathBuf)> = Vec::new();
    for entry in entries {
        let mut e = entry.map_err(|e| format!("读归档项失败：{}", e))?;
        let rel = e
            .path()
            .map_err(|e| format!("归档项路径非法：{}", e))?
            .into_owned();
        // 拒绝绝对路径与 ..（归档不许往目标目录外写）。
        if !safe_rel(&rel) {
            continue;
        }
        let out = dest.join(&rel);
        let kind = e.header().entry_type();
        if kind == tar::EntryType::Directory {
            fs::create_dir_all(&out).map_err(|x| format!("建目录 {}：{}", out.display(), x))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)
                .map_err(|x| format!("建目录 {}：{}", parent.display(), x))?;
        }
        if kind == tar::EntryType::Regular || kind == tar::EntryType::Continuous {
            let mut f = write_file(&out)?;
            io::copy(&mut e, &mut f).map_err(|x| format!("写 {}：{}", out.display(), x))?;
            continue;
        }
        if kind == tar::EntryType::Symlink {
            let target = e
                .link_name()
                .map_err(|x| format!("软链目标非法：{}", x))?
                .map(|p| p.into_owned());
            if let Some(t) = target.as_deref() {
                let tgt = out
                    .parent()
                    .map(|p| p.join(t))
                    .unwrap_or_else(|| t.to_path_buf());
                if !try_symlink(t, &tgt, &out) {
                    pending.push((out.clone(), tgt));
                }
            }
            continue;
        }
        if kind == tar::EntryType::Link {
            let target = e
                .link_name()
                .map_err(|x| format!("硬链目标非法：{}", x))?
                .map(|p| p.into_owned());
            if let Some(t) = target.as_deref() {
                let tgt = dest.join(t);
                if fs::hard_link(&tgt, &out).is_err() {
                    pending.push((out.clone(), tgt));
                }
            }
            continue;
        }
        // 其它类型（设备 / 管道等）宿主建不了也用不到，跳过。
    }
    // 记账的链接：目标可能后到、也可能自身是链接，反复补几轮。
    for _ in 0..4 {
        if pending.is_empty() {
            break;
        }
        let mut left = Vec::new();
        for (link, tgt) in pending.drain(..) {
            if !copy_if_exists(&tgt, &link) {
                left.push((link, tgt));
            }
        }
        pending = left;
    }
    Ok(())
}

fn safe_rel(p: &Path) -> bool {
    !p.is_absolute()
        && !p.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn write_file(out: &Path) -> Result<File, String> {
    match File::create(out) {
        Ok(f) => Ok(f),
        Err(_) => {
            // 已有只读残留：删掉再建一次（重复解同一目录时常见）。
            let _ = fs::remove_file(out);
            File::create(out).map_err(|e| format!("建不了 {}：{}", out.display(), e))
        }
    }
}

/// 目的：先试真链接；成不了就交给"复制"兜底（返回 false 表示要记账后补）。
fn try_symlink(target: &Path, tgt_abs: &Path, out: &Path) -> bool {
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(target, out).is_ok() {
            return true;
        }
    }
    #[cfg(windows)]
    {
        let r = if tgt_abs.is_dir() {
            std::os::windows::fs::symlink_dir(target, out)
        } else {
            std::os::windows::fs::symlink_file(target, out)
        };
        if r.is_ok() {
            return true;
        }
    }
    copy_if_exists(tgt_abs, out)
}

fn copy_if_exists(src: &Path, dst: &Path) -> bool {
    let Ok(mut s) = File::open(src) else {
        return false;
    };
    let Ok(mut d) = write_file(dst) else {
        return false;
    };
    io::copy(&mut s, &mut d).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        crate::tests::scratch(&format!("unpack-{}", tag))
    }

    #[test]
    fn extracts_plain_tar_and_tar_gz() {
        let src = temp("src");
        fs::write(src.join("a.txt"), b"hello").expect("写源文件");

        let tar_path = temp("tarsrc").join("x.tar");
        {
            let f = File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            b.append_dir_all(".", &src).expect("打包");
            b.finish().expect("收尾");
        }
        let out = temp("out1");
        extract(&tar_path, &out).expect("解 tar");
        assert_eq!(
            fs::read(out.join("a.txt")).expect("读回"),
            b"hello".to_vec()
        );

        let gz_path = temp("gzsrc").join("x.tar.gz");
        {
            let f = File::create(&gz_path).expect("建 gz");
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut b = tar::Builder::new(enc);
            b.append_dir_all(".", &src).expect("打包");
            b.finish().expect("收尾");
        }
        let out2 = temp("out2");
        extract(&gz_path, &out2).expect("解 tar.gz");
        assert_eq!(
            fs::read(out2.join("a.txt")).expect("读回"),
            b"hello".to_vec()
        );

        for d in [&src, &out, &out2] {
            let _ = fs::remove_dir_all(d);
        }
    }

    #[test]
    fn symlink_entries_are_materialized() {
        // Windows 上没有建链特权：这里必须退化成复制，而不是整个解包失败（MSYS2 包里大量这种软链）。
        let src = temp("linksrc");
        fs::write(src.join("real.txt"), b"data").expect("写文件");
        let tar_path = temp("linktar").join("s.tar");
        {
            let f = File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            b.append_dir_all(".", &src).expect("打包");
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, "link.txt", "real.txt")
                .expect("写软链");
            b.finish().expect("收尾");
        }
        let out = temp("linkout");
        extract(&tar_path, &out).expect("解含软链的 tar");
        assert_eq!(
            fs::read(out.join("link.txt")).expect("链接落点要能读到内容"),
            b"data".to_vec()
        );
        for d in [&src, &out] {
            let _ = fs::remove_dir_all(d);
        }
    }

    #[test]
    fn unknown_suffix_is_refused() {
        let dir = temp("bad");
        let p = dir.join("x.rar");
        fs::write(&p, b"x").expect("写");
        assert!(
            extract(&p, &dir.join("o")).is_err(),
            "不认识的归档后缀要如实拒收"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
