# Third-Party Notices

本文件记录了本项目自托管分发的第三方依赖包，以及它们的许可证和对应源码信息。

---

## 1. QEMU (Linux AppImage 二进制)

- **Binary Asset**: `QEMU-11.1.1-4-anylinux-x86_64.AppImage`
- **Version**: QEMU 11.1.1-4 (pkgforge 打包版本)
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**:
  - QEMU 官方源码: `qemu-11.1.1.tar.xz` (已随 `deps-qemu-11.1.1-4` Release 提供)
  - 打包脚本源码: `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz` (已随 `deps-qemu-11.1.1-4` Release 提供)
- **Note**: 对应源码已随 `deps-qemu-11.1.1-4` Release 直接提供，通常无需通过书面 Offer 获取。

## 2. QEMU (Windows 便携包)

- **Binary Asset**: `qemu-windows-x86_64-11.1.1.tar.zst`
- **Version**: QEMU 11.1.1
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**: `qemu-11.1.1.tar.xz` (已随 `deps-qemu-11.1.1-4` Release 提供)
- **编译方式**: 从 QEMU 官方源码 `qemu-11.1.1.tar.xz`，用 MSYS2（mingw-w64）工具链构建，`--enable-slirp`（user 网卡 / guestfwd 必需）；固件取自源码自带的预编译 `pc-bios/`，按真实文件打包（不留软链）。
- **打包的依赖库**（均为独立许可证，不影响 QEMU 自身的 GPL 状态）：
  - `libglib-2.0-0.dll` / `libgio-2.0-0.dll` / `libgobject-2.0-0.dll` / `libgmodule-2.0-0.dll` — LGPL-2.1
  - `libpixman-1-0.dll` — MIT
  - `libslirp-0.dll` — BSD-3-Clause
  - `libzstd.dll` — BSD-3-Clause / GPL-2.0 双许可（本项目按 BSD-3-Clause 使用）
  - `libbz2-1.dll` — BSD-3-Clause
  - `libncursesw6.dll` — X11 License
  - `zlib1.dll` — zlib
  - `libffi-8.dll` — MIT
  - `libpcre2-8-0.dll` — BSD-3-Clause
  - `libiconv-2.dll` / `libintl-8.dll` — LGPL-2.1
  - `libwinpthread-1.dll` — GCC Runtime Library Exception
- **Note**: 对应源码已随 `deps-qemu-11.1.1-4` Release 直接提供，通常无需通过书面 Offer 获取。

## 3. virtiofsd (Rust 版)

- **Binary Asset**: `virtiofsd-4bfa7125.zip`
- **Version**: v1.14.0 (commit 4bfa7125)
- **License**: Apache-2.0 (另有上游 BSD-3-Clause 双许可)
- **Source Code**: https://gitlab.com/virtio-fs/virtiofsd
- **Note**: 根据 Apache-2.0 许可证要求，`LICENSE-APACHE-2.0.txt` 已随 `deps-virtiofsd-1.14.0` Release 提供。无 GPL 源码提供义务。

## 4. smbd（SMB 共享 helper，独立仓）

