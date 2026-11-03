use crate::{
    model::{now_iso, Cycle, QuotaSummary, Usage},
    settings::Settings,
};
use chrono::{Datelike, Local, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ZSTD: u64 = 4 * 1024 * 1024 * 1024;
const MAX_LINE: u64 = 8 * 1024 * 1024;
const PARSER_REVISION: u8 = 2;
#[derive(Clone, Debug)]
pub struct Event {
    pub key: String,
    pub at: i64,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache: u64,
}
impl Event {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache
    }
}
#[derive(Default)]
pub struct Parsed {
    pub events: Vec<Event>,
    pub partial: bool,
}
fn counters(value: &Value) -> Option<[u64; 3]> {
    let input = value["input_tokens"].as_u64()?;
    let output = value["output_tokens"].as_u64()?;
    let cache = value["cached_input_tokens"].as_u64().unwrap_or(0);
    if cache > input || input.saturating_add(output) > 9_007_199_254_740_991 {
        return None;
    }
    Some([input, output, cache])
}
pub fn parse<R: BufRead>(mut reader: R, fallback: &str) -> Parsed {
    let mut result = Parsed::default();
    let mut native = fallback.to_string();
    let mut model = "unknown".to_string();
    let mut previous: Option<[u64; 3]> = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => break,
            Err(_) => {
                result.partial = true;
                break;
            }
            _ => {}
        }
        if line.len() as u64 > MAX_LINE {
            result.partial = true;
            if line.last() != Some(&b'\n') && reader.skip_until(b'\n').is_err() {
                break;
            }
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            result.partial = true;
            continue;
        };
        let payload = &record["payload"];
        match record["type"].as_str() {
            Some("session_meta") => {
                if let Some(id) = payload["id"].as_str() {
                    native = id.into();
                }
                if let Some(value) = payload["model"].as_str() {
                    model = value.into();
                }
            }
            Some("turn_context") => {
                if let Some(value) = payload["model"].as_str() {
                    model = value.into();
                }
            }
            _ => {}
        }
        if record["type"] != "event_msg" || payload["type"] != "token_count" {
            continue;
        }
        let info = &payload["info"];
        let Some(current) = counters(&info["total_token_usage"]) else {
            if !info.is_null() {
                result.partial = true;
            }
            continue;
        };
        if previous == Some(current) {
            continue;
        }
        if previous.is_some_and(|old| (0..3).any(|i| current[i] < old[i])) {
            previous = None;
            result.partial = true;
        }
        let old = previous.unwrap_or([0; 3]);
        let mut delta = [
            current[0] - old[0],
            current[1] - old[1],
            current[2] - old[2],
        ];
        if let Some(last) = counters(&info["last_token_usage"]) {
            if last[0] + last[1] < delta[0] + delta[1] {
                delta = last;
                result.partial = true;
            }
        }
        previous = Some(current);
        let Some(at) = record["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.timestamp_millis())
        else {
            result.partial = true;
            continue;
        };
        if delta[2] > delta[0] {
            result.partial = true;
            continue;
        }
        if delta[0] + delta[1] == 0 {
            continue;
        }
        result.events.push(Event {
            key: format!("{}:{}:{}:{}", native, current[0], current[1], current[2]),
            at,
            model: info["model"]
                .as_str()
                .unwrap_or(&model)
                .chars()
                .take(200)
                .collect(),
            input: delta[0] - delta[2],
            output: delta[1],
            cache: delta[2],
        });
    }
    result
}

