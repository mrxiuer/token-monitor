'use strict';

const { username, cycle } = require('../shared/duo');
const DEFAULT_HUB_URL = 'https://47.97.44.177/token-monitor';

function duoSettings(settings) {
  if (settings.duoProfileVersion === 1) return settings;
  // Keep native materials, appearance, device identity, credentials and archives
  // intact. This is a one-time migration of the edition's working defaults.
  return { ...settings, duoProfileVersion: 1, duoUsername: username(settings.duoUsername), duoCycles: [],
    clients: 'codex', limitProviders: 'codex', pluginsEnabled: false, discordRpcEnabled: false,
    codexResetForecastEnabled: false, notchEnabled: false, edgeDockEnabled: false, floatingBubbleEnabled: false,
    automaticAppUpdates: false, hubMode: 'client', hubUrl: settings.hubUrl || DEFAULT_HUB_URL,
    syncSessionTitles: false, showTrayIcon: true, language: settings.language || 'zh-CN',
    viewDisplayOrder: 'member,device,limits,model,session,project,home,tool,trends,status',
    hiddenViews: 'home,tool,trends,status,project,session',
    lastViewState: { period: 'cycle', breakdown: 'member' } };
}

function normalizedDuoPatch(patch) {
  const out = { ...patch };
  if (Object.hasOwn(out, 'duoUsername')) out.duoUsername = username(out.duoUsername);
  if (Object.hasOwn(out, 'duoCycles')) out.duoCycles = (Array.isArray(out.duoCycles) ? out.duoCycles : []).map(cycle).filter(Boolean).slice(-104);
  if (Object.hasOwn(out, 'duoDeviceName')) out.duoDeviceName = username(out.duoDeviceName);
  return out;
}

module.exports = { duoSettings, normalizedDuoPatch, DEFAULT_HUB_URL };
