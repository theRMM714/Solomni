# Third-Party Notices

本文件记录了本项目自托管分发的第三方依赖包，以及它们的许可证和对应源码信息。

---

## 1. QEMU (AppImage 二进制)

- **Binary Asset**: `QEMU-11.1.1-4-anylinux-x86_64.AppImage`
- **Version**: QEMU 11.1.1-4 (pkgforge 打包版本)
- **License**: GPL-2.0 (GNU General Public License v2.0)
- **Corresponding Source Code**:
  - QEMU 官方源码: `qemu-11.1.1.tar.xz` (已随本 Release 提供)
  - 打包脚本源码: `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz` (已随本 Release 提供)
- **Written Offer**: 对于包含 GPL 授权的二进制，项目在此提供有效期至少三年的书面 Offer。任何第三方可通过本页面获取完整的对应源码。由于对应源码已随本 Release 直接提供，通常无需通过书面 Offer 获取。

## 2. virtiofsd (Rust 版)

- **Binary Asset**: `virtiofsd-4bfa7125.zip`
- **Version**: v1.14.0 (commit 4bfa7125)
- **License**: Apache-2.0 (另有上游 BSD-3-Clause 双许可)
- **Source Code**: https://gitlab.com/virtio-fs/virtiofsd
- **Note**: 根据 Apache-2.0 许可证要求，已附上许可证副本。无 GPL 源码提供义务。

---

## 附：GPL 源码获取说明

对于本 Release 中包含 GPL 授权的二进制文件（目前为 QEMU AppImage），根据 GPLv2 第 3(a) 条，其完整的对应源码已在本 Release 页面的资产（Assets）中提供，任何接收该二进制文件的第三方均可直接从本页面下载获取。

本 Release 提供的对应源码包括：
- `qemu-11.1.1.tar.xz`（QEMU 官方源码）
- `QEMU-AppImage-11.1.1-4-2026-10-01_1790875330.tar.gz`（打包脚本源码）
