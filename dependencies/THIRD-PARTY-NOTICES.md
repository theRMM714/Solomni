# 第三方依赖声明（自托管便携包）

本目录记录项目自托管在 GitHub Release（tag `dependencies`）里的第三方依赖的开源许可与对应源码义务。
`links.yaml` 的 `release.base` 指向该 release；`dependencies.yaml` 是产品读取的依赖清单。

规则：自托管包一律**原样分发**，不改内容、不重新打包内部文件；新增包必须同步本文件与 `dependencies.yaml`。

## winlibs（winlibs.zip）
- 内容：MinGW-w64（GCC + binutils）便携工具链
- 许可：GPLv3（含 GCC Runtime Library Exception）+ GPL（binutils）
- 上游与对应源码：https://github.com/brechtsanders/winlibs_mingw/releases
- 说明：按上游同样的产物原样再分发；对应源码可从上游同一版本取得。

## virtiofsd（virtiofsd-4bfa7125.zip）
- 版本：v1.14.0（commit 4bfa7125）
- 许可：Apache-2.0（上游另提供 BSD-3-Clause 双许可）
- 上游与对应源码：https://gitlab.com/virtio-fs/virtiofsd
- 说明：Apache-2.0 无 GPL 对应源码义务；再分发时随包附许可文本。

## QEMU（可运行的分平台包待上传；对应源码已在本 release）
- 版本：11.1.2
- 许可：GPL-2.0
- 对应源码：本 release 的 `qemu-11.1.2.tar.xz`（QEMU 11.1.2 源码 tarball）
- 说明：可运行的便携包（`qemu-linux-x86_64.tar.zst` / `qemu-windows-x86_64.tar.zst`）与原样再分发的二进制一同提供对应源码，满足 GPLv2 第 3 条的源码提供义务。
