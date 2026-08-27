/// 单机入口：无核心，直接读写。
library;

import 'package:secrets/secrets.dart';

Future<void> main() async {
  final s = SecretsModule();
  await s.put('demo', 'value-123');
  print('[secrets] demo = ' + ((await s.get('demo')) as String));
}
