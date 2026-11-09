//! The mod's config and its saved sign-in, a port of `config/T3craftConfig.kt`.
//! Same files and keys as the Java mod (`config/t3craft/config.json` and
//! `credential.json`), so a player moving over keeps their settings.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// The HUD's corner, as `HudAnchor` in the Java mod.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HudAnchor {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

impl HudAnchor {
    pub const ALL: [HudAnchor; 4] = [
        HudAnchor::TopLeft,
        HudAnchor::TopRight,
        HudAnchor::BottomLeft,
        HudAnchor::BottomRight,
    ];

    pub fn is_right(self) -> bool {
        matches!(self, HudAnchor::TopRight | HudAnchor::BottomRight)
    }

    pub fn is_bottom(self) -> bool {
        matches!(self, HudAnchor::BottomLeft | HudAnchor::BottomRight)
    }

    pub fn label(self) -> &'static str {
        match self {
            HudAnchor::TopLeft => "Top left",
            HudAnchor::TopRight => "Top right",
            HudAnchor::BottomLeft => "Bottom left",
            HudAnchor::BottomRight => "Bottom right",
        }
    }
}

/// What the player can change, mirrored on the mod's settings page.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_server_url")]
    pub server_url: String,
    #[serde(default = "default_true")]
    pub hud_enabled: bool,
    #[serde(default = "default_anchor")]
    pub hud_anchor: HudAnchor,
    #[serde(default = "default_offset")]
    pub hud_offset_x: i32,
    #[serde(default = "default_offset")]
    pub hud_offset_y: i32,
    #[serde(default = "default_scale")]
    pub hud_scale: f32,
    #[serde(default = "default_rows")]
    pub hud_max_rows: i32,
    #[serde(default = "default_true")]
    pub show_ready_threads: bool,
    #[serde(default = "default_true")]
    pub notify_on_change: bool,
    #[serde(default = "default_poll")]
    pub poll_seconds: i32,
}

fn default_server_url() -> String {
    discover_local_server_url()
}
fn default_true() -> bool {
    true
}
fn default_anchor() -> HudAnchor {
    HudAnchor::TopRight
}
fn default_offset() -> i32 {
    4
}
fn default_scale() -> f32 {
    1.0
}
fn default_rows() -> i32 {
    6
}
fn default_poll() -> i32 {
    5
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_url: default_server_url(),
            hud_enabled: true,
            hud_anchor: default_anchor(),
            hud_offset_x: 4,
            hud_offset_y: 4,
            hud_scale: 1.0,
            hud_max_rows: 6,
            show_ready_threads: true,
            notify_on_change: true,
            poll_seconds: 5,
        }
    }
}

impl Config {
    pub fn normalized_server_url(&self) -> String {
        self.server_url.trim().trim_end_matches('/').to_owned()
    }

    /// Out-of-range values back inside the Java mod's own limits.
    pub fn sanitized(mut self) -> Self {
        let defaults = Config::default();
        if self.server_url.trim().is_empty() {
            self.server_url = defaults.server_url;
        }
        self.hud_scale = self.hud_scale.clamp(0.5, 2.0);
        self.hud_max_rows = self.hud_max_rows.clamp(1, 20);
        self.poll_seconds = self.poll_seconds.clamp(2, 60);
        self
    }
}

/// The access token OAuth issued, kept between runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCredential {
    pub server_url: String,
    pub access_token: String,
    pub expires_at_epoch_millis: i64,
}

impl StoredCredential {
    pub fn is_usable_for(&self, server: &str, now_millis: i64) -> bool {
        self.server_url == server && now_millis < self.expires_at_epoch_millis
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `config/t3craft/` under the game's config dir (`native_config_dir`).
pub struct ConfigStore {
    directory: PathBuf,
    pub config: Config,
    pub credential: Option<StoredCredential>,
}

impl ConfigStore {
    pub fn load(directory: PathBuf) -> Self {
        let config = read_json::<Config>(&directory.join("config.json"))
            .map(Config::sanitized)
            .unwrap_or_default();
        let credential = read_json::<StoredCredential>(&directory.join("credential.json"));
        let store = Self {
            directory,
            config,
            credential,
        };
        if !store.directory.join("config.json").exists() {
            store.save();
        }
        store
    }

    fn config_file(&self) -> PathBuf {
        self.directory.join("config.json")
    }

    fn credential_file(&self) -> PathBuf {
        self.directory.join("credential.json")
    }

    pub fn save(&self) {
        let _ = fs::create_dir_all(&self.directory);
        if let Ok(json) = serde_json::to_string_pretty(&self.config) {
            let _ = fs::write(self.config_file(), json);
        }
    }

    pub fn save_credential(&mut self, value: Option<StoredCredential>) {
        self.credential = value;
        match &self.credential {
            None => {
                let _ = fs::remove_file(self.credential_file());
            }
            Some(credential) => {
                let _ = fs::create_dir_all(&self.directory);
                if let Ok(json) = serde_json::to_string_pretty(credential) {
                    let file = self.credential_file();
                    if fs::write(&file, json).is_ok() {
                        restrict_to_owner(&file);
                    }
                }
            }
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(file: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(file).ok()?).ok()
}

/// `chmod 600` where the platform has it, as the Java mod does.
fn restrict_to_owner(file: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(file, fs::Permissions::from_mode(0o600));
    }
    let _ = file;
}

/// Where a T3 Code on this machine listens, from its runtime file
/// (`~/.t3/userdata/server-runtime.json`, or `$T3CODE_HOME`): the same
/// discovery the Java mod does.
pub fn discover_local_server_url() -> String {
    let home = std::env::var_os("T3CODE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".t3")))
        .unwrap_or_else(|| PathBuf::from(".t3"));
    let runtime_file = home.join("userdata").join("server-runtime.json");
    fs::read_to_string(runtime_file)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| json.get("origin")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "http://127.0.0.1:3773".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip_with_defaults() {
        let config: Config = serde_json::from_str::<Config>("{}").unwrap().sanitized();
        assert!(config.hud_enabled);
        assert_eq!(config.hud_max_rows, 6);
        assert_eq!(config.poll_seconds, 5);
        let out = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&out).unwrap();
        assert_eq!(back.hud_anchor, HudAnchor::TopRight);
    }

    #[test]
    fn clamps_like_the_java_mod() {
        let config = Config {
            hud_scale: 9.0,
            hud_max_rows: 100,
            poll_seconds: 1,
            ..Config::default()
        }
        .sanitized();
        assert_eq!(config.hud_scale, 2.0);
        assert_eq!(config.hud_max_rows, 20);
        assert_eq!(config.poll_seconds, 2);
    }

    #[test]
    fn credential_usability() {
        let cred = StoredCredential {
            server_url: "http://127.0.0.1:3773".into(),
            access_token: "t".into(),
            expires_at_epoch_millis: 1000,
        };
        assert!(cred.is_usable_for("http://127.0.0.1:3773", 999));
        assert!(!cred.is_usable_for("http://127.0.0.1:3773", 1000));
        assert!(!cred.is_usable_for("http://other", 0));
    }

    #[test]
    fn normalizes_the_server_url() {
        let config = Config {
            server_url: " http://x/ ".into(),
            ..Config::default()
        };
        assert_eq!(config.normalized_server_url(), "http://x");
    }
}
