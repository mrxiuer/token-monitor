'use strict';

const { cycle } = require('./duo');

// Quota observations identify windows; a lower percentage alone does not prove
// when a reset happened. Ambiguous boundary intervals stay separate from both
// adjacent cycles, so all devices can recompute from their numerical ledgers.
function refreshQuotaCycles(state, records, now = Date.now()) {
  state.uncertainRanges ||= [];
  if (state.version !== 2) { state.version = 2; state.manual = false; delete state.observation; delete state.pendingReset; }
  const candidates = [];
  for (const record of records) for (const provider of record.limits?.providers || []) {
    if (provider.provider !== 'codex' || provider.status !== 'ok' || !/pro/i.test(provider.planLabel || provider.accountLabel || '')) continue;
    const accountKey = provider.accountKey || '';
    if (state.source && (state.source.accountKey ? accountKey !== state.source.accountKey : record.deviceId !== state.source.deviceId)) continue;
    const at = Date.parse(provider.updatedAt || record.limits.updatedAt || record.updatedAt);
    if (!Number.isFinite(at) || at > now + 120000) continue;
    for (const window of provider.windows || []) {
      if (window.kind !== 'weekly' || window.additional || !window.resetsAt || !(Number(window.windowMinutes) > 0)) continue;
      const end = Date.parse(window.resetsAt), duration = Number(window.windowMinutes) * 60000;
      if (!Number.isFinite(end) || !Number.isFinite(duration) || duration > 32 * 86400000 || end <= now || end - duration > now) continue;
      const rawUsed = Number(window.usedPercent);
      const credits = provider.resetCredits;
      const count = credits?.availableCount;
      candidates.push({ at, end, duration, used: Number.isFinite(rawUsed) && window.usedPercent !== null ? rawUsed : null,
        count: typeof count === 'number' && Number.isFinite(count) ? count : null,
        expirations: credits?.expirations || [], deviceId: record.deviceId, accountKey });
    }
  }
  candidates.sort((a, b) => b.at - a.at);
  const current = candidates[0];
  if (!current) return;
  const previous = state.observation;
  if (previous && current.at <= previous.at) return;
  state.source = { accountKey: current.accountKey, deviceId: current.deviceId };
  const officialStart = current.end - current.duration;
  function remember(range) {
    if (!state.cycles.some((item) => item.id === range.id)) state.cycles.push(range);
    state.cycles.sort((a, b) => Date.parse(a.start) - Date.parse(b.start));
    state.cycles = state.cycles.slice(-104); state.cycle = range;
  }
  function replace(old, replacement) {
    state.cycles = state.cycles.filter((item) => item.id !== old.id);
    if (replacement && !state.cycles.some((item) => item.id === replacement.id)) state.cycles.push(replacement);
  }
  function transition(start, end, reason, interval = null) {
    const old = state.cycle;
    const oldStart = Date.parse(old.start), oldEnd = Date.parse(old.end);
    if (start <= oldStart || start >= end) return;
    const closeAt = Math.min(oldEnd, interval ? interval.start : start);
    replace(old, cycle({ start: old.start, end: new Date(closeAt).toISOString() }));
    const next = cycle({ start: new Date(start).toISOString(), end: new Date(end).toISOString() });
    if (!next) return;
    remember(next);
    if (interval) {
      const range = cycle({ start: new Date(interval.start).toISOString(), end: new Date(interval.end).toISOString() });
      if (range) state.uncertainRanges.push({ ...range, status: 'pending', reason,
        beforeStart: old.start, afterStart: next.start, observedBefore: range.start, observedAfter: range.end });
      state.uncertainRanges = state.uncertainRanges.slice(-104);
    }
    delete state.pendingReset;
    delete state.creditHint;
  }
  if (!state.cycle) remember(cycle({ start: new Date(officialStart).toISOString(), end: new Date(current.end).toISOString() }));
  else {
    const activeStart = Date.parse(state.cycle.start), activeEnd = Date.parse(state.cycle.end);
    const gap = state.uncertainRanges.find((range) => range.status === 'pending' && range.afterStart === state.cycle.start
      && officialStart >= Date.parse(range.start) && officialStart <= Date.parse(range.end) && current.end !== activeEnd);
    if (gap) {
      const old = state.cycles.find((range) => range.start === gap.beforeStart && range.end === gap.start);
      if (old) replace(old, cycle({ start: old.start, end: new Date(officialStart).toISOString() }));
      replace(state.cycle, null); remember(cycle({ start: new Date(officialStart).toISOString(), end: new Date(current.end).toISOString() }));
      gap.status = 'resolved'; gap.resolvedAt = new Date(officialStart).toISOString(); delete state.pendingReset;
    } else if (officialStart >= activeEnd) {
      transition(officialStart, current.end, 'scheduled-reset');
    } else if (previous && current.end !== activeEnd && officialStart > activeStart
      && officialStart >= previous.at - 5000 && officialStart <= current.at + 5000) {
      transition(officialStart, current.end, 'reported-window-reset');
    } else if (previous && current.at < activeEnd && previous.used !== null && current.used !== null) {
      const drop = previous.used - current.used;
      const expired = (previous.expirations || []).some((date) => Date.parse(date) > previous.at && Date.parse(date) <= current.at);
      const spent = previous.count !== null && current.count !== null && current.count < previous.count && !expired;
      if (spent) state.creditHint = { before: previous, at: current.at };
      const hint = state.creditHint;
      const delayedReset = hint && current.at - hint.at <= 10 * 60000 && hint.before.end === activeEnd
        && hint.before.used !== null && hint.before.used - current.used > 0.5 && (current.used <= 1 || hint.before.used - current.used >= 10);
      const pending = state.pendingReset;
      if (pending && current.end === pending.after.end && current.used < pending.before.used - 1) {
        transition(pending.after.at, current.end, 'confirmed-quota-drop', { start: pending.before.at, end: pending.after.at });
      } else if ((spent && drop > 0.5) || delayedReset) {
        transition(current.at, current.end, 'reset-credit-and-weekly-drop', { start: hint?.before.at || previous.at, end: current.at });
      } else if (drop >= 10 && current.used <= 5 && current.end === activeEnd) {
        state.pendingReset = { before: previous, after: current };
      } else if (pending) delete state.pendingReset;
    }
  }
  state.observation = current;
}

module.exports = { refreshQuotaCycles };
