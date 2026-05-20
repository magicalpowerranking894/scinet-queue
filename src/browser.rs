use directories::ProjectDirs;
use std::env;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::scinet::SCINET_URL;

const BROWSER_ENV: &str = "SCINET_QUEUE_BROWSER";

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Browser {
    pub(crate) engine: BrowserEngine,
    pub(crate) path: PathBuf,
}

impl Browser {
    pub(crate) fn launch_login(&self, profile_dir: &Path) -> Result<u32, BrowserError> {
        fs::create_dir_all(profile_dir)?;

        #[cfg(target_os = "macos")]
        if let Some(app_path) = app_bundle_path(&self.path) {
            let mut command = Command::new("open");
            command.arg("-na").arg(app_path).arg("--args");
            add_login_args(&mut command, self.engine, profile_dir);

            let child = command.spawn()?;
            return Ok(child.id());
        }

        let mut command = Command::new(&self.path);
        add_login_args(&mut command, self.engine, profile_dir);

        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(child.id())
    }

    pub(crate) fn launch_cdp(&self, profile_dir: &Path) -> Result<CdpBrowser, BrowserError> {
        self.launch_chromium_cdp(profile_dir, true)
    }

    pub(crate) fn launch_login_cdp(&self, profile_dir: &Path) -> Result<CdpBrowser, BrowserError> {
        self.launch_chromium_cdp(profile_dir, false)
    }

    fn launch_chromium_cdp(
        &self,
        profile_dir: &Path,
        headless: bool,
    ) -> Result<CdpBrowser, BrowserError> {
        if self.engine != BrowserEngine::Chromium {
            return Err(BrowserError::UnsupportedCdpEngine(self.engine));
        }

        fs::create_dir_all(profile_dir)?;
        let lock = ProfileLock::acquire(profile_dir)?;
        let active_port_path = profile_dir.join("DevToolsActivePort");
        let _ = fs::remove_file(&active_port_path);

        let mut command = Command::new(&self.path);
        command
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--remote-debugging-port=0")
            .arg("--remote-debugging-address=127.0.0.1")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(SCINET_URL)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if headless {
            command.arg("--headless=new").arg("--disable-gpu");
        } else {
            command.arg("--new-window");
        }

        let mut child = command.spawn()?;
        let port = match wait_for_devtools_port(&active_port_path, Duration::from_secs(10)) {
            Ok(port) => port,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };

        Ok(CdpBrowser {
            child,
            port,
            _lock: lock,
        })
    }
}

#[derive(Debug)]
pub(crate) struct CdpBrowser {
    child: Child,
    port: u16,
    _lock: ProfileLock,
}

impl CdpBrowser {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for CdpBrowser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum BrowserEngine {
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
pub(crate) enum BrowserError {
    Io(std::io::Error),
    NoProjectDirs,
    NoBrowserFound,
    EnvBrowserNotFound(PathBuf),
    ProfileLocked(PathBuf),
    UnsupportedCdpEngine(BrowserEngine),
    DevtoolsPortTimeout(PathBuf),
    InvalidDevtoolsPort { path: PathBuf, value: String },
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
            BrowserError::ProfileLocked(path) => {
                write!(
                    f,
                    "managed browser profile is already in use: {}; wait for the other snq command to finish",
                    path.display()
                )
            }
            BrowserError::UnsupportedCdpEngine(engine) => {
                write!(
                    f,
                    "CDP session probe is not supported for {engine} browsers yet"
                )
            }
            BrowserError::DevtoolsPortTimeout(path) => {
                write!(f, "timed out waiting for {}", path.display())
            }
            BrowserError::InvalidDevtoolsPort { path, value } => {
                write!(f, "invalid devtools port in {}: {value}", path.display())
            }
        }
    }
}

impl From<std::io::Error> for BrowserError {
    fn from(error: std::io::Error) -> Self {
        BrowserError::Io(error)
    }
}

