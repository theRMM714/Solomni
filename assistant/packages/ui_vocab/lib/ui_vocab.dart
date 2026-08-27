/// UI 词汇表：模块与 UI 模块共同依赖的字典。
/// 只有意图（组件 id/类型/绑定/事件），没有像素（坐标/颜色/布局）。
/// 组件 id 是契约身份（归模块）；呈现主权（布局/主题）归 UI 模块。
library;

/// 组件类型（公共词汇；模块自定义类型用自己的命名空间）
abstract final class UiKind {
  static const button = 'button';
  static const textInput = 'text_input';
  static const textStream = 'text_stream';
  static const textOutput = 'text_output';
  static const formField = 'form_field';
  static const list = 'list';
}

/// 组件可见域：公用（任意画布可用）/ 私有（仅本模块画布）
enum UiScope { public, private }

/// 贡献与事件的能力名（每个有 UI 的模块都提供同名能力，按模块 id 定向调用）
abstract final class UiCaps {
  static const contribution = 'ui.contribution';
  static const event = 'ui.event';
}

class UiComponent {
  final String id; // 契约身份：UI 不得改名
  final String kind;
  final UiScope scope;
  final String label;
  final String? bind; // 数据/动作绑定的能力地址（文档性；事件一律路由回模块）
  const UiComponent(this.id, this.kind,
      {this.scope = UiScope.public, this.label = '', this.bind});

  Map<String, Object?> toJson() => {
        'id': id,
        'kind': kind,
        'scope': scope.name,
        'label': label,
        if (bind != null) 'bind': bind,
      };

  factory UiComponent.fromJson(Map<String, Object?> j) => UiComponent(
        j['id'] as String,
        j['kind'] as String,
        scope: UiScope.values.byName(j['scope'] as String? ?? 'public'),
        label: j['label'] as String? ?? '',
        bind: j['bind'] as String?,
      );
}

class UiCommand {
  final String name;
  final List<String> args;
  final String description;
  final String component; // 触发哪个组件的哪个事件
  final String event;
  const UiCommand(this.name,
      {this.args = const [],
      this.description = '',
      required this.component,
      required this.event});

  Map<String, Object?> toJson() => {
        'name': name,
        'args': args,
        'description': description,
        'component': component,
        'event': event,
      };

  factory UiCommand.fromJson(Map<String, Object?> j) => UiCommand(
        j['name'] as String,
        args: [for (final a in (j['args'] as List? ?? const [])) a as String],
        description: j['description'] as String? ?? '',
        component: j['component'] as String,
        event: j['event'] as String,
      );
}

/// 模块的 UI 贡献：一份意图清单（不含任何呈现决策）
class UiContribution {
  final List<UiComponent> components;
  final List<UiCommand> commands;
  const UiContribution({this.components = const [], this.commands = const []});

  Map<String, Object?> toJson() => {
        'components': [for (final c in components) c.toJson()],
        'commands': [for (final c in commands) c.toJson()],
      };

  factory UiContribution.fromJson(Map<String, Object?> j) => UiContribution(
        components: [
          for (final c in (j['components'] as List? ?? const []))
            UiComponent.fromJson(Map<String, Object?>.from(c as Map))
        ],
        commands: [
          for (final c in (j['commands'] as List? ?? const []))
            UiCommand.fromJson(Map<String, Object?>.from(c as Map))
        ],
      );
}
