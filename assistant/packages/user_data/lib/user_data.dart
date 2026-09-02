/// 模块私有数据目录约定：数据归模块所有，落在模块自己文件夹内的 userdata/。
/// 锚定模块包自身（package:<id>/ -> lib/ -> ../userdata），
/// 与从哪个 cwd 运行无关，也与核心/产品无关。
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
      final d = Directory(override.replaceAll('\\', '/') + '/' + packageName);
      if (!d.existsSync()) d.createSync(recursive: true);
      return d;
    }
    final lib = Isolate.resolvePackageUriSync(Uri.parse('package:$packageName/'));
    if (lib == null) {
      throw StateError('无法解析模块包根: ' + packageName);
    }
    final d = Directory(lib.resolve('../userdata').toFilePath());
    if (!d.existsSync()) d.createSync(recursive: true);
    return d;
  }

  /// userdata 下的一个文件（父目录自动创建）
  static File file(String packageName, String name) =>
      File(dir(packageName).path + Platform.pathSeparator + name);
}
