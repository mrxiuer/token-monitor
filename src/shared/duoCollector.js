'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { cycle, sum } = require('./duo');
const { sharedDataDir, readJson } = require('./config');
const { codexHomeDir } = require('./sessionFiles');
const { readTranscriptLines } = require('./sessionDetail');
const { estimatedCost, resolveLocalUsagePricing } = require('./providers/codex/localUsage');
const { createLocalUsageStore } = require('./providers/codex/localUsageStore');
const { wslUsageHomes } = require('./wslUsage');

// Only numerical accounting records are retained. Prompts, replies and tool
// arguments are neither saved here nor added to the synchronization envelope.
function parseUsage(lines, fallbackId) {
  const events = [];
  let id = fallbackId, model = 'unknown', previous = null, partial = false;
  for (const line of lines) {
    if (!line.trim()) continue;
    let record;
    try { record = JSON.parse(line); } catch (_) { partial = true; continue; }
    const payload = record.payload || {};
    if (record.type === 'session_meta') { id = String(payload.id || id); model = payload.model || model; }
    if (record.type === 'turn_context' && payload.model) model = String(payload.model);
    if (record.type !== 'event_msg' || payload.type !== 'token_count' || !payload.info) continue;
    const total = payload.info.total_token_usage;
    const last = payload.info.last_token_usage;
    if (!total && !last) continue;
    const clean = (value) => {
      const count = (field) => Number.isSafeInteger(Number(value[field])) && Number(value[field]) >= 0 ? Number(value[field]) : 0;
      const input = count('input_tokens'), output = count('output_tokens');
      return { input, output, cache: Math.min(input, count('cached_input_tokens')) };
    };
    const current = clean(total || last);
    let delta;
    if (total) {
      if (previous && current.input === previous.input && current.output === previous.output && current.cache === previous.cache) continue;
      if (previous && (current.input < previous.input || current.output < previous.output || current.cache < previous.cache)) {
        partial = true; previous = null;
      }
      delta = { input: current.input - (previous?.input || 0), output: current.output - (previous?.output || 0), cache: current.cache - (previous?.cache || 0) };
      // An imported/truncated transcript may start with a historical cumulative
      // prefix. Attribute only its explicit last request, retaining the gap as
      // incomplete rather than moving the prefix to the first surviving date.
      if (last) {
        const first = clean(last);
        if (first.input + first.output < delta.input + delta.output) { delta = first; partial = true; }
      }
      previous = current;
    } else {
      // Without a cumulative identity, repeated last-usage records cannot be
      // distinguished from equally sized requests. Do not guess a subtotal.
      partial = true; continue;
    }
    const at = Date.parse(record.timestamp);
    if (!Number.isFinite(at) || delta.input < 0 || delta.output < 0 || delta.cache < 0 || delta.cache > delta.input) { partial = true; continue; }
    if (delta.input + delta.output === 0) continue;
    events.push({ key: `${id}:${current.input}:${current.output}:${current.cache}`, at,
      model: String(payload.info.model || model).slice(0, 200),
      input: delta.input - delta.cache, output: delta.output, cacheRead: delta.cache,
      cacheWrite: 0, total: delta.input + delta.output });
  }
  return { events, partial, nativeId: id };
}

