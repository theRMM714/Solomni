/* 目的：整页重画的编排——renderAll 一次刷标签 / 转录 / 裁决门 / 输入区，只读条单独一段。
 * 管：renderAll / renderReadonly 的调用顺序；不改任何会话内容。
 * 不管：各块的内部渲染（标签、转录、裁决门、输入区、工作卡各在自己模块）。
 * 联动：app.activeSession 给当前会话；工作卡经 app.loadWork 晚绑定拉取；恢复动作走 app 上的入口。 */

import { renderStream } from './stream.js';
import { renderGate } from './cards.js';
import { isBusy } from '../../stores/sessions.js';

let doc, app;

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

function renderAll() {
  const s = app.activeSession();
  app.renderTabs(); renderStream(); renderGate(s); app.syncComposer(s);
  if (s && s.workDirty) { s.workDirty = false; app.loadWork(s.sid, true); }
}

/* 只读历史视图：不渲染输入区；按会话状态给唯一的恢复动作——
 * 正在跑 → 不给按钮（跑完条子自己换回「启用会话」），
 * 已停止 → 「继续」（解冻并接着走），已归档 → 「恢复归档」（翻回 active，不唤醒），其余 → 「启用会话」。
 * 判据只用**本视图自己的** run 与运行态：历史列表里的 run 是打开那一刻的快照，会一直停在「已停止」。
 * 归档不是终态：这里不给删除，删除仍走侧栏菜单。 */
function renderReadonly(s) {
  const bar = $('#readonly-bar');
  if (!bar) return;
  bar.innerHTML = '';
  if (!s || !s.readonly) { bar.classList.add('hidden'); return; }
  bar.classList.remove('hidden');
  const run = s.run || 'active';
  const note = doc.createElement('span');
  note.className = 'readonly-note';
  if (s.workspace_missing) {
    note.textContent = '工作区已不存在：恢复目录或删除会话后才能启用（当前只能查看记录）';
    bar.appendChild(note);
    return;
  }
  const button = (label, primary, onClick, disabled) => {
    const b = doc.createElement('button');
    b.className = 'btn ' + (primary ? 'btn-primary' : 'btn-ghost');
    b.textContent = label;
    b.disabled = !!disabled;
    b.onclick = onClick;
    bar.appendChild(b);
    return b;
  };
  // 正在跑（本页刚点了「继续」，或别的窗口在跑这一轮）：这时没有「恢复」可言，
  // 给按钮就是假入口——上一版正是在这里露出「启用会话」，用户以为还要再点一次。
  if (isBusy(s)) {
    note.textContent = '这一轮正在跑：回答会上屏，跑完就能接着用';
    bar.appendChild(note);
    return;
  }
  if (run === 'archived') {
    note.textContent = '已归档：从会话列表收起，不再被派发或唤醒（记录都还在）';
    bar.appendChild(note);
    button('恢复归档', true, () => app.restoreArchive(s.sid));
    return;
  }
  if (run === 'stopped') {
    note.textContent = '已停止：不再被派发或唤醒；点「继续」解冻并接着走';
    bar.appendChild(note);
    button('继续', true, () => app.continueSession(s.sid));
    return;
  }
  note.textContent = '历史回放（只读）：启用后才能发送消息';
  bar.appendChild(note);
  button(s.activating ? '正在启用…' : '启用会话', true, app.activateFlow, s.activating);
}

export { renderAll, renderReadonly };

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ doc, app } = deps);
  Object.assign(root, { renderAll, renderReadonly });
}
