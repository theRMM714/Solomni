//! 目的：构建缓存卷的机制——卷落点、对账标识、guest 侧准备/同步脚本与同一虚拟根的构建锁。
//! 管：卷路径与大小、卷/模块的环境标识、工作区布局、源同步与差异回写脚本、构建锁。
//! 不管：编什么、什么时候编（`service.rs`）、guest 与盘怎么起（`vm`）。
//! 联动：由 `service.rs` 组装成 guest 脚本；口径见 docs/builds/README.md 的「目标环境」与单元地图。

use crate::capabilities::vm::api::GuestImage;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 目的：卷的布局版本——布局变了就清空旧卷，不把两种布局混在一起。
const FORMAT: &str = "v1";
/// 目的：guest 里的挂载点——构建期的工作区、HOME、TMPDIR 都在这下面，因此中间产物不占 guest 根盘。
pub(crate) const MOUNT: &str = "/var/lib/solomni/build";
/// 目的：guest 里挂载的设备名——构建 guest 只有根盘（vda）与缓存卷（vdb）两块 virtio-blk。
pub(crate) const DEVICE: &str = "/dev/vdb";
/// 目的：等锁的上限：一次慢构建可能超过十分钟；等不到就如实拒绝（绝不并写同一块卷）。
const LOCK_WAIT: Duration = Duration::from_secs(600);
/// 目的：锁文件超过这么久没被释放，视为上一次构建没收尾留下的死锁（按修改时间判）。
const LOCK_STALE: Duration = Duration::from_secs(3 * 3600);

/// 目的：缓存卷目录（`<产品根>/.vm/build-cache`）。
pub(crate) fn dir(root: &Path) -> PathBuf {
    root.join(".vm").join("build-cache")
}

/// 目的：缓存卷落点——每个虚拟根一块（同一模块在不同虚拟根里的工具链不同，缓存不能混）。
pub(crate) fn volume_path(root: &Path, guest: &str) -> PathBuf {
    dir(root).join(format!("{}.qcow2", safe_name(guest)))
}

/// 目的：构建锁落点（每个虚拟根一把）。
pub(crate) fn lock_path(root: &Path, guest: &str) -> PathBuf {
    dir(root).join(format!("{}.lock", safe_name(guest)))
}

/// 目的：卷的环境标识——镜像与 guest 运行能力变了，卷里的缓存就不再可信（工具链换了）。
/// 约束：只放"环境"事实；模块自己的能力由 `module_epoch` 单独对账，换一个模块的能力不该清掉别人的缓存。
pub(crate) fn volume_epoch(image: &GuestImage) -> String {
    format!(
        "{FORMAT}|{}|{}|{}|{}",
        image.id,
        image.arch.as_str(),
        image.os.as_str(),
        image.runtimes.join(",")
    )
}

/// 目的：模块的环境标识——构建期能力与它的实际提供者变了，这个模块的工作区就不可信。
pub(crate) fn module_epoch(needs: &[String], providers: &[String]) -> String {
    format!("{FORMAT}|{}|{}", needs.join(","), providers.join(","))
}

/// 目的：模块在卷上的目录（guest 路径）。
pub(crate) fn module_dir(id: &str) -> String {
    format!("{MOUNT}/modules/{}", safe_name(id))
}

/// 目的：模块的构建工作区（guest 路径）——模块树的本地副本，构建命令的 cwd。
pub(crate) fn workspace(id: &str) -> String {
    format!("{}/ws", module_dir(id))
}

/// 目的：模块的 HOME（guest 路径）——工具链缓存（`~/.cargo` / `~/.ccache` / `~/.npm`）自然落到卷上。
pub(crate) fn home(id: &str) -> String {
    format!("{}/home", module_dir(id))
}

/// 目的：模块的 TMPDIR（guest 路径）——编译器的临时文件也留在卷上，不进 guest 根盘。
pub(crate) fn tmp(id: &str) -> String {
    format!("{}/tmp", module_dir(id))
}

