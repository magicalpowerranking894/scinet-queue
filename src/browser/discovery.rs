use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{Browser, BrowserEngine, BrowserError};

pub(crate) const BROWSER_ENV: &str = "SCINET_QUEUE_BROWSER";
pub(crate) const BROWSER_PREFERENCE_FILE: &str = ".snq/browser.json";

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct BrowserChoice {
    pub(crate) engine: BrowserEngine,
    pub(crate) path: PathBuf,
    pub(crate) source: BrowserChoiceSource,
    pub(crate) available: bool,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum BrowserChoiceSource {
    Env,
    Preference,
    Candidate,
}

impl fmt::Display for BrowserChoiceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrowserChoiceSource::Env => f.write_str("env"),
            BrowserChoiceSource::Preference => f.write_str("preference"),
            BrowserChoiceSource::Candidate => f.write_str("candidate"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct BrowserPreference {
    engine: BrowserEngine,
    path: PathBuf,
}

pub(crate) fn detect_browser() -> Result<Browser, BrowserError> {
    if let Some(path) = env::var_os(BROWSER_ENV) {
        let path = PathBuf::from(path);

        if !path.exists() {
            return Err(BrowserError::EnvBrowserNotFound(path));
        }

        let browser = Browser {
            engine: infer_engine(&path),
            path,
        };

        return resolve_browser(browser.clone())
            .ok_or(BrowserError::EnvBrowserNotUsable(browser.path));
    }

    if let Some(browser) = preferred_browser()? {
        return Ok(browser);
    }

    browser_candidates()
        .into_iter()
        .find_map(resolve_browser)
        .ok_or(BrowserError::NoBrowserFound)
}

pub(crate) fn browser_choices() -> Vec<BrowserChoice> {
    if let Some(path) = env::var_os(BROWSER_ENV) {
        let path = PathBuf::from(path);
        let browser = Browser {
            engine: infer_engine(&path),
            path,
        };
        let available = resolve_browser(browser.clone()).is_some();

        return vec![BrowserChoice {
            engine: browser.engine,
            path: browser.path,
            source: BrowserChoiceSource::Env,
            available,
            selected: available,
        }];
    }

    let mut choices = Vec::new();
    let has_preference = match read_browser_preference() {
        Ok(Some(preference)) => {
            let browser = Browser {
                engine: preference.engine,
                path: preference.path,
            };
            let available = resolve_browser(browser.clone()).is_some();
            choices.push(BrowserChoice {
                engine: browser.engine,
                path: browser.path,
                source: BrowserChoiceSource::Preference,
                available,
                selected: available,
            });
            true
        }
        Ok(None) | Err(_) => false,
    };

    for candidate in browser_candidates() {
        let Some(browser) = resolve_browser(candidate) else {
            continue;
        };

        if choices
            .iter()
            .any(|choice: &BrowserChoice| choice.path == browser.path)
        {
            continue;
        }

        choices.push(BrowserChoice {
            engine: browser.engine,
            path: browser.path,
            source: BrowserChoiceSource::Candidate,
            available: true,
            selected: false,
        });
    }

    if !has_preference {
        if let Some(choice) = choices.first_mut() {
            choice.selected = true;
        }
    }

    choices
}

pub(crate) fn browser_preference_error() -> Option<String> {
    match read_browser_preference() {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    }
}

pub(crate) fn browser_preference_path() -> PathBuf {
    PathBuf::from(BROWSER_PREFERENCE_FILE)
}

pub(crate) fn browser_preference_exists() -> bool {
    browser_preference_path().exists()
}

pub(crate) fn browser_from_path(path: PathBuf) -> Result<Browser, BrowserError> {
    let original_path = path.clone();
    let browser = Browser {
        engine: infer_engine(&path),
        path,
    };

    resolve_browser(browser).ok_or_else(|| {
        if original_path.exists() {
            BrowserError::BrowserPathNotUsable(original_path)
        } else {
            BrowserError::BrowserPathNotFound(original_path)
        }
    })
}

pub(crate) fn available_browser_candidates() -> Vec<Browser> {
    let mut browsers = Vec::new();

    for candidate in browser_candidates() {
        let Some(browser) = resolve_browser(candidate) else {
            continue;
        };

        if browsers
            .iter()
            .any(|candidate: &Browser| candidate.path == browser.path)
        {
            continue;
        }

        browsers.push(browser);
    }

    browsers
}

pub(crate) fn save_browser_preference(browser: &Browser) -> Result<(), BrowserError> {
    let path = browser_preference_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let preference = BrowserPreference {
        engine: browser.engine,
        path: browser.path.clone(),
    };
    let mut contents = serde_json::to_string_pretty(&preference)?;
    contents.push('\n');
    fs::write(path, contents)?;

    Ok(())
}

pub(crate) fn clear_browser_preference() -> Result<bool, BrowserError> {
    match fs::remove_file(browser_preference_path()) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(BrowserError::Io(error)),
    }
}

fn infer_engine(path: &Path) -> BrowserEngine {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if name.contains("firefox") || name.contains("zen") {
        BrowserEngine::Firefox
    } else {
        BrowserEngine::Chromium
    }
}

fn preferred_browser() -> Result<Option<Browser>, BrowserError> {
    let Some(preference) = read_browser_preference()? else {
        return Ok(None);
    };

    if !preference.path.exists() {
        return Err(BrowserError::PreferenceBrowserNotFound(preference.path));
    }

    let browser = Browser {
        engine: preference.engine,
        path: preference.path,
    };

    resolve_browser(browser.clone())
        .map(Some)
        .ok_or(BrowserError::PreferenceBrowserNotUsable(browser.path))
}

fn read_browser_preference() -> Result<Option<BrowserPreference>, BrowserError> {
    let path = browser_preference_path();
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(BrowserError::Io(error)),
    };

    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|source| BrowserError::PreferenceJson { path, source })
}