pub struct Ledger {
    connection: Connection,
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Price {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache: Option<f64>,
}
impl Price {
    pub fn estimate(&self, event: &Event) -> (f64, u64) {
        let mut cost = 0.0;
        let mut unpriced = 0;
        for (count, rate) in [
            (event.input, self.input),
            (event.output, self.output),
            (event.cache, self.cache),
        ] {
            if let Some(rate) = rate {
                cost += count as f64 * rate;
            } else {
                unpriced += count;
            }
        }
        (cost, unpriced)
    }
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
impl Ledger {
    pub fn open(directory: &Path) -> Result<Self, String> {
        fs::create_dir_all(directory).map_err(|_| "无法建立用量目录")?;
        let file = directory.join("usage.sqlite");
        let connection = Connection::open(&file).map_err(|_| "无法读取本地用量账本")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600))
                .map_err(|_| "无法设置账本权限")?;
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=3000;
          CREATE TABLE IF NOT EXISTS files (file_key TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, partial INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS events (file_key TEXT NOT NULL, event_key TEXT NOT NULL, at INTEGER NOT NULL, model TEXT NOT NULL, input INTEGER NOT NULL, output INTEGER NOT NULL, cache INTEGER NOT NULL, PRIMARY KEY(file_key,event_key));
          CREATE INDEX IF NOT EXISTS events_at ON events(at);
          CREATE TABLE IF NOT EXISTS prices (model TEXT PRIMARY KEY, rates TEXT NOT NULL, updated INTEGER NOT NULL);")
            .map_err(|_|"无法准备本地用量账本")?;
        Ok(Self { connection })
    }
    pub fn scan(&mut self, roots: &[PathBuf]) -> Result<(Vec<Event>, u32), String> {
        let mut seen = HashSet::new();
        let mut missing = 0u32;
        let mut entries = 0usize;
        for root in roots {
            if !root.exists() {
                continue;
            }
            for entry in WalkDir::new(root)
                .max_depth(32)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    !["node_modules", ".git", "target"]
                        .contains(&entry.file_name().to_str().unwrap_or(""))
                })
            {
                entries += 1;
                if entries > 200_000 {
                    missing += 1;
                    break;
                }
                let Ok(entry) = entry else {
                    missing += 1;
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy();
                if !name.ends_with(".jsonl") && !name.ends_with(".jsonl.zst") {
                    continue;
                }
                let file = entry.path();
                let Ok(real) = fs::canonicalize(file) else {
                    missing += 1;
                    continue;
                };
                if !seen.insert(real.clone()) {
                    continue;
                }
                let Ok(meta) = fs::metadata(file) else {
                    missing += 1;
                    continue;
                };
                let stamp = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|time| time.as_nanos())
                    .unwrap_or_default();
                let fingerprint = format!("{PARSER_REVISION}:{}:{stamp}", meta.len());
                let key = hash(&real.to_string_lossy());
                let cached = self
                    .connection
                    .query_row(
                        "SELECT fingerprint FROM files WHERE file_key=?1",
                        [&key],
                        |row| row.get::<_, String>(0),
                    )
                    .ok();
                if cached.as_deref() == Some(&fingerprint) {
                    continue;
                }
                if meta.len() > MAX_FILE {
                    missing += 1;
                    continue;
                }
                let parsed = if name.ends_with(".zst") {
                    let decoded = (|| -> Result<Parsed, std::io::Error> {
                        let mut decoder = zstd::stream::read::Decoder::new(fs::File::open(file)?)?;
                        decoder.window_log_max(26)?;
                        let mut limited = decoder.take(MAX_ZSTD + 1);
                        let mut parsed = parse(BufReader::new(&mut limited), &name);
                        if limited.limit() == 0 {
                            parsed.partial = true;
                        }
                        Ok(parsed)
                    })();
                    match decoded {
                        Ok(parsed) => parsed,
                        _ => {
                            missing += 1;
                            continue;
                        }
                    }
                } else {
                    match fs::File::open(file) {
                        Ok(file) => parse(BufReader::new(file.take(MAX_FILE + 1)), &name),
                        Err(_) => {
                            missing += 1;
                            continue;
                        }
                    }
                };
                let transaction = self
                    .connection
                    .transaction()
                    .map_err(|_| "用量账本暂时忙碌")?;
                transaction
                    .execute("DELETE FROM events WHERE file_key=?1", [&key])
                    .map_err(|_| "无法更新账本")?;
                {
                    let mut insert = transaction
                        .prepare("INSERT OR IGNORE INTO events VALUES(?1,?2,?3,?4,?5,?6,?7)")
                        .map_err(|_| "无法更新账本")?;
                    for event in parsed.events {
                        insert
                            .execute(params![
                                key,
                                event.key,
                                event.at,
                                event.model,
                                event.input,
                                event.output,
                                event.cache
                            ])
                            .map_err(|_| "无法写入用量")?;
                    }
                }
                transaction
                    .execute(
                        "INSERT OR REPLACE INTO files VALUES(?1,?2,?3)",
                        params![key, fingerprint, parsed.partial],
                    )
                    .map_err(|_| "无法更新扫描状态")?;
                transaction.commit().map_err(|_| "无法保存用量账本")?;
            }
        }
        let partial: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM files WHERE partial=1", [], |row| {
                row.get(0)
            })
            .map_err(|_| "无法读取扫描状态")?;
        missing = missing.saturating_add(partial as u32);
        let mut query=self.connection.prepare("SELECT event_key,at,model,input,output,cache FROM (SELECT *,ROW_NUMBER() OVER(PARTITION BY event_key ORDER BY at,file_key) AS position FROM events) WHERE position=1 ORDER BY at").map_err(|_|"无法读取用量账本")?;
        let events = query
            .query_map([], |row| {
                Ok(Event {
                    key: row.get(0)?,
                    at: row.get(1)?,
                    model: row.get(2)?,
                    input: row.get(3)?,
                    output: row.get(4)?,
                    cache: row.get(5)?,
                })
            })
            .map_err(|_| "无法读取用量账本")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "用量账本记录无效")?;
        Ok((events, missing))
    }
    pub fn prices(&self) -> HashMap<String, (Price, i64)> {
        let Ok(mut statement) = self
            .connection
            .prepare("SELECT model,rates,updated FROM prices")
        else {
            return HashMap::new();
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) else {
            return HashMap::new();
        };
        rows.filter_map(Result::ok)
            .filter_map(|(model, json, at)| Some((model, (serde_json::from_str(&json).ok()?, at))))
            .collect()
    }
    pub fn seed_prices(&self, rates: &HashMap<String, Price>, replace: bool) -> Result<(), String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| "价格缓存暂时忙碌")?;
        {
            let mut statement = transaction
                .prepare(if replace {
                    "INSERT OR REPLACE INTO prices VALUES(?1,?2,?3)"
                } else {
                    "INSERT OR IGNORE INTO prices VALUES(?1,?2,?3)"
                })
                .map_err(|_| "无法准备价格缓存")?;
            for (model, price) in rates {
                statement
                    .execute(params![
                        model,
                        serde_json::to_string(price).unwrap(),
                        Utc::now().timestamp()
                    ])
                    .map_err(|_| "无法缓存价格")?;
            }
        }
        transaction.commit().map_err(|_| "无法保存价格缓存".into())
    }
}

