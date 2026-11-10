/* 目的：设置中心里「生成与上下文」「沙箱围栏」「权限默认值」「资源与进程」四页的控件。
 * 管：这四页的表单字段与说明文案，读写设置值。
 * 不管：提交与保存（见 pages.js 的 settingsForm / saveSettings）、分页框架（hub.js）。
 * 联动：pages.js 的 settingsForm 逐页调用。
 */
import { settingsCopy } from './copy.js';

let doc, api, ui;
let field;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ doc, api, ui } = deps);
  ({ field } = ui);
  Object.assign(root, { buildGeneratePage, buildSandboxPage, buildPermissionsPage, buildResourcesPage });
}
/// 分区标题：把一页的字段分成可扫的几组，组间画一条细线。
function secTitle(text) {
  const h = doc.createElement('div');
  h.className = 'form-sec-title';
  h.textContent = text;
  return h;
}

/// 1 页：生成与上下文。
function buildGeneratePage(c, f) {
  c.body.appendChild(secTitle('输出'));
  c.body.appendChild(f.stream.wrap);
  c.body.appendChild(f.cot.wrap);
  c.body.appendChild(field('思考等级', f.thinkLevels.node, settingsCopy.thinkingLevels));
  c.body.appendChild(secTitle('上下文与压缩'));
  c.body.appendChild(ui.attachHelp(f.to.wrap, settingsCopy.firstResponseTimeout));
  c.body.appendChild(ui.attachHelp(f.remind.wrap, settingsCopy.remindCap));
  c.body.appendChild(ui.attachHelp(f.pct.wrap, settingsCopy.compactAt));
  c.body.appendChild(ui.attachHelp(f.keep.wrap, settingsCopy.compactKeep));
  c.body.appendChild(field('压缩用模型', f.compactModel, settingsCopy.compactModel));
}

/// 2 页：沙箱围栏（全局硬约束）。
function buildSandboxPage(c, f) {
  c.body.appendChild(ui.detail(settingsCopy.sandboxHard));
  c.body.appendChild(secTitle('围栏策略'));
  c.body.appendChild(f.network.wrap);
  c.body.appendChild(field('文件系统围栏', f.enFs));
  c.body.appendChild(field('网络围栏（仅在网络关闭时需要）', f.enNet));
  c.body.appendChild(field('进程树围栏', f.enTree));
  c.body.appendChild(field('环境白名单', f.enEnv));
  c.body.appendChild(field('机制不可用时', f.onUnavailable));
  c.body.appendChild(secTitle('Windows 容器'));
  c.body.appendChild(f.writeAcl.wrap);
  c.body.appendChild(ui.detail(settingsCopy.windowsAcl));
}

/// 3 页：权限默认值。
function buildPermissionsPage(c, f) {
  c.body.appendChild(ui.detail(settingsCopy.permissionDefaults));
  c.body.appendChild(secTitle('可达范围'));
  c.body.appendChild(f.hostRoots.f);
  c.body.appendChild(f.hardDeny.f);
  c.body.appendChild(secTitle('默认授权'));
  c.body.appendChild(field('默认权限档位', f.defProfile));
  c.body.appendChild(f.defSharedRead.wrap);
  c.body.appendChild(f.defSharedWrite.wrap);
  c.body.appendChild(f.defTools.f);
}

/// 4 页：资源与进程。
function buildResourcesPage(c, f) {
  c.body.appendChild(secTitle('并发'));
  c.body.appendChild(ui.attachHelp(f.guests.wrap, settingsCopy.guestLimits));
  c.body.appendChild(secTitle('进程'));
  const title = doc.createElement('div');
  title.className = 'wf-label';
  title.textContent = '现在在跑什么';
  const box = doc.createElement('div');
  box.className = 'cfg-log';
  box.textContent = '读取中…';
  c.body.appendChild(title);
  c.body.appendChild(box);
  api('GET', '/api/processes').then((r) => {
    const list = (r && r.procs) || [];
    box.textContent = list.length ? list.map((p) =>
      (p.alive ? '在跑' : '已退出（等对账清）') + ' · ' + p.purpose + ' · ' + p.owner + ' · pid ' + p.pid +
      (p.orphanRisk ? ' · 父死子亡未挂上（核心被强杀会留下）：' + (p.guardNote || '') : '')
    ).join('\n') : '没有产品起的长驻进程。';
  }).catch((e) => { box.textContent = '读不到：' + e.message; });
}