fn resolve_browser(browser: Browser) -> Option<Browser> {
    if browser.path.components().count() > 1 || browser.path.is_absolute() {
        return usable_browser_path(&browser.path).map(|path| Browser { path, ..browser });
    }

    find_in_path(&browser.path).map(|path| Browser { path, ..browser })
}

fn find_in_path(command: &Path) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;

    for dir in env::split_paths(&paths) {
        let candidate = dir.join(command);

        if usable_browser_path(&candidate).is_some() {
            return Some(candidate);
        }

        #[cfg(target_os = "windows")]
        {
            let candidate = candidate.with_extension("exe");

            if usable_browser_path(&candidate).is_some() {
                return Some(candidate);
            }
        }
    }

    None
}

fn usable_browser_path(path: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Some(path) = app_bundle_executable(path) {
        return Some(path);
    }

    is_executable_file(path).then(|| path.to_path_buf())
}

#[cfg(target_os = "macos")]
fn app_bundle_executable(path: &Path) -> Option<PathBuf> {
    if path.extension().and_then(|value| value.to_str()) != Some("app") || !path.is_dir() {
        return None;
    }

    let executable_dir = path.join("Contents/MacOS");

    fs::read_dir(executable_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| is_executable_file(path))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    metadata.is_file() && (metadata.permissions().mode() & 0o111) != 0
}

#[cfg(windows)]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn browser_candidates() -> Vec<Browser> {
    let mut browsers = Vec::new();

    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "brave-browser",
        "microsoft-edge",
        "msedge",
    ] {
        browsers.push(Browser {
            engine: BrowserEngine::Chromium,
            path: PathBuf::from(name),
        });
    }

    browsers.extend(platform_browser_candidates());

    for name in ["firefox", "zen"] {
        browsers.push(Browser {
            engine: BrowserEngine::Firefox,
            path: PathBuf::from(name),
        });
    }

    browsers
}

