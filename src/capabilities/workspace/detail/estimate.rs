//! 目的：外部工作区的授权成本预估——有界递归计数 + 按卷速率换成耗时区间，只读、不写 ACL。
//! 管：`.home/fence-costs.json` 的样本读写，以及 `/api/workspace-cost` 要的那组数字；预算 250 ms / 5 万条目。
//! 不管：真写 ACL（在 kernel 围栏，由后台一次性授权调用）；页面呈现（前端按区间分档显示）。
//! 联动：`presentation/web.rs` 的路由读预估；后台授权成功后经 `record_grant_cost` 回写真实样本。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 目的：预估的墙钟预算——超了立即停，结果标 truncated（偏小但保证请求不被大目录拖死）。
const BUDGET: Duration = Duration::from_millis(250);
/// 目的：条目上限（文件 + 目录），与墙钟预算任一越界即停。
const MAX_ENTRIES: u64 = 50_000;
/// 目的：没有样本时的缺省速率（毫秒/对象）；对 SSD 上可继承 ACE 的本机实测取整偏保守。
const DEFAULT_MS_PER_OBJECT: f64 = 0.7;
/// 目的：每个卷最多留这么多样本（取最近的），成本文件不会无限长。
const MAX_SAMPLES: usize = 8;
/// 目的：成本样本文件名（落在 `.home/` 下）。
const COSTS_FILE: &str = "fence-costs.json";

/// 目的：`GET /api/workspace-cost` 的响应体；数字口径见各字段注释，单位毫秒。
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceCost {
    /// 目的：规范化后的工作区根（canonicalize 之后）。
    pub path: String,
    /// 目的：文件个数（不含根；符号链接不跟进、只记一个对象）。
    pub files: u64,
    /// 目的：子目录个数（不含根）。
    pub dirs: u64,
    /// 目的：会被 ACL 写波及的对象数 = files + dirs（真正的单价乘数）。
    pub objects: u64,
    /// 目的：文件字节数合计（只做提示，不参与耗时估算）。
    pub bytes: u64,
    /// 目的：是否因预算（墙钟 / 条目）提前停；true = 以下数字是下界。
    pub truncated: bool,
    /// 目的：速率所属的卷（Windows 形如 `D:`）；非 Windows 或无法识别时为 null。
    pub volume: Option<String>,
    /// 目的：这次估算用的速率（毫秒/对象）。
    pub ms_per_object: Option<f64>,
    /// 目的：点值耗时 = objects × ms_per_object。
    pub estimated_ms: Option<u64>,
    /// 目的：区间下界（点值 × 0.5），给用户看的"最乐观"。
    pub estimated_ms_min: Option<u64>,
    /// 目的：区间上界（点值 × 2.0），给用户看的"最坏"。
    pub estimated_ms_max: Option<u64>,
    /// 目的：速率来源：`measured` = 本机本卷有真实授权样本；`default` = 缺省值。
    pub sample: String,
}

/// 目的：`.home/fence-costs.json` 的形状：按卷存最近样本与聚合速率。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CostBook {
    #[serde(default)]
    volumes: BTreeMap<String, VolumeCost>,
}

/// 目的：一个卷的成本事实；`ms_per_object` 由样本按对象数加权平均得出。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct VolumeCost {
    #[serde(default)]
    ms_per_object: f64,
    #[serde(default)]
    samples: Vec<CostSample>,
    #[serde(default)]
    at: u64,
}

/// 目的：一次真实授权的样本（授权完成时回写），用于把速率钉在本机本卷上。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CostSample {
    at: u64,
    path: String,
    objects: u64,
    ms: u64,
}

/// 目的：估一个外部工作区「一次性授权」要多久：规范化 → 有界计数 → 按卷速率换算区间。
/// 约束：只读；不做权限判定、不写任何文件（成本样本由 `record_grant_cost` 单独写）。
pub fn estimate_workspace_cost(root: &Path, home: &Path) -> Result<WorkspaceCost, String> {
    if !root.is_absolute() {
        return Err(format!("路径必须是绝对路径：{}", root.display()));
    }
    if is_network_share(root) {
        return Err("不支持网络共享路径（UNC）：请映射成盘符或选本机目录".to_string());
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("工作区不可达（{}）：{}", root.display(), e))?;
    let meta = std::fs::metadata(&root)
        .map_err(|e| format!("工作区不可达（{}）：{}", root.display(), e))?;
    if !meta.is_dir() {
        return Err(format!("工作区不是目录：{}", root.display()));
    }
    let (files, dirs, bytes, truncated) = walk(&root, MAX_ENTRIES, BUDGET)?;
    let volume = volume_of(&root);
    let book = load_costs(home);
    let (rate, sample) = rate_for(&book, volume.as_deref());
    let objects = files + dirs;
    let point = (rate * objects as f64).round() as u64;
    let point_f = point as f64;
    Ok(WorkspaceCost {
        path: root.display().to_string(),
        files,
        dirs,
        objects,
        bytes,
        truncated,
        volume,
        ms_per_object: Some(rate),
        estimated_ms: Some(point),
        estimated_ms_min: Some((point_f * 0.5).round() as u64),
        estimated_ms_max: Some((point_f * 2.0).round() as u64),
        sample: sample.to_string(),
    })
}

