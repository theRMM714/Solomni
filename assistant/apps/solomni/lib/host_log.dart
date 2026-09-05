/// 宿主运行日志：每次启动器拉起产品，按时间戳在仓库根 logs/ 建一份。
/// 日志是宿主声明的能力（logs.append）：模块经 Need(preferShared) 消费，
/// 永不直接摸文件；宿主自身事件作为提供方直写（能力实现的一部分）。
library;

import 'dart:io';
import 'dart:isolate';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

class HostLog {
  static File? _file;

  /// 启动时建文件并写头一行；日志不可用不阻断产品（控制台照常）
  static void init() {
    try {
      final f = File(_newPath());
      if (!f.parent.existsSync()) f.parent.createSync(recursive: true);
      if (!f.existsSync()) f.createSync();
      _file = f;
      write('启动 pid=' + pid.toString() +
          ' 平台=' + Platform.operatingSystem);
    } catch (_) {
      _file = null;
    }
  }

  /// 仓库根/logs/<yyyyMMdd-HHmmss>.log（时间戳作文件名，Windows 无冒号）
  static String _newPath() {
    final self = Isolate.resolvePackageUriSync(
        Uri.parse('package:solomni/assembler.dart'))!;
    final root = self.resolve('../../../../').toFilePath(); // 仓库根
    final now = DateTime.now();
    String two(int v) => v.toString().padLeft(2, '0');
    final name = now.year.toString() +
        two(now.month) + two(now.day) + '-' +
        two(now.hour) + two(now.minute) + two(now.second) + '.log';
    return root + 'logs' + Platform.pathSeparator + name;
  }

  /// 追加一行事实（自带时:分:秒前缀）；写失败静默
  static void write(String msg) {
    final f = _file;
    if (f == null) return;
    try {
      final n = DateTime.now();
      String two(int v) => v.toString().padLeft(2, '0');
      f.writeAsStringSync(
          '[' + two(n.hour) + ':' + two(n.minute) + ':' + two(n.second) + '] ' +
              msg + '\n',
          mode: FileMode.append);
    } catch (_) {
      // 日志写失败不影响产品
    }
  }
}

/// logs 能力的提供方：宿主以普通模块的身份注册（hello 经协议到达核心）。
/// 其它模块 Need(Caps.logsAppend, preferShared) 即可使用；无提供方自动降级。
/// 载荷 = {'from': <发送方 id>, 'text': <内容>}——TCP 送达模块的封套
/// 一律由核心伪装 from='core'（既有投递约定），发送方身份须载荷自带。
final class HostLogProgram implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration(
        'logs',
        kind: ModuleKind.service,
        provides: [Provide(HostCaps.logsAppend)],
      );

  @override
  ModuleHandler bind(Outbound outbound) {
    return (env) async {
      if (env.method == HostCaps.logsAppend) {
        final p = env.params as Map?;
        HostLog.write((p?['from'] ?? env.from).toString() +
            ': ' +
            ((p?['text'] ?? '').toString()));
        return 'ok';
      }
      throw UnsupportedError('logs 不认识 ' + env.method);
    };
  }
}
