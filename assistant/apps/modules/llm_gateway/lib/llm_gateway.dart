/// llm_gateway 模块：愿意把对话能力当共同物的普通模块。
/// 真 OpenAI 兼容流式调用（SSE）；供应商（地址/模型）归本模块自己配置，
/// 钥匙经 prefer-shared 从 secrets 取（钥匙圈不管门）。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_vocab/ui_vocab.dart';
import 'package:user_data/user_data.dart';

/// 本模块的供应商配置：userdata/config.json = {"baseUrl": "...", "model": "..."}。
/// 优先级：构造参数 > 环境变量（临时覆盖）> 配置文件 > 内建默认。
Map<String, String> _fileConfig() {
  try {
    final f = UserData.file('llm_gateway', 'config.json');
    if (!f.existsSync()) return const {};
    final j = jsonDecode(f.readAsStringSync()) as Map<String, Object?>;
    return {
      if (j['baseUrl'] is String) 'baseUrl': j['baseUrl'] as String,
      if (j['model'] is String) 'model': j['model'] as String,
    };
  } catch (_) {
    return const {}; // 配置损坏则忽略，走环境变量/默认
  }
}

/// 单机出边：一切调用都抛 NoProvider，消费方自然走内建
final class BuiltinOnlyOutbound implements Outbound {
  @override
  Future<Object?> rpc(String method, Object? params) async {
    throw NoProviderException(method);
  }

  @override
  Stream<Object?> rpcStream(String method, Object? params) async* {
    throw NoProviderException(method);
  }

  @override
  Future<Object?> call(String moduleId, String method, Object? params) async {
    throw NoProviderException(method);
  }

  @override
  Stream<WiringSnapshot> get wiring => const Stream.empty();
}

Future<String?> _apiKey(Outbound out) async {
  try {
    return await out.rpc(Caps.secretsGet, 'llm') as String?;
  } on NoProviderException {
    return null; // secrets 不在场：同样视为未配置（模块自决拒绝）
  }
}

/// OpenAI 兼容流式调用：SSE data 行 -> delta token，[DONE] 结束。
/// baseUrl/model 可配（参数或环境变量 LLM_BASE_URL/LLM_MODEL）。
Stream<String> gatewayChatStream(Outbound out, List<dynamic> messages,
    {String? baseUrl, String? model}) async* {
  final url = (baseUrl ??
          Platform.environment['LLM_BASE_URL'] ??
          'https://api.deepseek.com') +
      '/chat/completions';
  final m = model ?? Platform.environment['LLM_MODEL'] ?? 'deepseek-chat';
  final key = await _apiKey(out);
  // 无密钥直接拒绝（拿不到密钥就不发起调用）：聊天链路自决，宿主不参与
  if (key == null || key.isEmpty) {
    throw StateError('未配置密钥 llm，无法调用 LLM');
  }
  // dart:io HttpClient：SDK 自带，仓库保持零 hosted 依赖（package_config 全相对路径）
  final client = HttpClient();
  try {
    final req = await client.postUrl(Uri.parse(url));
    req.headers.set('Authorization', 'Bearer ' + key);
    req.headers.set('Content-Type', 'application/json');
    // 显式 UTF-8 字节 + 定长：write(String) 会按槽默认 latin1 编码，中文炸
    final body = utf8.encode(jsonEncode(
        {'model': m, 'messages': messages, 'stream': true}));
    req.contentLength = body.length;
    req.add(body);
    final resp = await req.close();
    if (resp.statusCode != 200) {
      final body = await resp.transform(utf8.decoder).join();
      throw StateError('LLM HTTP ' + resp.statusCode.toString() + ': ' + body);
    }
    await for (final line in resp
        .transform(utf8.decoder)
        .transform(const LineSplitter())) {
      if (!line.startsWith('data:')) continue;
      final data = line.substring(5).trim();
      if (data == '[DONE]') return;
      final j = jsonDecode(data) as Map;
      final choices = j['choices'] as List?;
      if (choices == null || choices.isEmpty) continue;
      final delta = (choices.first as Map)['delta'] as Map?;
      final content = delta?['content'];
      if (content is String && content.isNotEmpty) yield content;
    }
  } finally {
    client.close(force: true); // 消费方中途取消也保证回收
  }
}

