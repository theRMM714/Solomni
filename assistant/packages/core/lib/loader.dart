/// 目录发现：扫描模块区，回答"这里有哪些可装卸的模块文件夹"。
/// 只认文件夹与入口约定，不读能力--能力由 hello 声明经协议到达核心。
library;

import 'dart:io';

class ModuleFolder {
  final String id;
  final String path;
  const ModuleFolder(this.id, this.path);
  @override
  String toString() => id;
}

/// 约定：<root>/<id>/bin/coordinated.dart 即模块守护入口
List<ModuleFolder> scanModules(String rootDir) {
  final dir = Directory(rootDir);
  if (!dir.existsSync()) return const [];
  return [
    for (final e in dir.listSync())
      if (e is Directory &&
          File(e.path + '/bin/coordinated.dart').existsSync())
        ModuleFolder(
            e.uri.pathSegments.where((s) => s.isNotEmpty).last, e.path),
  ];
}