pub(crate) fn detect_browser() -> Result<Browser, BrowserError> {
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

pub(crate) fn profile_dir(engine: BrowserEngine) -> Result<PathBuf, BrowserError> {
    let dirs = ProjectDirs::from("io.github", "tivris", "scinet-queue")
        .ok_or(BrowserError::NoProjectDirs)?;
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

fn wait_for_devtools_port(path: &Path, timeout: Duration) -> Result<u16, BrowserError> {
    let start = Instant::now();

    while start.elapsed() < timeout {
        if let Ok(contents) = fs::read_to_string(path) {
            if let Some(port) = parse_devtools_port(&contents) {
                return Ok(port);
            }

            return Err(BrowserError::InvalidDevtoolsPort {
                path: path.to_path_buf(),
                value: contents.lines().next().unwrap_or_default().to_string(),
            });
        }

        thread::sleep(Duration::from_millis(50));
    }

    Err(BrowserError::DevtoolsPortTimeout(path.to_path_buf()))
}

fn add_login_args(command: &mut Command, engine: BrowserEngine, profile_dir: &Path) {
    match engine {
        BrowserEngine::Chromium => {
            command
                .arg(format!("--user-data-dir={}", profile_dir.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--new-window")
                .arg(SCINET_URL);
        }
        BrowserEngine::Firefox => {
            command.arg("--profile").arg(profile_dir).arg(SCINET_URL);
        }
    }
}

#[derive(Debug)]
struct ProfileLock {
    path: PathBuf,
}

impl ProfileLock {
    fn acquire(profile_dir: &Path) -> Result<Self, BrowserError> {
        let path = profile_dir.join(".snq-profile.lock");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    BrowserError::ProfileLocked(path.clone())
                } else {
                    BrowserError::Io(error)
                }
            })?;

        writeln!(file, "{}", std::process::id())?;
        file.sync_all()?;

        Ok(Self { path })
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(target_os = "macos")]
fn app_bundle_path(binary_path: &Path) -> Option<PathBuf> {
    let mut path = binary_path;

    loop {
        if path.extension().and_then(|value| value.to_str()) == Some("app") {
            return Some(path.to_path_buf());
        }

        path = path.parent()?;
    }
}

fn parse_devtools_port(contents: &str) -> Option<u16> {
    contents.lines().next()?.trim().parse().ok()
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

    #[test]
    fn parses_devtools_active_port() {
        assert_eq!(
            parse_devtools_port("9333\n/devtools/browser/abc\n"),
            Some(9333)
        );
        assert_eq!(parse_devtools_port("nope\n/devtools/browser/abc\n"), None);
    }

    #[test]
    fn profile_lock_rejects_concurrent_acquire() {
        let dir = env::temp_dir().join(format!("snq-profile-lock-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let lock = ProfileLock::acquire(&dir).unwrap();
        let second = ProfileLock::acquire(&dir);

        assert!(matches!(second, Err(BrowserError::ProfileLocked(_))));

        drop(lock);
        assert!(ProfileLock::acquire(&dir).is_ok());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn failed_cdp_launch_cleans_up_child_process() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("snq-cdp-cleanup-test-{}", std::process::id()));
        let profile = dir.join("profile");
        let script = dir.join("fake-browser.sh");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &script,
            r#"#!/bin/sh
for arg in "$@"; do
  case "$arg" in
    --user-data-dir=*) profile="${arg#--user-data-dir=}" ;;
  esac
done
mkdir -p "$profile"
printf 'not-a-port\n' > "$profile/DevToolsActivePort"
sleep 60
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();

        let browser = Browser {
            engine: BrowserEngine::Chromium,
            path: script,
        };

        let result = browser.launch_cdp(&profile);
        assert!(matches!(
            result,
            Err(BrowserError::InvalidDevtoolsPort { .. })
        ));

        let output = Command::new("pgrep")
            .args(["-f", profile.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "leftover process: {}",
            String::from_utf8_lossy(&output.stdout)
        );

        let _ = fs::remove_dir_all(dir);
    }
}
