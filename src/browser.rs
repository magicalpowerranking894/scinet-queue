use directories::ProjectDirs;
#[cfg(not(windows))]
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs;
#[cfg(not(windows))]
use std::fs::File;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::locks::lock_token;
use crate::scinet::SCINET_URL;

pub(crate) const BROWSER_ENV: &str = "SCINET_QUEUE_BROWSER";
pub(crate) const BROWSER_PREFERENCE_FILE: &str = ".snq/browser.json";
const PROFILE_LOCK_TIMEOUT: Duration = Duration::from_secs(60);
const PROFILE_LOCK_POLL: Duration = Duration::from_millis(50);
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Browser {
    pub(crate) engine: BrowserEngine,
    pub(crate) path: PathBuf,
}

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

    #[cfg(all(test, unix))]
    pub(crate) fn launch_cdp(&self, profile_dir: &Path) -> Result<CdpBrowser, BrowserError> {
        self.launch_chromium_cdp(profile_dir, true)
    }

    #[cfg(all(test, unix))]
    fn launch_cdp_with_env(
        &self,
        profile_dir: &Path,
        envs: &[(&str, &Path)],
    ) -> Result<CdpBrowser, BrowserError> {
        self.launch_chromium_cdp_with_env(profile_dir, true, envs)
    }

    pub(crate) fn launch_session(
        &self,
        profile_dir: &Path,
        headless: bool,
    ) -> Result<ManagedBrowser, BrowserError> {
        match self.engine {
            BrowserEngine::Chromium => self
                .launch_chromium_cdp(profile_dir, headless)
                .map(ManagedBrowser::Cdp),
            BrowserEngine::Firefox => self
                .launch_firefox_bidi(profile_dir, headless)
                .map(ManagedBrowser::Bidi),
        }
    }

    fn launch_chromium_cdp(
        &self,
        profile_dir: &Path,
        headless: bool,
    ) -> Result<CdpBrowser, BrowserError> {
        self.launch_chromium_cdp_with_env(profile_dir, headless, &[])
    }

    fn launch_chromium_cdp_with_env(
        &self,
        profile_dir: &Path,
        headless: bool,
        envs: &[(&str, &Path)],
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
            .arg("--remote-debugging-port=0")
            .arg("--remote-debugging-address=127.0.0.1");
        add_chromium_profile_args(&mut command, profile_dir);
        command
            .arg(SCINET_URL)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if headless {
            command.arg("--headless=new").arg("--disable-gpu");
        } else {
            command.arg("--new-window");
        }

        for (key, value) in envs {
            command.env(key, value);
        }

        let mut child = command.spawn()?;
        let port =
            match wait_for_devtools_port(&active_port_path, &mut child, Duration::from_secs(10)) {
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

    fn launch_firefox_bidi(
        &self,
        profile_dir: &Path,
        headless: bool,
    ) -> Result<BidiBrowser, BrowserError> {
        if self.engine != BrowserEngine::Firefox {
            return Err(BrowserError::UnsupportedBidiEngine(self.engine));
        }

        fs::create_dir_all(profile_dir)?;
        let lock = ProfileLock::acquire(profile_dir)?;
        let port = reserve_loopback_port()?;

        let mut command = Command::new(&self.path);
        command
            .arg("--profile")
            .arg(profile_dir)
            .arg("--no-remote")
            .arg("--remote-debugging-port")
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if headless {
            command.arg("--headless");
        }

        let mut child = command.spawn()?;

        if let Err(error) = wait_for_tcp_port(port, &mut child, Duration::from_secs(10)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }

        Ok(BidiBrowser {
            child,
            port,
            _lock: lock,
        })
    }
}

#[derive(Debug)]
pub(crate) enum ManagedBrowser {
    Cdp(CdpBrowser),
    Bidi(BidiBrowser),
}

impl ManagedBrowser {
    pub(crate) fn port(&self) -> u16 {
        match self {
            ManagedBrowser::Cdp(browser) => browser.port(),
            ManagedBrowser::Bidi(browser) => browser.port(),
        }
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

#[derive(Debug)]
pub(crate) struct BidiBrowser {
    child: Child,
    port: u16,
    _lock: ProfileLock,
}

impl BidiBrowser {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for BidiBrowser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
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
    Json(serde_json::Error),
    PreferenceJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    NoProjectDirs,
    NoBrowserFound,
    EnvBrowserNotFound(PathBuf),
    BrowserPathNotFound(PathBuf),
    PreferenceBrowserNotFound(PathBuf),
    ProfileLocked(PathBuf),
    UnsupportedCdpEngine(BrowserEngine),
    UnsupportedBidiEngine(BrowserEngine),
    BrowserExited,
    BidiPortTimeout(u16),
    DevtoolsPortTimeout(PathBuf),
    InvalidDevtoolsPort {
        path: PathBuf,
        value: String,
    },
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrowserError::Io(error) => write!(f, "{error}"),
            BrowserError::Json(error) => write!(f, "{error}"),
            BrowserError::PreferenceJson { path, source } => write!(
                f,
                "could not parse browser preference {}: {}; run `snq browsers --pick`, `snq browsers --set <path>`, or `snq browsers --clear`",
                path.display(),
                source
            ),
            BrowserError::NoProjectDirs => write!(f, "could not resolve user data directory"),
            BrowserError::NoBrowserFound => write!(
                f,
                "no supported browser found; install a Chromium-compatible or Firefox/Gecko-based browser, or set {BROWSER_ENV}"
            ),
            BrowserError::EnvBrowserNotFound(path) => {
                write!(f, "{BROWSER_ENV} does not exist: {}", path.display())
            }
            BrowserError::BrowserPathNotFound(path) => {
                write!(f, "browser path does not exist: {}", path.display())
            }
            BrowserError::PreferenceBrowserNotFound(path) => write!(
                f,
                "configured browser does not exist: {}; run `snq browsers --pick`, `snq browsers --set <path>`, or `snq browsers --clear`",
                path.display()
            ),
            BrowserError::ProfileLocked(path) => {
                if cfg!(windows) {
                    write!(
                        f,
                        "managed browser profile lock exists: {}; close any snq command or managed browser using this profile, then remove the lock file if it is stale",
                        path.display()
                    )
                } else {
                    write!(
                        f,
                        "managed browser profile is already in use: {}; close any browser opened by `snq login --no-wait` or wait for the other snq command to finish",
                        path.display()
                    )
                }
            }
            BrowserError::UnsupportedCdpEngine(engine) => {
                write!(
                    f,
                    "CDP session probe is not supported for {engine} browsers yet"
                )
            }
            BrowserError::UnsupportedBidiEngine(engine) => {
                write!(
                    f,
                    "BiDi session probe is not supported for {engine} browsers"
                )
            }
            BrowserError::BrowserExited => {
                write!(
                    f,
                    "browser exited before automation became available; close any browser opened by `snq login --no-wait` and retry"
                )
            }
            BrowserError::BidiPortTimeout(port) => {
                write!(f, "timed out waiting for BiDi on 127.0.0.1:{port}")
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

impl From<serde_json::Error> for BrowserError {
    fn from(error: serde_json::Error) -> Self {
        BrowserError::Json(error)
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

        return Ok(Browser {
            engine: infer_engine(&path),
            path,
        });
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
        let available = path.exists();

        return vec![BrowserChoice {
            engine: infer_engine(&path),
            path,
            source: BrowserChoiceSource::Env,
            available,
            selected: available,
        }];
    }

    let mut choices = Vec::new();
    let has_preference = match read_browser_preference() {
        Ok(Some(preference)) => {
            let available = preference.path.exists();
            choices.push(BrowserChoice {
                engine: preference.engine,
                path: preference.path,
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

    resolve_browser(browser).ok_or(BrowserError::BrowserPathNotFound(original_path))
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

fn preferred_browser() -> Result<Option<Browser>, BrowserError> {
    let Some(preference) = read_browser_preference()? else {
        return Ok(None);
    };

    if !preference.path.exists() {
        return Err(BrowserError::PreferenceBrowserNotFound(preference.path));
    }

    Ok(Some(Browser {
        engine: preference.engine,
        path: preference.path,
    }))
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

fn wait_for_devtools_port(
    path: &Path,
    child: &mut Child,
    timeout: Duration,
) -> Result<u16, BrowserError> {
    let start = Instant::now();

    while start.elapsed() < timeout {
        if child.try_wait()?.is_some() {
            return Err(BrowserError::BrowserExited);
        }

        if let Ok(contents) = fs::read_to_string(path) {
            if let Some(port) = parse_devtools_port(&contents) {
                return Ok(port);
            }

            let value = contents.lines().next().unwrap_or_default().trim();
            if !value.is_empty() && value.parse::<u16>().is_err() {
                return Err(BrowserError::InvalidDevtoolsPort {
                    path: path.to_path_buf(),
                    value: value.to_string(),
                });
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    Err(BrowserError::DevtoolsPortTimeout(path.to_path_buf()))
}

fn reserve_loopback_port() -> Result<u16, BrowserError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn wait_for_tcp_port(port: u16, child: &mut Child, timeout: Duration) -> Result<(), BrowserError> {
    let start = Instant::now();

    while start.elapsed() < timeout {
        if child.try_wait()?.is_some() {
            return Err(BrowserError::BrowserExited);
        }

        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(50));
    }

    Err(BrowserError::BidiPortTimeout(port))
}

fn add_login_args(command: &mut Command, engine: BrowserEngine, profile_dir: &Path) {
    match engine {
        BrowserEngine::Chromium => {
            add_chromium_profile_args(command, profile_dir);
            command.arg("--new-window").arg(SCINET_URL);
        }
        BrowserEngine::Firefox => {
            command
                .arg("--profile")
                .arg(profile_dir)
                .arg("--no-remote")
                .arg(SCINET_URL);
        }
    }
}

fn add_chromium_profile_args(command: &mut Command, profile_dir: &Path) {
    command
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--password-store=basic")
        .arg("--use-mock-keychain");
}

#[derive(Debug)]
struct ProfileLock {
    #[cfg(not(windows))]
    _file: File,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    token: String,
}

impl ProfileLock {
    fn acquire(profile_dir: &Path) -> Result<Self, BrowserError> {
        Self::acquire_with_timeout(profile_dir, PROFILE_LOCK_TIMEOUT)
    }

    #[cfg(not(windows))]
    fn acquire_with_timeout(profile_dir: &Path, timeout: Duration) -> Result<Self, BrowserError> {
        let path = profile_dir.join(".snq-profile.lock");
        let token = lock_token();
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let start = Instant::now();

        loop {
            match file.try_lock_exclusive() {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= timeout {
                        return Err(BrowserError::ProfileLocked(path));
                    }

                    thread::sleep(PROFILE_LOCK_POLL);
                }
                Err(error) => return Err(BrowserError::Io(error)),
            }
        }

        file.set_len(0)?;
        writeln!(file, "{token}")?;
        file.sync_all()?;
        Ok(Self { _file: file })
    }

    #[cfg(windows)]
    fn acquire_with_timeout(profile_dir: &Path, timeout: Duration) -> Result<Self, BrowserError> {
        let path = profile_dir.join(".snq-profile.lock");
        let token = lock_token();
        let start = Instant::now();

        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    writeln!(file, "{token}")?;
                    file.sync_all()?;

                    return Ok(Self {
                        path: path.clone(),
                        token,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if start.elapsed() >= timeout {
                        return Err(BrowserError::ProfileLocked(path));
                    }

                    thread::sleep(PROFILE_LOCK_POLL);
                }
                Err(error) => return Err(BrowserError::Io(error)),
            }
        }
    }
}

#[cfg(not(windows))]
impl Drop for ProfileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._file);
    }
}

#[cfg(windows)]
impl Drop for ProfileLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path)
            .map(|contents| contents.trim() == self.token)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.path);
        }
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
    let mut lines = contents.lines();
    let port = lines.next()?.trim().parse().ok()?;

    if lines.next()?.trim().is_empty() {
        return None;
    }

    Some(port)
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
        assert_eq!(parse_devtools_port("9333\n"), None);
        assert_eq!(parse_devtools_port("nope\n/devtools/browser/abc\n"), None);
    }

    #[test]
    fn chromium_login_uses_managed_profile_without_keychain_prompts() {
        let profile = Path::new("/tmp/snq-profile");
        let mut command = Command::new("browser");

        add_login_args(&mut command, BrowserEngine::Chromium, profile);

        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args.contains(&format!("--user-data-dir={}", profile.display())));
        assert!(args.contains(&"--password-store=basic".to_string()));
        assert!(args.contains(&"--use-mock-keychain".to_string()));
        assert!(args.contains(&"--new-window".to_string()));
        assert!(args.contains(&SCINET_URL.to_string()));
    }

    #[test]
    fn firefox_login_uses_managed_profile_without_remote_handoff() {
        let profile = Path::new("/tmp/snq-profile");
        let mut command = Command::new("browser");

        add_login_args(&mut command, BrowserEngine::Firefox, profile);

        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args.contains(&"--profile".to_string()));
        assert!(args.contains(&profile.display().to_string()));
        assert!(args.contains(&"--no-remote".to_string()));
        assert!(args.contains(&SCINET_URL.to_string()));
    }

    #[test]
    fn profile_lock_rejects_concurrent_acquire() {
        let dir = env::temp_dir().join(format!("snq-profile-lock-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let lock = ProfileLock::acquire(&dir).unwrap();
        let second = ProfileLock::acquire_with_timeout(&dir, Duration::from_millis(1));

        assert!(matches!(second, Err(BrowserError::ProfileLocked(_))));

        drop(lock);
        assert!(ProfileLock::acquire(&dir).is_ok());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(windows)]
    fn profile_lock_error_explains_stale_windows_lock_file() {
        let path = PathBuf::from(r"C:\Users\snq\AppData\Local\profile\.snq-profile.lock");
        let message = BrowserError::ProfileLocked(path).to_string();

        assert!(message.contains("managed browser profile lock exists"));
        assert!(message.contains("remove the lock file if it is stale"));
    }

    #[test]
    #[cfg(not(windows))]
    fn profile_lock_error_explains_live_unix_lock() {
        let path = PathBuf::from("/tmp/snq-profile/.snq-profile.lock");
        let message = BrowserError::ProfileLocked(path).to_string();

        assert!(message.contains("managed browser profile is already in use"));
        assert!(message.contains("close any browser opened by `snq login --no-wait`"));
        assert!(message.contains("wait for the other snq command to finish"));
    }

    #[test]
    fn browser_exited_error_explains_open_login_browser() {
        let message = BrowserError::BrowserExited.to_string();

        assert!(message.contains("browser exited before automation became available"));
        assert!(message.contains("close any browser opened by `snq login --no-wait`"));
    }

    #[test]
    #[cfg(unix)]
    fn cdp_launch_retries_empty_devtools_port_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!(
            "snq-cdp-empty-port-test-{}-{}",
            std::process::id(),
            lock_token()
        ));
        let profile = dir.join("profile");
        let script = dir.join("fake-browser.sh");
        let args_path = dir.join("args.txt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &script,
            r#"#!/bin/sh
for arg in "$@"; do
  case "$arg" in
    --user-data-dir=*) profile="${arg#--user-data-dir=}" ;;
  esac
done
printf '%s\n' "$@" > "$SNQ_TEST_BROWSER_ARGS"
mkdir -p "$profile"
: > "$profile/DevToolsActivePort"
sleep 0.1
printf '9222\n/devtools/browser/fake\n' > "$profile/DevToolsActivePort"
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
        let cdp = browser
            .launch_cdp_with_env(&profile, &[("SNQ_TEST_BROWSER_ARGS", args_path.as_path())])
            .unwrap();

        assert_eq!(cdp.port(), 9222);
        let args = fs::read_to_string(&args_path).unwrap();
        assert!(args.contains("--password-store=basic"));
        assert!(args.contains("--use-mock-keychain"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn cdp_launch_retries_partial_devtools_port_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("snq-cdp-partial-port-test-{}", std::process::id()));
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
printf '9' > "$profile/DevToolsActivePort"
sleep 0.1
printf '9222\n/devtools/browser/fake\n' > "$profile/DevToolsActivePort"
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
        let cdp = browser.launch_cdp(&profile).unwrap();

        assert_eq!(cdp.port(), 9222);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn cdp_launch_reports_early_browser_exit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("snq-cdp-early-exit-test-{}", std::process::id()));
        let profile = dir.join("profile");
        let script = dir.join("fake-browser.sh");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();

        let browser = Browser {
            engine: BrowserEngine::Chromium,
            path: script,
        };

        assert!(matches!(
            browser.launch_cdp(&profile),
            Err(BrowserError::BrowserExited)
        ));

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
