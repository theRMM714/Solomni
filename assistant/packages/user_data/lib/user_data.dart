/// 模块私有数据目录约定：数据归模块所有，落在模块自己文件夹内的 userdata/。
/// 锚定模块包自身（package:<id>/ -> lib/ -> ../userdata），
/// 与从哪个 cwd 运行无关，也与核心/产品无关。
/// Flutter 运行时没有 package_config（resolvePackageUriSync 返回 null），
/// 此时退化为当前目录下的 userdata/（仅影响落盘位置不影响功能）；
/// Flutter 模块请由 dev 契约显式注入模块目录，勿依赖此退化路径。
library;

import 'dart:io';
import 'dart:isolate';

abstract final class UserData {
  /// <模块根>/userdata 目录（不存在时创建）。
  /// 默认落在模块自己文件夹内；设 SOLOMNI_USERDATA 环境变量可整体搬迁
  /// （验收/隔离用，避免碰真实数据）。
  static Directory dir(String packageName) {
    final override = Platform.environment['SOLOMNI_USERDATA'];
    if (override != null && override.isNotEmpty) {
      return _ensure(Directory(
          override.replaceAll('\\', '/') + '/' + packageName));
    }
    // 纯 Dart 运行时：按包根锚定；Flutter 运行时返回 null -> 退化当前目录
    final lib = Isolate.resolvePackageUriSync(Uri.parse('package:$packageName/'));
    if (lib == null) {
      return _ensure(Directory('userdata'));
    }
    return _ensure(Directory(lib.resolve('../userdata').toFilePath()));
  }

  static Directory _ensure(Directory d) {
    if (!d.existsSync()) d.createSync(recursive: true);
    return d;
  }

  /// userdata 下的一个文件（父目录自动创建）
  static File file(String packageName, String name) =>
      File(dir(packageName).path + Platform.pathSeparator + name);
}
