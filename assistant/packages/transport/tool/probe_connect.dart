/// 连接探针：以指定模块 id 接入核心，验证外部进程拓扑。
/// 用法：dart probe_connect.dart --id=ui_flutter --port=9200
library;

import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:transport/transport.dart';

Future<void> main(List<String> args) async {
  var id = 'probe';
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--id=')) id = a.substring(5);
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  final program = _ProbeProgram(id);
  try {
    final c = await ModuleClient.connect(
        InternetAddress.loopbackIPv4, port, program);
    print('[probe] ' + id + ' 已接入核心 :' + port.toString());
    await c.socket.close();
  } catch (e) {
    print('[probe] ' + id + ' 被拒绝: ' + e.toString());
    exit(2);
  }
  exit(0);
}

final class _ProbeProgram implements ModuleProgram {
  final String _id;
  _ProbeProgram(this._id);

  @override
  Declaration get declaration => Declaration(_id);

  @override
  ModuleHandler bind(Outbound outbound) =>
      (env) async => throw UnsupportedError('probe 不提供服务');
}