pub fn summarize(
    events: &[Event],
    prices: &HashMap<String, (Price, i64)>,
    start: i64,
    end: i64,
    missing: u32,
) -> Usage {
    let mut result = Usage {
        missing_sources: missing,
        status: if missing > 0 { "partial" } else { "complete" }.into(),
        ..Usage::default()
    };
    let empty = Price::default();
    for event in events
        .iter()
        .filter(|event| event.at >= start && event.at < end)
    {
        let (cost, unpriced) = prices
            .get(&event.model.to_lowercase())
            .map(|(price, _)| price)
            .unwrap_or(&empty)
            .estimate(event);
        result.total_tokens += event.total();
        result.input_tokens += event.input;
        result.output_tokens += event.output;
        result.cache_read_tokens += event.cache;
        result.cost_usd += cost;
        result.unpriced_tokens += unpriced;
        let model = result.models.entry(event.model.clone()).or_default();
        model.total_tokens += event.total();
        model.input_tokens += event.input;
        model.output_tokens += event.output;
        model.cache_read_tokens += event.cache;
        model.cost_usd += cost;
        model.unpriced_tokens += unpriced;
    }
    result
}
pub fn record(
    settings: &Settings,
    events: &[Event],
    prices: &HashMap<String, (Price, i64)>,
    cycles: &[Cycle],
    missing: u32,
) -> Value {
    let now = Local::now();
    let day = now.date_naive();
    let today = day.and_hms_opt(0, 0, 0).unwrap();
    let month = now
        .date_naive()
        .with_day(1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let today = Local
        .from_local_datetime(&today)
        .earliest()
        .unwrap()
        .timestamp_millis();
    let month = Local
        .from_local_datetime(&month)
        .earliest()
        .unwrap()
        .timestamp_millis();
    // Stamp the producer's calendar so an offline device's previous day/month
    // expires at its own midnight, including daylight-saving transitions.
    let tomorrow = Local
        .from_local_datetime(&day.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap();
    let next_month_day = if now.month() == 12 {
        chrono::NaiveDate::from_ymd_opt(now.year() + 1, 1, 1).unwrap()
    } else {
        chrono::NaiveDate::from_ymd_opt(now.year(), now.month() + 1, 1).unwrap()
    };
    let next_month = Local
        .from_local_datetime(&next_month_day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap();
    let end = Utc::now().timestamp_millis() + 1;
    let quota: Vec<_> = cycles
        .iter()
        .map(|cycle| {
            let (start, end) = cycle.bounds();
            QuotaSummary {
                cycle: cycle.clone(),
                usage: summarize(events, prices, start, end, missing),
            }
        })
        .collect();
    let platform = match std::env::consts::OS {
        "macos" => format!("darwin-{}", std::env::consts::ARCH),
        "windows" => format!("win32-{}", std::env::consts::ARCH),
        other => format!("{other}-{}", std::env::consts::ARCH),
    };
    json!({"deviceId":settings.device_id,"hostname":hostname::get().unwrap_or_default().to_string_lossy(),"platform":platform,
        "displayName":settings.device_name,"trackedClients":["codex"],"agentVersion":env!("CARGO_PKG_VERSION"),"agentRuntime":"rust-desktop",
        "updatedAt":now_iso(),"quotaPeriods":quota,
        "periodWindows":{"today":{"key":day.format("%Y-%m-%d").to_string(),"endsAt":crate::model::timestamp(tomorrow.timestamp_millis())},
            "month":{"key":day.format("%Y-%m").to_string(),"endsAt":crate::model::timestamp(next_month.timestamp_millis())}},
        "today":summarize(events,prices,today,end,missing).period_wire(),"month":summarize(events,prices,month,end,missing).period_wire(),
        "allTime":summarize(events,prices,0,end,missing).period_wire()})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn usage(at: &str, input: u64, output: u64, cache: u64) -> String {
        json!({"timestamp":at,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"output_tokens":output,"cached_input_tokens":cache},"last_token_usage":{"input_tokens":input,"output_tokens":output,"cached_input_tokens":cache}}}}).to_string()
    }
    #[test]
    fn cumulative_counts_split_exactly_at_reset_and_ignore_repeats() {
        let text = [
            usage("2026-11-01T11:59:59Z", 100, 20, 40),
            usage("2026-11-01T12:00:00Z", 150, 30, 60),
            usage("2026-11-01T12:00:01Z", 150, 30, 60),
            usage("2026-11-08T12:00:00Z", 200, 40, 80),
        ]
        .join("\n");
        let parsed = parse(BufReader::new(text.as_bytes()), "id");
        assert_eq!(parsed.events.len(), 3);
        assert!(!parsed.partial);
        let cycle = Cycle::new("2026-11-01T12:00:00Z", "2026-11-08T12:00:00Z").unwrap();
        let (a, b) = cycle.bounds();
        let result = summarize(&parsed.events, &HashMap::new(), a, b, 0);
        assert_eq!(result.total_tokens, 60);
        assert_eq!(result.input_tokens, 30);
        assert_eq!(result.cache_read_tokens, 20);
        assert_eq!(result.output_tokens, 10);
        assert_eq!(result.unpriced_tokens, 60);
    }
    #[test]
    fn archive_copy_is_deduplicated_and_deleted_logs_remain_observed() {
        let temp = tempfile::tempdir().unwrap();
        let logs = temp.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let text = format!(
            "{}\n{}",
            json!({"type":"session_meta","payload":{"id":"native"}}),
            usage("2026-11-01T12:00:00Z", 100, 20, 40)
        );
        fs::write(logs.join("a.jsonl"), &text).unwrap();
        fs::write(logs.join("b.jsonl"), &text).unwrap();
        let mut ledger = Ledger::open(&temp.path().join("db")).unwrap();
        let (events, missing) = ledger.scan(std::slice::from_ref(&logs)).unwrap();
        assert_eq!(missing, 0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].total(), 120);
        fs::remove_dir_all(&logs).unwrap();
        assert_eq!(ledger.scan(&[logs]).unwrap().0.len(), 1);
        let bytes = fs::read(temp.path().join("db/usage.sqlite")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("user_message"));
    }
    #[test]
    fn large_plain_logs_keep_usage_on_both_sides_of_an_oversized_record() {
        use std::io::{Seek, SeekFrom, Write};
        let temp = tempfile::tempdir().unwrap();
        let logs = temp.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let mut file = fs::File::create(logs.join("large.jsonl")).unwrap();
        writeln!(file, "{}", usage("2026-11-01T00:00:00Z", 100, 20, 40)).unwrap();
        file.seek(SeekFrom::Start(260 * 1024 * 1024)).unwrap();
        writeln!(file, "\n{}", usage("2026-11-01T00:01:00Z", 150, 30, 60)).unwrap();
        drop(file);
        let mut ledger = Ledger::open(&temp.path().join("db")).unwrap();
        let (events, missing) = ledger.scan(&[logs]).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events.iter().map(Event::total).sum::<u64>(), 180);
        assert_eq!(missing, 1);
    }
    #[test]
    fn large_zstd_logs_are_streamed_without_discarding_the_entire_session() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let logs = temp.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let file = fs::File::create(logs.join("large.jsonl.zst")).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(file, 1).unwrap();
        writeln!(encoder, "{}", usage("2026-11-01T00:00:00Z", 100, 20, 40)).unwrap();
        let other = format!(
            "{{\"type\":\"response_item\",\"payload\":\"{}\"}}\n",
            "a".repeat(1024 * 1024)
        );
        for _ in 0..65 {
            encoder.write_all(other.as_bytes()).unwrap();
        }
        writeln!(encoder, "{}", usage("2026-11-01T00:01:00Z", 150, 30, 60)).unwrap();
        encoder.finish().unwrap();
        let mut ledger = Ledger::open(&temp.path().join("db")).unwrap();
        let (events, missing) = ledger.scan(&[logs]).unwrap();
        assert_eq!(events.iter().map(Event::total).sum::<u64>(), 180);
        assert_eq!(missing, 0);
    }
    #[test]
    fn pricing_uses_exclusive_input_and_cache_rates() {
        let price = Price {
            input: Some(0.01),
            output: Some(0.03),
            cache: Some(0.001),
        };
        let event = Event {
            key: "x".into(),
            at: 0,
            model: "m".into(),
            input: 60,
            output: 20,
            cache: 40,
        };
        let (cost, unpriced) = price.estimate(&event);
        assert!((cost - 1.24).abs() < 1e-9);
        assert_eq!(unpriced, 0);
    }
}
