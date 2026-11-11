/* 目的：debug 构建里的组件陈列页：令牌、组件、图标与转录状态轨，一处看全。
 * 管：只读渲染；按钮只触发本地浮层，不发任何 API。
 * 不管：产品页面本身；发行版不嵌入（见 build.rs 的 dev 目录规则）。
 * 联动：组件来自 app/ui/，样式来自 tokens.css / ui.css / style.css。 */

import { createDom } from '../app/core/dom.js';
import { createIcons, ICONS } from '../app/ui/icons.js';
import { createButtons } from '../app/ui/buttons.js';
import { createForms } from '../app/ui/forms.js';
import { createOverlays } from '../app/ui/overlays.js';

const dom = createDom(window);
const { el } = dom;
const icons = createIcons(dom);
const { btn, busyButton, iconButton, statusChip } = createButtons(dom, icons);
const { field, textInput, selectInput, checkbox, radio, hint, emptyHint } = createForms(dom);
const { fold, menu, toast, popover, detail } = createOverlays(dom, icons);

const root = document.querySelector('#gallery');

function section(title, note) {
  const s = el('section', 'gal-sec');
  s.appendChild(el('h2', '', title));
  if (note) s.appendChild(el('p', 'gal-note', note));
  root.appendChild(s);
  return s;
}

/* 色彩：只列语义令牌，值由 tokens.css 决定。 */
const colors = [
  ['bg', '--color-bg'], ['surface', '--color-surface'], ['surface-2', '--color-surface-2'],
  ['border', '--color-border'], ['border-strong', '--color-border-strong'],
  ['text', '--color-text'], ['text-dim', '--color-text-dim'],
  ['accent', '--color-accent'], ['accent-weak', '--color-accent-weak'],
  ['ok', '--color-ok'], ['warn', '--color-warn'], ['err', '--color-err'],
];
{
  const s = section('色彩', '深色只有一层底；抬升靠 surface，分隔靠 border。');
  const grid = el('div', 'gal-swatches');
  for (const [name, v] of colors) {
    const box = el('div', 'gal-swatch');
    const chip = el('i');
    chip.style.background = 'var(' + v + ')';
    box.appendChild(chip);
    box.appendChild(el('span', '', name));
    grid.appendChild(box);
  }
  s.appendChild(grid);
}

/* 字号：5 档，正文 16。 */
{
  const s = section('字号阶梯', '12 只给等宽事实；中文正文最小 14。');
  const stack = el('div', 'gal-stack');
  for (const [v, label] of [['--fs-xs', 'xs 12 · 机器事实'], ['--fs-s', 's 14 · 辅助'],
    ['--fs-base', 'base 16 · 正文'], ['--fs-l', 'l 20 · 分区标题'], ['--fs-xl', 'xl 28 · 页面标题']]) {
    const p = el('div', '', label);
    p.style.fontSize = 'var(' + v + ')';
    stack.appendChild(p);
  }
  s.appendChild(stack);
}

/* 按钮与状态。 */
{
  const s = section('按钮', '主按钮只有一个；危险动作才用 danger。');
  const row = el('div', 'gal-row');
  row.appendChild(btn('普通'));
  row.appendChild(btn('主按钮', 'btn btn-primary'));
  row.appendChild(btn('幽灵', 'btn btn-ghost'));
  row.appendChild(btn('危险', 'btn btn-danger'));
  row.appendChild(iconButton({ icon: 'gear', label: '设置' }));
  const busy = btn('忙态');
  busy.onclick = () => { const release = busyButton(busy, '处理中…'); setTimeout(release, 1200); };
  row.appendChild(busy);
  row.appendChild(statusChip({ text: '工作区', hint: '点击查看' }));
  s.appendChild(row);
}

/* 表单。 */
{
  const s = section('表单', '标签在上，说明在下；焦点只换边框色。');
  s.appendChild(field('工作名称', textInput('必填，会话落盘目录名')));
  s.appendChild(field('执行档位', selectInput([{ value: 'host', label: '本机档' }, { value: 'vm', label: '虚拟机档' }], 'host')));
  const row = el('div', 'gal-row');
  row.appendChild(checkbox('流式传输', true));
  const r = radio('本机档（脚本在宿主上跑）', true, 'gal-tier');
  row.appendChild(r.wrap);
  s.appendChild(row);
  s.appendChild(detail({
    lead: '首层只说做决定需要的事实。',
    detail: '原理、影响范围与边界条件收进这里；安全与不可逆提示常显。',
    safety: '这条是安全提示，永远不折叠。',
  }));
  s.appendChild(hint('这是一条旁注。', 'warn'));
  s.appendChild(emptyHint('（空态提示）'));
}

/* 浮层。 */
{
  const s = section('浮层', '菜单、提示与折叠共用同一套边框与阴影。');
  const row = el('div', 'gal-row');
  const menuBtn = btn('打开菜单');
  menuBtn.onclick = () => menu(menuBtn, [
    { label: '重命名', icon: 'pencil' },
    { label: '归档', icon: 'box' },
    { label: '删除', icon: 'close', danger: true },
    { label: '不可用项', disabled: true, reason: '这里说明为什么不可用。' },
  ]);
  row.appendChild(menuBtn);
  const toastBtn = btn('显示 toast');
  toastBtn.onclick = () => toast('ok', '已保存');
  row.appendChild(toastBtn);
  const popBtn = btn('打开小卡片');
  popBtn.onclick = () => {
    const box = el('div', 'gal-stack');
    box.appendChild(el('div', '', '锚定小卡片'));
    box.appendChild(el('div', 'cfg-hint', '点外面或按 Esc 关闭。'));
    popover(popBtn, box, { label: '小卡片' });
  };
  row.appendChild(popBtn);
  s.appendChild(row);
  s.appendChild(fold({ label: '折叠块', count: 3, body: el('div', 'cfg-hint', '折叠内容在这里。') }));
}

/* 图标。 */
{
  const s = section('图标', '全部内联 SVG，1.8 描边，随文字颜色。');
  const grid = el('div', 'gal-icons');
  for (const name of Object.keys(ICONS)) {
    const box = el('div', 'gal-icon');
    box.appendChild(icons.icon(name));
    box.appendChild(el('span', '', name));
    grid.appendChild(box);
  }
  s.appendChild(grid);
}

/* 转录状态轨：这一版唯一的持续动效。 */
{
  const s = section('转录状态轨', '左缘 2px：工作中呼吸，完成收成 1px 中性线，失败转 err。');
  const box = el('div', 'gal-track');
  const line = (cls, text) => {
    const n = el('div', 'line ' + cls);
    n.appendChild(el('div', '', text));
    return n;
  };
  box.appendChild(line('', '完成的轮次：左缘是一条安静的中性线。'));
  box.appendChild(line('streaming', '正在工作：左缘 2px 冰青，1.6 秒呼吸。'));
  box.appendChild(line('bad', '失败：左缘转 err，状态不只靠颜色，正文也说明。'));
  s.appendChild(box);
}
