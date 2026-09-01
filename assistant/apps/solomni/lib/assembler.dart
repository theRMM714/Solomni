/// 产品装配：发现 -> 注册 -> 连接 -> 装配期校验。
/// 本文件是产品的唯一策略所在：扫描哪里、编译进了谁、排除谁。
/// 核心不读任何文件（原 loader 由核心包迁入产品，核心包只剩校验/配对/投递）。
library;

import 'dart:io';
import 'dart:isolate';
import 'package:core/core.dart';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:transport/transport.dart';
import 'package:conversation/conversation.dart';
import 'package:llm_gateway/llm_gateway.dart';
import 'package:secrets/secrets.dart';
import 'package:ui_cli/ui_cli.dart';

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
/// 约定：<root>/<id>/bin/coordinated.dart 即模块守护入口。
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

/// 产品内编译进的模块工厂（进程内组合是编译期引用，Dart 硬约束）。
/// 不在此列的文件夹 = 外部模块，只能作为独立进程拉起（见 -gui）。
final registry = <String, ModuleProgram Function()>{
  'conversation': ConversationProgram.new,
  'llm_gateway': LlmGatewayProgram.new,
  'secrets': SecretsProgram.new,
  'ui_cli': UiCliProgram.new,
};

/// 用户的替身：也是个普通模块，只消费 ui.render/ui.command/chatSend。
/// REPL（活人驱动）与验收脚本（脚本驱动）共用本声明。
final class DriverProgram implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration('driver', needs: [
        Need('ui.render', NeedVia.preferShared),
        Need('ui.command', NeedVia.preferShared),
        Need(Caps.chatSend, NeedVia.preferShared),
        Need(Caps.chatHistory, NeedVia.preferShared),
      ]);

  @override
  ModuleHandler bind(Outbound outbound) =>
      (env) => throw UnsupportedError('driver 不提供服务');
}

/// 装配结果：常驻的核心 + 已连接的替身 + 全部发现的文件夹。
class Assembled {
  final BrokerDaemon daemon;
  final ModuleClient driver;
  final List<ModuleFolder> folders;
  final Set<String> excluded;
  Assembled(this.daemon, this.driver, this.folders, this.excluded);

  /// 可拉起的外部模块：未编译进本产品、且未被 --without 排除
  List<ModuleFolder> get spawnable => [
        for (final f in folders)
          if (!registry.containsKey(f.id) && !excluded.contains(f.id)) f
      ];
}

/// 模块区的物理位置：锚定在本包自身（与从哪个入口运行无关）
String defaultModulesDir() {
  final self =
      Isolate.resolvePackageUriSync(Uri.parse('package:solomni/assembler.dart'));
  return self!.resolve('../../modules').toFilePath();
}

/// 装配：目录发现 -> 逐模块连接（hello 经协议到达）-> 替身连入 ->
/// 装配期校验（显式接线指向未知则失败；配对随成员变化自动维护）。
Future<Assembled> assemble({
  Set<String> excluded = const {},
  int port = 0,
  bool verbose = false,
  String? modulesDir,
}) async {
  final folders = scanModules(modulesDir ?? defaultModulesDir());
  print('[发现] ' + folders.map((f) => f.id).join(', '));
  final daemon = await BrokerDaemon.bind(InternetAddress.loopbackIPv4, port);
  for (final f in folders) {
    if (excluded.contains(f.id)) {
      print('[卸载] ' + f.id + ' 被排除（文件夹保留，可随时恢复）');
      continue;
    }
    final make = registry[f.id];
    if (make == null) {
      // 外部模块：不在进程内拉起，由 -gui 按需拉起为独立进程
      if (verbose) print('[外部] ' + f.id + ' 未编译进本产品');
      continue;
    }
    await ModuleClient.connect(daemon.address, daemon.port, make());
    if (verbose) print('[连接] ' + f.id);
  }
  // 消费替身连入：声明到达即配对（动态配对下接入顺序无关）
  final driver =
      await ModuleClient.connect(daemon.address, daemon.port, DriverProgram());
  await daemon.start();
  return Assembled(daemon, driver, folders, excluded);
}
