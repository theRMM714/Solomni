//! 目的：provision 的单元测试——清单语义、台账、取数续传、安装落盘与 guest 镜像构造。
//! 管：service 的编排与 domain 规则、detail 的纯函数（解包、选包、属性、调试工具）。
//! 不管：真实网络下载与 ext4 工具链的端到端（那些在集成用例里）。
//! 联动：被测实现是 capabilities::provision；夹具用 crate::tests::scratch 铺临时目录。

mod service {
    use crate::capabilities::provision::api::{JobState, Provision};
    use crate::capabilities::provision::detail::fetch::{
        choose_fastest, download_with_resume, mirror_of, part_path, part_source_marker, HttpFetch,
    };
    use crate::capabilities::provision::detail::guest_image::*;
    use crate::capabilities::provision::detail::host::RealHost;
    use crate::capabilities::provision::detail::install::HostInstallers;
    use crate::capabilities::provision::detail::ledger::{
        probe_present, read_ledger, record_installed, FileLedger,
    };
    use crate::capabilities::provision::detail::procs::BookProcs;
    use crate::capabilities::provision::domain::*;
    use crate::capabilities::provision::ports::Fetcher;
    use crate::capabilities::provision::service::*;
    use crate::capabilities::provision::service::{
        run as run_pipeline, verify_cached as verify_cached_pinned,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// 目的：测试里用一个真实取数实现（不联网的用例只用它的摘要与路径函数）。
    fn test_fetch() -> Arc<dyn Fetcher> {
        Arc::new(HttpFetch::new(15, 7200))
    }

    /// 目的：一套真实端口实现的搬运包——测试构造服务时少写几行类型。
    fn adapters(root: &Path) -> ProvisionDeps {
        let fetch = test_fetch();
        ProvisionDeps {
            host: Arc::new(RealHost),
            ledger: Arc::new(FileLedger::new(root)),
            installers: Arc::new(HostInstallers),
            images: Arc::new(
                crate::capabilities::provision::detail::guest_image::Ext4Images::new(
                    root,
                    Arc::clone(&fetch),
                ),
            ),
            procs: Arc::new(BookProcs::new(root)),
            fetch,
            probe: Arc::new(crate::kernel::detail::host_probe::HostProbeAdapter),
        }
    }

    /// 目的：测试用的服务构造——端口用真实实现，免得每条用例都写一遍装配。
    fn service(root: &Path) -> ProvisionService {
        ProvisionService::new(root, adapters(root))
    }

    /// 目的：遮蔽被测的 run，用例不必关心端口参数（真实实现即可）。
    fn run(
        root: &Path,
        all: &[Dep],
        links: &BTreeMap<String, String>,
        ids: &[String],
        cancel: &AtomicBool,
        on_line: &mut dyn FnMut(&str),
        vm: Option<&(dyn crate::capabilities::vm::api::Vm + Send + Sync)>,
    ) -> Result<(), String> {
        let deps = adapters(root);
        let pipeline = Pipeline {
            root,
            host: deps.host.as_ref(),
            ledger: deps.ledger.as_ref(),
            installers: deps.installers.as_ref(),
            images: deps.images.as_ref(),
            fetch: deps.fetch.as_ref(),
            probe: deps.probe.as_ref(),
            vm,
        };
        run_pipeline(&pipeline, all, links, ids, cancel, on_line)
    }

    fn verify_cached(
        id: &str,
        pinned: Option<&str>,
        cached: &Path,
        url: &str,
    ) -> Result<(), String> {
        let fetch = HttpFetch::new(15, 7200);
        verify_cached_pinned(id, pinned, cached, url, &fetch)
    }

    fn sha256_file(path: &Path) -> Result<String, String> {
        crate::capabilities::provision::detail::fetch::sha256_file(path)
    }

    /// 目的：造一条任意 kind 的清单条目（组别名/缺项判定的用例夹具）。
    fn dep_of(id: &str, kind: &str, install: &str, probe: &str) -> Dep {
        Dep {
            id: id.to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.to_string(),
            probe: probe.to_string(),
            source: "release:x".to_string(),
            kind: kind.to_string(),
            guest: None,
            toolchain: None,
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        }
    }

    /// 目的：`--ensure` 取值的两种解释——全为 kind 时按组别名，否则按 id 列表；混写如实拒绝。
    #[test]
    fn ensure_spec_splits_kinds_from_ids_and_refuses_mixed() {
        let all = [
            dep_of("toolchain.python.x", "toolchain", ".t/py", ".t/py/bin"),
            dep_of("vm.qemu.x", "archive", ".t/q", ".t/q/bin"),
        ];
        assert_eq!(
            split_ensure_spec(&all, "toolchain").expect("组别名"),
            EnsureSpec::Kinds(vec!["toolchain".to_string()])
        );
        assert_eq!(
            split_ensure_spec(&all, "vm.qemu.x,vm.guest.x").expect("id 列表"),
            EnsureSpec::Ids(vec!["vm.qemu.x".to_string(), "vm.guest.x".to_string()])
        );
        let err = split_ensure_spec(&all, "toolchain,vm.qemu.x").expect_err("混写要拒绝");
        assert!(err.contains("不能混写"), "{}", err);
    }

    /// 目的：组别名只挑「不在场」的条目——在位的不重装（260 MB 的包不重解）。
    #[test]
    fn pending_of_a_kind_lists_only_missing_entries() {
        let root = crate::tests::scratch("provision-kind-pending");
        let present = dep_of("toolchain.python.x", "toolchain", ".t/py", ".t/py/bin");
        let missing = dep_of("toolchain.node.x", "toolchain", ".t/node", ".t/node/bin");
        let other = dep_of("vm.qemu.x", "archive", ".t/q", ".t/q/bin");
        fs::create_dir_all(under(&root, ".t/py/bin")).expect("铺在位工具");
        record_installed(&root, &present).expect("台账");
        let all = [present, missing, other];
        assert_eq!(
            pending_of_kinds(&all, &FileLedger::new(&root), &["toolchain".to_string()]),
            vec!["toolchain.node.x".to_string()]
        );
        assert_eq!(
            pending_of_kinds(&all, &FileLedger::new(&root), &["archive".to_string()]),
            vec!["vm.qemu.x".to_string()]
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：安装身份跟着清单走——pin / 载荷变了身份就变，没变就稳定。
    #[test]
    fn install_identity_follows_the_manifest() {
        let dep = dep_of("vm.qemu.x", "archive", ".t/q", ".t/q/bin");
        let base = install_identity(&dep);
        assert_eq!(base, install_identity(&dep), "同一份清单身份稳定");
        let mut pinned = dep.clone();
        pinned.sha256 = "a".repeat(64);
        assert_ne!(base, install_identity(&pinned), "pin 变了身份要变");
        let mut resized = dep.clone();
        resized.size_mb = 999;
        assert_ne!(base, install_identity(&resized), "体积变了身份要变");
    }

    /// 目的：探针在场但台账里没有记录 = 还没按当前清单装好（升级后第一次跑要重装）。
    #[test]
    fn a_probe_without_a_ledger_entry_is_not_installed() {
        let root = crate::tests::scratch("provision-ledger-missing");
        let dep = dep_of("vm.qemu.x", "archive", ".t/q", ".t/q/bin");
        fs::create_dir_all(under(&root, ".t/q/bin")).expect("铺 probe");
        assert!(probe_present(&root, &dep), "探针确实在场");
        assert!(
            !installed(&FileLedger::new(&root), &read_ledger(&root), &dep),
            "没台账不算装好"
        );
        record_installed(&root, &dep).expect("台账");
        assert!(
            installed(&FileLedger::new(&root), &read_ledger(&root), &dep),
            "登记后算装好"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：台账里记的是旧身份（pin 升级）时，探针在场也要算缺项。
    #[test]
    fn a_changed_pin_makes_the_old_install_missing() {
        let root = crate::tests::scratch("provision-ledger-stale");
        let mut dep = dep_of("vm.qemu.x", "archive", ".t/q", ".t/q/bin");
        dep.sha256 = "a".repeat(64);
        fs::create_dir_all(under(&root, ".t/q/bin")).expect("铺 probe");
        record_installed(&root, &dep).expect("按旧身份登记");
        assert!(installed(
            &FileLedger::new(&root),
            &read_ledger(&root),
            &dep
        ));
        dep.sha256 = "b".repeat(64);
        assert!(
            !installed(&FileLedger::new(&root), &read_ledger(&root), &dep),
            "换了 pin 就是旧装"
        );
        assert_eq!(
            pending_of_kinds(
                &[dep.clone()],
                &FileLedger::new(&root),
                &["archive".to_string()]
            ),
            vec!["vm.qemu.x".to_string()]
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：`--ensure=toolchain` 在本机工具链都在场时是一条**不联网**的幂等命令（启动提示就靠它）。
    /// 约束：按当前宿主清单铺出 probe（缺当前平台条目就如实跳过）；不下载、不写安装目录。
    #[test]
    fn ensure_spec_group_is_a_noop_when_every_toolchain_is_present() {
        let root = crate::tests::scratch("provision-ensure-group");
        let all = for_host(
            &deps().expect("清单"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        let toolchains: Vec<&Dep> = all.iter().filter(|d| d.kind == "toolchain").collect();
        if toolchains.is_empty() {
            eprintln!("[探针] 当前平台没有工具链条目，跳过");
            return;
        }
        for dep in &toolchains {
            fs::create_dir_all(under(&root, &dep.probe)).expect("铺 probe");
            record_installed(&root, dep).expect("台账");
        }
        let svc = service(&root);
        let mut lines: Vec<String> = Vec::new();
        svc.ensure_spec("toolchain", &mut |l| lines.push(l.to_string()))
            .expect("都在场时不该碰网络");
        assert!(lines.iter().any(|l| l.contains("没有缺项")), "{:?}", lines);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：sha256 计算与已知向量一致（完整性闸的第一环：算法本身不能错）。
    #[test]
    fn sha256_matches_a_known_vector() {
        let dir = crate::tests::scratch("provision-sha256");
        let f = dir.join("abc.bin");
        fs::write(&f, b"abc").expect("写样本");
        assert_eq!(
            sha256_file(&f).expect("算 sha256"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 目的：缓存文件的校验按台账来——pin 对得上才放行；不符时**报出台账值、实际值与来源**（不删文件、不循环）；
    ///   没 pin 就如实不校验（`latest` 源 pin 上去必然腐烂）。
    #[test]
    fn cached_file_is_checked_against_the_pinned_hash() {
        let dir = crate::tests::scratch("provision-verify");
        let f = dir.join("asset.bin");
        fs::write(&f, b"abc").expect("写样本");
        let real = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let url = "https://example.test/asset.bin";
        // 大小写不敏感：台账里手抄的大写也要能对上。
        verify_cached("vm.demo", Some(&real.to_uppercase()), &f, url).expect("pin 对得上要放行");
        // 没 pin：不校验（否则 latest 源没法用）。
        verify_cached("vm.demo", None, &f, url).expect("没 pin 就不校验");
        // 不符：报台账值 / 实际值 / 来源，并且**不许删缓存**（删了就是重下死循环）。
        let bad = verify_cached("vm.demo", Some(&"0".repeat(64)), &f, url).expect_err("不符要拒绝");
        assert!(bad.contains(&"0".repeat(64)), "要报台账值：{}", bad);
        assert!(bad.contains(real), "要报实际值：{}", bad);
        assert!(bad.contains(url), "要报来源：{}", bad);
        assert!(
            f.is_file(),
            "不许静默删缓存（pin 过期时那会变成重下死循环）"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 目的：pin 的来源二选一——清单条目的 `sha256` 优先，链接类源回落到 `links.yaml` 的 `<source>.sha256`。
    #[test]
    fn pin_comes_from_the_manifest_entry_or_the_ledger_pair() {
        let from_ledger = "D5DB".repeat(16);
        let links = BTreeMap::from([(
            "smbd.windows.sha256".to_string(),
            format!("  {}  ", from_ledger),
        )]);
        let mut dep = Dep {
            id: "x".to_string(),
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
            install: ".vm/x".to_string(),
            probe: ".vm/x/x.exe".to_string(),
            source: "smbd.windows".to_string(),
            kind: "file".to_string(),
            guest: None,
            family: None,
            toolchain: None,
            strip: 0,
            size_mb: 1,
            sha256: String::new(),
            note: String::new(),
        };
        assert_eq!(
            pinned_sha256(&dep, &links).as_deref(),
            Some(from_ledger.to_ascii_lowercase().as_str()),
            "链接类源回落到台账里成对的那条（去掉首尾空白、转小写）"
        );
        let own = "E96F".repeat(16);
        dep.sha256 = own.clone();
        assert_eq!(
            pinned_sha256(&dep, &links).as_deref(),
            Some(own.to_ascii_lowercase().as_str()),
            "清单条目自己的 pin 优先"
        );
        dep.source = "guest.debian.image".to_string();
        dep.sha256 = String::new();
        assert_eq!(pinned_sha256(&dep, &links), None, "都没 pin = 不校验");
    }

    /// 目的：目录页里按版本挑最新 .deb（数字感知，不受字典序影响）。
    #[test]
    fn picks_newest_deb_numerically() {
        let listing = r#"<a href="linux-image-6.9.0-amd64-unsigned_6.9.0-1_amd64.deb">x</a>
<a href="linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb">y</a>"#;
        let got = pick_deb(listing, "linux-image-", "-amd64-unsigned_").expect("有候选");
        assert_eq!(
            got,
            "linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb"
        );
    }

    /// 目的：Debian 配方带齐 initrd 的配置 / 脚本 / 产物；脚本先固定 guest PATH，
    ///   宿主 PATH 缺 /usr/sbin 时也能找到 depmod / mkinitramfs。
    #[test]
    fn debian_family_carries_the_initrd_recipe() {
        let family = guest_family("debian").expect("debian 要有配方");
        assert_eq!(family.initrd_conf, "etc/initramfs-tools/modules");
        let conf = (family.initrd_conf_text)(&["9p".to_string(), "virtio".to_string()]);
        assert_eq!(conf, "9p\nvirtio\n");
        let script = (family.initrd_script)("6.12.0");
        assert!(
            script.contains("export PATH=/usr/sbin:/usr/bin:/sbin:/bin"),
            "{script}"
        );
        assert!(script.contains("depmod 6.12.0"), "{script}");
        assert!(
            script.contains("mkinitramfs -o /boot/initrd.img-6.12.0 6.12.0"),
            "{script}"
        );
        assert_eq!((family.initrd_artifact)("6.12.0"), "boot/initrd.img-6.12.0");
    }

    /// 目的：模块清单按原顺序合并、去空行、不重复；已有的条目不被改写。
    #[test]
    fn initrd_modules_are_merged_without_duplicates() {
        let got = merge_initrd_modules(" foo \n\n9p\n", &["9p", "virtio"]);
        assert_eq!(got, vec!["foo", "9p", "virtio"]);
    }

    /// 目的：清单没声明或声明了不认识的族都要如实拒绝，并指出预构造镜像的出路；不猜、不回落 Debian。
    #[test]
    fn unknown_family_is_refused_with_a_prebuilt_image_way_out() {
        let mut dep = Dep {
            id: "vm.guest.linux".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: ".vm/guest/debian/rootfs.ext4".to_string(),
            source: "guest.debian.image".to_string(),
            kind: "guest-rootfs".to_string(),
            guest: Some("debian".to_string()),
            family: Some("alpine".to_string()),
            toolchain: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let err = family_of(&dep).expect_err("没有 alpine 配方要拒绝");
        assert!(err.contains("alpine"), "{}", err);
        assert!(err.contains("预构造镜像"), "{}", err);
        dep.family = None;
        let err = family_of(&dep).expect_err("缺 family 也要拒绝");
        assert!(err.contains("family"), "{}", err);
        assert!(err.contains("预构造镜像"), "{}", err);
    }

    /// 目的：失败信息带退出码与子进程输出尾部，不改写成因；没有输出时也如实说清。
    #[test]
    fn initrd_failure_reports_the_real_output() {
        let err = initrd_failure(
            Some(127),
            b"/bin/bash: line 3: depmod: command not found\n",
            b"",
        );
        assert!(err.contains("退出码 127"), "{}", err);
        assert!(err.contains("depmod: command not found"), "{}", err);
        assert!(
            !err.contains("命名空间"),
            "不许把 PATH 问题推给内核：{}",
            err
        );
        let quiet = initrd_failure(None, b"", b"");
        assert!(quiet.contains("被信号终止"), "{}", quiet);
        assert!(quiet.contains("无输出"), "{}", quiet);
    }

    /// 目的：清单里的 Debian 虚拟根必须写明 family——构造配方只按显式声明取，不靠 id 或目录名猜。
    #[test]
    fn manifest_declares_the_debian_family() {
        let all = deps().expect("清单可解析");
        let dep = all
            .iter()
            .find(|d| d.id == "vm.guest.linux")
            .expect("有 Linux 虚拟根条目");
        assert_eq!(dep.family.as_deref(), Some("debian"));
    }

    /// 目的：单文件安装按 probe 的文件名落进安装目录。
    /// 目的：内核 ABI 必须带 9p 模块才算数——cloud 内核没有（`CONFIG_NET_9P` 没编），不能拿它当 9p 根的内核。
    #[test]
    fn only_a_kernel_with_9p_modules_counts() {
        let root = crate::tests::scratch("provision-kernel-9p");
        let base = root.join("base");
        let make = |abi: &str, with_9p: bool| {
            let dir = base
                .join("lib")
                .join("modules")
                .join(abi)
                .join("kernel")
                .join("net")
                .join("9p");
            fs::create_dir_all(&dir).expect("建模块目录");
            if with_9p {
                fs::write(dir.join("9pnet_virtio.ko.xz"), b"x").expect("写模块");
            }
        };
        make("7.0.0+deb13-cloud-amd64", false);
        make("6.12.111+deb13-amd64", true);
        assert!(
            !abi_has_9p(&base, "7.0.0+deb13-cloud-amd64"),
            "没有 9p 模块要判否"
        );
        assert!(
            abi_has_9p(&base, "6.12.111+deb13-amd64"),
            "带 9p 模块要判是"
        );
        // 已装的 ABI 里挑"能起 9p 根"的那个：`-rt-` 与 `-cloud-` 都不要。
        fs::create_dir_all(base.join("boot")).expect("建 boot");
        make("6.12.111+deb13-rt-amd64", true);
        fs::write(
            base.join("boot").join("vmlinuz-6.12.111+deb13-cloud-amd64"),
            b"k",
        )
        .expect("写内核");
        fs::write(
            base.join("boot").join("vmlinuz-6.12.111+deb13-rt-amd64"),
            b"k",
        )
        .expect("写内核");
        assert_eq!(usable_guest_abi(&base), None, "只有 cloud / rt 时不算可用");
        fs::write(base.join("boot").join("vmlinuz-6.12.111+deb13-amd64"), b"k").expect("写内核");
        assert_eq!(
            usable_guest_abi(&base).as_deref(),
            Some("6.12.111+deb13-amd64")
        );
        // 换内核要把两个稳定软链一起换指（否则 vmlinuz 还指着没 9p 的那个）；软链是 unix 概念。
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                "boot/vmlinuz-6.12.111+deb13-cloud-amd64",
                base.join("vmlinuz"),
            )
            .expect("先指 cloud");
            link_guest_kernel(&base, "6.12.111+deb13-amd64").expect("换指");
            assert_eq!(
                fs::read_link(base.join("vmlinuz")).expect("读链"),
                Path::new("boot/vmlinuz-6.12.111+deb13-amd64")
            );
        }
        // 没有软链的平台：换指如实报错，不假装成功。
        #[cfg(not(unix))]
        assert!(
            link_guest_kernel(&base, "6.12.111+deb13-amd64").is_err(),
            "不支持软链的平台要如实报错"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：内核包名要能取出 ABI，包的选择要排除 `-rt-` / `-cloud-`（它们起不了 / 不是我们要的 9p 根）。
    #[test]
    fn kernel_pick_avoids_rt_and_cloud_flavors() {
        let listing = r#"<a href="linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb">a</a>
<a href="linux-image-6.12.111+deb13-rt-amd64-unsigned_6.12.111-1_amd64.deb">b</a>
<a href="linux-image-6.12.111+deb13-cloud-amd64-unsigned_6.12.111-1_amd64.deb">c</a>
<a href="linux-image-7.2.11+deb14-amd64-unsigned_7.2.11-1_amd64.deb">d</a>"#;
        assert_eq!(
            pick_kernel_deb(listing, Some("13")).as_deref(),
            Some("linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb"),
            "同代内核里挑普通那个"
        );
        let only_bad = r#"<a href="linux-image-6.12.111+deb13-rt-amd64-unsigned_6.12.111-1_amd64.deb">b</a>
<a href="linux-image-6.12.111+deb13-cloud-amd64-unsigned_6.12.111-1_amd64.deb">c</a>"#;
        assert_eq!(pick_kernel_deb(only_bad, Some("13")), None);
        assert_eq!(
            pick_kernel_deb(only_bad, None),
            None,
            "兜底也要排除 rt / cloud"
        );
        assert_eq!(
            pick_kernel_deb(listing, None).as_deref(),
            Some("linux-image-7.2.11+deb14-amd64-unsigned_7.2.11-1_amd64.deb"),
            "没有版本偏好时挑最新的普通内核"
        );
        assert_eq!(
            abi_of_deb("linux-image-6.12.111+deb13-amd64-unsigned_6.12.111-1_amd64.deb").as_deref(),
            Some("6.12.111+deb13-amd64")
        );
        assert_eq!(abi_of_deb("nope.deb"), None);
        // rootfs 的 Debian 大版本：读 `/etc/debian_version`。
        let root = crate::tests::scratch("provision-debian-major");
        assert_eq!(debian_major(&root), None, "没有 debian_version 就不挑版本");
        fs::create_dir_all(root.join("etc")).expect("建 etc");
        fs::write(root.join("etc").join("debian_version"), "13.1\n").expect("写版本");
        assert_eq!(debian_major(&root).as_deref(), Some("13"));
        let _ = fs::remove_dir_all(&root);
    }
    /// 目的：归档里的入口程序装完必须能跑——归档不带 unix 权限位时（0644）要补上可执行位。
    #[cfg(unix)]
    #[test]
    fn archive_probe_gets_an_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::tests::scratch("provision-archive-exec");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("建源目录");
        let tool = src.join("tool");
        fs::write(&tool, b"#!/bin/sh\n").expect("写入口程序");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o644)).expect("归档里就是 0644");
        {
            let f = fs::File::create(cache.join("t.tar")).expect("建 tar");
            let mut b = tar::Builder::new(f);
            b.append_dir_all(".", &src).expect("打包");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "x.archive".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "archive".to_string(),
            install: ".vm/tool".to_string(),
            probe: ".vm/tool/tool".to_string(),
            guest: None,
            family: None,
            toolchain: None,
            source: "release:t.tar".to_string(),
            strip: 0,
            size_mb: 1,
            sha256: String::new(),
            note: String::new(),
        };
        let links = BTreeMap::from([(
            "release.base".to_string(),
            "https://example.invalid/".to_string(),
        )]);
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("补齐");
        let mode = fs::metadata(root.join(".vm").join("tool").join("tool"))
            .expect("读入口程序")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "入口程序要能跑（拿到 {:o}）",
            mode & 0o777
        );
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn install_file_uses_probe_name() {
        let root = crate::tests::scratch("provision-file");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("a.bin"), b"qcow2").expect("写缓存");
        let dep = Dep {
            id: "g".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "file".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: ".vm/guest/debian/rootfs.qcow2".to_string(),
            guest: Some("debian".to_string()),
            family: None,
            toolchain: None,
            source: "release:a.bin".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("单文件安装");
        assert!(root
            .join(".vm")
            .join("guest")
            .join("debian")
            .join("rootfs.qcow2")
            .is_file());
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：Windows guest 磁盘——qcow2 落盘并写 guest.yaml 画像。
    #[test]
    fn guest_disk_install_writes_guest_yaml() {
        let root = crate::tests::scratch("provision-disk");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("d.qcow2"), b"qcow2").expect("写缓存");
        let dep = Dep {
            id: "g".to_string(),
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
            kind: "guest-disk".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: ".vm/guest/debian/rootfs.qcow2".to_string(),
            guest: Some("debian".to_string()),
            family: None,
            toolchain: None,
            source: "release:d.qcow2".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("guest-disk 安装");
        let base = root.join(".vm").join("guest").join("debian");
        assert!(base.join("rootfs.qcow2").is_file());
        let y = fs::read_to_string(base.join("guest.yaml")).expect("guest.yaml");
        assert!(y.contains("agent: qga"), "{}", y);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：进度序号单调递增；行窗口只留最近 200 行（前端按 seq 增量取）。
    #[test]
    fn push_line_keeps_seq_and_window() {
        let j = Mutex::new(JobState::default());
        for i in 0..205 {
            push_line(&j, &format!("line {}", i));
        }
        let s = j.lock().expect("锁");
        assert_eq!(s.seq, 205);
        assert_eq!(s.lines.len(), 200);
        assert_eq!(s.lines.first().map(String::as_str), Some("line 5"));
        assert_eq!(s.lines.last().map(String::as_str), Some("line 204"));
    }

    /// 目的：内嵌清单与台账必须自洽——每条依赖的下载源都解析得成 https。
    #[test]
    fn embedded_manifest_and_links_resolve() {
        let deps = deps().expect("清单可解析");
        assert!(
            deps.iter().any(|d| d.id == "vm.qemu.linux"),
            "Linux QEMU 要在清单里"
        );
        let links = links().expect("台账可解析");
        for d in &deps {
            let url = source_url(&d.source, &links)
                .unwrap_or_else(|e| panic!("{} 的下载源解析不了：{}", d.id, e));
            assert!(
                url.starts_with("https://"),
                "{} 的源不是 https：{}",
                d.id,
                url
            );
        }
        let q = deps.iter().find(|d| d.id == "vm.qemu.linux").unwrap();
        assert!(q.source.starts_with("release:"), "自托管包用 release: 前缀");
        let v = deps
            .iter()
            .find(|d| d.id == "vm.virtiofsd.linux")
            .expect("virtiofsd 在清单里");
        assert_eq!(
            v.strip, 3,
            "zip 里 target/<三斜杠>/release/<bin> 四段，剥 3 段"
        );
    }

    /// 目的：自托管 release 源带版本 tag——`release:<tag>/<资产>` 拼成可下载 URL。
    #[test]
    fn release_source_carries_the_release_tag() {
        let all = links().expect("台账可解析");
        let base = all.get("release.base").expect("台账有 release.base");
        assert_eq!(
            source_url("release:virtiofsd/1.14.0/a.bin", &all).expect("要拼得出 URL"),
            format!("{}/virtiofsd/1.14.0/a.bin", base.trim_end_matches('/')),
            "tag 要原样保留在 URL 里"
        );
    }

    /// 目的：清单按本机 os/arch 筛（含 any）；平台不匹配的不出现。
    #[test]
    fn host_filter_selects_only_this_platform() {
        let all = deps().expect("清单可解析");
        let linux = for_host(&all, "linux", "x86_64");
        let ids: Vec<&str> = linux.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"vm.qemu.linux") && ids.contains(&"vm.virtiofsd.linux"));
        assert!(!ids.contains(&"vm.qemu.windows"), "windows 条目不该出现");
        assert!(ids.contains(&"vm.guest.linux"), "linux guest 在 linux 上");
        assert!(!ids.contains(&"vm.guest.windows"), "windows guest 不该出现");
    }

    /// 目的：虚拟根清单按 guest 归并——多条条目共用一个根，缺哪条如实列出来。
    #[test]
    fn guest_bases_merge_manifest_entries_by_guest() {
        let root = crate::tests::scratch("provision-guests");
        let base = root.join(".vm").join("guest").join("debian");
        fs::create_dir_all(&base).expect("建虚拟根");
        fs::write(base.join("vmlinuz"), b"x").expect("写内核");
        fs::write(
            base.join("guest.yaml"),
            "id: debian-test\nos: linux\narch: amd64\nagent: qga\nclients: []\nruntimes: [python, node]\n",
        )
        .expect("写画像");
        let dep = |id: &str, probe: &str, guest: &str| Dep {
            id: id.to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "file".to_string(),
            install: ".vm/guest/debian".to_string(),
            probe: probe.to_string(),
            guest: Some(guest.to_string()),
            family: None,
            toolchain: None,
            source: "release:a".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        // 镜像那条已按当前清单装好（台账登记过），运行期那条还没装。
        let image = dep("g.img", ".vm/guest/debian/vmlinuz", "debian");
        record_installed(&root, &image).expect("台账");
        let svc = service(&root);
        // 两条条目同属一个虚拟根：都在场 = 就绪；缺一条 = 如实列出缺哪条。
        let both = svc
            .guest_bases_of(&[
                dep("g.img", ".vm/guest/debian/vmlinuz", "debian"),
                dep("g.rt", ".vm/guest/debian/.runtimes-ready", "debian"),
            ])
            .expect("清单");
        assert_eq!(both.len(), 1, "同一个虚拟根归并成一条：{:?}", both);
        assert_eq!(both[0].guest, "debian");
        assert_eq!(both[0].label, "debian-test", "显示名取画像 id");
        assert_eq!(both[0].os, "linux");
        assert_eq!(
            both[0].runtimes,
            vec!["python".to_string(), "node".to_string()],
            "画像声明的运行能力要带出来（界面据此说明它自带什么）"
        );
        assert!(!both[0].ready, "少一条就不算装齐");
        assert_eq!(both[0].missing, vec!["g.rt".to_string()]);
        assert_eq!(both[0].install, ".vm/guest/debian");
        let one = svc
            .guest_bases_of(&[dep("g.img", ".vm/guest/debian/vmlinuz", "debian")])
            .expect("清单");
        assert!(one[0].ready && one[0].missing.is_empty(), "{:?}", one[0]);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：全量补齐时运行期编排起不了 guest 只如实跳过（不拖垮其余依赖）；显式点名它仍然硬失败。
    #[test]
    fn guest_runtime_failure_is_soft_for_the_whole_set_but_hard_when_named() {
        let root = crate::tests::scratch("provision-soft-runtime");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("a.bin"), b"x").expect("写缓存");
        let mk = |id: &str, kind: &str, install: &str, probe: &str, source: &str| Dep {
            id: id.to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: kind.to_string(),
            install: install.to_string(),
            probe: probe.to_string(),
            guest: Some("debian".to_string()),
            family: None,
            toolchain: None,
            source: source.to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let file_dep = mk("x.file", "file", ".vm/x", ".vm/x/a.bin", "release:a.bin");
        // 虚拟根还没装 + 没注入 vm 能力面：这一条必然失败，正好用来验软失败。
        let rt_dep = mk(
            "vm.guest-runtime.linux",
            "guest-runtime",
            ".vm/guest/debian",
            ".vm/guest/debian/.runtimes-ready",
            "release:a.bin",
        );
        let all = [file_dep, rt_dep];
        let links = BTreeMap::from([(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        )]);
        let mut lines: Vec<String> = Vec::new();
        let mut sink = |l: &str| lines.push(l.to_string());
        run(
            &root,
            &all,
            &links,
            &[],
            &AtomicBool::new(false),
            &mut sink,
            None,
        )
        .expect("全量补齐不该因为起不了 guest 而失败");
        assert!(
            lines.iter().any(|l| l.contains("运行期编排跳过")),
            "没点名时编排失败要如实跳过：{:?}",
            lines
        );
        assert!(
            root.join(".vm").join("x").join("a.bin").is_file(),
            "其余依赖照样装好"
        );
        // 显式点名这一条 = 你要的就是它：失败即失败。
        let err = run(
            &root,
            &all,
            &links,
            &["vm.guest-runtime.linux".to_string()],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect_err("显式点名要硬失败");
        assert!(err.contains("vm.guest-runtime.linux"), "{}", err);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：状态读真实文件系统——probe 在场才判在位。
    #[test]
    fn status_reads_real_filesystem() {
        let root = crate::tests::scratch("provision-status");
        let svc = service(&root);
        let items = svc.status().expect("状态");
        assert!(
            items.iter().all(|s| !s.present),
            "空根下都算缺：{:?}",
            items
        );
        // 按当前平台挑一条，把 probe 造出来（macOS 无清单条目，跳过）。
        let host = for_host(
            &deps().expect("清单"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        if let Some(dep) = host.into_iter().next() {
            let probe = under(&root, &dep.probe);
            fs::create_dir_all(probe.parent().expect("probe 有父目录")).expect("建目录");
            fs::write(&probe, b"x").expect("写 probe");
            record_installed(&root, &dep).expect("台账");
            let items = svc.status().expect("状态");
            let hit = items.iter().find(|s| s.id == dep.id).expect("条目在");
            assert!(hit.present, "probe 在场就该判在位：{}", dep.probe);
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：补齐内核——缓存命中时只解包不下载，装完按 probe 校验（不依赖网络）。
    #[test]
    fn install_core_uses_cache_and_checks_probe() {
        let root = crate::tests::scratch("provision-install");
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let tar_path = cache.join("a.tar");
        {
            let f = fs::File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            let mut h = tar::Header::new_gnu();
            h.set_size(5);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "f.txt", &b"hello"[..])
                .expect("写文件");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "x".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            kind: "archive".to_string(),
            install: ".vm/x".to_string(),
            probe: ".vm/x/f.txt".to_string(),
            guest: None,
            family: None,
            toolchain: None,
            source: "release:a.tar".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        let mut lines: Vec<String> = Vec::new();
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |l| lines.push(l.to_string()),
            None,
        )
        .expect("从缓存装");
        assert!(
            root.join(".vm").join("x").join("f.txt").is_file(),
            "装完 probe 要在：{:?}",
            lines
        );
        assert!(lines.iter().any(|l| l.contains("用缓存")), "{:?}", lines);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：给不存在的 id 要如实拒绝，且不碰网络。
    #[test]
    fn unknown_dep_id_is_refused() {
        let root = crate::tests::scratch("provision-missing");
        let p = service(&root);
        let err = p
            .ensure(&["不存在".to_string()], &mut |_| {})
            .expect_err("不存在的 id 要拒");
        assert!(err.contains("没有匹配"), "{}", err);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：缓存文件名取 URL 末段，去掉查询串。
    #[test]
    fn asset_name_takes_last_path_segment() {
        assert_eq!(asset_name("https://x/a/b.tar.zst?q=1"), "b.tar.zst");
        assert_eq!(asset_name("https://x/y.zip#frag"), "y.zip");
    }

    /// 目的：打出来的 guest 磁盘镜像必须是 4 KiB 块——位置参数按块大小解释，不给 -b 时 mke2fs 会
    ///   按小体积选 1 KiB 块的 small 布局，镜像缩成约四分之一而装不下量出来的内容（Linux 真机上炸过）。
    /// 约束：只在 Linux、且 unshare -r 与 mke2fs（e2fsprogs）可用时真跑；否则如实跳过（不假装验过）。
    #[test]
    fn packed_guest_image_uses_4k_blocks() {
        if !cfg!(target_os = "linux") {
            eprintln!("[探针] 打包镜像的块大小只在 Linux 上成立：本平台跳过");
            return;
        }
        use std::io::Read;
        use std::process::Stdio;
        let tools = std::process::Command::new("unshare")
            .args(["-r", "mke2fs", "-V"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !tools {
            eprintln!(
                "[探针] 没有可用的 unshare -r / mke2fs（要 e2fsprogs 与非特权用户命名空间）：跳过"
            );
            return;
        }
        let root = crate::tests::scratch("provision-pack-4k");
        let base = root.join("base");
        let cache = root.join("cache");
        fs::create_dir_all(&base).expect("建 base");
        fs::create_dir_all(&cache).expect("建 cache");
        fs::write(base.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
        fs::write(base.join("hello.txt"), b"hello").expect("写文件");
        pack_guest_base(&base, &cache, &mut |_| {}).expect("打包要成功");
        // ext4 超级块从镜像 1024 偏移开始：magic 在 +0x38，log_block_size 在 +0x18；块大小 = 1024 << log。
        let mut head = [0u8; 2048];
        fs::File::open(base.join("rootfs.ext4"))
            .expect("开镜像")
            .read_exact(&mut head)
            .expect("读镜像头");
        let sb = &head[1024..2048];
        let magic = u16::from_le_bytes([sb[0x38], sb[0x39]]);
        let log_block = u32::from_le_bytes([sb[0x18], sb[0x19], sb[0x1a], sb[0x1b]]);
        assert_eq!(magic, 0xEF53, "得是 ext4 超级块");
        assert_eq!(
            1024u64 << log_block,
            4096,
            "块大小必须是 4 KiB（1 KiB 的真机上装不下）"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：Linux 上真跑 ext4 工具链的前置（unshare -r + mke2fs + debugfs）；缺一个就不是"验过"。
    #[cfg(target_os = "linux")]
    fn guest_image_tools() -> bool {
        use std::process::Stdio;
        std::process::Command::new("unshare")
            .args(["-r", "mke2fs", "-V"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            && std::process::Command::new("debugfs")
                .arg("-V")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    }

    /// 目的：造一个带指定 mode 的普通文件（宿主属主；镜像里的属主由 mke2fs 的假 root 决定）。
    #[cfg(target_os = "linux")]
    fn write_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, b"#!/bin/sh\n").expect("写文件");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("设权限");
    }

    /// 目的：读宿主目录树上某个条目的权限位（只比低 12 位；解包后这里应当**丢了**特权位）。
    #[cfg(target_os = "linux")]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).expect("读属性").permissions().mode() & 0o7777
    }

    /// 目的：rdump / mke2fs 都会丢**特权位与属主、属组**：按源镜像回填之后，sudo 的 setuid、
    ///   ssh-agent 的 setgid + 组、/tmp 的粘滞位、/var/mail 的 setgid + 组、/etc/shadow 的 shadow 组
    ///   都要和源镜像一致。
    /// 约束：只在本机有 unshare -r / mke2fs / debugfs 时真跑；否则如实跳过（不假装验过）。
    #[cfg(target_os = "linux")]
    #[test]
    fn guest_rootfs_restores_privilege_bits_and_ownership() {
        use std::os::unix::fs::PermissionsExt;
        if !guest_image_tools() {
            eprintln!(
                "[探针] 没有可用的 unshare -r / mke2fs / debugfs（要 e2fsprogs 与非特权用户命名空间）：跳过"
            );
            return;
        }
        let root = crate::tests::scratch("provision-guest-attrs");
        let src = root.join("src");
        let tree = root.join("tree");
        let cache = root.join("cache");
        fs::create_dir_all(&cache).expect("建缓存目录");
        for d in [
            src.join("usr").join("bin"),
            src.join("etc"),
            src.join("tmp"),
            src.join("var").join("mail"),
        ] {
            fs::create_dir_all(&d).expect("建源目录");
        }
        write_mode(&src.join("usr").join("bin").join("sudo"), 0o4755);
        write_mode(&src.join("usr").join("bin").join("ssh-agent"), 0o2755);
        write_mode(&src.join("etc").join("shadow"), 0o640);
        fs::set_permissions(src.join("tmp"), fs::Permissions::from_mode(0o1777)).expect("设 /tmp");
        fs::set_permissions(
            src.join("var").join("mail"),
            fs::Permissions::from_mode(0o2775),
        )
        .expect("设 /var/mail");
        // 源镜像：mke2fs 的假 root 把属主写成 0；组用 sif 写成 cloud 镜像里的事实（宿主上 chown 不了）。
        let source = root.join("source.ext4");
        let st = std::process::Command::new("unshare")
            .args(["-r", "mke2fs", "-q", "-b", "4096", "-t", "ext4", "-F", "-d"])
            .arg(&src)
            .arg(&source)
            .arg("4096")
            .status()
            .expect("跑 mke2fs");
        assert!(st.success(), "造源镜像");
        let mut cmds = String::new();
        for (path, gid) in [
            ("/usr/bin/ssh-agent", 103u32),
            ("/etc/shadow", 42),
            ("/var/mail", 8),
        ] {
            cmds.push_str(&format!(
                "sif {} gid {}\n",
                debugfs_quote(path).expect("路径可写进命令表"),
                gid
            ));
        }
        run_debugfs(&source, &cmds, true).expect("给源镜像写属组");
        // 解包 → 清单 → 打包 → 回填。
        fs::create_dir_all(&tree).expect("建解包目录");
        extract_guest_rootfs(&source, &tree).expect("解包");
        assert_eq!(
            mode_of(&tree.join("usr").join("bin").join("sudo")),
            0o755,
            "rdump 本来就会丢 setuid——这正是回填要补的"
        );
        let attrs = guest_attrs_to_restore(&source, &tree).expect("读源属性");
        for want in [
            "/usr/bin/sudo",
            "/usr/bin/ssh-agent",
            "/etc/shadow",
            "/tmp",
            "/var/mail",
        ] {
            assert!(
                attrs.iter().any(|a| a.path == want),
                "清单里要有 {}：{:?}",
                want,
                attrs.iter().map(|a| a.path.clone()).collect::<Vec<_>>()
            );
        }
        fs::write(tree.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
        pack_guest_base(&tree, &cache, &mut |_| {}).expect("打包");
        let image = tree.join("rootfs.ext4");
        restore_guest_image_attrs(&image, &attrs).expect("回填并核对");
        let parents: Vec<String> = ["/", "/usr/bin", "/etc", "/var"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let got = image_attrs(&image, &parents).expect("读最终镜像");
        for want in [
            "/usr/bin/sudo",
            "/usr/bin/ssh-agent",
            "/etc/shadow",
            "/tmp",
            "/var/mail",
        ] {
            let a = attrs.iter().find(|a| a.path == want).expect("清单条目");
            let g = got
                .iter()
                .find(|g| g.path == want)
                .unwrap_or_else(|| panic!("最终镜像里没有 {}", want));
            assert_eq!(
                (g.mode, g.uid, g.gid),
                (a.mode, a.uid, a.gid),
                "{} 要回填成源镜像的属性",
                want
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：真机验收（opt-in）：拿项目内缓存的 Debian rootfs 分区跑同一条构造链，最终镜像里
    ///   **源镜像所有特权位 / 非 root 属主的条目**都与源一致（不碰 `.vm/guest/debian`，只写 target/test-scratch）。
    /// 约束：只在 SOLOMNI_VM_LIVE=1、本机有工具链、且 `.vm/cache/rootfs.ext4` 在场时真跑；否则 [探针] 如实跳过。
    #[cfg(target_os = "linux")]
    #[test]
    fn guest_rootfs_pipeline_keeps_source_attrs_on_the_real_image() {
        if !guest_image_tools() {
            eprintln!(
                "[探针] 没有可用的 unshare -r / mke2fs / debugfs（要 e2fsprogs 与非特权用户命名空间）：跳过"
            );
            return;
        }
        if std::env::var("SOLOMNI_VM_LIVE").as_deref() != Ok("1") {
            eprintln!(
                "[探针] 未开启虚拟根真机验收：设 SOLOMNI_VM_LIVE=1 后重跑（用 .vm/cache/rootfs.ext4）"
            );
            return;
        }
        let source = crate::tests::root_dir()
            .join(".vm")
            .join("cache")
            .join("rootfs.ext4");
        if !source.is_file() {
            eprintln!(
                "[探针] 本机没有源 rootfs 分区（先 solomni --provision --ensure=vm.guest-rootfs.linux --yes）：跳过"
            );
            return;
        }
        let root = crate::tests::scratch("provision-guest-attrs-live");
        let tree = root.join("tree");
        let cache = root.join("cache");
        fs::create_dir_all(&tree).expect("建解包目录");
        fs::create_dir_all(&cache).expect("建缓存目录");
        extract_guest_rootfs(&source, &tree).expect("解包真源镜像");
        let attrs = guest_attrs_to_restore(&source, &tree).expect("读真源镜像属性");
        let sudo = attrs
            .iter()
            .find(|a| a.path == "/usr/bin/sudo")
            .expect("真源镜像里要有 setuid 的 sudo");
        assert!(
            sudo.mode & 0o4000 != 0,
            "sudo 的 setuid 要读出来：{:o}",
            sudo.mode
        );
        assert!(
            attrs.iter().any(|a| a.gid != 0),
            "清单里要包含非 0 属组（例如 shadow）"
        );
        fs::write(tree.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
        pack_guest_base(&tree, &cache, &mut |_| {}).expect("打包");
        let image = tree.join("rootfs.ext4");
        restore_guest_image_attrs(&image, &attrs).expect("回填并核对");
        // 抽查 sudo 的最终 mode（回填已经在内部核对过全部条目）。
        let got = image_attrs(&image, &["/usr/bin".to_string()]).expect("读最终镜像");
        let sudo = got
            .iter()
            .find(|g| g.path == "/usr/bin/sudo")
            .expect("最终镜像里要有 sudo");
        assert_eq!(sudo.mode & 0o7777, 0o4755, "最终镜像里的 sudo 要带 setuid");
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：kind: toolchain 走同一条补齐内核——从缓存解归档到安装目录，装完把工具登记进 active.json
    ///   （启动期 entry::toolchains 只读这一份）；全程不碰网络。
    #[test]
    fn toolchain_install_extracts_and_registers_the_tool() {
        let root = crate::tests::scratch("provision-toolchain-install");
        let target = host_target();
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let install = format!(".tools/toolchains/{}/python/9.9.9", target);
        let tar_path = cache.join("py.tar");
        {
            let f = fs::File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            let mut h = tar::Header::new_gnu();
            h.set_size(2);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, "bin/python3", &b"py"[..])
                .expect("写文件");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "toolchain.python.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:py.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "python".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target.clone()),
                bins: vec!["bin".to_string()],
                wrappers: false,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        let mut lines: Vec<String> = Vec::new();
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |l| lines.push(l.to_string()),
            None,
        )
        .expect("从缓存装工具链");
        assert!(under(&root, &install).join("bin").is_dir(), "{:?}", lines);
        let active = root.join(".tools").join("toolchains").join("active.json");
        let raw = fs::read_to_string(&active).expect("要写 active.json");
        let doc: serde_json::Value = serde_json::from_str(&raw).expect("active.json 是 JSON");
        assert_eq!(doc["target"], serde_json::json!(target));
        assert_eq!(doc["profile"], serde_json::json!(["python"]));
        assert_eq!(doc["tools"][0]["id"], serde_json::json!("python"));
        assert_eq!(
            doc["tools"][0]["bin"][0],
            serde_json::json!(format!("{}/bin", install))
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：Zig 包解出来只有 zig 本体；装完补 cc / gcc / g++ / clang 包装（相对路径，产品根搬移后仍可用）。
    #[test]
    fn zig_toolchain_gets_compiler_wrappers() {
        let root = crate::tests::scratch("provision-toolchain-zig");
        let target = host_target();
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        let install = format!(".tools/toolchains/{}/cc/9.9.9", target);
        let tar_path = cache.join("zig.tar");
        {
            let f = fs::File::create(&tar_path).expect("建 tar");
            let mut b = tar::Builder::new(f);
            let mut h = tar::Header::new_gnu();
            h.set_size(3);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, "zig-demo/zig", &b"zig"[..])
                .expect("写 zig");
            b.finish().expect("收尾");
        }
        let dep = Dep {
            id: "toolchain.cc.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:zig.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "cc".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target),
                bins: vec!["bin".to_string()],
                wrappers: true,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect("装 Zig 工具链");
        let bin = under(&root, &install).join("bin");
        let zig = fs::read_to_string(bin.join("zig")).expect("zig 包装");
        assert!(
            zig.contains("../zig-demo/zig"),
            "zig 包装要指向包内本体：{}",
            zig
        );
        assert!(
            zig.contains("ZIG_GLOBAL_CACHE_DIR") && zig.contains("cache/global"),
            "zig 包装要把缓存钉在安装目录内：{}",
            zig
        );
        let gcc = fs::read_to_string(bin.join("gcc")).expect("gcc 包装");
        assert!(gcc.contains("zig\" cc"), "gcc 包装要调 zig cc：{}", gcc);
        let gxx = fs::read_to_string(bin.join("g++")).expect("g++ 包装");
        assert!(gxx.contains("zig\" c++"), "g++ 包装要调 zig c++：{}", gxx);
        assert!(
            gcc.contains("--target=*unknown-*") && gcc.contains("--target=\\1-"),
            "gcc 包装要把 Rust 三元组的 unknown- 去掉且保留 --target= 前缀：{}",
            gcc
        );
        for name in ["cc", "gcc", "g++", "c++", "clang", "clang++"] {
            assert!(bin.join(name).is_file(), "缺包装：{}", name);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(bin.join("gcc"))
                .expect("看模式")
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "包装要有执行位：{:o}", mode);
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：全量补齐时已在场的基础工具链不重装（260 MB 的 MinGW 不该每次重解）；显式点名才强制重装。
    #[test]
    fn present_toolchain_is_skipped_on_full_ensure() {
        let root = crate::tests::scratch("provision-toolchain-skip");
        let target = host_target();
        let install = format!(".tools/toolchains/{}/python/9.9.9", target);
        fs::create_dir_all(under(&root, &install).join("bin")).expect("铺已装好的工具");
        let dep = Dep {
            id: "toolchain.python.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:missing.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "python".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target),
                bins: vec!["bin".to_string()],
                wrappers: false,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        };
        record_installed(&root, &dep).expect("台账");
        let mut links = BTreeMap::new();
        links.insert("release.base".to_string(), "http://127.0.0.1:1".to_string());
        let mut lines: Vec<String> = Vec::new();
        run(
            &root,
            std::slice::from_ref(&dep),
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |l| lines.push(l.to_string()),
            None,
        )
        .expect("在位就跳过，不碰网络");
        assert!(
            lines.iter().any(|l| l.contains("已就位（跳过）")),
            "{:?}",
            lines
        );
        // 显式点名 = 你要的就是它：缓存里没有、源不可达时如实失败，不假装成功。
        let err = run(
            &root,
            &[dep],
            &links,
            &["toolchain.python.test".to_string()],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect_err("点名就必须真装");
        assert!(err.contains("下载失败"), "{}", err);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：缓存与 pin 不符时换一份重下，而不是把旧缓存当证据；换不到就如实失败、安装目录一个字节都不写。
    #[test]
    fn cache_pin_mismatch_is_replaced_not_trusted() {
        let root = crate::tests::scratch("provision-toolchain-pin");
        let target = host_target();
        let cache = root.join(".vm").join("cache");
        fs::create_dir_all(&cache).expect("建缓存");
        fs::write(cache.join("py.tar"), b"abc").expect("写缓存");
        let install = format!(".tools/toolchains/{}/python/9.9.9", target);
        let dep = Dep {
            id: "toolchain.python.test".to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.clone(),
            probe: format!("{}/bin", install),
            source: "release:py.tar".to_string(),
            kind: "toolchain".to_string(),
            guest: None,
            toolchain: Some(ToolchainSpec {
                tool: "python".to_string(),
                version: "9.9.9".to_string(),
                target: Some(target),
                bins: vec!["bin".to_string()],
                wrappers: false,
            }),
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: "0".repeat(64),
            note: String::new(),
        };
        let mut links = BTreeMap::new();
        links.insert(
            "release.base".to_string(),
            "https://example.invalid".to_string(),
        );
        let err = run(
            &root,
            &[dep],
            &links,
            &[],
            &AtomicBool::new(false),
            &mut |_| {},
            None,
        )
        .expect_err("pin 不符不能装");
        assert!(err.contains("下载失败"), "换不到源要如实失败：{}", err);
        assert!(!under(&root, &install).exists(), "装不上不许落盘");
        assert!(
            !cache.join("py.tar").is_file(),
            "不符的旧缓存要被换掉，不能当证据留着"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：镜像只是候选之一（上游在前），GitHub 资产才加；选源排除不可达的候选，全都不可达时说清状态码。
    #[test]
    fn mirror_and_source_choice() {
        assert_eq!(
            mirror_of("https://github.com/a/b/c.zip", "https://gh.example/"),
            Some("https://gh.example/github.com/a/b/c.zip".to_string())
        );
        assert_eq!(
            mirror_of("https://example.test/a.zip", "https://gh.example"),
            None
        );
        assert_eq!(mirror_of("https://github.com/a/b", ""), None);
        let sample = |url: &str, code: u16, kbps: f64| (url.to_string(), code, kbps);
        let picked = choose_fastest(&[
            sample("https://a", 200, 10.0),
            sample("https://b", 200, 900.0),
            sample("https://c", 404, 0.0),
        ])
        .expect("要选出最快的可达源");
        assert_eq!(picked, "https://b");
        let err = choose_fastest(&[sample("https://a", 0, 0.0)]).unwrap_err();
        assert!(err.contains("没有可用的下载源"), "{}", err);
    }

    /// 目的：断点续传——分片与来源标记一致时带 Range 续上；来源不明就从头下（不把两份字节接起来）。
    #[test]
    fn resume_uses_range_only_for_a_known_source() {
        use std::sync::Arc;
        use std::time::Duration;
        let body: &'static [u8] = b"0123456789";
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("起本地源"));
        let addr = server.server_addr().to_ip().expect("TCP 地址");
        let url = format!("http://127.0.0.1:{}/a.bin", addr.port());
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let (srv, st, sn) = (Arc::clone(&server), Arc::clone(&stop), Arc::clone(&seen));
        let handle = std::thread::spawn(move || {
            while !st.load(Ordering::Relaxed) {
                let Ok(Some(req)) = srv.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let range = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Range"))
                    .map(|h| h.value.as_str().to_string());
                sn.lock().expect("锁").push(range.clone());
                let resp = match range {
                    Some(r) => {
                        let start: usize = r
                            .trim_start_matches("bytes=")
                            .trim_end_matches('-')
                            .parse()
                            .unwrap_or(0);
                        tiny_http::Response::from_data(body[start..].to_vec()).with_status_code(206)
                    }
                    None => tiny_http::Response::from_data(body.to_vec()),
                };
                let _ = req.respond(resp);
            }
        });
        let root = crate::tests::scratch("provision-resume");
        let dest = root.join("a.bin");
        let part = part_path(&dest);
        // 分片 + 来源标记一致：续传（Range: bytes=<断点>-）。
        fs::write(&part, &body[..4]).expect("写分片");
        fs::write(part_source_marker(&part), &url).expect("写来源标记");
        let agent = crate::kernel::detail::outbound::agent(5, 30);
        download_with_resume(&agent, &url, &dest, &mut |_| {}).expect("续传");
        assert_eq!(
            fs::read(&dest).expect("读下载结果"),
            body,
            "续传后内容要完整"
        );
        assert!(!part.exists(), "成功后分片要改名掉");
        // 来历不明的分片：从头下（不发 Range）。
        fs::write(&part, b"XYZ").expect("写来历不明的分片");
        download_with_resume(&agent, &url, &dest, &mut |_| {}).expect("从头下");
        assert_eq!(fs::read(&dest).expect("读下载结果"), body);
        let ranges = seen.lock().expect("锁").clone();
        assert_eq!(ranges.len(), 2, "{:?}", ranges);
        assert_eq!(ranges[0].as_deref(), Some("bytes=4-"), "首个请求要带断点");
        assert_eq!(ranges[1], None, "来源不明必须从头下");
        stop.store(true, Ordering::Relaxed);
        handle.join().expect("收本地源线程");
        let _ = fs::remove_dir_all(&root);
    }
}

mod guest {
    use crate::capabilities::provision::domain::Dep;
    use crate::capabilities::provision::domain::{deps, under};
    use crate::capabilities::provision::guest::*;
    use crate::capabilities::vm::api::classify_base;
    use std::path::Path;

    /// 目的：造一条 guest-runtime 清单条目（字段取自 dependencies.yaml 的真实形状）。
    fn dep(id: &str, os: &str, guest: &str, install: &str, probe: &str) -> Dep {
        Dep {
            id: id.to_string(),
            os: os.to_string(),
            arch: "x86_64".to_string(),
            kind: "guest-runtime".to_string(),
            install: install.to_string(),
            probe: probe.to_string(),
            guest: Some(guest.to_string()),
            family: None,
            toolchain: None,
            source: "guest.debian.image".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        }
    }

    /// 目的：造一条磁盘根的条目（Windows 形态）。
    fn disk_dep() -> Dep {
        dep(
            "vm.guest-runtime.windows",
            "windows",
            "debian-13",
            ".vm/guest/debian-13",
            ".vm/guest/debian-13/.runtimes-ready",
        )
    }

    /// 目的：在临时产品根里铺一份磁盘根虚拟根 + QEMU 落点。
    fn seed_disk_root(root: &Path) -> std::path::PathBuf {
        let base = root.join(".vm").join("guest").join("debian-13");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::write(base.join("rootfs.qcow2"), b"qcow2").expect("disk");
        std::fs::write(
            base.join("guest.yaml"),
            "id: debian-13-genericcloud-amd64\narch: amd64\nagent: qga\nclients: []\n",
        )
        .expect("yaml");
        let qemu = root
            .join(".vm")
            .join("qemu")
            .join("bin")
            .join("qemu-system-x86_64.exe");
        std::fs::create_dir_all(qemu.parent().expect("parent")).expect("qemu dir");
        std::fs::write(&qemu, b"exe").expect("qemu");
        base
    }

    /// 目的：没有 vm 能力面时如实拒绝（不假装能编排）。
    #[test]
    fn guest_runtime_refuses_without_vm() {
        let root = crate::tests::scratch("provision-guest-no-vm");
        let err = install_guest_runtime(
            &root,
            &disk_dep(),
            &crate::kernel::detail::host_probe::HostProbeAdapter,
            None,
            false,
            &mut |_| {},
        )
        .expect_err("要拒");
        assert!(err.contains("vm 能力"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：虚拟根不在场时如实拒绝——先装 vm.guest.名字 那一条。
    #[test]
    fn guest_runtime_refuses_without_image() {
        let root = crate::tests::scratch("provision-guest-no-image");
        let vm = crate::tests::doubles::RecordingVm::default();
        let err = install_guest_runtime(
            &root,
            &disk_dep(),
            &crate::kernel::detail::host_probe::HostProbeAdapter,
            Some(&vm),
            false,
            &mut |_| {},
        )
        .expect_err("要拒");
        assert!(err.contains("虚拟根不可用"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：磁盘根（Windows 形态）走完首启 + 维护启动，落两个标记，并把客户端写成 cifs。
    #[test]
    fn guest_runtime_writes_markers_and_declares_cifs() {
        let root = crate::tests::scratch("provision-guest-ok");
        let base = seed_disk_root(&root);
        let vm = crate::tests::doubles::RecordingVm::default();
        install_guest_runtime(
            &root,
            &disk_dep(),
            &crate::kernel::detail::host_probe::HostProbeAdapter,
            Some(&vm),
            false,
            &mut |_| {},
        )
        .expect("编排");
        assert!(base.join(".qga-ready").is_file(), "首启标记要落");
        assert!(base.join(".runtimes-ready").is_file(), "运行包标记要落");
        let y = std::fs::read_to_string(base.join("guest.yaml")).expect("yaml");
        assert!(y.contains("clients: [cifs]"), "{}", y);
        assert!(y.contains("runtimes: [python, node, cc, gcc]"), "{}", y);
        assert!(y.contains("arch: amd64"), "arch 要用画像认的写法：{}", y);
        // 写出的画像必须仍可被识别（arch 写法错了就会在这里炸），并如实声明装好的运行能力。
        let image = crate::capabilities::vm::api::classify_base(
            &base,
            &crate::kernel::detail::HostProbeAdapter,
        )
        .expect("运行期编排写出的画像要仍可识别");
        assert_eq!(
            image.runtimes,
            vec![
                "cc".to_string(),
                "gcc".to_string(),
                "node".to_string(),
                "python".to_string()
            ],
            "镜像要如实声明它装好的运行能力"
        );
        let runs = vm.runs.lock().expect("锁").clone();
        assert!(
            runs.iter().any(|c| c.contains("cifs-utils")),
            "磁盘根的运行包清单要带 cifs-utils：{:?}",
            runs
        );
        assert!(
            runs.iter().any(|c| c.contains("nodejs.org/dist/v24.11.0")),
            "node 要从固定版本官方二进制装：{:?}",
            runs
        );
        assert!(
            runs.iter().any(|c| c.contains("gcc") && c.contains("g++")),
            "C/C++ 要随 apt 装进 guest：{:?}",
            runs
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：已有运行标记但 Windows 画像缺 cifs 的旧资产只拒绝，不自动修补。
    #[test]
    fn stale_windows_guest_is_refused_instead_of_repaired() {
        let root = crate::tests::scratch("provision-guest-stale");
        let base = seed_disk_root(&root);
        std::fs::write(base.join(".runtimes-ready"), "old\n").expect("marker");
        let vm = crate::tests::doubles::RecordingVm::default();
        let err = install_guest_runtime(
            &root,
            &disk_dep(),
            &crate::kernel::detail::host_probe::HostProbeAdapter,
            Some(&vm),
            false,
            &mut |_| {},
        )
        .expect_err("旧画像要拒");
        assert!(err.contains("不是当前 Windows 基线"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：Linux 磁盘根的维护启动必须先配网——磁盘根没有 cloud-init 首启，网卡没人配，
    ///   apt 与 Node 下载只能解析失败（真机验收就是这么露的）。
    #[test]
    fn linux_maintenance_brings_up_the_network_first() {
        use std::fs;
        let root = crate::tests::scratch("provision-guest-net");
        let base = root.join(".vm").join("guest").join("debian");
        fs::create_dir_all(&base).expect("建虚拟根");
        fs::write(base.join("rootfs.ext4"), b"img").expect("磁盘镜像");
        fs::write(
            base.join("guest.yaml"),
            "id: t\nos: linux\narch: amd64\nagent: qga\nclients: [9p, virtiofs]\n",
        )
        .expect("画像");
        let qemu = root
            .join(".vm")
            .join("qemu")
            .join("bin")
            .join("qemu-system-x86_64");
        fs::create_dir_all(qemu.parent().expect("父目录")).expect("建 qemu 目录");
        fs::write(&qemu, b"exe").expect("qemu");
        let vm = crate::tests::doubles::RecordingVm::default();
        let d = dep(
            "vm.guest-runtime.linux",
            "linux",
            "debian",
            ".vm/guest/debian",
            ".vm/guest/debian/.runtimes-ready",
        );
        let mut lines: Vec<String> = Vec::new();
        install_guest_runtime(
            &root,
            &d,
            &crate::kernel::detail::host_probe::HostProbeAdapter,
            Some(&vm),
            true,
            &mut |l| lines.push(l.to_string()),
        )
        .expect("编排");
        let runs = vm.runs.lock().expect("锁").clone();
        for want in [
            "刷新 apt 索引",
            "安装基础工具链",
            "安装 Node",
            "自证 python / node / C-C++",
        ] {
            assert!(
                lines.iter().any(|l| l.contains(want)),
                "进度行缺「{}」：{:?}",
                want,
                lines
            );
        }
        assert!(
            runs.iter().filter(|c| c.contains("apt-get update")).count() == 1
                && runs
                    .iter()
                    .filter(|c| c.contains("apt-get install"))
                    .count()
                    == 1,
            "索引与安装要分两条命令：{:?}",
            runs
        );
        assert!(
            runs.iter()
                .any(|c| c.contains("systemd-networkd") && c.contains("10.0.2.15")),
            "Linux 维护启动要先配网：{:?}",
            runs
        );
        assert!(
            !runs.iter().any(|c| c.contains("cifs-utils")),
            "Linux 不需要 cifs-utils：{:?}",
            runs
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：真机验收（默认跳过）：Linux 虚拟根的运行期编排真起一次 guest，把自证过的能力写进画像。
    /// 约束：只在 SOLOMNI_VM_LIVE=1、本机是 Linux、虚拟根与 QEMU 都在场时执行；否则打印 [探针] 如实跳过。
    #[test]
    fn provision_linux_runtime_proves_runtimes_live() {
        use crate::kernel::ports::HostProbe;
        let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
        if !live || std::env::consts::OS != "linux" {
            eprintln!("[探针] 未开启 Linux 运行期编排的真机验收：设 SOLOMNI_VM_LIVE=1（仅 Linux 宿主）后重跑");
            return;
        }
        let root = crate::tests::root_dir();
        let Some(dep) = deps()
            .expect("清单")
            .into_iter()
            .find(|d| d.id == "vm.guest-runtime.linux")
        else {
            eprintln!("[探针] 清单里没有 vm.guest-runtime.linux");
            return;
        };
        let base = under(&root, &dep.install);
        let probe = crate::kernel::detail::HostProbeAdapter;
        let qemu_ok = qemu_path(&root, &dep.os)
            .map(|p| p.is_file())
            .unwrap_or(false);
        if !probe.is_dir(&base) || !qemu_ok {
            eprintln!("[探针] 虚拟根或 QEMU 不在场：先 solomni --provision --ensure=vm.qemu.linux,vm.guest.linux --yes");
            return;
        }
        // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
        let _live = crate::tests::doubles::live_guest_lock();
        let vm = crate::capabilities::vm::service::VmService::new(
            std::sync::Arc::new(crate::capabilities::vm::detail::QemuEngine::new()),
            vec![std::sync::Arc::new(
                crate::capabilities::vm::detail::NoneShare::new(),
            )],
        );
        let mut sink = |l: &str| eprintln!("[探针] {}", l);
        install_guest_runtime(&root, &dep, &probe, Some(&vm), true, &mut sink)
            .expect("真机运行期编排要成功");
        let image = classify_base(&base, &probe).expect("编排后的画像要仍可识别");
        assert!(
            image.runtimes.iter().any(|r| r == "python"),
            "python 必须自证并写进画像：{:?}",
            image.runtimes
        );
        assert!(base.join(".runtimes-ready").is_file(), "运行包标记要落");
    }
}

mod ports {
    use crate::capabilities::provision::detail::fetch::HttpFetch;
    use crate::capabilities::provision::detail::host::RealHost;
    use crate::capabilities::provision::detail::ledger::FileLedger;
    use crate::capabilities::provision::detail::procs::BookProcs;
    use crate::capabilities::provision::domain::{install_identity, Dep};
    use crate::capabilities::provision::ports::{Fetcher, HostFs, Ledger, Procs};
    use std::fs;

    /// 目的：造一条任意安装方式的清单条目（端口契约用例夹具）。
    fn dep_of(id: &str, install: &str, probe: &str) -> Dep {
        Dep {
            id: id.to_string(),
            os: "any".to_string(),
            arch: "any".to_string(),
            install: install.to_string(),
            probe: probe.to_string(),
            source: "release:x".to_string(),
            kind: "file".to_string(),
            guest: None,
            toolchain: None,
            family: None,
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        }
    }

    /// 目的：宿主落点端口的契约——建、查、删都能在临时根上往返，静默失败即不成立。
    #[test]
    fn host_fs_round_trips_directories_files_and_the_exec_bit() {
        let root = crate::tests::scratch("provision-ports-host");
        let host = RealHost;
        let dir = root.join("a").join("b");
        assert!(!host.exists(&dir), "临时根上不该凭空存在");
        host.mkdir_all(&dir).expect("建目录要成功");
        assert!(host.is_dir(&dir));
        let file = dir.join("tool");
        fs::write(&file, b"x").expect("写文件");
        assert!(host.is_file(&file));
        host.make_executable(&file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file).expect("元数据").permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "执行位要真的落上：{:o}", mode);
        }
        host.remove_file(&file).expect("删文件要成功");
        assert!(!host.exists(&file));
        host.remove_dir_all(&root.join("a")).expect("删目录要成功");
        assert!(!host.exists(&root.join("a")));
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：台账端口的契约——在场判定看探针、登记按安装身份、划掉只剩空台账。
    #[test]
    fn ledger_records_by_identity_and_forgets_the_entry() {
        let root = crate::tests::scratch("provision-ports-ledger");
        let ledger = FileLedger::new(&root);
        let dep = dep_of("demo", "inst/demo", "inst/demo/demo.txt");
        assert!(!ledger.present(&dep), "没落盘就不该算在场");
        fs::create_dir_all(root.join("inst").join("demo")).expect("建安装目录");
        fs::write(root.join("inst").join("demo").join("demo.txt"), b"x").expect("落探针");
        assert!(ledger.present(&dep), "探针在场就要算在场");
        assert!(ledger.snapshot().is_empty(), "还没登记台账应当为空");
        ledger.record(&dep).expect("登记要成功");
        assert_eq!(ledger.snapshot().get("demo"), Some(&install_identity(&dep)));
        ledger.forget("demo").expect("划掉要成功");
        assert!(ledger.snapshot().is_empty(), "划掉后台账应当为空");
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：取数端口的契约——摘要按内容算成小写十六进制，同一份内容稳定。
    #[test]
    fn fetcher_digests_file_contents_as_lowercase_hex() {
        let root = crate::tests::scratch("provision-ports-digest");
        let file = root.join("payload.bin");
        fs::write(&file, b"abc").expect("写文件");
        let fetch = HttpFetch::new(15, 7200);
        let digest = fetch.digest(&file).expect("摘要要成功");
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(fetch.digest(&file).expect("再算一次"), digest);
        let _ = fs::remove_dir_all(&root);
    }

    /// 目的：进程端口的契约——台账为空时如实说没有，不凭空造条目。
    #[test]
    fn procs_reports_nothing_before_any_guest_runs() {
        let root = crate::tests::scratch("provision-ports-procs");
        let procs = BookProcs::new(&root);
        assert!(procs.entries().is_empty(), "空台账不该有条目");
        assert!(!procs.guest_running(), "没有条目就没有 guest 在跑");
        let _ = fs::remove_dir_all(&root);
    }
}