const stores = new Map();
async function collectQuotaPeriods(options = {}) {
  const ranges = (options.duoCycles || []).map(cycle).filter(Boolean);
  if (!options.duoEnabled || !ranges.length) return [];
  const first = Math.min(...ranges.map((item) => Date.parse(item.start)));
  const home = options.homeDir || os.homedir();
  const dataPath = options.duoStorePath || path.join(sharedDataDir(), 'duo-usage.json');
  let cache = stores.get(dataPath);
  if (!cache) {
    cache = readJson(dataPath, { version: 1, files: {} });
    if (cache?.version !== 1 || !cache.files || typeof cache.files !== 'object' || Array.isArray(cache.files)) throw new Error('周期用量缓存格式无效');
    stores.set(dataPath, cache);
  }
  const codexHome = codexHomeDir(home, { env: options.env || process.env });
  const roots = [path.join(codexHome, 'sessions'), path.join(codexHome, 'archived_sessions'), ...(options.customScanPaths?.codex || [])];
  const visited = new Set();
  let changed = false, missingSources = 0, visitedEntries = 0;
  if ((options.platform || process.platform) === 'win32' && options.wslScanEnabled !== false) {
    if (!options.wslAnchor || options.refreshWsl || !cache.wslHomes) {
      cache.wslHomes = wslUsageHomes(options.wslDeps); changed = true;
    }
    for (const wslHome of cache.wslHomes) for (const name of ['sessions', 'archived_sessions']) roots.push(path.join(wslHome, '.codex', name));
    if (options.wslHasCodex && !cache.wslHomes.length) missingSources++;
  }
  function walk(directory, depth = 0) {
    if (depth > 32 || visitedEntries > 100000) { missingSources++; return; }
    let real, entries;
    try { real = fs.realpathSync(directory); if (visited.has(real)) return; visited.add(real); entries = fs.readdirSync(real, { withFileTypes: true }); }
    catch (error) { if (error.code !== 'ENOENT') missingSources++; return; }
    for (const entry of entries) {
      if (visitedEntries >= 100000) { missingSources++; break; }
      visitedEntries++;
      if (entry.isDirectory()) { walk(path.join(real, entry.name), depth + 1); continue; }
      if (!entry.isFile() || !/\.jsonl(?:\.zst)?$/.test(entry.name)) continue;
      const file = path.join(real, entry.name);
      const key = crypto.createHash('sha256').update(file).digest('hex');
      try {
        const stat = fs.statSync(file);
        const fingerprint = `${stat.size}:${stat.mtimeMs}:${stat.ctimeMs}`;
        if (cache.files[key]?.fingerprint === fingerprint) continue;
        // A transcript that has not changed before the earliest requested
        // cycle cannot own newer events. Retained cached observations survive.
        if (stat.mtimeMs < first && !cache.files[key]) continue;
        const parsed = parseUsage(readTranscriptLines(file), entry.name.replace(/\.jsonl(?:\.zst)?$/, ''));
        cache.files[key] = { fingerprint, partial: parsed.partial, nativeId: parsed.nativeId,
          events: parsed.events.filter((event) => event.at >= first) };
        changed = true;
      } catch (_) { missingSources++; }
    }
  }
  roots.forEach((root) => walk(root));
  if (changed) {
    fs.mkdirSync(path.dirname(dataPath), { recursive: true });
    const temporary = `${dataPath}.${process.pid}.tmp`;
    try { fs.writeFileSync(temporary, JSON.stringify(cache), { mode: 0o600 }); fs.renameSync(temporary, dataPath); }
    finally { try { fs.unlinkSync(temporary); } catch (_) {} }
  }
  const events = new Map();
  const nativeIds = new Set();
  let partial = missingSources > 0;
  for (const file of Object.values(cache.files)) {
    if (!Array.isArray(file.events)) throw new Error('周期用量缓存记录无效');
    if (file.nativeId) nativeIds.add(file.nativeId);
    if (file.partial) partial = true;
    for (const event of file.events) {
      const existing = events.get(event.key);
      if (!existing || event.at < existing.at) events.set(event.key, event);
    }
  }
  // Reuse the upstream observed-request ledger. A native rollout always owns
  // its whole thread, including retained rollouts, so the two sources never add
  // the same conversation twice. Dots records remain explicitly incomplete.
  if (options.codexLocalUsageEnabled !== false && options.codexDotsVisible !== false) {
    const ledger = options.codexLocalUsageStore || createLocalUsageStore(options);
    try {
      const rows = ledger.rows();
      for (const [index, row] of rows.entries()) {
        if (row.nativeBacked || nativeIds.has(row.threadId)) continue;
        const at = Date.parse(row.observedAt);
        if (!Number.isFinite(at) || at < first || at > Date.now()) continue;
        events.set(`dots:${index}`, { ...row.usage, model: row.model, at });
        partial = true;
      }
      if (options.codexDotsEnabled) partial = true;
    } catch (_) { missingSources++; partial = true; }
    finally { if (!options.codexLocalUsageStore) ledger.close(); }
  }
  const values = [...events.values()];
  const prices = await resolveLocalUsagePricing(values, { ...options,
    lookupModelPricing: options.lookupModelPricing || (async () => null) });
  return ranges.map((range) => {
    const result = { ...sum([]), ...range, status: partial ? 'partial' : 'complete', missingSources,
      updatedAt: new Date().toISOString() };
    for (const event of values) {
      if (event.at < Date.parse(range.start) || event.at >= Date.parse(range.end)) continue;
      const estimate = estimatedCost(event, prices[event.model.toLowerCase()]);
      result.totalTokens += event.total;
      result.inputTokens += event.input;
      result.outputTokens += event.output;
      result.cacheReadTokens += event.cacheRead;
      result.costUsd += estimate.cost;
      result.unpricedTokens += estimate.unpricedTokens;
    }
    return result;
  });
}

module.exports = { collectQuotaPeriods, parseUsage };
