use directories::ProjectDirs;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BROWSER_ENV: &str = "SCINET_QUEUE_BROWSER";
const LOGIN_URL: &str = "https://sci-net.xyz/";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Browser {
    pub engine: BrowserEngine,
    pub path: PathBuf,
}

impl Browser {
    pub fn launch_login(&self, profile_dir: &Path) -> Result<u32, BrowserError> {
        fs::create_dir_all(profile_dir)?;

        let mut command = Command::new(&self.path);

        match self.engine {
            BrowserEngine::Chromium => {
                command
                    .arg(format!("--user-data-dir={}", profile_dir.display()))
                    .arg("--no-first-run")
                    .arg("--no-default-browser-check")
                    .arg(LOGIN_URL);
            }
            BrowserEngine::Firefox => {
                command.arg("--profile").arg(profile_dir).arg(LOGIN_URL);
            }
        }

        let child = command.spawn()?;
        Ok(child.id())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BrowserEngine {
    Chromium,
    Firefox,
}

impl BrowserEngine {
    fn profile_name(self) -> &'static str {
        match self {
            BrowserEngine::Chromium => "chromium",
            BrowserEngine::Firefox => "firefox",
        }
    }
}

impl fmt::Display for BrowserEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrowserEngine::Chromium => f.write_str("chromium"),
            BrowserEngine::Firefox => f.write_str("firefox"),
        }
    }
}

#[derive(Debug)]
pub enum BrowserError {
    Io(std::io::Error),
    NoProjectDirs,
    NoBrowserFound,
    EnvBrowserNotFound(PathBuf),
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrowserError::Io(error) => write!(f, "{error}"),
            BrowserError::NoProjectDirs => write!(f, "could not resolve user data directory"),
            BrowserError::NoBrowserFound => write!(
                f,
                "no supported browser found; install Chrome, Chromium, Brave, Edge, or Firefox, or set {BROWSER_ENV}"
            ),
            BrowserError::EnvBrowserNotFound(path) => {
                write!(f, "{BROWSER_ENV} does not exist: {}", path.display())
            }
        }
    }
}

impl From<std::io::Error> for BrowserError {
    fn from(error: std::io::Error) -> Self {
        BrowserError::Io(error)
    }
}

pub fn detect_browser() -> Result<Browser, BrowserError> {
    if let Some(path) = env::var_os(BROWSER_ENV) {
        let path = PathBuf::from(path);

        if !path.exists() {
            return Err(BrowserError::EnvBrowserNotFound(path));
        }

        return Ok(Browser {
            engine: infer_engine(&path),
            path,
        });
    }

    browser_candidates()
        .into_iter()
        .find_map(resolve_browser)
        .ok_or(BrowserError::NoBrowserFound)
}

pub fn profile_dir(engine: BrowserEngine) -> Result<PathBuf, BrowserError> {
    let dirs =
        ProjectDirs::from("com", "tivris", "scinet-queue").ok_or(BrowserError::NoProjectDirs)?;
    let state_dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());

    Ok(state_dir.join("browser").join(engine.profile_name()))
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

fn resolve_browser(browser: Browser) -> Option<Browser> {
    if browser.path.components().count() > 1 || browser.path.is_absolute() {
        return browser.path.exists().then_some(browser);
    }

    find_in_path(&browser.path).map(|path| Browser { path, ..browser })
}

fn find_in_path(command: &Path) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;

    for dir in env::split_paths(&paths) {
        let candidate = dir.join(command);

        if candidate.exists() {
            return Some(candidate);
        }

        #[cfg(target_os = "windows")]
        {
            let candidate = candidate.with_extension("exe");

            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
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
}
