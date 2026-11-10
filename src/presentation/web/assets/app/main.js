/* 目的：浏览器端的模块入口，也是唯一的组装点：建共享上下文、挂载各功能、填迁移桥。
 * 管：装配顺序与依赖注入（state / api / ui / app）；功能之间互不 import，只认 app 上的入口。
 * 不管：任何业务、渲染与 DOM 细节；功能自己决定用到哪些注入。
 * 联动：`index.html` 先引本文件（type="module"），再引 defer 的 `md.js` / `app.js`；
 *       冒烟（`*.smoke.cjs`）对沙箱调用 `boot`，与浏览器走同一段装配。 */

import { installBridge } from './bridge.js';
import * as format from './core/format.js';
import { mount as mountSettingsPages } from './features/settings/pages.js';
import { mount as mountSettingsRuntime } from './features/settings/runtime.js';
import { mount as mountSettingsStorage } from './features/settings/storage.js';
import { mount as mountSettingsHub } from './features/settings/hub.js';
import { mount as mountAgents } from './features/agents/agents.js';
import { mount as mountFiles } from './features/files/files.js';
import { mount as mountSetup } from './features/setup/setup.js';
import { mount as mountModules } from './features/modules/modules.js';

/* 迁移期：这些入口仍在经典脚本 app.js 里（阶段 5–7 陆续迁走），功能经 app.* 晚绑定调用。 */
const APP_GLOBALS = [
  "openModal", "openModalLayer", "closeModal", "notice", "confirmBox", "refreshState",
  "modelOptions", "oneLine", "thinkingLevels", "actionUrl", "activeSession", "loadWork",
  "cfgDiv", "cfgHint", "closeDrawer", "setupSidebar", "openWizard", "pollLoop", "onRegistryChange",
];

/* 迁移期：已迁入口按原名挂到全局，供尚未改成按模块 import 的旧冒烟调用（阶段 7 删）。 */
const MIGRATED = [
  "escapeRe", "fmtBytes",
  "providersPage", "openProvidersModal", "thinkingTagsEditor", "thinkingMapEditor", "modelsPage",
  "openModelsModal", "coreModelBlock", "openCoreModal", "settingsForm", "parseSettingLines",
  "collectSettings", "saveSettings", "SETTINGS_PAGES", "choiceModal",
  "buildGeneratePage", "buildSandboxPage", "buildPermissionsPage", "buildResourcesPage",
  "buildStoragePage", "renderStorage", "cleanStorage", "resetGuest",
  "openSettingsModal", "openSettingsHub", "openAgentsModal", "agentsPage",
  "moduleById", "moduleMatches", "moduleChips", "openModulePicker", "cloneAgent", "agentPreview",
  "agentForm", "openAgentEditor", "openTempAgentSheet", "openRegistryAgentPicker", "agentCard",
  "filesCache", "pickUploadFile", "suggestAltName", "sendUpload", "conflictUpload", "renameUpload",
  "setupAction", "openSetupModal", "diagnosisText",
  "modulesPage", "openModulesPanel", "renderModulesPanel", "useRow", "openUseCard", "fire", "panelSection", "actionRunner",
];

/** 装配一次：装桥、挂功能、补转发；浏览器与冒烟沙箱走同一条路径。 */
export function boot(g) {
  const core = installBridge(g);
  const app = {};
  core.app = app;
  const deps = { g, doc: g.document, state: core.state, api: core.api, ui: g.ui, app };
  Object.assign(app, format);
  mountSettingsPages(app, deps);
  mountSettingsRuntime(app, deps);
  mountSettingsStorage(app, deps);
  mountSettingsHub(app, deps);
  mountAgents(app, deps);
  mountFiles(app, deps);
  mountSetup(app, deps);
  mountModules(app, deps);
  for (const name of APP_GLOBALS) app[name] = (...args) => g[name](...args);
  for (const name of MIGRATED) g[name] = app[name];
  return app;
}

/* 浏览器里由本文件自启；Node（冒烟）由调用方显式 boot(沙箱)。 */
if (typeof window !== "undefined" && typeof document !== "undefined") boot(globalThis);
