//! 目的：归档解包的机制——跨平台、纯 Rust，不依赖用户装了什么解压工具。
//! 管：按后缀认容器与压缩（tar / tar.gz / tgz / tar.zst / tzst / tar.xz / txz / zip），解到目标目录；
//!   软/硬链接建不了就退化成复制；`strip` 可按段剥掉归档里的顶层目录。
//! 不管：下载与网络、归档的来源与布局（调用方定）。
//! 联动：入口层的隐藏模式 --unpack 走 extract；依赖补齐走 extract_into（带 strip）。

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// 目的：把归档解到目录（按后缀认 tar / tar.gz(tgz) / tar.zst(tzst) / tar.xz(txz) / zip），不剥层。
/// 返回：成功给 Ok；后缀不认识或解码失败给一句为什么。
pub fn extract(archive: &Path, dest: &Path) -> Result<(), String> {
    extract_into(archive, dest, 0)
}

/// 目的：同 extract，但可按段剥掉归档里的顶层目录（`strip` 段）——依赖补齐的便携包常带一层包装目录。
pub fn extract_into(archive: &Path, dest: &Path, strip: usize) -> Result<(), String> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    fs::create_dir_all(dest).map_err(|e| format!("建不了目标目录 {}：{}", dest.display(), e))?;
    if name.ends_with(".zip") {
        return unpack_zip(archive, dest, strip);
    }
    let file =
        File::open(archive).map_err(|e| format!("打不开归档 {}：{}", archive.display(), e))?;
    if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
        let mut file = file;
        let dec = ruzstd::decoding::StreamingDecoder::new(&mut file)
            .map_err(|e| format!("不是有效的 zstd（{}）：{}", archive.display(), e))?;
        unpack_tar(dec, dest, strip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        unpack_tar(flate2::read::GzDecoder::new(file), dest, strip)
    } else if name.ends_with(".tar") {
        unpack_tar(file, dest, strip)
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        // xz 没有现成的 Read 适配：先解到临时 tar，再走同一条 tar 解包路径（解完即删）。
        let tmp = xz_temp();
        {
            let mut file = std::io::BufReader::new(file);
            let mut out = File::create(&tmp)
                .map_err(|e| format!("建不了临时文件 {}：{}", tmp.display(), e))?;
            lzma_rs::xz_decompress(&mut file, &mut out)
                .map_err(|e| format!("xz 解压失败（{}）：{:?}", archive.display(), e))?;
        }
        let r = File::open(&tmp)
            .map_err(|e| format!("打不开临时文件 {}：{}", tmp.display(), e))
            .and_then(|f| unpack_tar(f, dest, strip));
        let _ = fs::remove_file(&tmp);
        r
    } else {
        Err(format!(
            "不认识的归档后缀：{}（认 .tar / .tar.gz / .tgz / .tar.zst / .tzst / .tar.xz / .txz / .zip）",
            name
        ))
    }
}

/// 目的：一个唯一的临时 tar 路径（xz 中转用；进程号 + 时间纳秒，避免并发相撞）。
fn xz_temp() -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("solomni-xz-{}-{}.tar", std::process::id(), n))
}

/// 目的：解 tar 本体——**宽容链接**：宿主建不了链接（Windows 没建链特权）就退化成复制；
/// 链接目标还没解出来就先记账、解完再补（MSYS2 包里大量同目录软链）。
fn unpack_tar<R: Read>(reader: R, dest: &Path, strip: usize) -> Result<(), String> {
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
        let Some(rel) = strip_rel(&rel, strip) else {
            continue;
        };
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
            // 权限位要在拷贝**之前**从头上取（拷贝会把 entry 用掉）；非 unix 上不取也不设。
            #[cfg(unix)]
            let mode = e.header().mode().ok();
            {
                let mut f = write_file(&out)?;
                io::copy(&mut e, &mut f).map_err(|x| format!("写 {}：{}", out.display(), x))?;
            }
            // 还原权限位（可执行位就在这里）：不还的话，归档里解出来的程序一律 0644，跑不起来。
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(m) = mode {
                    let _ = fs::set_permissions(&out, fs::Permissions::from_mode(m & 0o7777));
                }
            }
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

