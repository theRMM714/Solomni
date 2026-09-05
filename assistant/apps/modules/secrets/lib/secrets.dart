/// secrets 模块：钥匙圈。名字 + 钥匙 + 备注，只管钥匙，不问去哪扇门。
/// 供应商/地址/模型归消费方（如 llm_gateway）自己的配置；这里零知识。
/// 谁都可以再写一个实现替换它（比如硬件密钥版），核心与其他模块零改动。
library;

import 'dart:convert';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_vocab/ui_vocab.dart';
import 'package:user_data/user_data.dart';

/// 一个钥匙条目：钥匙本体 + 人类备注（管理用，不出现在取钥匙的链路里）
class SecretEntry {
  final String value;
  final String remark;
  const SecretEntry(this.value, this.remark);

  Map<String, Object?> toJson() => {'value': value, 'remark': remark};
}

class SecretsModule {
  /// 未写入的密钥不存在（无默认值：无密钥的后果由消费方自决，见 PRODUCT.md）
  final _store = <String, SecretEntry>{};
  final File _file;
  bool _loaded = false;

  SecretsModule({File? store})
      : _file = store ?? UserData.file('secrets', 'keys.json');

  Future<void> _ensureLoaded() async {
    if (_loaded) return;
    _loaded = true;
    if (await _file.exists()) {
      try {
        final j = jsonDecode(await _file.readAsString()) as Map<String, Object?>;
        for (final e in j.entries) {
          // 兼容旧格式（裸字符串 = 无备注的钥匙）
          if (e.value is Map) {
            final m = Map<String, Object?>.from(e.value as Map);
            _store[e.key] = SecretEntry(
                (m['value'] ?? '').toString(), (m['remark'] ?? '').toString());
          } else {
            _store[e.key] = SecretEntry(e.value.toString(), '');
          }
        }
      } catch (_) {
        // 密钥文件损坏则从空开始，不崩溃
      }
    }
  }

  Future<void> _save() async {
    await _file.parent.create(recursive: true);
    await _file.writeAsString(
        jsonEncode({for (final e in _store.entries) e.key: e.value.toJson()}),
        flush: true);
    if (!Platform.isWindows) {
      // 密钥明文 + 收紧权限：Unix 侧 0600（只有属主可读写）
      await Process.run('chmod', ['600', _file.path]);
    }
  }

  Future<Object?> get(String name) async {
    await _ensureLoaded();
    return _store[name]?.value;
  }

  /// 写入/覆盖；remark 缺省时保留原备注（只换钥匙不清备注）
  Future<Object?> put(String name, String value, {String? remark}) async {
    await _ensureLoaded();
    final old = _store[name];
    _store[name] = SecretEntry(value, remark ?? old?.remark ?? '');
    await _save();
    return null;
  }

  /// 列举：只报名字与备注，绝不出钥匙值（列表不是取钥匙的通道）
  Future<List<Map<String, Object?>>> list() async {
    await _ensureLoaded();
    return [
      for (final e in _store.entries)
        {'name': e.key, 'remark': e.value.remark}
    ];
  }
}

final class SecretsProgram implements ModuleProgram {
  final SecretsModule _inner = SecretsModule();

  @override
  Declaration get declaration => const Declaration(
        'secrets',
        kind: ModuleKind.service,
        provides: [
          Provide(Caps.secretsGet),
          Provide(Caps.secretsPut),
          Provide(Caps.secretsList),
          Provide(UiCaps.contribution),
          Provide(UiCaps.event),
        ],
      );

  /// UI 贡献：密钥表单与列表只在 secrets 自己的私有画布出现
  static const contribution = UiContribution(
    components: [
      UiComponent('key_input', UiKind.formField,
          scope: UiScope.private,
          label: '密钥',
          bind: Caps.secretsPut,
          fields: [
            UiField('name', label: '名字'),
            UiField('value', label: '钥匙', secret: true),
            UiField('remark', label: '备注'),
          ]),
      UiComponent('key_list', UiKind.list,
          scope: UiScope.private, label: '已存密钥', bind: Caps.secretsList),
    ],
    commands: [
      UiCommand('set-key', args: ['name', 'value'],
          description: '写入密钥（备注走表单）', component: 'key_input', event: 'submit'),
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
        return _inner.put(m['name'] as String, m['value'] as String,
            remark: m['remark'] as String?);
      }
      if (env.method == Caps.secretsList) {
        return _inner.list();
      }
      if (env.method == UiCaps.contribution) {
        return contribution.toJson();
      }
      if (env.method == UiCaps.event) {
        final p = env.params as Map;
        if (p['component'] == 'key_input' && p['event'] == 'submit') {
          final payload = (p['payload'] as Map?) ?? const {};
          // 表单带 name/value/remark；CLI 命令带 name/value；单字段旧形态默认写 llm
          final name = (payload['name'] as String? ?? '').trim().isEmpty
              ? 'llm'
              : (payload['name'] as String).trim();
          final value =
              payload['value'] as String? ?? payload['text'] as String? ?? '';
          if (value.isEmpty) return 'empty';
          return _inner
              .put(name, value, remark: payload['remark'] as String?)
              .then((_) => 'ok');
        }
        throw UnsupportedError('secrets 不认识的组件事件: ' + p.toString());
      }
      throw UnsupportedError('secrets 不认识 ' + env.method);
    };
  }
}
