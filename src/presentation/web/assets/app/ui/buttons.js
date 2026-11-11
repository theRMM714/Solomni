/* 目的：按钮类组件（文字按钮、图标按钮、忙态、状态 chip）。
 * 管：建节点与接线；忙态返回复位函数（调用方放进 finally，失败也一定恢复可点）。
 * 不管：按钮背后的动作与业务判断。
 * 联动：样式类 `.btn` / `.icon-btn` / `.status-chip` 在 `assets/ui.css`；图标见 `app/ui/icons.js`。 */

export function createButtons(dom, icons) {
  const { el, attr } = dom;

  function btn(label, cls) {
    const b = dom.doc.createElement('button');
    b.type = 'button'; b.className = cls || 'btn'; b.textContent = label;
    return b;
  }

  function busyButton(b, label) {
    const was = b.textContent;
    const wasDisabled = !!b.disabled;
    b.disabled = true;
    b.classList.add('busy');
    if (label) b.textContent = label;
    return () => {
      b.classList.remove('busy');
      b.textContent = was;
      b.disabled = wasDisabled;
    };
  }

  function iconButton(opts) {
    const o = opts || {};
    const b = el('button', 'icon-btn' + (o.cls ? ' ' + o.cls : '') + (o.danger ? ' danger' : ''));
    b.type = 'button';
    const label = o.label || '';
    if (label) {
      attr(b, 'aria-label', label);
      b.title = label;
    }
    const tip = o.tip === undefined ? label : o.tip;
    if (tip) b.dataset.tip = tip;
    b.appendChild(icons.icon(o.icon));
    if (typeof o.onClick === 'function') b.onclick = o.onClick;
    if (o.disabled) b.disabled = true;
    return b;
  }

  /* 状态 chip：图标 + 简要文字，语义是按钮（展开详情小卡片）。 */
  function statusChip(opts) {
    const o = opts || {};
    const b = el('button', 'status-chip' + (o.cls ? ' ' + o.cls : ''));
    b.type = 'button';
    if (o.label) attr(b, 'aria-label', o.label);
    if (o.title) b.title = o.title;
    if (o.icon) b.appendChild(icons.icon(o.icon));
    if (o.text != null) b.appendChild(el('span', 'status-chip-text', o.text));
    if (o.hint != null) b.appendChild(el('span', 'status-chip-hint', o.hint));
    if (typeof o.onClick === 'function') {
      // 有动作才给箭头：光看文字不知道 chip 能点开。
      const caret = icons.icon('chevron');
      caret.classList.add('status-chip-caret');
      b.appendChild(caret);
      b.onclick = o.onClick;
    }
    return b;
  }

  return { btn, busyButton, iconButton, statusChip };
}
