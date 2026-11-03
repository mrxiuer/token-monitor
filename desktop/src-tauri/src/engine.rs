use crate::{
    catalog,
    ledger::{self, Ledger},
    model::{self, Cycle},
    quota,
    settings::{self, Credentials, Settings},
};
use futures_util::StreamExt;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::{watch, Notify};

#[derive(Clone)]
struct Upload {
    generation: u64,
    record: Value,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub connected: bool,
    pub pending: bool,
    pub collecting: bool,
    pub sync_message: String,
    pub collection_message: String,
    pub quota_message: String,
    pub last_uploaded_at: String,
    pub last_collected_at: String,
    pub warning: String,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            connected: false,
            pending: false,
            collecting: true,
            sync_message: "等待连接 Hub".into(),
            collection_message: "正在读取本机日志…".into(),
            quota_message: String::new(),
            last_uploaded_at: String::new(),
            last_collected_at: String::new(),
            warning: String::new(),
        }
    }
}
struct Snapshot {
    hub: Value,
    local: Value,
    limits: Value,
    status: Status,
}
pub struct Engine {
    app: AppHandle,
    pub directory: PathBuf,
    pub settings: Mutex<(Settings, Credentials)>,
    snapshot: Mutex<Snapshot>,
    ledger: Mutex<Ledger>,
    cycles: Mutex<Vec<Cycle>>,
    pub collect: Notify,
    pub refresh_quota: Notify,
    generation: AtomicU64,
    configuration: watch::Sender<u64>,
    outbox: watch::Sender<Option<Upload>>,
    client: reqwest::Client,
}
fn compact_stats(value: &Value) -> Value {
    let devices: Vec<Value> = value["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|record| {
            let mut output = json!({});
            for key in [
                "deviceId",
                "hostname",
                "displayName",
                "platform",
                "updatedAt",
                "receivedAt",
                "periodWindows",
                "memberId",
                "memberName",
                "stale",
                "quotaPeriods",
                "agentVersion",
                "agentRuntime",
                "osName",
                "osVersion",
            ] {
                if let Some(value) = record.get(key) {
                    output[key] = value.clone();
                }
            }
            output["periods"] = json!({});
            for period in ["today", "month", "allTime"] {
                let raw = &record["periods"][period];
                if !raw.is_object() {
                    continue;
                }
                let mut projected = json!({});
                for field in [
                    "clients",
                    "clientCosts",
                    "clientCacheReads",
                    "clientOutputs",
                    "clientUnpricedTokens",
                ] {
                    if let Some(number) = raw[field]["codex"].as_f64() {
                        projected[field] = json!({"codex":number.max(0.0)});
                    }
                }
                for field in [
                    "clientModels",
                    "clientModelCosts",
                    "models",
                    "modelCosts",
                    "modelCacheReads",
                    "modelOutputs",
                    "modelUnpricedTokens",
                    "duoModels",
                    "capabilities",
                ] {
                    if let Some(value) = raw.get(field) {
                        projected[field] = value.clone();
                    }
                }
                output["periods"][period] = projected;
            }
            let providers: Vec<Value> = record["limits"]["providers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["provider"] == "codex")
                .map(|p| {
                    let mut out = json!({});
                    for field in [
                        "provider",
                        "accountKey",
                        "planLabel",
                        "status",
                        "windows",
                        "updatedAt",
                    ] {
                        out[field] = p[field].clone();
                    }
                    out
                })
                .collect();
            output["limits"] = json!({"providers":providers});
            output
        })
        .collect();
    json!({"devices":devices,"duo":value["duo"]})
}
impl Engine {
    pub fn new(app: AppHandle) -> Result<Arc<Self>, String> {
        let directory = settings::app_dir();
        let (settings, credentials, mut warning) = settings::load(&directory);
        if std::env::var_os("TOKEN_MONITOR_DUO_TEST").is_some() {
            warning = "隔离测试数据 · 尚未连接真实 Hub".into();
        }
        let ledger = Ledger::open(&directory)?;
        ledger.seed_prices(&catalog::bundled(), false)?;
        if let Ok(bytes) = std::fs::read(directory.join("catalog-cache.json")) {
            if let Ok(cached) = serde_json::from_slice::<Value>(&bytes) {
                ledger.seed_prices(&catalog::parse(&cached), true)?;
            }
        }
        let cached = std::fs::read(directory.join("hub-cache.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .map(|value| compact_stats(&value))
            .unwrap_or(json!({"devices":[]}));
        let (configuration, _) = watch::channel(0);
        let (outbox, _) = watch::channel(None);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(6))
            .user_agent(concat!("Token-Monitor-Duo/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "无法初始化同步连接")?;
        let state = Arc::new(Self {
            app,
            directory,
            settings: Mutex::new((settings, credentials)),
            ledger: Mutex::new(ledger),
            snapshot: Mutex::new(Snapshot {
                hub: cached,
                local: Value::Null,
                limits: Value::Null,
                status: Status {
                    warning,
                    ..Status::default()
                },
            }),
            cycles: Mutex::new(vec![]),
            collect: Notify::new(),
            refresh_quota: Notify::new(),
            generation: AtomicU64::new(0),
            configuration,
            outbox,
            client,
        });
        let cached = state.snapshot.lock().unwrap().hub.clone();
        state.adopt(cached, 0);
        Ok(state)
    }
    fn emit(&self) {
        let _ = self.app.emit("duo-changed", ());
    }
    fn context(&self) -> (Settings, Credentials, u64) {
        let guard = self.settings.lock().unwrap();
        (
            guard.0.clone(),
            guard.1.clone(),
            self.generation.load(Ordering::SeqCst),
        )
    }
    pub fn bootstrap(&self) -> Value {
        let guard = self.settings.lock().unwrap();
        json!({"settings":guard.0,"secretConfigured":!guard.1.hub_secret.is_empty(),"version":env!("CARGO_PKG_VERSION")})
    }
    pub fn status(&self) -> Status {
        self.snapshot.lock().unwrap().status.clone()
    }
    fn message(&self, callback: impl FnOnce(&mut Status)) {
        callback(&mut self.snapshot.lock().unwrap().status);
        self.emit();
    }
    pub fn view(&self, period: &str, group: &str, cycle: &str) -> model::View {
        let (settings, _, _) = self.context();
        let snapshot = self.snapshot.lock().unwrap();
        let mut stats = snapshot.hub.clone();
        if let Some(record) = snapshot.local.as_object() {
            let mut local = Value::Object(record.clone());
            local["periods"] =
                json!({"today":local["today"],"month":local["month"],"allTime":local["allTime"]});
            local["limits"] = json!({"providers":if snapshot.limits.is_null(){vec![]}else{vec![snapshot.limits.clone()]}});
            let id = &stats["duo"]["bindings"][&settings.device_id];
            let member = stats["duo"]["members"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|member| &member["id"] == id);
            local["memberId"] = member
                .map(|member| member["id"].clone())
                .unwrap_or(json!(format!("local:{}", settings.username.to_lowercase())));
            local["memberName"] =
                member
                    .map(|member| member["name"].clone())
                    .unwrap_or(json!(if settings.username.is_empty() {
                        "未分组"
                    } else {
                        &settings.username
                    }));
            local["stale"] = json!(false);
            let mut devices = stats["devices"].as_array().cloned().unwrap_or_default();
            if let Some(position) = devices
                .iter()
                .position(|device| device["deviceId"] == settings.device_id)
            {
                local["receivedAt"] = devices[position]["receivedAt"].clone();
                devices[position] = local;
            } else {
                devices.push(local);
            }
            stats["devices"] = json!(devices);
        }
        model::view(&stats, period, group, cycle)
    }
    fn adopt(&self, stats: Value, generation: u64) {
        if generation != self.generation.load(Ordering::SeqCst) {
            return;
        }
        let stats = compact_stats(&stats);
        let mut guard = self.settings.lock().unwrap();
        if generation != self.generation.load(Ordering::SeqCst) {
            return;
        }
        let mut cycles: Vec<Cycle> = stats["duo"]["cycles"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .take(104)
            .filter_map(|range| Cycle::new(range["start"].as_str()?, range["end"].as_str()?))
            .collect();
        cycles.extend(
            stats["duo"]["uncertainRanges"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|range| range["status"] == "pending")
                .take(104)
                .filter_map(|range| Cycle::new(range["start"].as_str()?, range["end"].as_str()?)),
        );
        cycles.sort_by(|a, b| a.start.cmp(&b.start));
        cycles.dedup_by(|a, b| a.id == b.id);
        let changed = {
            let mut current = self.cycles.lock().unwrap();
            let changed = *current != cycles;
            if changed {
                *current = cycles;
            }
            changed
        };
        {
            if guard.0.joined {
                let id = &stats["duo"]["bindings"][&guard.0.device_id];
                if let Some(name) = stats["duo"]["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|member| &member["id"] == id)
                    .and_then(|member| member["name"].as_str())
                {
                    if guard.0.username != name {
                        guard.0.username = name.into();
                        let _ = settings::persist(&self.directory, &guard.0, &guard.1);
                    }
                }
            }
        }
        self.snapshot.lock().unwrap().hub = stats.clone();
        let _ = settings::private_write(
            &self.directory.join("hub-cache.json"),
            &serde_json::to_vec(&stats).unwrap_or_default(),
        );
        drop(guard);
        if changed {
            self.collect.notify_one();
        }
        self.emit();
    }
    pub fn update(&self, mut settings: Settings, secret: Option<String>) -> Result<(), String> {
        settings.validate()?;
        let mut guard = self.settings.lock().unwrap();
        let mut credentials = guard.1.clone();
        if let Some(secret) = secret {
            if !secret.is_empty() {
                credentials.hub_secret = secret;
            }
        }
        settings.joined = guard.0.joined;
        let destination = guard.0.hub_url != settings.hub_url
            || guard.0.device_id != settings.device_id
            || guard.1.hub_secret != credentials.hub_secret;
        if destination || guard.0.username != settings.username {
            settings.joined = false;
        }
        settings::persist(&self.directory, &settings, &credentials)?;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *guard = (settings, credentials);
        drop(guard);
        if destination {
            self.snapshot.lock().unwrap().hub = json!({"devices":[]});
            self.cycles.lock().unwrap().clear();
        }
        self.outbox.send_replace(None);
        self.configuration.send_replace(generation);
        self.collect.notify_one();
        self.refresh_quota.notify_one();
        self.emit();
        Ok(())
    }
    async fn fetch_stats(&self, generation: u64) -> Result<(), String> {
        let (settings, credentials, current) = self.context();
        if current != generation {
            return Ok(());
        }
        let response = self
            .client
            .get(format!("{}/api/stats", settings.hub_url))
            .bearer_auth(&credentials.hub_secret)
            .timeout(Duration::from_secs(12))
            .send()
            .await
            .map_err(|_| "Hub 暂时无法连接")?;
        if !response.status().is_success() {
            return Err(format!("Hub 连接失败（{}）", response.status().as_u16()));
        }
        let stats: Value = response.json().await.map_err(|_| "Hub 数据无法读取")?;
        self.adopt(stats, generation);
        Ok(())
    }
    pub fn start(self: &Arc<Self>) {
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            engine.collect_loop().await;
        });
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            engine.upload_loop().await;
        });
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            engine.stream_loop().await;
        });
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            engine.quota_loop().await;
        });
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            engine.catalog_loop().await;
        });
    }
    async fn collect_loop(self: Arc<Self>) {
        let mut watcher: Option<RecommendedWatcher> = None;
        let mut roots_key = String::new();
        let mut full_scan = tokio::time::interval(Duration::from_secs(300));
        full_scan.tick().await;
        loop {
            let (settings, _, generation) = self.context();
            let mut roots = vec![
                Path::new(&settings.codex_home).join("sessions"),
                Path::new(&settings.codex_home).join("archived_sessions"),
            ];
            roots.extend(settings.extra_roots.iter().map(PathBuf::from));
            let key = serde_json::to_string(&roots).unwrap_or_default();
            if key != roots_key {
                let weak = Arc::downgrade(&self);
                let candidate =
                    notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                        if let Ok(event) = event {
                            if event.paths.iter().any(|path| {
                                path.to_string_lossy().ends_with(".jsonl")
                                    || path.to_string_lossy().ends_with(".zst")
                            }) {
                                if let Some(engine) = weak.upgrade() {
                                    engine.collect.notify_one();
                                }
                            }
                        }
                    });
                watcher = candidate.ok();
                if let Some(watcher) = watcher.as_mut() {
                    for root in &roots {
                        if root.exists() {
                            let _ = watcher.watch(root, RecursiveMode::Recursive);
                        } else if let Some(parent) = root.parent().filter(|parent| parent.exists())
                        {
                            let _ = watcher.watch(parent, RecursiveMode::Recursive);
                        }
                    }
                }
                roots_key = key;
            }
            self.message(|status| status.collecting = true);
            let engine = self.clone();
            let scanned = tauri::async_runtime::spawn_blocking(move || {
                engine.ledger.lock().unwrap().scan(&roots)
            })
            .await;
            match scanned {
                Ok(Ok((events, missing))) => {
                    if generation != self.generation.load(Ordering::SeqCst) {
                        continue;
                    }
                    let prices = self.ledger.lock().unwrap().prices();
                    let cycles = self.cycles.lock().unwrap().clone();
                    let mut record = ledger::record(&settings, &events, &prices, &cycles, missing);
                    let mut snapshot = self.snapshot.lock().unwrap();
                    record["limits"] = json!({"providers":if snapshot.limits.is_null(){vec![]}else{vec![snapshot.limits.clone()]}});
                    snapshot.local = record.clone();
                    snapshot.status.collecting = false;
                    snapshot.status.last_collected_at = model::now_iso();
                    snapshot.status.collection_message = if missing > 0 {
                        "部分来源未读取，当前为已知用量".into()
                    } else {
                        "已读取可用日志".into()
                    };
                    if !settings.username.is_empty() && !settings.hub_url.is_empty() {
                        if !settings.joined {
                            record["username"] = json!(settings.username);
                        }
                        snapshot.status.pending = true;
                        self.outbox
                            .send_replace(Some(Upload { generation, record }));
                    }
                    drop(snapshot);
                    self.emit();
                }
                _ => self.message(|status| {
                    status.collecting = false;
                    status.collection_message = "本机统计暂不可用，已保留上次结果".into();
                }),
            }
            tokio::select! {_=full_scan.tick()=>{},_=self.collect.notified()=>{tokio::time::sleep(Duration::from_millis(700)).await;}}
            // Keep the watcher alive across ticks; one Notify coalesces storms.
            let _ = &watcher;
        }
    }
    async fn catalog_loop(self: Arc<Self>) {
        loop {
            if let Ok(response) = self
                .client
                .get(catalog::URL)
                .timeout(Duration::from_secs(15))
                .send()
                .await
            {
                if response.status().is_success()
                    && response.content_length().unwrap_or(0) <= 16 * 1024 * 1024
                {
                    let mut stream = response.bytes_stream();
                    let mut bytes = Vec::new();
                    let mut valid = true;
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(chunk) => bytes.extend_from_slice(&chunk),
                            Err(_) => {
                                valid = false;
                                break;
                            }
                        }
                        if bytes.len() > 16 * 1024 * 1024 {
                            valid = false;
                            break;
                        }
                    }
                    if valid {
                        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                            let rates = catalog::parse(&value);
                            if !rates.is_empty() {
                                let _ = self.ledger.lock().unwrap().seed_prices(&rates, true);
                                let cache = json!({"rates":rates,"source":catalog::URL,"fetchedAt":model::now_iso()});
                                let _ = settings::private_write(
                                    &self.directory.join("catalog-cache.json"),
                                    &serde_json::to_vec(&cache).unwrap_or_default(),
                                );
                                self.collect.notify_one();
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    }
    async fn upload_loop(self: Arc<Self>) {
        let mut receiver = self.outbox.subscribe();
        loop {
            if receiver.changed().await.is_err() {
                return;
            }
            loop {
                let Some(upload) = receiver.borrow_and_update().clone() else {
                    break;
                };
                let (settings, credentials, generation) = self.context();
                if upload.generation != generation || settings.hub_url.is_empty() {
                    break;
                }
                let result = self
                    .client
                    .post(format!("{}/api/ingest", settings.hub_url))
                    .bearer_auth(&credentials.hub_secret)
                    .json(&upload.record)
                    .timeout(Duration::from_secs(12))
                    .send()
                    .await;
                if generation != self.generation.load(Ordering::SeqCst) {
                    break;
                }
                match result {
                    Ok(response) if response.status().is_success() => {
                        if let Ok(result) = response.json::<Value>().await {
                            if result["member"]["id"].is_string() {
                                let mut guard = self.settings.lock().unwrap();
                                guard.0.joined = true;
                                let _ = settings::persist(&self.directory, &guard.0, &guard.1);
                            }
                        }
                        self.message(|status| {
                            status.pending = receiver.has_changed().unwrap_or(false);
                            status.last_uploaded_at = model::now_iso();
                            status.sync_message = "已连接 Hub".into();
                        });
                        let _ = self.fetch_stats(generation).await;
                        break;
                    }
                    Ok(response) => self.message(|status| {
                        status.sync_message = format!(
                            "Hub 上传失败（{}），本机记录已保存",
                            response.status().as_u16()
                        );
                    }),
                    Err(_) => self.message(|status| {
                        status.sync_message = "Hub 离线，本机记录已保存，稍后重试".into();
                    }),
                }
                tokio::select! {_=tokio::time::sleep(Duration::from_secs(15))=>{},_=receiver.changed()=>{}}
            }
        }
    }
    async fn stream_loop(self: Arc<Self>) {
        let mut configuration = self.configuration.subscribe();
        loop {
            let (settings, credentials, generation) = self.context();
            if settings.hub_url.is_empty() {
                let _ = configuration.changed().await;
                continue;
            }
            let _ = self.fetch_stats(generation).await;
            let request = self
                .client
                .get(format!("{}/api/stats/stream", settings.hub_url))
                .bearer_auth(&credentials.hub_secret)
                .header("accept", "text/event-stream")
                .send();
            let response =
                tokio::select! {response=request=>response,_=configuration.changed()=>continue};
            match response {
                Ok(response) if response.status().is_success() => {
                    self.message(|status| {
                        status.connected = true;
                        status.sync_message = "已连接 Hub".into();
                    });
                    let mut stream = response.bytes_stream();
                    let mut buffer = Vec::new();
                    loop {
                        let chunk = tokio::select! {chunk=stream.next()=>chunk,_=configuration.changed()=>break};
                        let Some(Ok(chunk)) = chunk else {
                            break;
                        };
                        buffer.extend_from_slice(&chunk);
                        if buffer.len() > 8 * 1024 * 1024 {
                            break;
                        }
                        while let Some(end) = buffer.windows(2).position(|bytes| bytes == b"\n\n") {
                            let frame = buffer.drain(..end + 2).collect::<Vec<_>>();
                            let frame = String::from_utf8_lossy(&frame);
                            for data in frame.lines().filter_map(|line| line.strip_prefix("data: "))
                            {
                                if let Ok(payload) = serde_json::from_str::<Value>(data) {
                                    let stats = payload.get("stats").unwrap_or(&payload);
                                    if stats["devices"].is_array() {
                                        self.adopt(stats.clone(), generation);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            self.message(|status| status.connected = false);
            tokio::select! {_=tokio::time::sleep(Duration::from_secs(5))=>{},_=configuration.changed()=>{}}
        }
    }
    async fn quota_loop(self: Arc<Self>) {
        loop {
            let (settings, _, generation) = self.context();
            match quota::fetch(&self.client, Path::new(&settings.codex_home)).await {
                Ok(provider) => {
                    if generation == self.generation.load(Ordering::SeqCst) {
                        self.snapshot.lock().unwrap().limits = provider;
                        self.message(|status| status.quota_message = String::new());
                        self.collect.notify_one();
                    }
                }
                Err(error) => self.message(|status| status.quota_message = error),
            }
            tokio::select! {_=tokio::time::sleep(Duration::from_secs(60))=>{},_=self.refresh_quota.notified()=>{}}
        }
    }
    pub fn persist_window(&self, width: f64, height: f64, x: i32, y: i32) {
        let mut guard = self.settings.lock().unwrap();
        guard.0.width = width;
        guard.0.height = height;
        guard.0.x = Some(x);
        guard.0.y = Some(y);
        let _ = settings::persist(&self.directory, &guard.0, &guard.1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_cache_drops_sessions_and_credentials() {
        let compact = compact_stats(
            &json!({"devices":[{"deviceId":"a","access_token":"SECRET","periods":{"today":{"sessions":{"title":"PRIVATE"},"clients":{"codex":10}}}}]}),
        );
        let text = compact.to_string();
        assert!(!text.contains("SECRET"));
        assert!(!text.contains("PRIVATE"));
        assert_eq!(
            compact["devices"][0]["periods"]["today"]["clients"]["codex"],
            10.0
        );
    }
}