- **Binary Asset**: `smbd-windows-x86_64.exe`
- **Version**: v0.1.0（独立仓 tag `smbd-v0.1.0`）
- **License**: MIT（上游 rust-smb-server 亦为 MIT）
- **Source Code**: https://github.com/theRMM714/rust-smb-server
- **Note**: 上游 [paltaio/rust-smb-server](https://github.com/paltaio/rust-smb-server)（MIT）的 fork；上游代码不改，本仓只新增 helper CLI 与发布工作流，由该仓 CI 构建并发布 `smbd.exe` 与 `.sha256` 资产；Solomni 只按 URL 下载，不携带其提交史。

## 5. 基础工具链（运行前提，首次安装；不随仓分发）

以下组件由 `setup-runtime.js` 默认、`setup-toolchains.js` 按需在用户同意后从上游下载到项目内 `.tools/toolchains/`；本仓不携带二进制：

- **Python**：python-build-standalone（CPython 3.14.9），PSF-2.0；来源 https://github.com/astral-sh/python-build-standalone 。
- **Node.js**：Node 24.11.0，MIT；来源 https://nodejs.org/dist/ 。
- **Zig**：0.15.2，MIT；来源 https://ziglang.org/download/ 。
- **MinGW-w64（winlibs）**：GCC 16.2.0 / MinGW-w64 UCRT，GPL/LGPL（含 GCC Runtime Library Exception）；来源 https://github.com/brechtsanders/winlibs_mingw 。

每个资产的 URL 与 sha256 成对登记在 `links.yaml`；校验值由 `node pins.js update` 生成，升级时同 diff 更新。

---

## 附：GPL 源码获取说明

对于 `deps-qemu-11.1.1-4` Release 中包含 GPL 授权的二进制文件（QEMU Linux AppImage 和 QEMU Windows 便携包），根据 GPLv2 第 3(a) 条，其完整的对应源码已在该 Release 页面的资产（Assets）中提供，任何接收该二进制文件的第三方均可直接从该页面下载获取。

`deps-qemu-11.1.1-4` Release 提供的对应源码包括：
- `qemu-11.1.1.tar.xz`（QEMU 官方源码，适用于 Windows 与 Linux 两个便携包）
- `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz`（打包脚本源码，仅适用于 Linux AppImage）

---

# Third-Party Notices (English Version)

This file documents the third-party dependency bundles self-hosted and distributed by this project, along with their license and corresponding source information.

## 1. QEMU (Linux AppImage Binary)

- **Binary Asset**: `QEMU-11.1.1-4-anylinux-x86_64.AppImage`
- **Version**: QEMU 11.1.1-4 (pkgforge packaged version)
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**:
  - Official QEMU Source: `qemu-11.1.1.tar.xz` (provided with the `deps-qemu-11.1.1-4` Release)
  - Packaging Script Source: `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz` (provided with the `deps-qemu-11.1.1-4` Release)
- **Note**: The Corresponding Source is already provided directly in the `deps-qemu-11.1.1-4` Release.

## 2. QEMU (Windows Portable Bundle)

- **Binary Asset**: `qemu-windows-x86_64-11.1.1.tar.zst`
- **Version**: QEMU 11.1.1
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**: `qemu-11.1.1.tar.xz` (provided with the `deps-qemu-11.1.1-4` Release)
- **Build Method**: Built from the official QEMU source `qemu-11.1.1.tar.xz` with the MSYS2 (mingw-w64) toolchain, `--enable-slirp` (required for the user-mode network backend / guestfwd); firmware comes from the prebuilt `pc-bios/` shipped in the source and is packaged as real files (no symlinks).
- **Bundled Dependency Libraries** (independent licenses; do not affect QEMU's own GPL status):
  - `libglib-2.0-0.dll` / `libgio-2.0-0.dll` / `libgobject-2.0-0.dll` / `libgmodule-2.0-0.dll` — LGPL-2.1
  - `libpixman-1-0.dll` — MIT
  - `libslirp-0.dll` — BSD-3-Clause
  - `libzstd.dll` — BSD-3-Clause / GPL-2.0 dual-licensed (used here under BSD-3-Clause)
  - `libbz2-1.dll` — BSD-3-Clause
  - `libncursesw6.dll` — X11 License
  - `zlib1.dll` — zlib
  - `libffi-8.dll` — MIT
  - `libpcre2-8-0.dll` — BSD-3-Clause
  - `libiconv-2.dll` / `libintl-8.dll` — LGPL-2.1
  - `libwinpthread-1.dll` — GCC Runtime Library Exception
- **Note**: The Corresponding Source is already provided directly in the `deps-qemu-11.1.1-4` Release.

## 3. virtiofsd (Rust Version)

- **Binary Asset**: `virtiofsd-4bfa7125.zip`
- **Version**: v1.14.0 (commit 4bfa7125)
- **License**: Apache-2.0 (also dual-licensed upstream under BSD-3-Clause)
- **Source Code**: https://gitlab.com/virtio-fs/virtiofsd
- **Note**: `LICENSE-APACHE-2.0.txt` is provided with the `deps-virtiofsd-1.14.0` Release as required by the Apache-2.0 License. There is no GPL source provision obligation.

## 4. smbd (SMB share helper, separate repository)

- **Binary Asset**: `smbd-windows-x86_64.exe`
- **Version**: v0.1.0 (tag `smbd-v0.1.0` in the separate repository)
- **License**: MIT (upstream rust-smb-server is MIT as well)
- **Source Code**: https://github.com/theRMM714/rust-smb-server
- **Note**: A fork of upstream `paltaio/rust-smb-server` (MIT); upstream code is unchanged, and the fork only adds the helper CLI and the release workflow. Its CI builds and publishes the `smbd.exe` and `.sha256` assets; Solomni only downloads it by URL and does not carry its commit history.

## 5. Base toolchains (runtime prerequisite, first install; not shipped)

The following components are installed by `setup-runtime.js` by default (or `setup-toolchains.js` on demand) with
user consent into the project's `.tools/toolchains/`; this repository does not carry their binaries:

- **Python**: python-build-standalone (CPython 3.14.9), PSF-2.0; https://github.com/astral-sh/python-build-standalone .
- **Node.js**: Node 24.11.0, MIT; https://nodejs.org/dist/ .
- **Zig**: 0.15.2, MIT; https://ziglang.org/download/ .
- **MinGW-w64 (winlibs)**: GCC 16.2.0 / MinGW-w64 UCRT, GPL/LGPL (with the GCC Runtime Library Exception); https://github.com/brechtsanders/winlibs_mingw .

Each asset's URL and sha256 are paired in `links.yaml`; checksums are generated by `node pins.js update`, and upgrades update both in the same diff.

---

## Appendix: GPL Source Availability Statement

For GPL-licensed binaries included in the `deps-qemu-11.1.1-4` Release (the QEMU Linux AppImage and the QEMU Windows portable bundle), pursuant to GPLv2 Section 3(a), the complete Corresponding Source is provided in the Assets section of that Release page, and any third party receiving these binaries may download it directly from that page.

The Corresponding Source provided in that Release includes:
- `qemu-11.1.1.tar.xz` (official QEMU source, applicable to both Windows and Linux bundles)
- `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz` (packaging script source, applicable only to the Linux AppImage)
