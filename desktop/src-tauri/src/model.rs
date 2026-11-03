use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cycle {
    pub id: String,
    pub start: String,
    pub end: String,
}
impl Cycle {
    pub fn new(start: &str, end: &str) -> Option<Self> {
        let first = DateTime::parse_from_rfc3339(start).ok()?.timestamp_millis();
        let last = DateTime::parse_from_rfc3339(end).ok()?.timestamp_millis();
        if last <= first || last - first > 32 * 86_400_000 {
            return None;
        }
        Some(Self {
            id: format!("{first}:{last}"),
            start: timestamp(first),
            end: timestamp(last),
        })
    }
    pub fn bounds(&self) -> (i64, i64) {
        (
            DateTime::parse_from_rfc3339(&self.start)
                .unwrap()
                .timestamp_millis(),
            DateTime::parse_from_rfc3339(&self.end)
                .unwrap()
                .timestamp_millis(),
        )
    }
}
pub fn timestamp(milliseconds: i64) -> String {
    DateTime::from_timestamp_millis(milliseconds)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelUsage {
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub unpriced_tokens: u64,
    pub components_available: bool,
    pub price_available: bool,
}
impl Default for ModelUsage {
    fn default() -> Self {
        Self {
            total_tokens: 0,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            unpriced_tokens: 0,
            components_available: true,
            price_available: true,
        }
    }
}
impl ModelUsage {
    fn add(&mut self, other: &Self) {
        self.total_tokens += other.total_tokens;
        self.cost_usd += other.cost_usd;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.unpriced_tokens += other.unpriced_tokens;
        self.components_available &= other.components_available;
        self.price_available &= other.price_available;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub unpriced_tokens: u64,
    pub status: String,
    pub updated_at: String,
    pub missing_sources: u32,
    pub components_available: bool,
    pub models: BTreeMap<String, ModelUsage>,
}
impl Default for Usage {
    fn default() -> Self {
        Self {
            total_tokens: 0,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            unpriced_tokens: 0,
            status: "complete".into(),
            updated_at: now_iso(),
            missing_sources: 0,
            components_available: true,
            models: BTreeMap::new(),
        }
    }
}
impl Usage {
    pub fn unavailable() -> Self {
        Self {
            status: "unavailable".into(),
            updated_at: String::new(),
            components_available: false,
            ..Self::default()
        }
    }
    pub fn add(&mut self, other: &Self) {
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.unpriced_tokens = self.unpriced_tokens.saturating_add(other.unpriced_tokens);
        self.cost_usd += other.cost_usd;
        self.missing_sources += other.missing_sources;
        self.components_available &= other.components_available;
        for (name, model) in &other.models {
            self.models.entry(name.clone()).or_default().add(model);
        }
        if other.status != "complete" {
            self.status = "partial".into();
        }
        if other.updated_at > self.updated_at {
            self.updated_at = other.updated_at.clone();
        }
    }
    pub fn period_wire(&self) -> Value {
        let tokens: BTreeMap<_, _> = self
            .models
            .iter()
            .map(|(name, model)| (name, model.total_tokens))
            .collect();
        let costs: BTreeMap<_, _> = self
            .models
            .iter()
            .map(|(name, model)| (name, model.cost_usd))
            .collect();
        let cache: BTreeMap<_, _> = self
            .models
            .iter()
            .map(|(name, model)| (name, model.cache_read_tokens))
            .collect();
        let outputs: BTreeMap<_, _> = self
            .models
            .iter()
            .map(|(name, model)| (name, model.output_tokens))
            .collect();
        json!({ "totalTokens": self.total_tokens, "costUsd": self.cost_usd,
            "inputTokens":self.input_tokens,"outputTokens":self.output_tokens,"cacheReadTokens":self.cache_read_tokens,"cacheWriteTokens":0,
            "capabilities":{"tokenComponents":true},
            "clients": {"codex": self.total_tokens}, "clientCosts": {"codex": self.cost_usd},
            "clientCacheReads": {"codex": self.cache_read_tokens}, "clientOutputs": {"codex": self.output_tokens},
            "clientUnpricedTokens": {"codex": self.unpriced_tokens}, "unpricedTokens": self.unpriced_tokens,
            "models":tokens,"modelCosts":costs,"clientModels":{"codex":tokens},"clientModelCosts":{"codex":costs},
            "modelCacheReads":cache,"modelOutputs":outputs,"duoModels":self.models })
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct QuotaSummary {
    #[serde(flatten)]
    pub cycle: Cycle,
    #[serde(flatten)]
    pub usage: Usage,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub member_id: String,
    pub member_name: String,
    pub usage: Usage,
    pub updated_at: String,
    pub received_at: String,
    pub stale: bool,
    pub platform: String,
    pub agent_version: String,
    pub os_name: String,
    pub os_version: String,
    pub pending_usage: Usage,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub name: String,
    pub usage: Usage,
    pub devices: Vec<DeviceView>,
    pub pending_usage: Usage,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub total: Usage,
    pub rows: Vec<Row>,
    pub cycle: Option<Cycle>,
    pub cycles: Vec<Cycle>,
    pub expired: bool,
    pub manual: bool,
    pub source: Value,
    pub members: Value,
    pub devices: Vec<DeviceView>,
    pub pro_sources: Vec<Value>,
    pub pending_ranges: Vec<Cycle>,
    pub pending_total: Usage,
    pub cycle_status: String,
}
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}
fn number(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 9_007_199_254_740_991.0)
                .map(|n| n.round() as u64)
        })
        .unwrap_or_default()
}
fn period_models(raw: &Value) -> BTreeMap<String, ModelUsage> {
    if let Some(value) = raw.get("duoModels") {
        return serde_json::from_value(value.clone()).unwrap_or_default();
    }
    let single = raw["clients"].as_object().is_some_and(|clients| {
        clients
            .iter()
            .filter(|(_, value)| number(value) > 0)
            .all(|(id, _)| id == "codex")
    });
    let models = raw["clientModels"]["codex"].as_object().or_else(|| {
        if single {
            raw["models"].as_object()
        } else {
            None
        }
    });
    let mut result = BTreeMap::new();
    for (name, tokens) in models.into_iter().flatten().take(128) {
        let total = number(tokens);
        let price = raw["clientModelCosts"]["codex"].get(name).or_else(|| {
            if single {
                raw["modelCosts"].get(name)
            } else {
                None
            }
        });
        let components = single && raw["capabilities"]["tokenComponents"].as_bool() == Some(true);
        let cache = if components {
            number(&raw["modelCacheReads"][name])
        } else {
            0
        };
        let output = if components {
            number(&raw["modelOutputs"][name])
        } else {
            0
        };
        result.insert(
            name.clone(),
            ModelUsage {
                total_tokens: total,
                cost_usd: price.and_then(Value::as_f64).unwrap_or_default(),
                input_tokens: total.saturating_sub(cache + output),
                output_tokens: output,
                cache_read_tokens: cache,
                unpriced_tokens: number(&raw["modelUnpricedTokens"][name]),
                components_available: components,
                price_available: price.is_some(),
            },
        );
    }
    result
}
fn aggregate(devices: &[DeviceView]) -> Usage {
    let mut result = Usage::default();
    for device in devices {
        result.add(&device.usage);
    }
    if devices.is_empty() || devices.iter().all(|d| d.usage.status == "unavailable") {
        result.status = "unavailable".into();
    }
    result
}
fn period_expired(record: &Value, period: &str) -> bool {
    if period == "allTime" {
        return false;
    }
    let now = Utc::now();
    if let Some(end) = record["periodWindows"][period]["endsAt"]
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    {
        return now >= end;
    }
    let Some(at) = record["updatedAt"]
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    else {
        return false;
    };
    let format = if period == "today" {
        "%Y-%m-%d"
    } else {
        "%Y-%m"
    };
    at.with_timezone(&Utc).format(format).to_string() != now.format(format).to_string()
}
pub fn view(stats: &Value, period: &str, group: &str, selected: &str) -> View {
    let configuration = &stats["duo"];
    let cycles: Vec<Cycle> = configuration["cycles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| Cycle::new(value["start"].as_str()?, value["end"].as_str()?))
        .collect();
    let current = if selected.is_empty() {
        Cycle::new(
            configuration["cycle"]["start"].as_str().unwrap_or(""),
            configuration["cycle"]["end"].as_str().unwrap_or(""),
        )
    } else {
        cycles.iter().find(|cycle| cycle.id == selected).cloned()
    };
    let pending_ranges: Vec<Cycle> = if period == "cycle" {
        configuration["uncertainRanges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|range| range["status"] == "pending")
            .filter_map(|range| Cycle::new(range["start"].as_str()?, range["end"].as_str()?))
            .filter(|range| {
                current
                    .as_ref()
                    .is_some_and(|active| range.start == active.end || range.end == active.start)
            })
            .collect()
    } else {
        vec![]
    };
    let mut devices = Vec::new();
    let mut pro_sources = Vec::new();
    for record in stats["devices"].as_array().into_iter().flatten() {
        let id = text(record, "deviceId");
        let name = ["displayName", "hostname", "deviceId"]
            .iter()
            .map(|key| text(record, key))
            .find(|name| !name.is_empty())
            .unwrap_or_default();
        let usage = if period == "cycle" {
            current
                .as_ref()
                .and_then(|cycle| {
                    record["quotaPeriods"]
                        .as_array()?
                        .iter()
                        .find(|range| range["id"].as_str() == Some(&cycle.id))
                })
                .and_then(|value| serde_json::from_value::<Usage>(value.clone()).ok())
                .unwrap_or_else(Usage::unavailable)
        } else {
            let raw = &record["periods"][period];
            if raw.is_object() && !period_expired(record, period) {
                let total = number(&raw["clients"]["codex"]);
                let cache = number(&raw["clientCacheReads"]["codex"]);
                let output = number(&raw["clientOutputs"]["codex"]);
                Usage {
                    total_tokens: total,
                    cost_usd: raw["clientCosts"]["codex"].as_f64().unwrap_or_default(),
                    input_tokens: total.saturating_sub(cache + output),
                    output_tokens: output,
                    cache_read_tokens: cache,
                    unpriced_tokens: number(&raw["clientUnpricedTokens"]["codex"]),
                    updated_at: text(record, "updatedAt"),
                    components_available: raw.get("duoModels").is_some()
                        || (raw["clientOutputs"]["codex"].is_number()
                            && raw["clientCacheReads"]["codex"].is_number())
                        || total == 0,
                    models: period_models(raw),
                    ..Usage::default()
                }
            } else {
                Usage::unavailable()
            }
        };
        for provider in record["limits"]["providers"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if provider["provider"] == "codex"
                && text(provider, "planLabel").to_lowercase().contains("pro")
            {
                pro_sources.push(json!({"deviceId": id, "accountKey": text(provider, "accountKey"), "label": format!("{} · {}",name,text(provider,"planLabel")), "provider": provider}));
            }
        }
        let mut pending_usage = Usage::default();
        let mut reported = 0;
        for range in &pending_ranges {
            if let Some(value) = record["quotaPeriods"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|value| value["id"] == range.id)
            {
                if let Ok(usage) = serde_json::from_value::<Usage>(value.clone()) {
                    pending_usage.add(&usage);
                    reported += 1;
                }
            }
        }
        if reported < pending_ranges.len() {
            pending_usage.status = if reported == 0 {
                "unavailable"
            } else {
                "partial"
            }
            .into();
        }
        devices.push(DeviceView {
            id,
            name,
            usage,
            member_id: text(record, "memberId"),
            member_name: record["memberName"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("未分组")
                .into(),
            updated_at: text(record, "updatedAt"),
            received_at: text(record, "receivedAt"),
            stale: record["stale"].as_bool().unwrap_or(false),
            platform: text(record, "platform"),
            agent_version: text(record, "agentVersion"),
            os_name: text(record, "osName"),
            os_version: text(record, "osVersion"),
            pending_usage,
        });
    }
    let mut grouped: BTreeMap<String, Row> = BTreeMap::new();
    for device in &devices {
        let (id, name) = if group == "device" {
            (device.id.clone(), device.name.clone())
        } else if device.member_id.is_empty() {
            (
                format!("unassigned:{}", device.id),
                format!("未分组 · {}", device.name),
            )
        } else {
            (device.member_id.clone(), device.member_name.clone())
        };
        grouped
            .entry(id.clone())
            .or_insert_with(|| Row {
                id,
                name,
                usage: Usage::default(),
                devices: vec![],
                pending_usage: Usage::default(),
            })
            .devices
            .push(device.clone());
    }
    let mut rows: Vec<Row> = grouped
        .into_values()
        .map(|mut row| {
            row.usage = aggregate(&row.devices);
            for device in &row.devices {
                row.pending_usage.add(&device.pending_usage);
            }
            if row
                .devices
                .iter()
                .all(|device| device.pending_usage.status == "unavailable")
            {
                row.pending_usage.status = "unavailable".into();
            }
            row
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.usage.total_tokens));
    let mut pending_total = Usage::default();
    for device in &devices {
        pending_total.add(&device.pending_usage);
    }
    if devices.is_empty()
        || devices
            .iter()
            .all(|device| device.pending_usage.status == "unavailable")
    {
        pending_total.status = "unavailable".into();
    }
    View {
        pending_ranges,
        pending_total,
        cycle_status: configuration["cycleStatus"]
            .as_str()
            .unwrap_or("waiting")
            .into(),
        total: aggregate(&devices),
        expired: selected.is_empty()
            && current
                .as_ref()
                .is_some_and(|range| range.bounds().1 <= Utc::now().timestamp_millis()),
        cycle: current,
        cycles,
        rows,
        devices,
        pro_sources,
        manual: configuration["manual"].as_bool().unwrap_or(false),
        source: configuration["source"].clone(),
        members: configuration["members"].clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn member_sum_preserves_missing_device_coverage() {
        let cycle = Cycle::new("2026-11-01T12:00:00Z", "2026-11-08T12:00:00Z").unwrap();
        let result = view(
            &json!({"duo":{"cycle":cycle,"cycles":[cycle]},"devices":[
            {"deviceId":"a","memberId":"member","memberName":"Owner","quotaPeriods":[{"id":cycle.id,"totalTokens":10}]},
            {"deviceId":"b","memberId":"member","memberName":"Owner"}]}),
            "cycle",
            "member",
            "",
        );
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.total.total_tokens, 10);
        assert_eq!(result.total.status, "partial");
    }
    #[test]
    fn cycle_requires_valid_half_open_range() {
        assert!(Cycle::new("2026-11-08T12:00:00Z", "2026-11-01T12:00:00Z").is_none());
        assert!(Cycle::new("bad", "bad").is_none());
    }
    #[test]
    fn offline_calendar_snapshots_expire_without_losing_all_time_or_cycle_history() {
        let record = json!({"deviceId":"offline", "periodWindows":{"today":{"endsAt":"2020-01-02T00:00:00Z"}},
            "periods":{"today":{"clients":{"codex":30}},"allTime":{"clients":{"codex":80}}}});
        let stats = json!({"devices":[record]});
        let day = view(&stats, "today", "device", "");
        assert_eq!(day.total.status, "unavailable");
        assert_eq!(day.total.total_tokens, 0);
        assert_eq!(view(&stats, "allTime", "device", "").total.total_tokens, 80);
    }
    #[test]
    fn unassigned_boundary_usage_is_separate_from_the_current_cycle_and_its_models() {
        let range = Cycle::new("2026-11-03T00:01:00Z", "2026-11-08T12:00:00Z").unwrap();
        let gap = Cycle::new("2026-11-03T00:00:00Z", "2026-11-03T00:01:00Z").unwrap();
        let stats = json!({"duo":{"cycle":range,"cycles":[range],"uncertainRanges":[{"id":gap.id,"start":gap.start,"end":gap.end,"status":"pending"}]},
        "devices":[{"deviceId":"mac","memberId":"one","memberName":"Alice","quotaPeriods":[
            {"id":range.id,"totalTokens":30,"models":{"gpt-test":{"totalTokens":30}}},
            {"id":gap.id,"totalTokens":20,"models":{"gpt-test":{"totalTokens":20}}}
        ]}]});
        let result = view(&stats, "cycle", "member", "");
        assert_eq!(result.total.total_tokens, 30);
        assert_eq!(result.pending_total.total_tokens, 20);
        assert_eq!(
            result.rows[0].devices[0].usage.models["gpt-test"].total_tokens,
            30
        );
        assert_eq!(
            result.rows[0].devices[0].pending_usage.models["gpt-test"].total_tokens,
            20
        );
        assert_eq!(result.pending_ranges.len(), 1);
    }
}