#[cfg(target_os = "macos")]
fn platform_browser_candidates() -> Vec<Browser> {
    vec![
        chromium_app("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        chromium_app("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        chromium_app("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"),
        chromium_app("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        firefox_app("/Applications/Firefox.app/Contents/MacOS/firefox"),
        firefox_app("/Applications/Firefox.app/Contents/MacOS/firefox-bin"),
        firefox_app("/Applications/Zen Browser.app/Contents/MacOS/zen"),
        firefox_app("/Applications/Zen.app/Contents/MacOS/zen"),
    ]
}

#[cfg(target_os = "windows")]
fn platform_browser_candidates() -> Vec<Browser> {
    let mut candidates = Vec::new();

    for root in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        let Some(root) = env::var_os(root) else {
            continue;
        };

        let root = PathBuf::from(root);
        candidates.push(Browser {
            engine: BrowserEngine::Chromium,
            path: root.join("Google/Chrome/Application/chrome.exe"),
        });
        candidates.push(Browser {
            engine: BrowserEngine::Chromium,
            path: root.join("Microsoft/Edge/Application/msedge.exe"),
        });
        candidates.push(Browser {
            engine: BrowserEngine::Chromium,
            path: root.join("BraveSoftware/Brave-Browser/Application/brave.exe"),
        });
        candidates.push(Browser {
            engine: BrowserEngine::Firefox,
            path: root.join("Mozilla Firefox/firefox.exe"),
        });
        candidates.push(Browser {
            engine: BrowserEngine::Firefox,
            path: root.join("Zen Browser/zen.exe"),
        });
    }

    candidates
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn platform_browser_candidates() -> Vec<Browser> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn chromium_app(path: &str) -> Browser {
    Browser {
        engine: BrowserEngine::Chromium,
        path: PathBuf::from(path),
    }
}

#[cfg(target_os = "macos")]
fn firefox_app(path: &str) -> Browser {
    Browser {
        engine: BrowserEngine::Firefox,
        path: PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn infers_firefox_like_browsers() {
        assert_eq!(
            infer_engine(Path::new(
                "/Applications/Firefox.app/Contents/MacOS/firefox"
            )),
            BrowserEngine::Firefox
        );
        assert_eq!(
            infer_engine(Path::new(
                "/Applications/Zen Browser.app/Contents/MacOS/zen"
            )),
            BrowserEngine::Firefox
        );
    }

    #[test]
    fn defaults_unknown_browser_to_chromium() {
        assert_eq!(
            infer_engine(Path::new("/usr/bin/google-chrome")),
            BrowserEngine::Chromium
        );
    }

    #[test]
    fn unresolved_relative_browser_uses_path_lookup() {
        let browser = Browser {
            engine: BrowserEngine::Chromium,
            path: PathBuf::from("definitely-not-installed-browser"),
        };

        assert!(resolve_browser(browser).is_none());
    }

    #[test]
    fn browser_from_path_rejects_directories() {
        let dir = env::temp_dir().join(format!("snq-browser-dir-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert!(matches!(
            browser_from_path(dir.clone()),
            Err(BrowserError::BrowserPathNotUsable(_))
        ));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn browser_from_path_rejects_non_executable_files() {
        let path = env::temp_dir().join(format!("snq-browser-file-test-{}", std::process::id()));
        fs::write(&path, "").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&path, permissions).unwrap();

        assert!(matches!(
            browser_from_path(path.clone()),
            Err(BrowserError::BrowserPathNotUsable(_))
        ));

        let _ = fs::remove_file(path);
    }

    #[test]
    #[cfg(unix)]
    fn browser_from_path_accepts_executable_files() {
        let path = env::temp_dir().join(format!(
            "snq-browser-executable-test-{}",
            std::process::id()
        ));
        fs::write(&path, "").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();

        let browser = browser_from_path(path.clone()).unwrap();

        assert_eq!(browser.path, path);

        let _ = fs::remove_file(browser.path);
    }
}
