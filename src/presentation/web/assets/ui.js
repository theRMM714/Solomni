'use strict';
/* Solomni 组件工厂：内联图标、图标按钮、折叠、浮层菜单、toast、弹层关闭规则。
 * 约束：无依赖、no-build；只创建节点，不读业务状态；样式类在 /ui.css，数值在 /tokens.css。
 * 联动：/app.js 只调用这里，不再自己拼同一套 DOM；冒烟见 ui.smoke.cjs。 */
(function (global) {
  const doc = global.document;
  const SVG_HEAD = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">';
  const ICONS = {
    plus: '<path d="M12 5v14M5 12h14"/>',
    undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h9a7 7 0 0 1 0 14h-1"/>',
    pencil: '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
    gear: '<path d="M4 7h6M14 7h6M4 12h10M18 12h2M4 17h4M12 17h8"/>' +
      '<circle cx="12" cy="7" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="17" r="2"/>',
    chevron: '<path d="m6 9 6 6 6-6"/>',
    close: '<path d="M18 6 6 18M6 6l12 12"/>',
    check: '<path d="m5 13 4 4L19 7"/>',
    alert: '<path d="M12 9v4M12 17h.01"/><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 8v4l3 2"/>',
    folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>',
    cpu: '<rect x="7" y="7" width="10" height="10" rx="2"/><path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3"/>',
    shield: '<path d="M12 3l7 3v6c0 4-3 7-7 9-4-2-7-5-7-9V6Z"/>',
    users: '<circle cx="9" cy="8" r="3"/><path d="M3 20a6 6 0 0 1 12 0"/><path d="M16 5a3 3 0 0 1 0 6"/><path d="M18 20a6 6 0 0 0-3-5"/>',
    box: '<path d="m12 3 8 4v10l-8 4-8-4V7Z"/><path d="m4 7 8 4 8-4M12 11v10"/>',
    panel: '<path d="M3 5h18v14H3z"/><path d="M9 5v14"/>',
    more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
    dot: '<circle cx="12" cy="12" r="4"/>',
  };

  function el(tag, cls, text) {
    const n = doc.createElement(tag || 'div');
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  function attr(n, name, value) {
    if (typeof n.setAttribute === 'function') n.setAttribute(name, value);
    else if (name === 'aria-label') n.ariaLabel = value;
  }

  function icon(name) {
    const n = el('span', 'ui-icon');
    n.innerHTML = SVG_HEAD + (ICONS[name] || ICONS.dot) + '</svg>';
    attr(n, 'aria-hidden', 'true');
    return n;
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
    b.appendChild(icon(o.icon));
    if (typeof o.onClick === 'function') b.onclick = o.onClick;
    if (o.disabled) b.disabled = true;
    return b;
  }

  function fold(opts) {
    const o = opts || {};
    const d = el('details', 'fold' + (o.cls ? ' ' + o.cls : ''));
    if (o.open) d.open = true;
    const s = el('summary');
    s.appendChild(el('span', 'fold-label', o.label || ''));
    if (o.count != null) s.appendChild(el('span', 'fold-count', String(o.count)));
    d.appendChild(s);
    if (o.body) {
      const body = el('div', 'fold-body');
      body.appendChild(o.body);
      d.appendChild(body);
    }
    if (typeof o.onToggle === 'function') d.addEventListener('toggle', () => o.onToggle(!!d.open));
    return d;
  }

  let openMenu = null;
  function closeMenu() {
    if (!openMenu) return;
    const m = openMenu;
    openMenu = null;
    if (m._cleanup) m._cleanup();
    if (m.parentNode && m.parentNode.removeChild) m.parentNode.removeChild(m);
    else if (m.remove) m.remove();
  }

  function menu(anchor, items) {
    closeMenu();
    const m = el('div', 'menu');
    attr(m, 'role', 'menu');
    for (const it of (items || [])) {
      if (it.sep) { m.appendChild(el('div', 'menu-sep')); continue; }
      const b = el('button', 'menu-item' + (it.danger ? ' danger' : ''));
      b.type = 'button';
      attr(b, 'role', 'menuitem');
      if (it.icon) b.appendChild(icon(it.icon));
      b.appendChild(el('span', '', it.label || ''));
      if (it.disabled) {
        b.disabled = true;
        if (it.reason) b.title = it.reason;
      }
      if (typeof it.onClick === 'function' && !it.disabled) {
        b.onclick = () => { closeMenu(); it.onClick(); };
      }
      m.appendChild(b);
    }
    const r = anchor && typeof anchor.getBoundingClientRect === 'function'
      ? anchor.getBoundingClientRect()
      : { left: 0, bottom: 0 };
    if (m.style) {
      m.style.left = Math.round(r.left) + 'px';
      m.style.top = Math.round(r.bottom + 4) + 'px';
    }
    const root = doc.querySelector('#menu-root') || doc.body;
    if (root) root.appendChild(m);
    openMenu = m;
    const onDown = (e) => { if (e.target !== m && !m.contains?.(e.target)) closeMenu(); };
    const onKey = (e) => {
      if (e.key === 'Escape') {
        if (e.stopPropagation) e.stopPropagation();
        closeMenu();
      }
    };
    if (doc.addEventListener) {
      doc.addEventListener('pointerdown', onDown, true);
      doc.addEventListener('keydown', onKey, true);
    }
    m._cleanup = () => {
      if (doc.removeEventListener) {
        doc.removeEventListener('pointerdown', onDown, true);
        doc.removeEventListener('keydown', onKey, true);
      }
    };
    return m;
  }

  function toast(kind, text) {
    const root = doc.querySelector('#toast-root');
    if (!root) return null;
    const n = el('div', 'toast ' + (kind || ''), text);
    attr(n, 'role', 'status');
    root.appendChild(n);
    while ((root.children || []).length > 3) {
      const first = root.children[0];
      if (root.removeChild) root.removeChild(first); else break;
    }
    global.setTimeout(() => {
      if (n.parentNode && n.parentNode.removeChild) n.parentNode.removeChild(n);
      else if (n.remove) n.remove();
    }, 2400);
    return n;
  }

  /* 弹层关闭判据：按下与松开**都在遮罩上**才算“点外面”。纯函数，冒烟直接测四种组合。 */
  function dismisses(overlay, downTarget, clickTarget) {
    return !!overlay && downTarget === overlay && clickTarget === overlay;
  }

  /* 弹层栈：Esc 只关最上面那层（notice 盖在 modal 上时不会一起关）。 */
  const layers = [];
  function bindDismiss(overlay, panel, close) {
    let down = null;
    const onDown = (e) => { down = e.target; };
    const onClick = (e) => {
      if (dismisses(overlay, down, e.target)) close();
      down = null;
    };
    const onKey = (e) => {
      if (e.key === 'Escape' && layers[layers.length - 1] === overlay) close();
    };
    overlay.addEventListener('pointerdown', onDown);
    overlay.addEventListener('click', onClick);
    if (doc.addEventListener) doc.addEventListener('keydown', onKey);
    layers.push(overlay);
    return () => {
      const i = layers.lastIndexOf(overlay);
      if (i >= 0) layers.splice(i, 1);
      if (overlay.removeEventListener) {
        overlay.removeEventListener('pointerdown', onDown);
        overlay.removeEventListener('click', onClick);
      }
      if (doc.removeEventListener) doc.removeEventListener('keydown', onKey);
    };
  }

  global.ui = {
    el,
    icon,
    iconButton,
    fold,
    menu,
    closeMenu,
    toast,
    dismisses,
    bindDismiss,
  };
})(typeof window !== 'undefined' ? window : globalThis);
