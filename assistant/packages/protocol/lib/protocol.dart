/// 协议包：边界上的唯一共同物。
/// 它是字典不是基座：只有数据形状与编解码，无运行时、无服务、无生命周期。
library;

// ---- 消息封套：边界上只允许消息，永不共享对象引用 ----

/// 消息种类：声明 / 请求 / 应答 / 事件
enum EnvelopeKind { hello, rpc, ok, err, event }

/// 封套是模块间通信的唯一形状
class Envelope {
  final String from; // 发送方模块 id（核心为 "core"）
  final String to; // 接收方模块 id 或能力地址
  final EnvelopeKind kind;
  final String? id; // rpc 关联 id，供 request/response 配对
  final String method; // 能力地址，如 "llm.chat"
  final Object? params; // 纯数据：Map/List/String/num/bool/null

  const Envelope({
    required this.from,
    required this.to,
    required this.kind,
    required this.method,
    this.id,
    this.params,
  });

  Map<String, Object?> toJson() => {
        'from': from,
        'to': to,
        'kind': kind.name,
        if (id != null) 'id': id,
        'method': method,
        'params': params,
      };

  static Envelope fromJson(Map<String, Object?> j) => Envelope(
        from: j['from'] as String,
        to: j['to'] as String,
        kind: EnvelopeKind.values.byName(j['kind'] as String),
        id: j['id'] as String?,
        method: j['method'] as String,
        params: j['params'],
      );
}

// ---- 声明：模块对世界的全部认知 ----

/// 消费方策略：核心据此机械装配，不做任何价值判断
enum NeedVia {
  preferShared, // 有共享用共享，没有用内建（中心化是能力不是前提）
  builtinOnly, // 模块私有，永不共享
  sharedOnly, // 内建造不出来，缺失时功能降级而非崩溃
}

class Need {
  final String cap;
  final NeedVia via;
  /// 多提供方在线时的首选（消费方策略选择；核心只机械执行，不代为决策）
  final String? provider;
  const Need(this.cap, this.via, {this.provider});
}

class Provide {
  final String cap; // 提供方主权声明：我愿意把这个能力当共同物
  const Provide(this.cap);
}

class Declaration {
  final String id;
  final List<Provide> provides;
  final List<Need> needs;
  const Declaration(this.id, {this.provides = const [], this.needs = const []});

  Map<String, Object?> toJson() => {
        'id': id,
        'provides': [for (final p in provides) p.cap],
        'needs': [
          for (final n in needs)
            {
              'cap': n.cap,
              'via': n.via.name,
              if (n.provider != null) 'provider': n.provider,
            }
        ],
      };

  /// 声明从线路还原（hello 消息的 params）
  factory Declaration.fromJson(Map<String, Object?> j) => Declaration(
        j['id'] as String,
        provides: [
          for (final p in (j['provides'] as List)) Provide(p as String)
        ],
        needs: [
          for (final n in (j['needs'] as List))
            Need((n as Map)['cap'] as String,
                NeedVia.values.byName(n['via'] as String),
                provider: n['provider'] as String?)
        ],
      );
}

// ---- 知名能力词汇表 ----
// 只是名字的集中登记，方便引用与避免漂移；不是行为，不构成上帝。
// 私有能力用模块前缀（如 "conversation."），对外能力用反向域名前缀。

/// 核心元能力词汇（机制，非业务：注册表只读列举，返回事实不做决策）
abstract final class CoreCaps {
  static const modules = 'core.modules';

  /// 配对快照推送：成员变化重算后，核心推给每个在线模块自己的配对（事实，非决策）
  static const wiring = 'core.wiring';
}

/// 流式结果的包装：模块处理器返回它，核心把 chunks 逐条作为
/// 同 id 的 event 封发给消费方，最后以 ok（全量文本）收尾。
/// 非流式消费方只见 ok，无感知。
class Streamed {
  final Stream<Object?> chunks;
  const Streamed(this.chunks);
}

abstract final class Caps {
  // secrets 模块提供
  static const secretsGet = 'secrets.get';
  static const secretsPut = 'secrets.put';

  // llm 网关模块提供
  static const llmChat = 'llm.chat';

  // conversation 模块提供
  static const chatSend = 'conversation.send';
  static const chatStream = 'conversation.stream';
  static const chatHistory = 'conversation.history';
}
