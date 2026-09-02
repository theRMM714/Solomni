/// llm_gateway 模块：愿意把对话能力当共同物的普通模块。
/// 真 OpenAI 兼容流式调用（SSE）；密钥经 prefer-shared 从 secrets 取。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

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
  final String baseUrl;
  final String model;

  LlmGatewayProgram({String? baseUrl, String? model})
      : baseUrl = baseUrl ?? Platform.environment['LLM_BASE_URL'] ?? 'https://api.deepseek.com',
        model = model ?? Platform.environment['LLM_MODEL'] ?? 'deepseek-chat';

  @override
  Declaration get declaration => const Declaration(
        'llm_gateway',
        kind: ModuleKind.service,
        provides: [Provide(Caps.llmChat)],
        needs: [Need(Caps.secretsGet, NeedVia.preferShared)],
      );

  @override
  ModuleHandler bind(Outbound outbound) {
    return (env) async {
      if (env.method == Caps.llmChat) {
        final messages = (env.params as Map)['messages'] as List;
        return Streamed(
            gatewayChatStream(outbound, messages, baseUrl: baseUrl, model: model));
      }
      throw UnsupportedError('llm_gateway 不认识 ' + env.method);
    };
  }
}
