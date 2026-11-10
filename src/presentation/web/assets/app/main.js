/* 目的：浏览器端的模块入口，也是唯一的组装点：建共享上下文、挂载各功能、填迁移桥。
 * 管：装配顺序与依赖注入（state / api / ui / app）；功能之间互不 import，只认 app 上的入口。
 * 不管：任何业务、渲染与 DOM 细节；功能自己决定用到哪些注入。
 * 联动：`index.html` 先引本文件（type="module"），再引 defer 的 `md.js` / `app.js`；
 *       冒烟（`*.smoke.cjs`）对沙箱调用 `boot`，与浏览器走同一段装配。 */

import { installBridge } from './bridge.js';
import * as format from './core/format.js';
import * as stateStore from './stores/state.js';
import { mount as mountSessions } from './stores/sessions.js';
import { mount as mountSettings } from './stores/settings.js';
import { mount as mountSettingsPages } from './features/settings/pages.js';
import { mount as mountSettingsRuntime } from './features/settings/runtime.js';
import { mount as mountSettingsStorage } from './features/settings/storage.js';
import { mount as mountSettingsHub } from './features/settings/hub.js';
import { mount as mountAgents } from './features/agents/agents.js';
import { mount as mountFiles } from './features/files/files.js';
import { mount as mountSetup } from './features/setup/setup.js';
import { mount as mountModules } from './features/modules/modules.js';
import { mount as mountSidebar } from './features/sidebar/sidebar.js';
import { mount as mountWizard } from './features/wizard/wizard.js';
import { mount as mountSessionConfig } from './features/session-config/session-config.js';
import { mount as mountTabs } from './features/tabs/tabs.js';
import { mount as mountStatusBar } from './features/status-bar/status-bar.js';
import { mount as mountComposer } from './features/composer/composer.js';
import { mount as mountTranscript } from './features/transcript/transcript.js';

/* 迁移期：这些入口仍在经典脚本 app.js 里（阶段 5–7 陆续迁走），功能经 app.* 晚绑定调用。 */
const APP_GLOBALS = [
  "act",
  "actionUrl",
  "activateFlow",
  "closeModal",
  "compactNow",
  "confirmBox",
  "ctxChipClass",
  "eventError",
  "loadWork",
  "modelOptions",
  "notice",
  "openContextCard",
  "openModal",
  "openModalLayer",
  "openShellCard",
  "openToolsMenu",
  "openWorkCard",
  "paintShellCard",
  "refreshState",
  "restoreMark",
  "rewindTo",
  "shellAbnormal",
  "shellRunning",
  "shellSecs",
  "statusPrintOf",
  "withdrawAgree",
];

