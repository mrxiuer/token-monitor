'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const duo = require('../../src/shared/duo');
const { parseUsage, collectQuotaPeriods } = require('../../src/shared/duoCollector');
const { normalizeDeviceRecord, mergeDeviceRecord, aggregateDevices } = require('../../src/shared/usage');
const { duoSettings } = require('../../src/electron/duoSettings');

const range = duo.cycle({ start: '2026-11-01T12:00:00Z', end: '2026-11-08T12:00:00Z' });
function usage(at, input, output, cached = 0) {
  return JSON.stringify({ type: 'event_msg', timestamp: at, payload: { type: 'token_count', info: {
    total_token_usage: { input_tokens: input, output_tokens: output, cached_input_tokens: cached },
    last_token_usage: { input_tokens: input, output_tokens: output, cached_input_tokens: cached }
  } } });
}

test('cycle parsing counts cumulative increments once across turn and reset boundaries', () => {
  const lines = [JSON.stringify({ type: 'session_meta', payload: { id: 'same-session', model: 'test-model' } }),
    usage('2026-11-01T11:59:59Z', 100, 20, 40),
    usage('2026-11-01T12:00:00Z', 150, 30, 60),
    usage('2026-11-01T12:00:01Z', 150, 30, 60),
    usage('2026-11-08T12:00:00Z', 200, 40, 80)];
  const result = parseUsage(lines, 'fallback');
  assert.equal(result.partial, false);
  assert.equal(result.events.length, 3);
  const inside = result.events.filter((event) => event.at >= Date.parse(range.start) && event.at < Date.parse(range.end));
  assert.equal(inside.length, 1);
  assert.deepEqual([inside[0].input, inside[0].cacheRead, inside[0].output, inside[0].total], [30, 20, 10, 60]);
});

test('truncated cumulative prefixes and unidentified last-only records are incomplete', () => {
  const truncated = JSON.parse(usage(range.start, 1000, 100));
  truncated.payload.info.last_token_usage = { input_tokens: 10, output_tokens: 2 };
  const result = parseUsage([JSON.stringify(truncated)], 'x');
  assert.equal(result.partial, true);
  assert.equal(result.events[0].total, 12);
  delete truncated.payload.info.total_token_usage;
  assert.equal(parseUsage([JSON.stringify(truncated)], 'x').events.length, 0);
});

test('native cache deduplicates archived copies, prices buckets, survives deleted logs and contains no conversation text', async () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'duo-collector-'));
  try {
    const sessions = path.join(home, '.codex', 'sessions');
    const archives = path.join(home, '.codex', 'archived_sessions');
    fs.mkdirSync(sessions, { recursive: true }); fs.mkdirSync(archives);
    const lines = [JSON.stringify({ type: 'session_meta', payload: { id: 'id', model: 'test-model' } }),
      JSON.stringify({ type: 'event_msg', payload: { type: 'user_message', message: 'PRIVATE PROMPT' } }), usage(range.start, 100, 20, 40)];
    fs.writeFileSync(path.join(sessions, 'id.jsonl'), lines.join('\n'));
    fs.writeFileSync(path.join(archives, 'id.jsonl'), lines.join('\n'));
    const store = path.join(home, 'numeric.json');
    let lookups = 0;
    const options = { duoEnabled: true, duoCycles: [range], homeDir: home, env: {}, duoStorePath: store,
      pricingRevision: home, lookupModelPricing: async () => { lookups++; return { pricing: { inputCostPerToken: .01, outputCostPerToken: .03, cacheReadInputTokenCost: .001 } }; } };
    const first = (await collectQuotaPeriods(options))[0];
    assert.equal(first.totalTokens, 120);
    assert.ok(Math.abs(first.costUsd - 1.24) < 1e-9);
    assert.equal(first.unpricedTokens, 0);
    assert.equal(first.status, 'complete');
    assert.ok(!fs.readFileSync(store, 'utf8').includes('PRIVATE PROMPT'));
    fs.rmSync(sessions, { recursive: true }); fs.rmSync(archives, { recursive: true });
    assert.equal((await collectQuotaPeriods(options))[0].totalTokens, 120);
    assert.equal(lookups, 1);
  } finally { fs.rmSync(home, { recursive: true, force: true }); }
});

test('wire projection retains exact zero corrections and rejects raw fields', () => {
  const record = normalizeDeviceRecord({ deviceId: 'one', quotaPeriods: [{ ...range, totalTokens: 8, status: 'complete', prompt: 'PRIVATE' }] });
  assert.equal(record.quotaPeriods[0].totalTokens, 8);
  assert.ok(!JSON.stringify(record).includes('PRIVATE'));
  const retained = mergeDeviceRecord(record, { deviceId: 'one', limitsOnly: true });
  assert.equal(retained.quotaPeriods[0].totalTokens, 8);
  const corrected = mergeDeviceRecord(record, { deviceId: 'one', quotaPeriods: [{ ...range, totalTokens: 0, status: 'complete' }] });
  assert.equal(aggregateDevices([corrected], 0).devices[0].quotaPeriods[0].totalTokens, 0);
});

