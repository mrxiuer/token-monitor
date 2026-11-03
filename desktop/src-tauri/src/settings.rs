use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;

pub const DEFAULT_HUB: &str = "";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hub_url: String,
    pub username: String,
    pub device_id: String,
    pub device_name: String,
    pub codex_home: String,
    pub extra_roots: Vec<String>,
    pub theme: String,
    pub opacity: f64,
    pub zoom: f64,
    pub system_glass: bool,
    pub glass_style: String,
    pub start_at_login: bool,
    pub shortcut: String,
    pub always_on_top: bool,
    pub width: f64,
    pub height: f64,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub joined: bool,
}
impl Default for Settings {
    fn default() -> Self {
        let hostname = hostname::get()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        Self {
            hub_url: DEFAULT_HUB.into(),
            username: String::new(),
            device_id: if hostname.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                hostname
            },
            device_name: String::new(),
            codex_home: std::env::var("CODEX_HOME")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .unwrap_or_default()
                        .join(".codex")
                        .to_string_lossy()
                        .into()
                }),
            extra_roots: vec![],
            theme: "dark".into(),
            opacity: 0.68,
            zoom: 1.0,
            system_glass: true,
            glass_style: "hud".into(),
            start_at_login: false,
            shortcut: String::new(),
            always_on_top: false,
            width: 340.0,
            height: 650.0,
            x: None,
            y: None,
            joined: false,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub hub_secret: String,
}
#[derive(Serialize, Deserialize)]
struct Profile {
    settings: Settings,
    credentials: Credentials,
}

pub fn normalize_name(value: &str) -> String {
    value
        .nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(40)
        .collect()
}
impl Settings {
    pub fn validate(&mut self) -> Result<(), String> {
        self.username = normalize_name(&self.username);
        self.device_name = normalize_name(&self.device_name);
        self.device_id = self.device_id.trim().chars().take(120).collect();
        if self.device_id.is_empty() {
            return Err("设备 ID 不能为空".into());
        }
        self.hub_url = self.hub_url.trim().trim_end_matches('/').into();
        if !self.hub_url.is_empty() {
            let url = reqwest::Url::parse(&self.hub_url).map_err(|_| "Hub 地址格式无效")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("请填写 HTTP 或 HTTPS Hub 地址，不要包含密钥或查询参数".into());
            }
        }
        if !self.opacity.is_finite() || !self.zoom.is_finite() {
            return Err("外观数值无效".into());
        }
        self.opacity = self.opacity.clamp(0.1, 1.0);
        self.zoom = self.zoom.clamp(0.7, 1.6);
        self.width = self.width.clamp(280.0, 1200.0);
        self.height = self.height.clamp(360.0, 1400.0);
        if !["dark", "light", "system"].contains(&self.theme.as_str()) {
            self.theme = "dark".into();
        }
        if !["hud", "liquid", "clear"].contains(&self.glass_style.as_str()) {
            self.glass_style = "hud".into();
        }
        self.extra_roots = self
            .extra_roots
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .take(8)
            .collect();
        Ok(())
    }
}

pub fn app_dir() -> PathBuf {
    std::env::var_os("TOKEN_MONITOR_DUO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("com.mrxiuer.tokenmonitor.duo")
        })
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("保存路径无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法建立数据目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法设置数据目录权限")?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| "写入设置失败，原文件未替换".into())
}
pub fn persist(
    directory: &Path,
    settings: &Settings,
    credentials: &Credentials,
) -> Result<(), String> {
    private_write(
        &directory.join("settings.json"),
        &serde_json::to_vec_pretty(&Profile {
            settings: settings.clone(),
            credentials: credentials.clone(),
        })
        .map_err(|_| "设置格式错误")?,
    )
}
pub fn load(directory: &Path) -> (Settings, Credentials, String) {
    let settings_path = directory.join("settings.json");
    let mut warning = String::new();
    let profile: Option<Profile> = fs::read(&settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    if settings_path.exists() && profile.is_none() {
        let backup = directory.join(format!(
            "settings-unreadable-{}.json",
            chrono::Utc::now().timestamp_millis()
        ));
        warning = if fs::copy(&settings_path, backup).is_ok() {
            "设置文件无法读取，已备份原文件并暂用默认值"
        } else {
            "设置文件无法读取，请检查数据目录权限"
        }
        .into();
    }
    let (mut settings, mut credentials) = profile
        .map(|profile| (profile.settings, profile.credentials))
        .unwrap_or_default();
    if !settings_path.exists()
        && directory == app_dir()
        && std::env::var_os("TOKEN_MONITOR_DUO_DIR").is_none()
    {
        let legacy = dirs::config_dir().unwrap_or_default().join("Token Monitor");
        if let Ok(value) = fs::read(legacy.join("settings.json")).and_then(|bytes| {
            serde_json::from_slice::<Value>(&bytes).map_err(std::io::Error::other)
        }) {
            if let Some(id) = value["deviceId"].as_str() {
                settings.device_id = id.into();
            }
            if let Some(url) = value["hubUrl"].as_str().filter(|s| !s.is_empty()) {
                settings.hub_url = url.into();
            }
            settings.username = normalize_name(value["duoUsername"].as_str().unwrap_or(""));
            settings.device_name = normalize_name(value["duoDeviceName"].as_str().unwrap_or(""));
            if let Some(opacity) = value["opacity"].as_f64() {
                settings.opacity = opacity;
            }
            if let Some(glass) = value["systemGlass"].as_bool() {
                settings.system_glass = glass;
            }
            if let Some(theme) = value["theme"].as_str() {
                settings.theme = theme.into();
            }
            if let Ok(value) = serde_json::from_slice::<Value>(
                &fs::read(legacy.join("credentials.json")).unwrap_or_default(),
            ) {
                credentials.hub_secret = value["credentials"]["hub"]["clientSecret"]
                    .as_str()
                    .unwrap_or_default()
                    .into();
            }
            warning = "已读取原版的 Hub、设备与外观设置；请填写用户名。原版数据未修改。".into();
        }
    }
    if settings.validate().is_err() {
        settings.hub_url.clear();
        warning = "保存的 Hub 配置无效，请重新填写；原数据仍保留".into();
    }
    (settings, credentials, warning)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_destinations_are_normalized() {
        assert_eq!(normalize_name("  我\n 的电脑  "), "我 的电脑");
        let mut settings = Settings {
            hub_url: "https://user:password@example.com/".into(),
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
        settings.hub_url = "https://example.com/token-monitor/".into();
        settings.validate().unwrap();
        assert_eq!(settings.hub_url, "https://example.com/token-monitor");
    }
    #[test]
    fn renderer_settings_have_no_secret() {
        assert!(!serde_json::to_string(&Settings::default())
            .unwrap()
            .contains("secret"));
    }
}
