/* 目的：表单类组件（文本 / 数字 / 多行 / 下拉 / 勾选 / 单选、带标签的字段、空态提示）。
 * 管：建节点；下拉原地换选项并保留当前选中（值已不在选项里时退回第一个）。
 * 不管：校验规则、选项从哪来、保存。
 * 联动：样式类 `.field-input` / `.wf-field` / `.chk` / `.reg-empty` 在 `assets/style.css` 与 `assets/ui.css`。 */

export function createForms(dom, overlays) {
  const doc = dom.doc;

  function textInput(placeholder, type) {
    const i = doc.createElement('input');
    i.className = 'field-input'; i.placeholder = placeholder || ''; i.autocomplete = 'off';
    if (type) i.type = type;
    return i;
  }

  function field(labelText, input, helpCopy) {
    const wrap = doc.createElement('div'); wrap.className = 'wf-field';
    const lab = doc.createElement('div'); lab.className = 'wf-label'; lab.textContent = labelText;
    wrap.appendChild(lab); wrap.appendChild(input);
    return helpCopy ? attachHelp(wrap, helpCopy) : wrap;
  }

  /**
   * 把说明挂到已有字段上：感叹号按钮进入标题行，lead 挂在标题下方（CSS 定位，不改节点顺序），
   * safety 常显在控件下方。控件工厂（numberInput 等）先建好节点，调用方拿到 copy 再挂。
   */
  function attachHelp(wrap, copy) {
    if (!wrap || !copy) return wrap;
    let lab = null;
    for (const n of wrap.children || wrap.childNodes || []) {
      if (n.className === 'wf-label') { lab = n; break; }
    }
    if (lab && overlays && copy.detail) lab.appendChild(overlays.help(copy));
    if (lab && copy.lead) {
      const lead = doc.createElement('p'); lead.className = 'wf-lead'; lead.textContent = copy.lead;
      wrap.appendChild(lead);
    }
    if (copy.safety) {
      const s = doc.createElement('p');
      s.className = 'detail-safety' + (copy.safetyKind ? ' ' + copy.safetyKind : '');
      s.textContent = copy.safety;
      wrap.appendChild(s);
    }
    return wrap;
  }

  /** 数字输入 + 标签 + 边界：返回 `{input, wrap}`（`wrap` 才是要挂进表单的节点）。 */
  function numberInput(labelText, value, min, max) {
    const input = textInput('', 'number');
    input.min = String(min); input.max = String(max);
    input.value = String(value === undefined || value === null ? '' : value);
    return { input, wrap: field(labelText, input) };
  }

  function areaInput(placeholder) {
    const t = doc.createElement('textarea');
    t.className = 'field-input'; t.rows = 3; t.placeholder = placeholder || '';
    return t;
  }

  function selectInput(options, value) {
    const s = doc.createElement('select');
    s.className = 'field-input';
    for (const o of options) {
      const op = doc.createElement('option');
      op.value = o.value; op.textContent = o.label;
      s.appendChild(op);
    }
    if (value != null) s.value = value;
    return s;
  }

  function syncSelect(sel, options, current) {
    const want = current === undefined || current === null ? sel.value : current;
    sel.innerHTML = '';
    for (const o of options) {
      const op = doc.createElement('option');
      op.value = o.value; op.textContent = o.label;
      sel.appendChild(op);
    }
    const has = options.some((o) => o.value === want);
    sel.value = has ? want : (options.length ? options[0].value : '');
    return sel.value;
  }

  function choice(type, labelText, checked, note) {
    const wrap = doc.createElement('label'); wrap.className = 'chk';
    const box = doc.createElement('input'); box.type = type; box.checked = !!checked;
    const text = doc.createElement('span'); text.className = 'chk-text';
    text.textContent = labelText;
    if (note) {
      const small = doc.createElement('span'); small.className = 'chk-note'; small.textContent = note;
      text.appendChild(small);
    }
    wrap.appendChild(box); wrap.appendChild(text);
    return { wrap, box };
  }

  function checkbox(labelText, checked, note) { return choice('checkbox', labelText, checked, note); }

  /** 单选：同 `group` 即互斥；`disabled` = 这一项此刻不可选。 */
  function radio(labelText, checked, group, disabled, note) {
    const r = choice('radio', labelText, checked, note);
    r.box.name = group; r.box.disabled = !!disabled;
    return r;
  }

  /// 说明块：面板里的段落容器，cls 决定是分区标题、提示行还是错误提示。
  function div(cls, text) {
    const d = doc.createElement('div'); d.className = cls;
    if (text != null) d.textContent = text;
    return d;
  }

  /// 旁注 / 提示：kind 是 err / warn / ok 时带对应配色（样式类 .cfg-hint）。
  function hint(text, kind) {
    return div('cfg-hint' + (kind ? ' ' + kind : ''), text);
  }

  function emptyHint(text) {
    const e = doc.createElement('div'); e.className = 'reg-empty'; e.textContent = text;
    return e;
  }

  return { div, hint, textInput, field, attachHelp, numberInput, areaInput, selectInput, syncSelect, checkbox, radio, emptyHint };
}