/// 目的：剥掉归档路径的前 `strip` 段；剥完什么都不剩（或只剩空段）给 None（跳过这一项）。
fn strip_rel(rel: &Path, strip: usize) -> Option<PathBuf> {
    if strip == 0 {
        return Some(rel.to_path_buf());
    }
    let comps: Vec<Component> = rel.components().collect();
    if comps.len() <= strip {
        return None;
    }
    let mut out = PathBuf::new();
    for c in &comps[strip..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 目的：解 zip 本体——目录/普通文件按条写；unix 权限位（可执行位）按归档保留。
fn unpack_zip(archive: &Path, dest: &Path, strip: usize) -> Result<(), String> {
    let file =
        File::open(archive).map_err(|e| format!("打不开归档 {}：{}", archive.display(), e))?;
    let mut z = zip::ZipArchive::new(file)
        .map_err(|e| format!("不是有效的 zip（{}）：{}", archive.display(), e))?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|x| format!("读归档项失败：{}", x))?;
        let Some(rel) = e.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        if !safe_rel(&rel) {
            continue;
        }
        let Some(rel) = strip_rel(&rel, strip) else {
            continue;
        };
        let out = dest.join(&rel);
        if e.is_dir() {
            fs::create_dir_all(&out).map_err(|x| format!("建目录 {}：{}", out.display(), x))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)
                .map_err(|x| format!("建目录 {}：{}", parent.display(), x))?;
        }
        let mut f = write_file(&out)?;
        io::copy(&mut e, &mut f).map_err(|x| format!("写 {}：{}", out.display(), x))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = e.unix_mode() {
                let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode));
            }
        }
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

        // 权限位（可执行位）要还原：归档里的程序解出来必须还能跑。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exe_src = temp("exesrc");
            let exe = exe_src.join("tool");
            fs::write(&exe, b"#!/bin/sh\n").expect("写可执行源文件");
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).expect("给可执行位");
            let plain = exe_src.join("data.txt");
            fs::write(&plain, b"data").expect("写普通文件");
            fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).expect("普通权限");
            let exe_tar = temp("exetar").join("x.tar");
            {
                let f = File::create(&exe_tar).expect("建 tar");
                let mut b = tar::Builder::new(f);
                b.append_dir_all(".", &exe_src).expect("打包");
                b.finish().expect("收尾");
            }
            let exe_out = temp("exeout");
            extract(&exe_tar, &exe_out).expect("解 tar");
            let got = fs::metadata(exe_out.join("tool"))
                .expect("读权限")
                .permissions()
                .mode();
            assert_eq!(
                got & 0o777,
                0o755,
                "可执行位要还原（拿到 {:o}）",
                got & 0o777
            );
            let data = fs::metadata(exe_out.join("data.txt"))
                .expect("读权限")
                .permissions()
                .mode();
            assert_eq!(data & 0o777, 0o644, "普通文件权限也要照原样");
            for d in [&exe_src, &exe_out] {
                let _ = fs::remove_dir_all(d);
            }
        }
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

    /// 目的：zip 解包 + `strip` 剥一层——落点正确，unix 可执行位保留。
    #[test]
    fn extracts_zip_and_strips_leading_dir() {
        use std::io::Write;
        let dir = temp("zip");
        let zip_path = dir.join("x.zip");
        {
            let f = File::create(&zip_path).expect("建 zip");
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            w.add_directory("pkg/", opts).expect("目录项");
            w.start_file("pkg/a/b.txt", opts).expect("文件项");
            w.write_all(b"zipped").expect("写内容");
            w.finish().expect("收尾");
        }
        let out = dir.join("out");
        extract_into(&zip_path, &out, 1).expect("解 zip 并剥一层");
        assert_eq!(
            fs::read(out.join("a").join("b.txt")).expect("读回"),
            b"zipped".to_vec()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(out.join("a").join("b.txt"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "可执行位要保留：{:o}", mode);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// 目的：tar.xz 解包（先把 tar 压成 xz，再解回来）。
    #[test]
    fn extracts_tar_xz() {
        let dir = temp("xz");
        fs::write(dir.join("a.txt"), b"xzdata").expect("写");
        let tar_path = dir.join("x.tar");
        {
            let f = File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            b.append_path_with_name(dir.join("a.txt"), "a.txt")
                .expect("打包");
            b.finish().expect("收尾");
        }
        let xz_path = dir.join("x.tar.xz");
        {
            let mut input = std::io::BufReader::new(File::open(&tar_path).expect("开 tar"));
            let mut out = File::create(&xz_path).expect("建 xz");
            lzma_rs::xz_compress(&mut input, &mut out).expect("压 xz");
        }
        let out = dir.join("out");
        extract(&xz_path, &out).expect("解 tar.xz");
        assert_eq!(
            fs::read(out.join("a.txt")).expect("读回"),
            b"xzdata".to_vec()
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
