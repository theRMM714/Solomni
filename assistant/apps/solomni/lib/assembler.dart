/// 核心宿主：装配 = 发现 -> 拉起 service -> 极简菜单（唤起 surface）-> 退出。
/// 「拉起谁」的决策在核心侧（宿主），不在产品入口：产品入口只剩「拉宿主」一个动作。
/// 纯 broker（packages/core）不读文件、不起进程——扫描/拉起/退出是宿主的职责。
library;

import 'dart:io';
import 'dart:isolate';
import 'package:core/core.dart';
import 'package:protocol/protocol.dart';

/// 一个可装卸单元：modules/ 下一个文件夹（id = 文件夹名）
class ModuleFolder {
  final String id;
  final String path;
  const ModuleFolder(this.id, this.path);
  @override
  String toString() => id;
}

/// 目录发现：扫描模块区，回答"这里有哪些可装卸的模块文件夹"。
/// 只认文件夹与入口约定，不读能力——能力由 hello 声明经协议到达核心。
/// 约定：<root>/<id>/bin/coordinated.dart 即模块入口。
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

/// 模块区的物理位置：锚定在本包自身（与从哪个入口运行无关）
String defaultModulesDir() {
  final self =
      Isolate.resolvePackageUriSync(Uri.parse('package:solomni/assembler.dart'));
  return self!.resolve('../../modules').toFilePath();
}

/// 是否有 surface 启动契约（dev 脚本存在性即「怎么跑」的模块自决声明）。
/// 有 dev 契约 = 交互面（按需唤起）；无 = 无头 service（默认 dart 拉起）。
bool hasLaunchContract(ModuleFolder f) =>
    File(f.path + (Platform.isWindows ? r'\dev.bat' : '/dev')).existsSync();

/// 拉起 service 模块：无特殊依赖，用默认 dart 跑守护入口；cwd = 模块自己文件夹。
Future<Process> spawnService(ModuleFolder f, int port) =>
    Process.start(Platform.resolvedExecutable,
        ['bin/coordinated.dart', '--port=' + port.toString()],
        workingDirectory: f.path, mode: ProcessStartMode.inheritStdio);

/// 拉起 surface 模块：只认模块自备的 dev 启动契约脚本，端口注入。
/// 怎么跑（依赖自愈/设备/工具链）是模块自己的事，宿主一概不知。
/// Windows 的 .bat 必须经 cmd.exe 执行。
Future<Process> spawnSurface(ModuleFolder f, int port) {
  final args = ['--port=' + port.toString()];
  if (Platform.isWindows) {
    return Process.start('cmd.exe', ['/c', f.path + r'\dev.bat', ...args],
        workingDirectory: f.path, mode: ProcessStartMode.inheritStdio);
  }
  return Process.start(f.path + '/dev', args,
      workingDirectory: f.path, mode: ProcessStartMode.inheritStdio);
}

/// 终止模块进程：Windows 下 cmd 的子进程不随父死，须杀进程树
Future<void> killProcess(Process p) async {
  if (Platform.isWindows) {
    await Process.run('taskkill', ['/PID', p.pid.toString(), '/T', '/F']);
  } else {
    p.kill();
  }
}

/// 装配结果：常驻核心 + 已拉起的 service + 全部发现的文件夹。
class Assembled {
  final BrokerDaemon daemon;
  final List<ModuleFolder> folders;
  final Set<String> excluded;
  final List<ModuleFolder> serviceFolders;
  final List<Process> services;
  Assembled(this.daemon, this.folders, this.excluded, this.serviceFolders,
      this.services);

  /// 可唤起的 surface：有 dev 启动契约、未被 --without 排除
  List<ModuleFolder> get surfaces => [
        for (final f in folders)
          if (!excluded.contains(f.id) && hasLaunchContract(f)) f
      ];

  /// 等一个模块 id 的声明到达（hello 经协议到达，宿主据此校验身份，事实非决策）
  Future<Declaration?> waitDeclaration(String id,
      {Duration timeout = const Duration(seconds: 8)}) async {
    final sw = Stopwatch()..start();
    while (sw.elapsed < timeout) {
      for (final d in daemon.declarations) {
        if (d.id == id) return d;
      }
      await Future.delayed(const Duration(milliseconds: 100));
    }
    return null;
  }

  /// 等全部 service 到场（它们的 hello 到达核心），超时照实警告但不崩
  Future<void> waitServices(
      {Duration timeout = const Duration(seconds: 8)}) async {
    final sw = Stopwatch()..start();
    while (sw.elapsed < timeout) {
      final ids = daemon.declarations.map((d) => d.id).toSet();
      if (serviceFolders.every((f) => ids.contains(f.id))) return;
      await Future.delayed(const Duration(milliseconds: 100));
    }
    final ids = daemon.declarations.map((d) => d.id).toSet();
    for (final f in serviceFolders) {
      if (!ids.contains(f.id)) {
        stderr.writeln('[警告] service ' + f.id + ' 未在时限内连接');
      }
    }
  }
}

/// 装配：目录发现 -> 拉起全部 service（进程，cwd=自己文件夹）-> 核心常驻。
/// surface 不自动拉起，由菜单按需唤起。
Future<Assembled> assemble({
  Set<String> excluded = const {},
  int port = 0,
  bool verbose = false,
  String? modulesDir,
}) async {
  final folders = scanModules(modulesDir ?? defaultModulesDir());
  print('[发现] ' + folders.map((f) => f.id).join(', '));
  final daemon = await BrokerDaemon.bind(InternetAddress.loopbackIPv4, port);
  final serviceFolders = <ModuleFolder>[];
  final services = <Process>[];
  for (final f in folders) {
    if (excluded.contains(f.id)) {
      print('[卸载] ' + f.id + ' 被排除（文件夹保留，可随时恢复）');
      continue;
    }
    if (hasLaunchContract(f)) {
      // surface：按需唤起，不自动拉起
      if (verbose) print('[界面] ' + f.id + '（菜单唤起）');
      continue;
    }
    serviceFolders.add(f);
    services.add(await spawnService(f, daemon.port));
    if (verbose) print('[拉起] service ' + f.id);
  }
  await daemon.start();
  return Assembled(daemon, folders, excluded, serviceFolders, services);
}
