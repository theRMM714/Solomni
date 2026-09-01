/// secrets 模块：一个愿意提供密钥能力的普通模块。
/// 谁都可以再写一个实现替换它（比如硬件密钥版），核心与其他模块零改动。
library;

import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_vocab/ui_vocab.dart';

class SecretsModule {
  /// 未写入的密钥不存在（无默认值：无密钥的后果由消费方自决，见 PRODUCT.md）
  final _store = <String, String>{};

  Future<Object?> get(String name) async => _store[name];
  Future<Object?> put(String name, String value) async {
    _store[name] = value;
    return null;
  }
}

final class SecretsProgram implements ModuleProgram {
  final SecretsModule _inner = SecretsModule();

  @override
  Declaration get declaration => const Declaration(
        'secrets',
        provides: [Provide(Caps.secretsGet), Provide(Caps.secretsPut), Provide(UiCaps.contribution), Provide(UiCaps.event)],
      );

  /// UI 贡献：密钥表单只在 secrets 自己的私有画布出现
  static const contribution = UiContribution(
    components: [
      UiComponent('key_input', UiKind.formField, scope: UiScope.private, label: 'API Key', bind: Caps.secretsPut),
      UiComponent('key_list', UiKind.list, scope: UiScope.private, label: '已存密钥', bind: Caps.secretsGet),
    ],
    commands: [
      UiCommand('set-key', args: ['name', 'value'], description: '写入密钥', component: 'key_input', event: 'submit'),
    ],
  );

  @override
  ModuleHandler bind(Outbound outbound) {
    return (env) async {
      if (env.method == Caps.secretsGet) {
        return _inner.get(env.params as String);
      }
      if (env.method == Caps.secretsPut) {
        final m = env.params as Map;
        return _inner.put(m['name'] as String, m['value'] as String);
      }
      if (env.method == UiCaps.contribution) {
        return contribution.toJson();
      }
      if (env.method == UiCaps.event) {
        final p = env.params as Map;
        if (p['component'] == 'key_input' && p['event'] == 'submit') {
          final payload = (p['payload'] as Map?) ?? const {};
          // CLI 命令带 name/value；UI 表单单字段 text 时默认写 llm
          final name = payload['name'] as String? ?? 'llm';
          final value =
              payload['value'] as String? ?? payload['text'] as String? ?? '';
          if (value.isEmpty) return 'empty';
          return _inner.put(name, value).then((_) => 'ok');
        }
        throw UnsupportedError('secrets 不认识的组件事件: ' + p.toString());
      }
      throw UnsupportedError('secrets 不认识 ' + env.method);
    };
  }
}