/// 目的：把一次真实机器级授权的（对象数, 耗时）记进本卷样本，并刷新加权速率。
/// 约束：尽力而为——写失败不影响授权本身；非 Windows（识别不出卷）静默跳过。
pub fn record_grant_cost(home: &Path, root: &Path, objects: u64, ms: u64) {
    if objects == 0 {
        return;
    }
    let Some(volume) = volume_of(root) else {
        return;
    };
    let mut book = load_costs(home);
    let entry = book.volumes.entry(volume).or_default();
    entry.samples.push(CostSample {
        at: now_unix(),
        path: root.display().to_string(),
        objects,
        ms,
    });
    if entry.samples.len() > MAX_SAMPLES {
        let drop = entry.samples.len() - MAX_SAMPLES;
        entry.samples.drain(0..drop);
    }
    let total_ms: u64 = entry.samples.iter().map(|s| s.ms).sum();
    let total_objects: u64 = entry.samples.iter().map(|s| s.objects).sum();
    if total_objects > 0 {
        entry.ms_per_object = total_ms as f64 / total_objects as f64;
    }
    entry.at = now_unix();
    save_costs(home, &book);
}

/// 目的：有界递归计数（文件 / 子目录 / 字节）；符号链接只记一个对象、不跟进，避免环。
/// 约束：根读不开 = Err；子目录读不开 = truncated（数字是下界，不假装完整）。
fn walk(root: &Path, max_entries: u64, budget: Duration) -> Result<(u64, u64, u64, bool), String> {
    let started = Instant::now();
    let mut stack = vec![root.to_path_buf()];
    let (mut files, mut dirs, mut bytes) = (0u64, 0u64, 0u64);
    let mut truncated = false;
    while let Some(dir) = stack.pop() {
        if truncated || started.elapsed() > budget {
            truncated = true;
            break;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if dir == root => {
                return Err(format!("读目录失败（{}）：{}", dir.display(), e));
            }
            Err(_) => {
                truncated = true;
                continue;
            }
        };
        for entry in rd.flatten() {
            if files + dirs >= max_entries || started.elapsed() > budget {
                truncated = true;
                break;
            }
            let Ok(kind) = entry.file_type() else {
                truncated = true;
                continue;
            };
            if kind.is_dir() {
                dirs += 1;
                stack.push(entry.path());
            } else {
                files += 1;
                if kind.is_file()
                    && let Ok(md) = entry.metadata()
                {
                    bytes += md.len();
                }
            }
        }
    }
    Ok((files, dirs, bytes, truncated))
}

/// 目的：按卷取速率与来源；该卷没有样本（或速率为 0）时用缺省值。
fn rate_for<'a>(book: &'a CostBook, volume: Option<&str>) -> (f64, &'a str) {
    let hit = volume
        .and_then(|v| book.volumes.get(v))
        .filter(|c| c.ms_per_object > 0.0);
    match hit {
        Some(c) => (c.ms_per_object, "measured"),
        None => (DEFAULT_MS_PER_OBJECT, "default"),
    }
}