/* 迁移期：已迁入口按原名挂到全局，供尚未改成按模块 import 的旧冒烟调用（阶段 7 删）。 */
const MIGRATED = [
  "escapeRe", "fmtBytes",
  "providersPage", "openProvidersModal", "thinkingTagsEditor", "thinkingMapEditor", "modelsPage",
  "openModelsModal", "coreModelBlock", "openCoreModal", "settingsForm", "parseSettingLines",
  "collectSettings", "saveSettings", "SETTINGS_PAGES", "choiceModal",
  "buildGeneratePage", "buildSandboxPage", "buildPermissionsPage", "buildResourcesPage",
  "buildStoragePage", "renderStorage", "cleanStorage", "resetGuest",
  "oneLine", "escHtml", "fmtInt", "ctxPercent", "fmtDuration", "timeAgo", "fmtDateTime",
  "activeSession", "snapshotRunning", "isBusy", "generating", "historyByParent", "activePath",
  "sessionParent", "knownChildren",
  "DEFAULT_THINKING_LEVELS", "thinkingLevels", "thinkingLabel", "PERMISSION_ORDER", "PERMISSION_LABELS",
  "openSettingsModal", "openSettingsHub", "openAgentsModal", "agentsPage",
  "moduleById", "moduleMatches", "moduleChips", "openModulePicker", "cloneAgent", "agentPreview",
  "agentForm", "openAgentEditor", "openTempAgentSheet", "openRegistryAgentPicker", "agentCard",
  "filesCache", "pickUploadFile", "suggestAltName", "sendUpload", "conflictUpload", "renameUpload",
  "setupAction", "openSetupModal", "diagnosisText",
  "modulesPage", "openModulesPanel", "renderModulesPanel", "useRow", "openUseCard", "fire", "panelSection", "actionRunner",
  "historyMenu", "renderHistory", "warnIfTierUnavailable", "hydrateHistory", "openHistory", "deleteHistory", "renderSidebar", "RAIL_KEY", "isNarrow", "readRail", "saveRail", "applyRail", "openDrawer", "closeDrawer", "toggleSidebar", "setupSidebar",
  "openWizard", "startSession", "setActive", "applyPathRoots", "loadPathRoots",
  "openConfig", "loadConfig", "buildConfigForm", "cfgAgentBlock", "cfgRuntimeBlock", "guestPicker", "cfgRadio",
  "tabLabel", "tabBusy", "tabPending", "tabPendingDeep", "closeView", "openPath", "renderTabs",
  "openPermissionMenu", "openThinkingMenu", "switchThinking", "switchPermission", "syncStatusbar",
  "atState", "COMMANDS", "atMenuEl", "atToken", "atClose", "atHint", "atLoad", "atFetch", "AT_QUOTE_CHARS", "AT_QUOTE_RE", "atQuotePath", "atItems", "atRender", "atOpen", "atInsert", "atKey", "cmdSync", "atOnInput", "autoGrow", "stopGeneration", "onSend", "syncComposer", "syncSendButton",
 "applySnapshot", "progress", "absorb", "absorbEvents", "applyBatch", "applyLines", "trimAlign", "resyncActive", "markNeedState", "lineParts", "mdNode", "appendBody", "foldOpen", "foldBind", "scrollKey", "flushScroll", "toolDetail", "toolCard", "buildRootList", "shortPath", "reasoningBlock", "rewindButton", "lineBlock", "sessionNote", "buildTurn", "nodeCard", "appendFinalRow", "appendFinalLine", "renderDone", "liveTurnNode", "updateLiveGroup", "renderLive", "renderStream", "scheduleLiveTick", "renderLiveTick", "renderGate", "renderCard", "answerCard", "gateCard", "updateTaskFlow", "updateTask", "takeInput", "renderAll", "renderReadonly", "syncTyping", "nearBottom", "buildStreamBoxes", "turnWorthy", "turnKey", "turnText", "turnReasoning", "pendingScroll", "doneState", "liveTickScheduled",];

/** 装配一次：装桥、挂功能、补转发；浏览器与冒烟沙箱走同一条路径。 */
export function boot(g) {
  const core = installBridge(g);
  const app = {};
  core.app = app;
  const deps = { g, doc: g.document, state: core.state, stores: stateStore, api: core.api, ui: g.ui, app };
  Object.assign(app, format);
  mountSessions(app, deps);
  mountSettings(app, deps);
  mountSettingsPages(app, deps);
  mountSettingsRuntime(app, deps);
  mountSettingsStorage(app, deps);
  mountSettingsHub(app, deps);
  mountAgents(app, deps);
  mountFiles(app, deps);
  mountSetup(app, deps);
  mountModules(app, deps);
  mountSidebar(app, deps);
  mountWizard(app, deps);
  mountSessionConfig(app, deps);
  mountTabs(app, deps);
  mountStatusBar(app, deps);
  mountComposer(app, deps);
  mountTranscript(app, deps);
  for (const name of APP_GLOBALS) app[name] = (...args) => g[name](...args);
  for (const name of MIGRATED) g[name] = app[name];
  return app;
}

/* 浏览器里由本文件自启；Node（冒烟）由调用方显式 boot(沙箱)。 */
if (typeof window !== "undefined" && typeof document !== "undefined") boot(globalThis);
