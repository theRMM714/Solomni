/* 目的：浮层类组件（折叠、浮层菜单、toast、弹层关闭判据与弹层栈、锚定小卡片）。
 * 管：建节点、定位（夹在视口内）、点外面 / Esc 关闭；弹层栈保证 Esc 只关最上面一层。
 * 不管：浮层里装什么、关掉之后做什么（调用方的回调）。
 * 联动：挂载点 `#menu-root` / `#toast-root` 在 `assets/index.html`；样式在 `assets/ui.css`；
 *   弹层关闭判据见 `docs/presentation/web/README.md` 的「弹层关闭判据」。 */

export function createOverlays(dom, icons) {
  const { el, attr, doc } = dom;
  const g = dom.g;

  function fold(opts) {
    const o = opts || {};
    const d = el('details', 'fold' + (o.cls ? ' ' + o.cls : ''));
    if (o.open) d.open = true;
    const s = el('summary');
    s.appendChild(icons.icon('chevron-right'));
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
      if (it.icon) b.appendChild(icons.icon(it.icon));
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
    // 菜单优先贴锚点下方；下方放不下就翻到上方（状态栏靠近视口底部，不翻会被裁掉）。
    const box = typeof m.getBoundingClientRect === 'function' ? m.getBoundingClientRect() : null;
    const vh = g.innerHeight || 600;
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
    g.setTimeout(() => {
      if (n.parentNode && n.parentNode.removeChild) n.parentNode.removeChild(n);
      else if (n.remove) n.remove();
    }, 2400);
    return n;
  }

  /* 弹层关闭判据：按下与松开都在遮罩上才算「点外面」。纯函数，冒烟直接测四种组合。 */
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

  /* 锚定小卡片：状态 chip 的详情。点外面 / Esc 关闭；上一张没关就换下一张。 */
  let openPopover = null;
  function closePopover() {
    if (!openPopover) return;
    const p = openPopover;
    openPopover = null;
    if (p._cleanup) p._cleanup();
    if (typeof p.onClose === 'function') p.onClose();
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
    // 锚点可能被整帧重建换掉（状态栏会重建 chip）：沿用最后一次量到的位置，别把卡片弹到 0,0。
    let lastRect = null;
    const place = () => {
      const now = anchor && typeof anchor.getBoundingClientRect === 'function'
        ? anchor.getBoundingClientRect()
        : null;
      if (now && (now.width || now.height || now.top || now.bottom)) lastRect = now;
      const r = lastRect || now || { left: 8, right: 128, top: 40, bottom: 64 };
      const box = typeof node.getBoundingClientRect === 'function'
        ? node.getBoundingClientRect()
        : { width: 360, height: 240 };
      const pw = box.width || 360;
      const ph = box.height || 240;
      const vw = g.innerWidth || 800;
      const vh = g.innerHeight || 600;
      let top = o.placement === 'bottom' ? r.bottom + 8 : r.top - ph - 8;
      // 上面放不下就翻到下面；无论翻到哪边，整张卡都夹在视口里（底边不越界）。
      if (o.placement !== 'bottom' && top < 8) top = r.bottom + 8;
      top = Math.max(8, Math.min(top, vh - ph - 8));
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
    // 页面 / 视口滚动 → 收起（卡片是 fixed，锚点会跑）；卡片自己的滚动（长回执）不关。
    const onViewport = (e) => {
      const t = e && e.target;
      if (t && (t === node || (node.contains && node.contains(t)))) return;
      closePopover();
    };
    if (doc.addEventListener) {
      doc.addEventListener('pointerdown', onDown, true);
      doc.addEventListener('keydown', onKey, true);
    }
    if (typeof g.addEventListener === 'function') {
      g.addEventListener('resize', place);
      g.addEventListener('scroll', onViewport, true);
    }
    if (anchor && anchor.setAttribute) anchor.setAttribute('aria-expanded', 'true');
    openPopover = {
      anchor,
      node,
      onClose: o.onClose,
      _cleanup: () => {
        if (doc.removeEventListener) {
          doc.removeEventListener('pointerdown', onDown, true);
          doc.removeEventListener('keydown', onKey, true);
        }
        if (typeof g.removeEventListener === 'function') {
          g.removeEventListener('resize', place);
          g.removeEventListener('scroll', onViewport, true);
        }
      },
    };
    return { node, close: closePopover, place };
  }

  return { fold, menu, closeMenu, toast, dismisses, bindDismiss, popover, closePopover };
}
