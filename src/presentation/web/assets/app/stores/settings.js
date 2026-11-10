/* 目的：设置项与枚举的只读投影——思考档位、权限档位都来自 /api/state 的 settings。
 * 管：DEFAULT_THINKING_LEVELS / thinkingLevels / thinkingLabel / PERMISSION_ORDER / PERMISSION_LABELS。
 * 不管：不读写其它状态、不发请求、不碰 DOM。
 * 联动：状态栏与设置中心按这里的档位渲染，兜底值只在本文件出现一次。 */

let state;

/* 思考强度的兜底目录：后端 /api/state 的 settings.thinking 是权威；桩或加载失败时仍能画出四档。 */
const DEFAULT_THINKING_LEVELS = [
  { id: 'off' },
  { id: 'low' },
  { id: 'high' },
  { id: 'max' },
];

function thinkingLevels() {
  const t = state.settings && state.settings.thinking;
  const levels = t && Array.isArray(t.levels) ? t.levels : [];
  return levels.length ? levels : DEFAULT_THINKING_LEVELS;
}

function thinkingLabel(id) {
  return String(id || '');
}


const PERMISSION_ORDER = ['view', 'workspace', 'external', 'all_reach'];

const PERMISSION_LABELS = {
  view: '仅可查看',
  workspace: '工作区内修改',
  external: '外部可达（逐次确认）',
  all_reach: '全可达（自动放行）',
};


/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ state } = deps);
  Object.assign(root, { DEFAULT_THINKING_LEVELS, thinkingLevels, thinkingLabel, PERMISSION_ORDER, PERMISSION_LABELS });
}