/// 目的：读成本样本；文件缺失或坏掉一律当空账（预估不能因它失败）。
fn load_costs(home: &Path) -> CostBook {
    let Ok(text) = std::fs::read_to_string(home.join(COSTS_FILE)) else {
        return CostBook::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// 目的：写回成本样本；失败忽略（授权事实已落 `workspace-grants.json`，这里只是速率缓存）。
fn save_costs(home: &Path, book: &CostBook) {
    if std::fs::create_dir_all(home).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(book) {
        let _ = std::fs::write(home.join(COSTS_FILE), text);
    }
}

/// 目的：判网络共享（UNC / VerbatimUNC）——围栏对 UNC 工作区没有机器级只读语义，直接拒绝。
/// 约束：按路径前缀判定，不能先 canonicalize（扩展路径 `\\?\C:\` 会被误判成 UNC）。
fn is_network_share(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(
        path.components().next(),
        Some(Component::Prefix(p))
            if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    )
}

/// 目的：速率按卷分组——Windows 取盘符前缀（D:），其它平台没有这个维度。
/// 约束：canonicalize 之后的扩展写法（\\?\D:\）要归一成 D:，否则同一个卷会记成两个键。
fn volume_of(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        use std::path::Component;
        match path.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                std::path::Prefix::VerbatimDisk(d) => Some(format!("{}:", d as char)),
                _ => Some(p.as_os_str().to_string_lossy().into_owned()),
            },
            _ => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// 目的：样本时间戳（unix 秒）；只用于成本文件的"最近样本"排序与人工排查。
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：造一棵一次性夹具树；返回 (根, 子目录名) 与整体清理句柄由调用方负责。
    fn fixture(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "solomni-cost-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).expect("建夹具");
        std::fs::write(root.join("a.txt"), b"aa").expect("写夹具");
        std::fs::write(root.join("b.txt"), b"bbb").expect("写夹具");
        std::fs::write(root.join("sub").join("c.txt"), b"c").expect("写夹具");
        root
    }

    #[test]
    fn counts_files_dirs_and_bytes_under_the_budget() {
        let root = fixture("count");
        let home = std::env::temp_dir().join(format!("solomni-cost-home-{}", std::process::id()));
        let cost = estimate_workspace_cost(&root, &home).expect("应能估");
        assert_eq!(cost.files, 3, "{:?}", cost);
        assert_eq!(cost.dirs, 1, "{:?}", cost);
        assert_eq!(cost.objects, 4, "{:?}", cost);
        assert_eq!(cost.bytes, 6, "{:?}", cost);
        assert!(!cost.truncated, "{:?}", cost);
        assert_eq!(cost.sample, "default", "{:?}", cost);
        assert_eq!(cost.ms_per_object, Some(DEFAULT_MS_PER_OBJECT));
        let point = (DEFAULT_MS_PER_OBJECT * 4.0).round() as u64;
        assert_eq!(cost.estimated_ms, Some(point));
        assert_eq!(
            cost.estimated_ms_min,
            Some((point as f64 * 0.5).round() as u64)
        );
        assert_eq!(
            cost.estimated_ms_max,
            Some((point as f64 * 2.0).round() as u64)
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn walk_stops_at_the_entry_budget() {
        let root = fixture("budget");
        let (files, dirs, _, truncated) = walk(&root, 2, Duration::from_secs(30)).expect("应能走");
        assert!(truncated, "条目预算越界要标 truncated");
        assert!(files + dirs <= 3, "最多越过一条：{} + {}", files, dirs);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_relative_missing_and_file_paths() {
        let home = std::env::temp_dir();
        assert!(
            estimate_workspace_cost(Path::new("相对路径"), &home)
                .unwrap_err()
                .contains("绝对路径")
        );
        let missing = std::env::temp_dir().join("solomni-cost-missing-does-not-exist");
        assert!(
            estimate_workspace_cost(&missing, &home)
                .unwrap_err()
                .contains("不可达")
        );
        let root = fixture("file-path");
        let file = root.join("a.txt");
        assert!(
            estimate_workspace_cost(&file, &home)
                .unwrap_err()
                .contains("不是目录")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn measured_samples_drive_the_estimate() {
        let root = fixture("measured");
        let home = std::env::temp_dir().join(format!("solomni-cost-home2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        record_grant_cost(&home, &root, 100, 100);
        record_grant_cost(&home, &root, 100, 300);
        let cost = estimate_workspace_cost(&root, &home).expect("应能估");
        assert_eq!(cost.sample, "measured", "{:?}", cost);
        assert_eq!(cost.ms_per_object, Some(2.0), "按对象数加权：{:?}", cost);
        assert_eq!(cost.estimated_ms, Some(8), "{:?}", cost);
        // 样本上限：写过 8 条后只留最近 8 条。
        for i in 0..10 {
            record_grant_cost(&home, &root, 10, i);
        }
        let text = std::fs::read_to_string(home.join(COSTS_FILE)).expect("成本文件");
        let book: CostBook = serde_json::from_str(&text).expect("形状");
        assert_eq!(book.volumes.len(), 1, "只记本卷");
        for vol in book.volumes.values() {
            assert_eq!(vol.samples.len(), MAX_SAMPLES);
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&home);
    }
}
