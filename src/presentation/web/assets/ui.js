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
    chart: '<path d="M4 19V5"/><path d="M4 19h16"/><path d="m7 15 4-4 3 2 5-6"/>',
    git: '<circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path d="M6 8v4a6 6 0 0 0 6 6h4"/><path d="M13 7h5v5"/>',
    file: '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/>',
    terminal: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/>',
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
      : { left: 0, top: 0, bottom: 0 };
    const root = doc.querySelector('#menu-root') || doc.body;
    if (root) root.appendChild(m);
    // 菜单优先贴锚点下方；下方放不下就翻到上方。状态栏靠近视口底部，不翻会直接被裁掉。
    const box = typeof m.getBoundingClientRect === 'function' ? m.getBoundingClientRect() : null;
    const vh = global.innerHeight || 600;
    const mh = (box && box.height) || Math.min(320, 44 * (items || []).length);
    let top = r.bottom + 4;
    if (top + mh > vh - 8) top = Math.max(8, r.top - mh - 4);
    if (m.style) {
      m.style.left = Math.round(r.left) + 'px';
      m.style.top = Math.round(top) + 'px';
    }
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

  /* 状态 chip：图标 + 简要文字，语义是按钮（展开详情小卡片）。 */
  function statusChip(opts) {
    const o = opts || {};
    const b = el('button', 'status-chip' + (o.cls ? ' ' + o.cls : ''));
    b.type = 'button';
    if (o.label) attr(b, 'aria-label', o.label);
    if (o.title) b.title = o.title;
    if (o.icon) b.appendChild(icon(o.icon));
    if (o.text != null) b.appendChild(el('span', 'status-chip-text', o.text));
    if (o.hint != null) b.appendChild(el('span', 'status-chip-hint', o.hint));
    if (typeof o.onClick === 'function') b.onclick = o.onClick;
    return b;
  }

  /* 锚定小卡片：状态 chip 的详情。点外面 / Esc 关闭；上一张没关就换下一张。 */
  let openPopover = null;
  function closePopover() {
    if (!openPopover) return;
    const p = openPopover;
    openPopover = null;
    if (p._cleanup) p._cleanup();
    if (p.node && p.node.parentNode) p.node.parentNode.removeChild(p.node);
    else if (p.node && p.node.remove) p.node.remove();
    if (p.anchor && p.anchor.setAttribute) p.anchor.setAttribute('aria-expanded', 'false');
  }

  function popover(anchor, content, opts) {
    closePopover();
    const o = opts || {};
    const node = el('div', 'popover' + (o.cls ? ' ' + o.cls : ''));
    attr(node, 'role', 'dialog');
    if (o.label) attr(node, 'aria-label', o.label);
    node.appendChild(content);
    const root = doc.querySelector('#menu-root') || doc.body;
    root.appendChild(node);
    const place = () => {
      const r = anchor && typeof anchor.getBoundingClientRect === 'function'
        ? anchor.getBoundingClientRect()
        : { left: 8, right: 128, top: 40, bottom: 64 };
      const box = typeof node.getBoundingClientRect === 'function'
        ? node.getBoundingClientRect()
        : { width: 360, height: 240 };
      const pw = box.width || 360;
      const ph = box.height || 240;
      const vw = global.innerWidth || 800;
      const vh = global.innerHeight || 600;
      const topWanted = o.placement === 'bottom' ? r.bottom + 8 : r.top - ph - 8;
      const top = topWanted < 8 ? Math.min(r.bottom + 8, vh - 8) : topWanted;
      const left = Math.max(8, Math.min(r.left, vw - pw - 8));
      if (node.style) {
        node.style.left = Math.round(left) + 'px';
        node.style.top = Math.round(top) + 'px';
      }
    };
    place();
    const onDown = (e) => {
      if (e.target === node || (node.contains && node.contains(e.target))) return;
      if (e.target === anchor || (anchor.contains && anchor.contains(e.target))) return;
      closePopover();
    };
    const onKey = (e) => {
      if (e.key === 'Escape') {
        if (e.stopPropagation) e.stopPropagation();
        closePopover();
      }
    };
    const onViewport = () => closePopover();
    if (doc.addEventListener) {
      doc.addEventListener('pointerdown', onDown, true);
      doc.addEventListener('keydown', onKey, true);
    }
    if (typeof global.addEventListener === 'function') {
      global.addEventListener('resize', place);
      global.addEventListener('scroll', onViewport, true);
    }
    if (anchor && anchor.setAttribute) anchor.setAttribute('aria-expanded', 'true');
    openPopover = {
      anchor,
      node,
      _cleanup: () => {
        if (doc.removeEventListener) {
          doc.removeEventListener('pointerdown', onDown, true);
          doc.removeEventListener('keydown', onKey, true);
        }
        if (typeof global.removeEventListener === 'function') {
          global.removeEventListener('resize', place);
          global.removeEventListener('scroll', onViewport, true);
        }
      },
    };
    return { node, close: closePopover };
  }

  global.ui = {
    el,
    icon,
    iconButton,
    statusChip,
    popover,
    closePopover,
    fold,
    menu,
    closeMenu,
    toast,
    dismisses,
    bindDismiss,
  };
})(typeof window !== 'undefined' ? window : globalThis);
