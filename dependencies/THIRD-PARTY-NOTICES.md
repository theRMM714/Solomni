# Third-Party Notices

本文件记录了本项目自托管分发的第三方依赖包，以及它们的许可证和对应源码信息。

---

## 1. QEMU (Linux AppImage 二进制)

- **Binary Asset**: `QEMU-11.1.1-4-anylinux-x86_64.AppImage`
- **Version**: QEMU 11.1.1-4 (pkgforge 打包版本)
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**:
  - QEMU 官方源码: `qemu-11.1.1.tar.xz` (已随本 Release 提供)
  - 打包脚本源码: `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz` (已随本 Release 提供)
- **Note**: 对应源码已随本 Release 直接提供，通常无需通过书面 Offer 获取。

## 2. QEMU (Windows 便携包)

- **Binary Asset**: `qemu-windows-x86_64-11.1.1.tar.zst`
- **Version**: QEMU 11.1.1
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**: `qemu-11.1.1.tar.xz` (已随本 Release 提供)
- **编译方式**: 从 QEMU 官方源码 `qemu-11.1.1.tar.xz`，在 Arch Linux 上使用 MinGW-w64 交叉编译工具链编译生成。
- **打包的依赖库**（均为独立许可证，不影响 QEMU 自身的 GPL 状态）：
  - `libglib-2.0-0.dll` / `libgio-2.0-0.dll` / `libgobject-2.0-0.dll` / `libgmodule-2.0-0.dll` — LGPL-2.1
  - `libpixman-1-0.dll` — MIT
  - `SDL2.dll` — zlib
  - `zlib1.dll` — zlib
  - `libffi-8.dll` — MIT
  - `libpcre2-8-0.dll` — BSD-3-Clause
  - `libiconv-2.dll` / `libintl-8.dll` — LGPL-2.1
  - `libwinpthread-1.dll` / `libgcc_s_seh-1.dll` / `libssp-0.dll` — GCC Runtime Library Exception
- **Note**: 对应源码已随本 Release 直接提供，通常无需通过书面 Offer 获取。

## 3. virtiofsd (Rust 版)

- **Binary Asset**: `virtiofsd-4bfa7125.zip`
- **Version**: v1.14.0 (commit 4bfa7125)
- **License**: Apache-2.0 (另有上游 BSD-3-Clause 双许可)
- **Source Code**: https://gitlab.com/virtio-fs/virtiofsd
- **Note**: 根据 Apache-2.0 许可证要求，已附上许可证副本。无 GPL 源码提供义务。

---

## 附：GPL 源码获取说明

对于本 Release 中包含 GPL 授权的二进制文件（目前为 QEMU Linux AppImage 和 QEMU Windows 便携包），根据 GPLv2 第 3(a) 条，其完整的对应源码已在本 Release 页面的资产（Assets）中提供，任何接收该二进制文件的第三方均可直接从本页面下载获取。

本 Release 提供的对应源码包括：
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
  - Official QEMU Source: `qemu-11.1.1.tar.xz` (Provided as part of this Release)
  - Packaging Script Source: `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz` (Provided as part of this Release)
- **Note**: The Corresponding Source is already provided directly in this Release.

## 2. QEMU (Windows Portable Bundle)

- **Binary Asset**: `qemu-windows-x86_64-11.1.1.tar.zst`
- **Version**: QEMU 11.1.1
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**: `qemu-11.1.1.tar.xz` (Provided as part of this Release)
- **Build Method**: Cross-compiled from the official QEMU source `qemu-11.1.1.tar.xz` on Arch Linux using the MinGW-w64 cross-compilation toolchain.
- **Bundled Dependency Libraries** (independent licenses; do not affect QEMU's own GPL status):
  - `libglib-2.0-0.dll` / `libgio-2.0-0.dll` / `libgobject-2.0-0.dll` / `libgmodule-2.0-0.dll` — LGPL-2.1
  - `libpixman-1-0.dll` — MIT
  - `SDL2.dll` — zlib
  - `zlib1.dll` — zlib
  - `libffi-8.dll` — MIT
  - `libpcre2-8-0.dll` — BSD-3-Clause
  - `libiconv-2.dll` / `libintl-8.dll` — LGPL-2.1
  - `libwinpthread-1.dll` / `libgcc_s_seh-1.dll` / `libssp-0.dll` — GCC Runtime Library Exception
- **Note**: The Corresponding Source is already provided directly in this Release.

## 3. virtiofsd (Rust Version)

- **Binary Asset**: `virtiofsd-4bfa7125.zip`
- **Version**: v1.14.0 (commit 4bfa7125)
- **License**: Apache-2.0 (also dual-licensed upstream under BSD-3-Clause)
- **Source Code**: https://gitlab.com/virtio-fs/virtiofsd
- **Note**: A copy of the license is included as required by the Apache-2.0 License. There is no GPL source provision obligation.

---

## Appendix: GPL Source Availability Statement

For GPL-licensed binaries included in this Release (currently the QEMU Linux AppImage and the QEMU Windows portable bundle), pursuant to GPLv2 Section 3(a), the complete Corresponding Source is provided in the Assets section of this Release page, and any third party receiving these binaries may download it directly from this page.

The Corresponding Source provided in this Release includes:
- `qemu-11.1.1.tar.xz` (official QEMU source, applicable to both Windows and Linux bundles)
- `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz` (packaging script source, applicable only to the Linux AppImage)