/// 目的：上一次的源清单（guest 路径）——用来删掉"上次是源、这次没有了"的文件。
fn source_list(id: &str) -> String {
    format!("{}/src.list", module_dir(id))
}

/// 目的：构建前的文件快照 / 构建后的差异清单（guest 路径）。
fn before_list(id: &str) -> String {
    format!("{}/files.before", module_dir(id))
}
fn changed_list(id: &str) -> String {
    format!("{}/files.changed", module_dir(id))
}

/// 目的：挂载脚本——把缓存卷挂上并按环境标识对账（卷级只在整卷不可信时清空）。
/// 约束：没有文件系统就建；挂不上说明卷坏了——重建文件系统（缓存可以丢）再挂，仍挂不上就如实失败。
#[cfg(test)]
pub(crate) fn mount_script(volume_epoch: &str) -> String {
    mount_script_with(volume_epoch, 0, &[], &[])
}

/// 目的：带按龄 / 待清理列表的挂载脚本；模块目录用 USED 时间戳判断。
/// 参数：keep_secs = 保留秒数（0 = 不按龄）；keep = 本次构建的模块 id；purge = 已排队删除的模块 id。
pub(crate) fn mount_script_with(
    volume_epoch: &str,
    keep_secs: u64,
    keep: &[String],
    purge: &[String],
) -> String {
    let cutoff = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(keep_secs))
        .unwrap_or(0);
    let list = |ids: &[String]| {
        let mut out = String::from(" ");
        for id in ids {
            out.push_str(&safe_name(id));
            out.push(' ');
        }
        out
    };
    format!(
        r#"set -e
MNT={mnt}
DEV={dev}
KEEP={keep}
PURGE={purge}
CUTOFF={cutoff}
mkdir -p "$MNT"
if [ -z "$(blkid -o value -s TYPE "$DEV" 2>/dev/null)" ]; then mkfs.ext4 -q -F "$DEV" >/dev/null; fi
if ! mount -t ext4 "$DEV" "$MNT" 2>/dev/null; then
  mkfs.ext4 -q -F "$DEV" >/dev/null 2>&1 || {{ printf '构建缓存卷建不出文件系统：%s' "$DEV"; exit 1; }}
  mount -t ext4 "$DEV" "$MNT" || {{ printf '构建缓存卷挂不上：%s（怀疑卷损坏就在设置→存储空间清理该虚拟根缓存）' "$DEV"; exit 1; }}
fi
VE="$MNT/EPOCH"; WANT={volume}
if [ ! -f "$VE" ] || [ "$(cat "$VE")" != "$WANT" ]; then
  find "$MNT" -mindepth 1 -maxdepth 1 ! -name EPOCH -exec rm -rf {{}} + 2>/dev/null || true
  printf '%s' "$WANT" > "$VE"
fi
if [ -d "$MNT/modules" ]; then
  for d in "$MNT"/modules/*; do
    [ -d "$d" ] || continue
    n=$(basename "$d")
    case "$PURGE" in *" $n "*) rm -rf "$d"; continue;; esac
    [ "$CUTOFF" -gt 0 ] || continue
    u=$(cat "$d/USED" 2>/dev/null || echo 0)
    if [ "$u" -gt 0 ] && [ "$u" -lt "$CUTOFF" ]; then
      case "$KEEP" in *" $n "*) ;; *) rm -rf "$d";; esac
    fi
  done
fi
fstrim "$MNT" >/dev/null 2>&1 || true"#,
        mnt = sh_quote(MOUNT),
        dev = sh_quote(DEVICE),
        volume = sh_quote(volume_epoch),
        keep = sh_quote(&list(keep)),
        purge = sh_quote(&list(purge)),
        cutoff = cutoff,
    )
}

/// 目的：模块脚本——建好这个模块的工作区目录，按模块标识对账（换能力只清这个模块的树）。
pub(crate) fn module_script(id: &str, module_epoch: &str) -> String {
    format!(
        r#"set -e
MOD={module}; ME="$MOD/EPOCH"; WANTM={module_epoch}
mkdir -p "$MOD/home" "$MOD/tmp" "$MOD/ws"
date +%s > "$MOD/USED" 2>/dev/null || true
if [ ! -f "$ME" ] || [ "$(cat "$ME")" != "$WANTM" ]; then
  rm -rf "$MOD/ws"; mkdir -p "$MOD/ws"; printf '%s' "$WANTM" > "$ME"
fi"#,
        module = sh_quote(&module_dir(id)),
        module_epoch = sh_quote(module_epoch),
    )
}

/// 目的：源同步脚本——把共享盘上的模块树镜像进工作区（`userdata/` 不参与），并删掉被删掉的源。
/// 参数：`src` = 模块目录在 guest 里的共享路径；`entry` = 构建命令的相对入口（没有就不 chmod）。
/// 约束：源清单只记源文件；构建产物不在清单里，因此跨构建保留（增量构建靠它）。
pub(crate) fn sync_in_script(id: &str, src: &str, entry: Option<&str>) -> String {
    let ws = workspace(id);
    let list = source_list(id);
    let chmod = match entry {
        Some(e) => format!(
            "E={} ; if [ -f \"$WS/$E\" ]; then chmod 0755 \"$WS/$E\"; fi",
            sh_quote(e)
        ),
        None => String::new(),
    };
    format!(
        r#"set -e
SRC={src}
WS={ws}
LIST={list}
NEW="$LIST.new"
mkdir -p "$WS"
find "$SRC" -type f ! -path '*/userdata/*' -printf '%P\n' | sort > "$NEW"
if [ -f "$LIST" ]; then
  comm -13 "$NEW" "$LIST" | while IFS= read -r f; do if [ -n "$f" ]; then rm -f "$WS/$f"; fi; done
fi
tar -C "$SRC" --exclude=./userdata --exclude=./userdata/* -cf {tar_src} . || {{ printf '读不到模块源：%s' "$SRC"; exit 1; }}
tar -C "$WS" -xpf {tar_src}
rm -f {tar_src}
rm -rf "$WS/userdata"
mv "$NEW" "$LIST"
{chmod}"#,
        src = sh_quote(src),
        ws = sh_quote(&ws),
        list = sh_quote(&list),
        tar_src = sh_quote(&format!("{}/src.tar", module_dir(id))),
        chmod = chmod,
    )
}

/// 目的：构建前的文件快照（相对路径 + 大小 + mtime）——用来算"这次构建新增/修改了什么"。
pub(crate) fn snapshot_script(id: &str) -> String {
    format!(
        "find {} -type f -printf '%P\\t%s\\t%T@\\n' | sort > {}",
        sh_quote(&workspace(id)),
        sh_quote(&before_list(id)),
    )
}

/// 目的：回写脚本——本次新增/修改的文件 + 模块声明的入口，一起 tar 回共享盘上的模块目录。
/// 参数：`dst` = 模块目录在 guest 里的共享路径；`entries` = 入口（相对工作区）。
/// 约束：入口一律回写（宿主那份可能被手工删掉，差异里看不出来）；不镜像删除，失败不回写。
///   回写只保留权限位，不保留 guest 属主——共享区（virtiofs / 9p）以宿主用户提供服务，chown 到 root 必失败。
pub(crate) fn sync_out_script(id: &str, dst: &str, entries: &[String]) -> String {
    let ws = workspace(id);
    let before = before_list(id);
    let changed = changed_list(id);
    let mid = module_dir(id);
    let mut lines = String::new();
    for e in entries {
        lines.push_str(&format!(
            "if [ -f \"$WS/{e}\" ]; then printf '%s\\n' {e} >> \"$LIST\"; fi\n",
            e = sh_quote(e)
        ));
    }
    format!(
        r#"set -e
WS={ws}
BEFORE={before}
AFTER={mid}/files.after
LIST={changed}
TMP={mid}/out.tar
find "$WS" -type f -printf '%P\t%s\t%T@\n' | sort > "$AFTER"
awk -F'\t' 'NR==FNR{{o[$1]=$2"\t"$3;next}} {{ if (!($1 in o) || o[$1] != $2"\t"$3) print $1 }}' "$BEFORE" "$AFTER" > "$LIST"
{entries}sort -u "$LIST" -o "$LIST"
if [ -s "$LIST" ]; then
  tar -C "$WS" -cf "$TMP" -T "$LIST"
  # 产物在 guest 里属 root；共享后端以宿主用户提供服务，无法 chown 到 0。
  # 只保留权限位，属主交给共享后端（宿主用户）。
  tar -C {dst} --no-same-owner -xpf "$TMP"
  rm -f "$TMP"
fi
rm -f "$BEFORE" "$AFTER" "$LIST""#,
        ws = sh_quote(&ws),
        before = sh_quote(&before),
        mid = sh_quote(&mid),
        changed = sh_quote(&changed),
        entries = lines,
        dst = sh_quote(dst),
    )
}

/// 目的：收尾脚本——清掉临时目录，把空块还给宿主（`fstrim` 让 qcow2 的 discard 真正释放）。
pub(crate) fn cleanup_script(id: &str) -> String {
    format!(
        "rm -rf {}/* 2>/dev/null || true; fstrim {} >/dev/null 2>&1 || true",
        sh_quote(&tmp(id)),
        sh_quote(MOUNT),
    )
}

/// 目的：构建缓存卷旁的元数据落点（builds 写，storage 读；不参与判据）。
pub(crate) fn meta_path(root: &Path, guest: &str) -> PathBuf {
    dir(root).join(format!("{}.json", safe_name(guest)))
}

/// 目的：读已排队的按模块清理标记；执行成功后再删文件（挂载失败要能重试）。
pub(crate) fn read_purge(root: &Path, guest: &str) -> Vec<String> {
    let p = dir(root).join(format!("{}.purge", safe_name(guest)));
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 目的：清理已执行的按模块标记。
pub(crate) fn clear_purge(root: &Path, guest: &str) {
    let _ = std::fs::remove_file(dir(root).join(format!("{}.purge", safe_name(guest))));
}

/// 目的：写卷的模块占用元数据（只给界面与回收参考；失败不阻断构建）。
pub(crate) fn write_meta(
    root: &Path,
    guest: &str,
    modules: &std::collections::BTreeMap<String, (u64, u64)>,
) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mods: serde_json::Map<String, serde_json::Value> = modules
        .iter()
        .map(|(id, (bytes, used))| {
            (
                id.clone(),
                serde_json::json!({ "bytes": bytes, "last_used": used }),
            )
        })
        .collect();
    let text = serde_json::json!({
        "version": 1,
        "guest": safe_name(guest),
        "last_used": now,
        "modules": mods,
    });
    let p = meta_path(root, guest);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, text.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// 目的：构建批次结束后统计每个模块工作区（字节 + USED 时间戳），给宿主写元数据。
pub(crate) fn usage_script() -> String {
    format!(
        r#"for d in {mnt}/modules/*; do
  [ -d "$d" ] || continue
  n=$(basename "$d")
  b=$(du -sB1 "$d" 2>/dev/null | awk '{{print $1}}')
  t=$(cat "$d/USED" 2>/dev/null || echo 0)
  printf '%s	%s	%s
' "$n" "$b" "$t"
done"#,
        mnt = sh_quote(MOUNT),
    )
}

/// 目的：同一虚拟根的构建锁——占住它就独占这块缓存卷（获取失败如实拒绝，不并写）。
#[derive(Debug)]
pub(crate) struct VolumeLock {
    path: PathBuf,
}

impl VolumeLock {
    /// 目的：拿锁（有界等待）；上一把锁太旧视为死锁，清掉重来。
    pub(crate) fn acquire(path: PathBuf) -> Result<VolumeLock, String> {
        VolumeLock::acquire_with(path, LOCK_WAIT, LOCK_STALE)
    }

    /// 目的：拿锁的实现（等待与死锁阈值可注入，用例不必真等十分钟）。
    fn acquire_with(path: PathBuf, wait: Duration, stale: Duration) -> Result<VolumeLock, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("建构建缓存目录失败（{}）：{}", dir.display(), e))?;
        }
        let deadline = std::time::Instant::now() + wait;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = write!(f, "pid {} at {:?}", std::process::id(), SystemTime::now());
                    return Ok(VolumeLock { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let dead = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map(|t| SystemTime::now().duration_since(t).unwrap_or_default() > stale)
                        .unwrap_or(false);
                    if dead {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(format!(
                            "同一虚拟根的另一个构建还占着缓存卷（{}）：等不到就如实拒绝；确无构建在跑时删掉这个锁文件",
                            path.display()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => return Err(format!("建构建锁失败（{}）：{}", path.display(), e)),
            }
        }
    }
}

impl Drop for VolumeLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 目的：把一段字面量安全地放进 guest 的 shell 脚本里（单引号包裹，内部单引号按 POSIX 拆合）。
pub(crate) fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 目的：把名字收成一段安全的路径段（虚拟根名与模块 id 都只该是一段，这里再兜一层）。
fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn image() -> GuestImage {
        GuestImage::linux("debian-13", PathBuf::from("/img/root.qcow2"), None, None)
    }

    /// 目的：卷/锁落点在 `.vm/build-cache` 下，按虚拟根一格一块。
    #[test]
    fn volume_and_lock_live_under_build_cache() {
        let root = Path::new("/proj");
        assert_eq!(
            volume_path(root, "windows"),
            PathBuf::from("/proj/.vm/build-cache/windows.qcow2")
        );
        assert_eq!(
            lock_path(root, "windows"),
            PathBuf::from("/proj/.vm/build-cache/windows.lock")
        );
        assert_eq!(
            volume_path(root, "a/b"),
            PathBuf::from("/proj/.vm/build-cache/a_b.qcow2"),
            "名字里的分隔符不能带进路径"
        );
    }

    /// 目的：环境标识只看"环境"；模块标识只看构建期能力与提供者——换一个模块的能力不清别人的缓存。
    #[test]
    fn epochs_separate_environment_from_module_capabilities() {
        let mut img = image();
        img.runtimes = vec!["gcc".to_string()];
        let a = volume_epoch(&img);
        img.runtimes = vec!["gcc".to_string(), "node".to_string()];
        assert_ne!(a, volume_epoch(&img), "运行能力变了，卷就不该再用");
        let needs = vec!["gcc".to_string()];
        let providers = vec!["gcc=sys@gcc-13".to_string()];
        assert!(module_epoch(&needs, &providers).contains("gcc=sys@gcc-13"));
        assert_ne!(
            module_epoch(&needs, &providers),
            module_epoch(&needs, &[]),
            "提供者变了，模块工作区要清"
        );
    }

    /// 目的：挂载脚本要能自愈——没文件系统就建、挂不上重建；模块脚本按模块标识对账。
    #[test]
    fn mount_script_heals_and_module_script_reconciles() {
        let s = mount_script("v1|img|amd64");
        assert!(s.contains("mkfs.ext4 -q -F"), "{}", s);
        assert!(s.contains("mount -t ext4"), "{}", s);
        assert!(
            s.contains("挂不上") && s.contains("mkfs.ext4 -q -F \"$DEV\""),
            "挂不上要重建文件系统再挂：{}",
            s
        );
        assert!(s.contains("EPOCH"), "{}", s);
        let m = module_script("indexer", "v1|gcc|gcc=sys@gcc-13");
        assert!(
            m.contains("/var/lib/solomni/build/modules/indexer"),
            "{}",
            m
        );
        assert!(m.contains("rm -rf \"$MOD/ws\""), "{}", m);
        assert!(m.contains("USED"), "模块要记最近使用时间：{}", m);
    }

    /// 目的：挂载时按龄 / 排队列表清理模块工作区；批次结束的统计脚本给宿主元数据。
    #[test]
    fn mount_script_prunes_old_modules_and_usage_script_reports_them() {
        let s = mount_script_with("v1", 3600, &["keep".to_string()], &["gone".to_string()]);
        assert!(s.contains("PURGE=' gone '"), "{}", s);
        assert!(s.contains("KEEP=' keep '"), "{}", s);
        assert!(s.contains("CUTOFF="), "{}", s);
        assert!(s.contains("fstrim"), "{}", s);
        let u = usage_script();
        assert!(u.contains("du -sB1"), "{}", u);
        assert!(u.contains("USED"), "{}", u);
    }

    /// 目的：源同步排除 `userdata/`、按上次清单删掉被删掉的源，并给构建入口执行位。
    #[test]
    fn sync_in_mirrors_sources_and_keeps_outputs() {
        let s = sync_in_script("indexer", "/solomni-host/d/m", Some("tools/build"));
        assert!(s.contains("--exclude=./userdata"), "{}", s);
        assert!(s.contains("rm -rf \"$WS/userdata\""), "{}", s);
        assert!(s.contains("comm -13 \"$NEW\" \"$LIST\""), "{}", s);
        assert!(s.contains("chmod 0755 \"$WS/$E\""), "{}", s);
    }

    /// 目的：回写 = 差异 + 声明入口；入口不存在就不进清单（由复验去说缺什么）。
    #[test]
    fn sync_out_writes_changed_files_and_entries() {
        let s = sync_out_script(
            "indexer",
            "/solomni-host/d/m",
            &["build/indexer".to_string()],
        );
        assert!(s.contains("awk -F"), "{}", s);
        assert!(s.contains("[ -f \"$WS/'build/indexer'\" ]"), "{}", s);
        assert!(s.contains("sort -u \"$LIST\" -o \"$LIST\""), "{}", s);
        assert!(
            s.contains("tar -C '/solomni-host/d/m' --no-same-owner -xpf"),
            "回写不能保留 guest 属主（共享区无法 chown 到 root）：{}",
            s
        );
    }

    /// 目的：收尾要清临时目录并归还空块。
    #[test]
    fn cleanup_trims_the_volume() {
        let s = cleanup_script("indexer");
        assert!(s.contains("fstrim"), "{}", s);
        assert!(s.contains("/tmp"), "{}", s);
    }

    /// 目的：锁是独占的——第二把拿不到；上一把太旧视为死锁，清掉后能拿到。
    #[test]
    fn lock_is_exclusive_and_heals_stale_files() {
        let root = crate::tests::scratch("build-lock");
        let path = root.join("v.lock");
        let held = VolumeLock::acquire_with(
            path.clone(),
            Duration::from_millis(50),
            Duration::from_secs(3600),
        )
        .expect("第一把要拿得到");
        let err = VolumeLock::acquire_with(
            path.clone(),
            Duration::from_millis(50),
            Duration::from_secs(3600),
        )
        .expect_err("第二把要拿不到");
        assert!(err.contains("占着缓存卷"), "{}", err);
        drop(held);
        assert!(
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .is_ok(),
            "放一个旧锁"
        );
        let healed = VolumeLock::acquire_with(
            path.clone(),
            Duration::from_millis(50),
            Duration::from_secs(0),
        );
        assert!(healed.is_ok(), "过旧的锁要能自愈");
        drop(healed);
        let _ = std::fs::remove_dir_all(&root);
    }
}