test('member summaries retain unavailable device coverage instead of assuming zero usage', () => {
  const stats = { duo: { cycle: range }, devices: [
    { deviceId: 'mac', memberId: 'a', memberName: 'A', quotaPeriods: [{ ...range, totalTokens: 10, status: 'complete' }] },
    { deviceId: 'win', memberId: 'a', memberName: 'A' }
  ] };
  const data = duo.snapshot(stats);
  assert.equal(data.members.length, 1);
  assert.equal(data.total.totalTokens, 10);
  assert.equal(data.total.status, 'partial');
  assert.equal(data.devices[1].usage.status, 'unavailable');
});

test('edition migration preserves native glass and original usage history', () => {
  const old = { systemGlass: true, glassOpacity: .7, blur: 32, deviceId: 'stable', archivedClientUsage: { old: true } };
  const migrated = duoSettings(old);
  assert.equal(migrated.systemGlass, true); assert.equal(migrated.glassOpacity, .7); assert.equal(migrated.blur, 32);
  assert.equal(migrated.deviceId, 'stable'); assert.deepEqual(migrated.archivedClientUsage, old.archivedClientUsage);
  assert.equal(migrated.clients, 'codex'); assert.equal(migrated.pluginsEnabled, false);
  assert.deepEqual(duoSettings({ ...migrated, lastViewState: { period: 'today' } }).lastViewState, { period: 'today' });
});

test('counter resets resume at explicit new requests without a negative delta', () => {
  const result = parseUsage([usage(range.start, 100, 20), usage('2026-11-01T13:00:00Z', 10, 2),
    usage('2026-11-01T14:00:00Z', 20, 4)], 'reset');
  assert.equal(result.partial, true);
  assert.deepEqual(result.events.map((event) => event.total), [120, 12, 12]);
});

test('Dots requests use observation times and yield to retained native threads', async () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'duo-dots-'));
  try {
    const sessions = path.join(home, '.codex', 'sessions'); fs.mkdirSync(sessions, { recursive: true });
    fs.writeFileSync(path.join(sessions, 'native.jsonl'), [JSON.stringify({ type: 'session_meta', payload: { id: 'native' } }), usage(range.start, 10, 2)].join('\n'));
    const row = { threadId: 'dots', model: 'unknown', observedAt: range.start,
      usage: { input: 10, output: 2, cacheRead: 0, cacheWrite: 0, total: 12 } };
    const options = { duoEnabled: true, duoCycles: [range], homeDir: home, env: {}, duoStorePath: path.join(home, 'numeric.json'),
      codexLocalUsageStore: { rows: () => [row, { ...row, threadId: 'native' }, { ...row, observedAt: range.end }] } };
    const result = (await collectQuotaPeriods(options))[0];
    assert.equal(result.totalTokens, 24);
    assert.equal(result.status, 'partial');
    assert.equal(result.unpricedTokens, 24);
    fs.rmSync(sessions, { recursive: true });
    assert.equal((await collectQuotaPeriods(options))[0].totalTokens, 24);
  } finally { fs.rmSync(home, { recursive: true, force: true }); }
});

test('all missing devices stay unavailable and CSV names cannot become formulas', () => {
  assert.equal(duo.snapshot({ duo: { cycle: range }, devices: [{ deviceId: 'legacy' }] }).total.status, 'unavailable');
  assert.equal(duo.csvCell('=HYPERLINK("x")'), '"\'=HYPERLINK(""x"")"');
  assert.equal(duo.csvCell('normal, name'), '"normal, name"');
});
test('cycle and calendar model details survive Hub normalization without raw conversation fields', () => {
  const model = { totalTokens: 12, costUsd: 1.2, inputTokens: 5, outputTokens: 4, cacheReadTokens: 3, unpricedTokens: 0, prompt: 'PRIVATE' };
  const payload = { deviceId: 'native', quotaPeriods: [{ ...range, totalTokens: 12, models: { 'gpt-test': model } }],
    today: { totalTokens: 12, clients: { codex: 12 }, duoModels: { 'gpt-test': model } } };
  const device = aggregateDevices([normalizeDeviceRecord(payload)], 0).devices[0];
  assert.equal(device.quotaPeriods[0].models['gpt-test'].totalTokens, 12);
  assert.equal(device.periods.today.duoModels['gpt-test'].costUsd, 1.2);
  assert.ok(!JSON.stringify(device).includes('PRIVATE'));
});
