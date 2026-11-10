/* 目的：整页重画的编排——renderAll 一次刷标签 / 转录 / 裁决门 / 输入区，只读条单独一段。
 * 管：renderAll / renderReadonly 的调用顺序；不改任何会话内容。
 * 不管：各块的内部渲染（标签、转录、裁决门、输入区、工作卡各在自己模块）。
 * 联动：app.activeSession 给当前会话；工作卡经 app.loadWork 晚绑定拉取。 */

import { renderStream } from './stream.js';
import { renderGate } from './cards.js';

let g, doc, state, stores, api, ui, app;

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

function renderAll() {
  const s = app.activeSession();
  app.renderTabs(); renderStream(); renderGate(s); app.syncComposer(s);
  if (s && s.workDirty) { s.workDirty = false; app.loadWork(s.sid, true); }
}

/* 只读历史视图：不渲染输入区；只留「启用会话」。启用是后端动作，前端只渲染结果。 */
function renderReadonly(s) {
  const bar = $('#readonly-bar');
  if (!bar) return;
  bar.innerHTML = '';
  if (!s || !s.readonly) { bar.classList.add('hidden'); return; }
  bar.classList.remove('hidden');
  const note = doc.createElement('span');
  note.className = 'readonly-note';
  if (s.workspace_missing) {
    note.textContent = '工作区已不存在：恢复目录或删除会话后才能启用（当前只能查看记录）';
    bar.appendChild(note);
    return;
  }
  note.textContent = s.run === 'closed' ? '历史回放（只读）· 已关闭' : '历史回放（只读）：启用后才能发送消息';
  bar.appendChild(note);
  if (s.run === 'closed') return;
  const b = doc.createElement('button');
  b.className = 'btn btn-primary';
  b.textContent = s.activating ? '正在启用…' : '启用会话';
  b.disabled = !!s.activating;
  b.onclick = app.activateFlow;
  bar.appendChild(b);
}

export { renderAll, renderReadonly };

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, stores, api, ui, app } = deps);
  Object.assign(root, { renderAll, renderReadonly });
}
