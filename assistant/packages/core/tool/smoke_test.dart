/// 骨架冒烟测试：与 test/broker_test.dart 同一套用例的断言版。
/// 环境限制（无子进程管道）下用 dart 直接 VM 调用执行。
library;

import 'package:core/core.dart';
import 'package:protocol/protocol.dart';

int _passed = 0;

void check(String name, bool cond) {
  if (!cond) {
    throw StateError('FAILED: ' + name);
  }
  _passed++;
  print('PASS: ' + name);
}

Broker brokerOf(List<Declaration> ds) {
  final b = Broker();
  for (final d in ds) {
    b.register(d);
  }
  return b;
}

void main() {
  // 1. preferShared 无提供方 -> 内建兜底
  final w1 = brokerOf([
    const Declaration('conversation', needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
  ]).resolve();
  check('preferShared 无提供方 -> 内建兜底', w1['conversation']![Caps.llmChat] == '');

  // 2. 唯一提供方 -> 接线
  final w2 = brokerOf([
    const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
    const Declaration('conversation', needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
  ]).resolve();
  check('唯一提供方 -> 接线', w2['conversation']![Caps.llmChat] == 'llm_gateway');

  // 3. 多候选 -> fail-fast
  var threw = false;
  try {
    brokerOf([
      const Declaration('llm_a', provides: [Provide(Caps.llmChat)]),
      const Declaration('llm_b', provides: [Provide(Caps.llmChat)]),
      const Declaration('conversation', needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
    ]).resolve();
  } on WiringException {
    threw = true;
  }
  check('多候选 -> fail-fast', threw);

  // 4. 显式接线优先
  final w4 = brokerOf([
    const Declaration('llm_a', provides: [Provide(Caps.llmChat)]),
    const Declaration('llm_b', provides: [Provide(Caps.llmChat)]),
    const Declaration('conversation', needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
  ]).resolve(explicit: {
    'conversation': {Caps.llmChat: 'llm_b'}
  });
  check('显式接线优先', w4['conversation']![Caps.llmChat] == 'llm_b');

  // 5. sharedOnly 无提供方 -> 失败
  threw = false;
  try {
    brokerOf([
      const Declaration('ui', needs: [Need('stt.transcribe', NeedVia.sharedOnly)]),
    ]).resolve();
  } on WiringException {
    threw = true;
  }
  check('sharedOnly 无提供方 -> 失败', threw);

  // 6. 成环 -> 失败
  threw = false;
  try {
    brokerOf([
      const Declaration('a', provides: [Provide('cap.y')], needs: [Need('cap.x', NeedVia.preferShared)]),
      const Declaration('b', provides: [Provide('cap.x')], needs: [Need('cap.y', NeedVia.preferShared)]),
    ]).resolve();
  } on WiringException {
    threw = true;
  }
  check('成环 -> 失败', threw);

  // 7. builtinOnly 不成环
  final w7 = brokerOf([
    const Declaration('a', provides: [Provide('cap.y')], needs: [Need('cap.x', NeedVia.builtinOnly)]),
    const Declaration('b', provides: [Provide('cap.x')], needs: [Need('cap.y', NeedVia.builtinOnly)]),
  ]).resolve();
  check('builtinOnly 不成环', w7.isNotEmpty);

  // 8. 模块 id 全局唯一：重复注册 fail-fast（定向调用无歧义的根基）
  threw = false;
  try {
    final b = Broker()..register(const Declaration('dup'));
    b.register(const Declaration('dup'));
  } on WiringException {
    threw = true;
  }
  check('重复模块 id -> 失败', threw);

  print('');
  print('全部通过: ' + _passed.toString() + ' / 8');
}