final class LlmGatewayProgram implements ModuleProgram {
  final String? baseUrl; // 显式覆盖（测试用）；一般走 配置文件/环境变量/默认
  final String? model;

  LlmGatewayProgram({this.baseUrl, this.model});

  /// 供应商配置能力（私有词汇：llm_gateway.*）
  static const configCap = 'llm_gateway.config';

  @override
  Declaration get declaration => const Declaration(
        'llm_gateway',
        kind: ModuleKind.service,
        provides: [
          Provide(Caps.llmChat),
          Provide(UiCaps.contribution),
          Provide(UiCaps.event),
        ],
        needs: [Need(Caps.secretsGet, NeedVia.preferShared)],
      );

  /// UI 贡献：供应商配置台只在 llm_gateway 自己的私有画布出现
  static const contribution = UiContribution(
    components: [
      UiComponent('provider_input', UiKind.formField,
          scope: UiScope.private,
          label: '供应商',
          bind: configCap,
          fields: [
            UiField('baseUrl', label: '供应商地址'),
            UiField('model', label: '模型'),
          ]),
      UiComponent('provider_view', UiKind.textOutput,
          scope: UiScope.private, label: '当前配置', bind: configCap),
    ],
    commands: [
      UiCommand('set-provider', args: ['baseUrl', 'model'],
          description: '设置供应商地址与模型（缺模型保留原值）',
          component: 'provider_input', event: 'submit'),
    ],
  );

  /// 端点实时解析：配置文件被表单/CLI 改动后，下一次调用即刻生效。
  /// 优先级：显式参数 > 环境变量（临时覆盖）> 配置文件 > 内建默认。
  ({String url, String model}) endpoint() => (
        url: baseUrl ??
            Platform.environment['LLM_BASE_URL'] ??
            _fileConfig()['baseUrl'] ??
            'https://api.deepseek.com',
        model: model ??
            Platform.environment['LLM_MODEL'] ??
            _fileConfig()['model'] ??
            'deepseek-chat',
      );

  /// 保存供应商配置（模型缺省时保留原值）
  Future<String> _saveProvider(String baseUrl, String model) async {
    final f = UserData.file('llm_gateway', 'config.json');
    await f.parent.create(recursive: true);
    await f.writeAsString(
        jsonEncode({
          'baseUrl': baseUrl,
          'model': model.isEmpty ? (_fileConfig()['model'] ?? '') : model,
        }),
        flush: true);
    return 'ok';
  }

  Future<String> _configSummary() async {
    final c = _fileConfig();
    return '供应商地址: ' +
        (c['baseUrl'] ?? '（未设置：走环境变量或默认）') +
        '\n模型: ' +
        (c['model'] ?? '（未设置：走环境变量或默认）');
  }

  @override
  ModuleHandler bind(Outbound outbound) {
    return (env) async {
      if (env.method == Caps.llmChat) {
        final messages = (env.params as Map)['messages'] as List;
        final ep = endpoint();
        return Streamed(gatewayChatStream(outbound, messages,
            baseUrl: ep.url, model: ep.model));
      }
      if (env.method == configCap) {
        return _configSummary();
      }
      if (env.method == UiCaps.contribution) {
        return contribution.toJson();
      }
      if (env.method == UiCaps.event) {
        final p = env.params as Map;
        if (p['component'] == 'provider_input' && p['event'] == 'submit') {
          final payload = (p['payload'] as Map?) ?? const {};
          final baseUrl = (payload['baseUrl'] as String? ?? '').trim();
          if (baseUrl.isEmpty) return 'empty';
          return _saveProvider(
              baseUrl, (payload['model'] as String? ?? '').trim());
        }
        throw UnsupportedError('llm_gateway 不认识的组件事件: ' + p.toString());
      }
      throw UnsupportedError('llm_gateway 不认识 ' + env.method);
    };
  }
}
