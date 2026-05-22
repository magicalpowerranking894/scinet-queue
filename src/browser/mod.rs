use directories::ProjectDirs;
#[cfg(not(windows))]
use fs2::FileExt;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::env;
use std::fmt;
use std::fs;
#[cfg(not(windows))]
use std::fs::File;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::locks::lock_token;
use crate::scinet::SCINET_URL;

mod discovery;

pub(crate) use discovery::{
    BROWSER_ENV, BrowserChoice, available_browser_candidates, browser_choices, browser_from_path,
    browser_preference_error, browser_preference_exists, browser_preference_path,
    clear_browser_preference, detect_browser, save_browser_preference,
};

const PROFILE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const PROFILE_LOCK_POLL: Duration = Duration::from_millis(50);
const BIDI_LAUNCH_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Browser {
    pub(crate) engine: BrowserEngine,
    pub(crate) path: PathBuf,
}

impl Browser {
    fn profile_name(&self) -> &'static str {
        match self.engine {
            BrowserEngine::Chromium => "chromium",
            BrowserEngine::Firefox if is_zen_path(&self.path) => "zen",
            BrowserEngine::Firefox => "firefox",
        }
    }

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
        ensure_native_profile_unlocked(profile_dir)?;
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
        ensure_native_profile_unlocked(profile_dir)?;
        let lock = ProfileLock::acquire(profile_dir)?;

        let mut last_error = None;

        for _ in 0..BIDI_LAUNCH_ATTEMPTS {
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

            match wait_for_bidi_port(port, &mut child, Duration::from_secs(10)) {
                Ok(()) => {
                    return Ok(BidiBrowser {
                        child,
                        port,
                        _lock: lock,
                    });
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or(BrowserError::BidiPortTimeout(0)))
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

    pub(crate) fn wait_for_exit(&mut self, timeout: Duration) -> Result<bool, BrowserError> {
        match self {
            ManagedBrowser::Cdp(browser) => browser.wait_for_exit(timeout),
            ManagedBrowser::Bidi(browser) => browser.wait_for_exit(timeout),
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

    fn wait_for_exit(&mut self, timeout: Duration) -> Result<bool, BrowserError> {
        wait_for_child_exit(&mut self.child, timeout)
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

    fn wait_for_exit(&mut self, timeout: Duration) -> Result<bool, BrowserError> {
        wait_for_child_exit(&mut self.child, timeout)
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool, BrowserError> {
    let start = Instant::now();

    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }

        if start.elapsed() >= timeout {
            return Ok(false);
        }

        thread::sleep(Duration::from_millis(50));
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
    EnvBrowserNotUsable(PathBuf),
    BrowserPathNotFound(PathBuf),
    BrowserPathNotUsable(PathBuf),
    PreferenceBrowserNotFound(PathBuf),
    PreferenceBrowserNotUsable(PathBuf),
    ProfileLocked(PathBuf),
    NativeProfileLocked {
        profile_dir: PathBuf,
        lock_path: PathBuf,
    },
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
            BrowserError::EnvBrowserNotUsable(path) => write!(
                f,
                "{BROWSER_ENV} is not an executable browser path: {}",
                path.display()
            ),
            BrowserError::BrowserPathNotFound(path) => {
                write!(f, "browser path does not exist: {}", path.display())
            }
            BrowserError::BrowserPathNotUsable(path) => {
                write!(
                    f,
                    "browser path is not an executable file: {}",
                    path.display()
                )
            }
            BrowserError::PreferenceBrowserNotFound(path) => write!(
                f,
                "configured browser does not exist: {}; run `snq browsers --pick`, `snq browsers --set <path>`, or `snq browsers --clear`",
                path.display()
            ),
            BrowserError::PreferenceBrowserNotUsable(path) => write!(
                f,
                "configured browser is not an executable file: {}; run `snq browsers --pick`, `snq browsers --set <path>`, or `snq browsers --clear`",
                path.display()
            ),
            BrowserError::ProfileLocked(path) => {
                let owner = lock_owner_hint(path)
                    .map(|owner| format!("; lock owner: {owner}"))
                    .unwrap_or_default();

                if cfg!(windows) {
                    write!(
                        f,
                        "managed browser profile lock exists: {}{}; close any snq command or managed browser using this profile, then remove the lock file if it is stale",
                        path.display(),
                        owner
                    )
                } else {
                    write!(
                        f,
                        "managed browser profile is already in use: {}{}; close any browser opened by `snq login --no-wait` or wait for the other snq command to finish",
                        path.display(),
                        owner
                    )
                }
            }
            BrowserError::NativeProfileLocked {
                profile_dir,
                lock_path,
            } => write!(
                f,
                "managed browser profile appears to be open: {}; close the browser opened by `snq login --no-wait` before running authenticated commands, or remove stale browser lock file {} if no managed browser is running",
                profile_dir.display(),
                lock_path.display()
            ),
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

pub(crate) fn profile_dir(browser: &Browser) -> Result<PathBuf, BrowserError> {
    let dirs = ProjectDirs::from("io.github", "tivris", "scinet-queue")
        .ok_or(BrowserError::NoProjectDirs)?;
    let state_dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());

    Ok(state_dir.join("browser").join(browser.profile_name()))
}

fn is_zen_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase().contains("zen"))
        .unwrap_or(false)
}

fn ensure_native_profile_unlocked(profile_dir: &Path) -> Result<(), BrowserError> {
    cleanup_stale_native_profile_locks(profile_dir)?;

    for lock_name in ["SingletonLock", "SingletonSocket", "parent.lock"] {
        let lock_path = profile_dir.join(lock_name);

        if lock_path.exists() || fs::symlink_metadata(&lock_path).is_ok() {
            return Err(BrowserError::NativeProfileLocked {
                profile_dir: profile_dir.to_path_buf(),
                lock_path,
            });
        }
    }

    Ok(())
}

fn cleanup_stale_native_profile_locks(profile_dir: &Path) -> Result<(), BrowserError> {
    #[cfg(unix)]
    {
        let lock_path = profile_dir.join("SingletonLock");

        if let Some(pid) = native_lock_pid(&lock_path) {
            if !process_exists(pid) {
                for lock_name in [
                    "SingletonLock",
                    "SingletonSocket",
                    "SingletonCookie",
                    "DevToolsActivePort",
                ] {
                    let path = profile_dir.join(lock_name);
                    match fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(BrowserError::Io(error)),
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(unix)]
fn native_lock_pid(lock_path: &Path) -> Option<u32> {
    let target = fs::read_link(lock_path).ok()?;
    let target = target.to_string_lossy();
    let (_, pid) = target.rsplit_once('-')?;

    pid.parse().ok()
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

fn lock_owner_hint(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.lines().next().map(str::trim).map(str::to_string))
        .filter(|line| !line.is_empty())
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

fn wait_for_bidi_port(port: u16, child: &mut Child, timeout: Duration) -> Result<(), BrowserError> {
    let start = Instant::now();
    let url = format!("ws://127.0.0.1:{port}/session");

    while start.elapsed() < timeout {
        if child.try_wait()?.is_some() {
            return Err(BrowserError::BrowserExited);
        }

        if let Ok((mut socket, _)) = tungstenite::connect(&url) {
            let _ = socket.close(None);
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn firefox_family_profiles_are_browser_specific() {
        let firefox = Browser {
            engine: BrowserEngine::Firefox,
            path: PathBuf::from("/Applications/Firefox.app/Contents/MacOS/firefox"),
        };
        let firefox_bin = Browser {
            engine: BrowserEngine::Firefox,
            path: PathBuf::from("/Applications/Firefox.app/Contents/MacOS/firefox-bin"),
        };
        let zen = Browser {
            engine: BrowserEngine::Firefox,
            path: PathBuf::from("/Applications/Zen Browser.app/Contents/MacOS/zen"),
        };
        let chromium = Browser {
            engine: BrowserEngine::Chromium,
            path: PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        };

        assert_eq!(firefox.profile_name(), "firefox");
        assert_eq!(firefox_bin.profile_name(), "firefox");
        assert_eq!(zen.profile_name(), "zen");
        assert_eq!(chromium.profile_name(), "chromium");
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
    fn profile_lock_error_includes_owner_when_available() {
        let dir = env::temp_dir().join(format!(
            "snq-profile-lock-owner-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".snq-profile.lock");
        fs::write(&path, "pid=123 host=test\n").unwrap();

        let message = BrowserError::ProfileLocked(path).to_string();

        assert!(message.contains("lock owner: pid=123 host=test"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn native_profile_lock_reports_open_no_wait_browser() {
        let dir = env::temp_dir().join(format!(
            "snq-native-profile-lock-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("parent.lock"), "").unwrap();

        let error = ensure_native_profile_unlocked(&dir).unwrap_err();
        let message = error.to_string();

        assert!(matches!(error, BrowserError::NativeProfileLocked { .. }));
        assert!(message.contains("managed browser profile appears to be open"));
        assert!(message.contains("snq login --no-wait"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn stale_chromium_native_lock_is_removed() {
        use std::os::unix::fs::symlink;

        let dir = env::temp_dir().join(format!(
            "snq-stale-native-profile-lock-test-{}",
            lock_token()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        symlink("host-99999999", dir.join("SingletonLock")).unwrap();
        symlink("/tmp/snq-missing-socket", dir.join("SingletonSocket")).unwrap();
        symlink("cookie", dir.join("SingletonCookie")).unwrap();
        fs::write(
            dir.join("DevToolsActivePort"),
            "9222\n/devtools/browser/fake\n",
        )
        .unwrap();

        ensure_native_profile_unlocked(&dir).unwrap();

        assert!(fs::symlink_metadata(dir.join("SingletonLock")).is_err());
        assert!(fs::symlink_metadata(dir.join("SingletonSocket")).is_err());
        assert!(fs::symlink_metadata(dir.join("SingletonCookie")).is_err());
        assert!(fs::symlink_metadata(dir.join("DevToolsActivePort")).is_err());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn active_chromium_native_lock_still_blocks() {
        use std::os::unix::fs::symlink;

        let dir = env::temp_dir().join(format!(
            "snq-active-native-profile-lock-test-{}",
            lock_token()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        symlink(
            format!("host-{}", std::process::id()),
            dir.join("SingletonLock"),
        )
        .unwrap();

        let error = ensure_native_profile_unlocked(&dir).unwrap_err();

        assert!(matches!(error, BrowserError::NativeProfileLocked { .. }));
        assert!(fs::symlink_metadata(dir.join("SingletonLock")).is_ok());

        let _ = fs::remove_dir_all(dir);
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
