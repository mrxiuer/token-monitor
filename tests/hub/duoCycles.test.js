'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { refreshQuotaCycles } = require('../../src/shared/duoCycles');
function state() { return { members: [], bindings: {}, cycles: [], source: null }; }
function observe(state, at, used, count, options = {}) {
  const provider = { provider: 'codex', status: 'ok', accountKey: options.accountKey || 'shared-pro', planLabel: options.plan || 'Pro 100', updatedAt: at,
    windows: [{ kind: 'weekly', windowMinutes: 10080, usedPercent: used, resetsAt: options.end || '2026-11-08T12:00:00Z' }],
    resetCredits: { availableCount: count, expirations: options.expirations || [] } };
  refreshQuotaCycles(state, [{ deviceId: options.deviceId || 'device-a', updatedAt: at, limits: { providers: [provider] } }], Date.parse(at));
}
test('automatic Pro cycles are account-bound and can use another device on that account', () => {
  const s = state(); observe(s, '2026-11-03T00:00:00Z', 50, 2);
  observe(s, '2026-11-03T00:01:00Z', 51, 2, { deviceId: 'device-b' });
  assert.equal(s.source.deviceId, 'device-b');
  observe(s, '2026-11-03T00:02:00Z', 0, 1, { accountKey: 'another-account' });
  assert.equal(s.cycles.length, 1);
  observe(s, '2026-11-08T12:01:00Z', 0, 2, { deviceId: 'device-b', end: '2026-11-15T12:00:00Z' });
  assert.equal(s.cycle.start, '2026-11-08T12:00:00.000Z'); assert.equal(s.cycles.length, 2);
});
test('an early official window boundary closes the prior cycle without overlap', () => {
  const s = state(); observe(s, '2026-11-03T00:00:00Z', 90, 2);
  observe(s, '2026-11-03T00:01:00Z', 0, 1, { end: '2026-11-10T00:00:30Z' });
  assert.equal(s.cycles[0].end, '2026-11-03T00:00:30.000Z');
  assert.equal(s.cycle.start, s.cycles[0].end); assert.equal(s.uncertainRanges.length, 0);
});
test('a reset card plus weekly drop keeps the observation interval unassigned', () => {
  const s = state(); observe(s, '2026-11-03T00:00:00Z', 90, 2);
  observe(s, '2026-11-03T00:01:00Z', 0, 1);
  assert.equal(s.cycles[0].end, '2026-11-03T00:00:00.000Z');
  assert.equal(s.cycle.start, '2026-11-03T00:01:00.000Z');
  assert.deepEqual([s.uncertainRanges[0].start, s.uncertainRanges[0].end, s.uncertainRanges[0].status], ['2026-11-03T00:00:00.000Z', '2026-11-03T00:01:00.000Z', 'pending']);
});
test('short-window-only card use and expiration do not create weekly cycles', () => {
  const s = state(); observe(s, '2026-11-03T00:00:00Z', 50, 2);
  observe(s, '2026-11-03T00:01:00Z', 51, 1);
  observe(s, '2026-11-03T00:02:00Z', 52, 0);
  assert.equal(s.cycles.length, 1); assert.equal(s.uncertainRanges.length, 0);
  const expired = state(); observe(expired, '2026-11-03T00:00:00Z', 50, 2, { expirations: ['2026-11-03T00:00:30Z'] });
  observe(expired, '2026-11-03T00:01:00Z', 48, 1);
  assert.equal(expired.cycles.length, 1); assert.equal(expired.uncertainRanges.length, 0);
});
test('percentage-only resets need a second fresh reading; corrections and stale data cannot switch', () => {
  const s = state(); observe(s, '2026-11-03T00:00:00Z', 70, null);
  observe(s, '2026-11-03T00:01:00Z', 0, null); assert.ok(s.pendingReset); assert.equal(s.cycles.length, 1);
  observe(s, '2026-11-03T00:00:30Z', 0, null); assert.equal(s.cycles.length, 1);
  observe(s, '2026-11-03T00:02:00Z', 70, null); assert.equal(s.pendingReset, undefined);
  observe(s, '2026-11-03T00:03:00Z', 0, null); observe(s, '2026-11-03T00:04:00Z', 2, null);
  assert.equal(s.cycles.length, 2); assert.equal(s.uncertainRanges[0].end, '2026-11-03T00:03:00.000Z');
});
test('later official evidence resolves a pending interval and recomputes both adjacent ranges', () => {
  const s = state(); observe(s, '2026-11-03T00:00:00Z', 90, 2); observe(s, '2026-11-03T00:01:00Z', 0, 1);
  observe(s, '2026-11-03T00:02:00Z', 1, 1, { end: '2026-11-10T00:00:30Z' });
  assert.equal(s.uncertainRanges[0].status, 'resolved');
  assert.equal(s.cycles[0].end, '2026-11-03T00:00:30.000Z'); assert.equal(s.cycle.start, s.cycles[0].end);
});
test('a delayed quota response can still pair a weekly reset with the recent credit change', () => {
  const s = state(); observe(s, '2026-11-03T00:00:00Z', 5, 2);
  observe(s, '2026-11-03T00:01:00Z', 5, 1); assert.equal(s.cycles.length, 1);
  observe(s, '2026-11-03T00:02:00Z', 0, 1);
  assert.equal(s.cycles.length, 2); assert.equal(s.uncertainRanges[0].start, '2026-11-03T00:00:00.000Z');
});
