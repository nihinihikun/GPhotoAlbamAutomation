use crate::config::{self, Config};
use crate::Result;
use serde_json::{json, Value};
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::protocol::{Message, WebSocket, WebSocketConfig};

const MARKER: &[u8] = b"GPAE Chrome profile v1";
pub const PHOTOS: &str = "https://photos.google.com/";
const MAX_CDP_MESSAGE: usize = 8 * 1024 * 1024;

pub fn find(config: &Config) -> Result<PathBuf> {
    if let Some(path) = &config.browser_path {
        return if path.is_file() {
            Ok(path.clone())
        } else {
            Err(format!("browser_path が見つかりません: {}", path.display()).into())
        };
    }
    #[cfg(windows)]
    for relative in [
        "Google/Chrome/Application/chrome.exe",
        "Microsoft/Edge/Application/msedge.exe",
    ] {
        for root in [
            "ProgramW6432",
            "PROGRAMFILES",
            "PROGRAMFILES(X86)",
            "LOCALAPPDATA",
        ] {
            if let Some(root) = env::var_os(root) {
                let path = PathBuf::from(root).join(relative);
                if path.is_file() {
                    return Ok(path);
                }
            }
        }
    }
    #[cfg(not(windows))]
    for binary in [
        "/usr/bin/google-chrome",
        "/usr/bin/microsoft-edge",
        "/usr/bin/chromium",
    ] {
        if Path::new(binary).is_file() {
            return Ok(PathBuf::from(binary));
        }
    }
    Err("Chrome または Edge が見つかりません。browser_path を指定してください".into())
}

fn ordinary_profile_path(canonical: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = canonical.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err("専用プロファイルにローカルドライブ上のパスを指定してください".into());
        };
        match prefix.kind() {
            Prefix::Disk(_) => return Ok(canonical.to_path_buf()),
            Prefix::VerbatimDisk(drive) => {
                let mut normal = PathBuf::from(format!("{}:", char::from(drive)));
                normal.push(components.as_path());
                if normal.is_absolute() && fs::canonicalize(&normal)? == canonical {
                    return Ok(normal);
                }
            }
            _ => {}
        }
        Err("専用プロファイルにローカルドライブ上のパスを指定してください".into())
    }
    #[cfg(not(windows))]
    Ok(canonical.to_path_buf())
}

fn path_overlaps(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    let (a, b) = (
        PathBuf::from(a.as_os_str().to_string_lossy().to_lowercase()),
        PathBuf::from(b.as_os_str().to_string_lossy().to_lowercase()),
    );
    a.starts_with(&b) || b.starts_with(&a)
}

pub fn prepare(config: &Config) -> Result<PathBuf> {
    config::check_directories(&config.profile_dir)?;
    let resolved_profile = config::resolved_directory(&config.profile_dir)?;
    let parent = config
        .profile_dir
        .parent()
        .ok_or("専用プロファイルにはドライブのルートを指定できません")?;
    #[cfg(windows)]
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        for suffix in [
            "Google/Chrome/User Data",
            "Google/Chrome Beta/User Data",
            "Google/Chrome SxS/User Data",
            "Microsoft/Edge/User Data",
            "Microsoft/Edge Beta/User Data",
            "Microsoft/Edge Dev/User Data",
            "Microsoft/Edge SxS/User Data",
        ] {
            let normal = PathBuf::from(&local).join(suffix);
            if path_overlaps(&config.profile_dir, &normal)
                || config::resolved_directory(&normal)
                    .ok()
                    .is_some_and(|normal| path_overlaps(&resolved_profile, &normal))
            {
                return Err(
                    "Chrome / Edge の通常プロファイルと重ならない専用フォルダを指定してください"
                        .into(),
                );
            }
        }
    }
    if path_overlaps(
        &resolved_profile,
        &config::resolved_directory(&config.output_dir)?,
    ) {
        return Err("専用プロファイルと出力フォルダは別々に指定してください".into());
    }
    config::create_directory(parent)?;
    let marker = config.profile_dir.join(".gpae-profile");
    match fs::create_dir(&config.profile_dir) {
        Ok(()) => {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)?
                .write_all(MARKER)?;
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let marked = config::check_file(&marker)? && read_small(&marker, 64)? == MARKER;
            let same_default = fs::canonicalize(&config.default_profile).ok()
                == Some(fs::canonicalize(&config.profile_dir)?);
            let legacy = same_default
                && (config::check_file(&config.profile_dir.join("Local State"))?
                    || fs::read_dir(&config.profile_dir)?.next().is_none());
            if !marked && !legacy {
                return Err("既存のフォルダを専用プロファイルに指定できません。新しい専用フォルダを指定してください".into());
            }
        }
        Err(e) => return Err(e.into()),
    }
    config::check_directories(&config.profile_dir)?;
    ordinary_profile_path(&fs::canonicalize(&config.profile_dir)?)
}

fn command(exe: &Path, profile: &Path) -> Command {
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile);
    let mut command = Command::new(exe);
    command.arg(profile_arg).args([
        "--disable-extensions",
        "--disable-background-mode",
        "--no-first-run",
        "--no-default-browser-check",
        "--new-window",
    ]);
    command
}

pub fn setup(exe: &Path, profile: &Path) -> Result<()> {
    let _lock = config::lock_file(&profile.join(".gpae-browser.lock"))?;
    command(exe, profile)
        .arg(PHOTOS)
        .spawn()
        .map_err(|e| format!("専用ブラウザを開けません: {e}"))?;
    Ok(())
}

#[cfg(windows)]
fn ensure_profile_available(profile: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    let lock = profile.join("lockfile");
    if !config::check_file(&lock)? {
        return Ok(());
    }
    // Chromium holds this file open for writing while the profile is in use.
    match OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(&lock)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Err(
            "専用ブラウザが起動中です。SETUP.bat で開いた Chrome / Edge の右上の「…」メニューから「終了」を選び、RUN.bat をもう一度実行してください。"
                .into(),
        ),
        Err(e) => Err(format!("専用ブラウザの使用状況を確認できませんでした ({e})").into()),
    }
}

struct BrowserProcess(Child);

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() && self.0.kill().is_ok() {
            let _ = self.0.wait();
        }
    }
}

pub struct Cdp {
    socket: WebSocket<TcpStream>,
    next_id: u64,
    session: Option<String>,
    deadline: Option<Instant>,
}

#[derive(Debug)]
pub struct RequestTimeout(&'static str);

impl std::fmt::Display for RequestTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CDP の応答がタイムアウトしました ({})", self.0)
    }
}

impl std::error::Error for RequestTimeout {}

pub struct Browser {
    pub cdp: Cdp,
    process: BrowserProcess,
    _profile_lock: File,
}

impl AsMut<Cdp> for Browser {
    fn as_mut(&mut self) -> &mut Cdp {
        &mut self.cdp
    }
}

impl AsMut<Cdp> for Cdp {
    fn as_mut(&mut self) -> &mut Cdp {
        self
    }
}

fn read_small(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("ブラウザの接続情報が大きすぎます".into());
    }
    Ok(bytes)
}

fn endpoint(bytes: &[u8]) -> Result<(u16, &str)> {
    let mut lines = std::str::from_utf8(bytes)?.lines();
    let port = lines
        .next()
        .ok_or("CDP ポートがありません")?
        .parse::<u16>()?;
    let path = lines.next().ok_or("CDP パスがありません")?;
    let id = path
        .strip_prefix("/devtools/browser/")
        .ok_or("CDP パスが不正です")?;
    if port == 0
        || id.len() != 36
        || lines.next().is_some()
        || !id.bytes().enumerate().all(|(index, c)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
    {
        return Err("CDP 接続情報が不正です".into());
    }
    Ok((port, path))
}

impl Browser {
    pub fn launch(exe: &Path, profile: &Path, deadline: Option<Instant>) -> Result<Self> {
        let lock = config::lock_file(&profile.join(".gpae-browser.lock"))?;
        #[cfg(windows)]
        ensure_profile_available(profile)?;
        let discovery = profile.join("DevToolsActivePort");
        let before = if config::check_file(&discovery)? {
            read_small(&discovery, 512)?
        } else {
            Vec::new()
        };
        println!("専用ブラウザで Google フォトを開きます");
        let mut process = BrowserProcess(
            command(exe, profile)
                .args([
                    "--remote-debugging-port=0",
                    "--remote-debugging-address=127.0.0.1",
                    "about:blank",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("専用ブラウザを開けません: {e}"))?,
        );
        let startup = Instant::now() + Duration::from_secs(15);
        let startup_deadline = deadline.map_or(startup, |deadline| deadline.min(startup));
        let mut startup_error = "ブラウザが新しい CDP 接続情報を作成しませんでした".to_owned();
        let cdp = loop {
            if let Some(status) = process.0.try_wait()? {
                #[cfg(windows)]
                ensure_profile_available(profile)?;
                return Err(format!(
                    "CDP 接続前に専用ブラウザの起動プロセスが終了しました ({status})"
                )
                .into());
            }
            if Instant::now() >= startup_deadline {
                return Err(
                    format!("CDP の起動を待機中にタイムアウトしました。{startup_error}").into(),
                );
            }
            if config::check_file(&discovery)? {
                match read_small(&discovery, 512) {
                    Ok(bytes) if bytes != before => {
                        match endpoint(&bytes)
                            .and_then(|(port, path)| Cdp::connect(port, path, deadline))
                        {
                            Ok(cdp) => break cdp,
                            Err(e) => startup_error = format!("CDP に接続できませんでした ({e})"),
                        }
                    }
                    Ok(_) => {}
                    Err(e) => startup_error = format!("CDP の接続情報を読み取れませんでした ({e})"),
                }
            }
            thread::sleep(Duration::from_millis(100));
        };
        let mut browser = Self {
            cdp,
            process,
            _profile_lock: lock,
        };
        browser.cdp.open_photos()?;
        Ok(browser)
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        self.cdp.set_deadline(Some(deadline));
        let _ = self.cdp.command("Browser.close", json!({}), None);
        while Instant::now() < deadline {
            if self.process.0.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Cdp {
    fn connect(port: u16, path: &str, deadline: Option<Instant>) -> Result<Self> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
        tcp.set_read_timeout(Some(Duration::from_secs(2)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(2)))?;
        let options = WebSocketConfig {
            max_message_size: Some(MAX_CDP_MESSAGE),
            max_frame_size: Some(MAX_CDP_MESSAGE),
            ..WebSocketConfig::default()
        };
        let url = format!("ws://{address}{path}");
        let (socket, _) = tungstenite::client::client_with_config(url.as_str(), tcp, Some(options))
            .map_err(|_| "ローカル CDP WebSocket に接続できません")?;
        Ok(Self {
            socket,
            next_id: 0,
            session: None,
            deadline,
        })
    }

    fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64> {
        self.next_id += 1;
        let mut request = json!({"id":self.next_id,"method":method,"params":params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        self.socket.send(Message::Text(request.to_string()))?;
        Ok(self.next_id)
    }

    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }

    fn command(
        &mut self,
        method: &'static str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value> {
        let request = Instant::now() + Duration::from_secs(22);
        let limit = self
            .deadline
            .map_or(request, |deadline| deadline.min(request));
        if Instant::now() >= limit {
            return Err("全体の制限時間を超えました".into());
        }
        let id = self.send(method, params, session)?;
        loop {
            let remaining = limit.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if self
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    return Err("全体の制限時間を超えました".into());
                }
                return Err(RequestTimeout(method).into());
            }
            self.socket
                .get_mut()
                .set_read_timeout(Some(remaining.min(Duration::from_secs(1))))?;
            let message = match self.socket.read() {
                Ok(Message::Text(message)) => message,
                Ok(Message::Ping(_)) => {
                    self.socket.flush()?;
                    continue;
                }
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue
                }
                Err(_) => return Err(format!("CDP 接続が切れました ({method})").into()),
            };
            let response: Value =
                serde_json::from_str(&message).map_err(|_| "CDP 応答の JSON が不正です")?;
            if response["method"] == "Fetch.requestPaused"
                && response["sessionId"].as_str() == self.session.as_deref()
            {
                if let (Some(request_id), Some(session)) = (
                    response["params"]["requestId"].as_str(),
                    self.session.clone(),
                ) {
                    self.send(
                        "Fetch.failRequest",
                        json!({"requestId":request_id,"errorReason":"BlockedByClient"}),
                        Some(&session),
                    )?;
                }
            }
            if response["id"].as_u64() != Some(id) {
                continue;
            }
            if response.get("error").is_some() {
                return Err(format!("ブラウザが CDP コマンドを拒否しました ({method})").into());
            }
            return response
                .get("result")
                .cloned()
                .ok_or_else(|| "CDP 応答に結果がありません".into());
        }
    }

    fn open_photos(&mut self) -> Result<()> {
        let targets = self.command("Target.getTargets", json!({}), None)?;
        let target = targets["targetInfos"]
            .as_array()
            .and_then(|infos| {
                infos
                    .iter()
                    .find(|info| info["type"] == "page" && info["url"] == "about:blank")
            })
            .and_then(|info| info["targetId"].as_str())
            .ok_or("新規の専用ブラウザタブが見つかりません")?;
        let result = self.command(
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
            None,
        )?;
        self.session = Some(
            result["sessionId"]
                .as_str()
                .ok_or("CDP セッション ID がありません")?
                .to_owned(),
        );
        self.page(
            "Fetch.enable",
            json!({"patterns":[{"resourceType":"Image"},{"resourceType":"Media"}]}),
        )?;
        let navigation = self.page("Page.navigate", json!({"url":PHOTOS}))?;
        if navigation.get("errorText").is_some() {
            return Err("Google フォトを開けません".into());
        }
        Ok(())
    }

    fn page(&mut self, method: &'static str, params: Value) -> Result<Value> {
        let session = self
            .session
            .clone()
            .ok_or("ブラウザタブに接続していません")?;
        self.command(method, params, Some(&session))
    }

    pub fn evaluate(&mut self, expression: String, by_value: bool) -> Result<Value> {
        let reply = self.page(
            "Runtime.evaluate",
            json!({
                "expression":expression,"returnByValue":by_value,"awaitPromise":true,
                "silent":true,"objectGroup":"gpae"
            }),
        )?;
        checked_runtime(reply)
    }

    pub fn adapter_call(&mut self, object: &str, op: &str, argument: Value) -> Result<Value> {
        let reply = self.page(
            "Runtime.callFunctionOn",
            json!({
                "objectId":object,
                "functionDeclaration":"function(op, arg) { return this(op, arg); }",
                "arguments":[{"value":op},{"value":argument}],
                "returnByValue":true,"awaitPromise":true,"silent":true
            }),
        )?;
        let mut result = checked_runtime(reply)?;
        result
            .as_object_mut()
            .and_then(|object| object.remove("value"))
            .ok_or_else(|| "ブラウザの取得処理から応答がありません".into())
    }
}

fn checked_runtime(reply: Value) -> Result<Value> {
    if reply.get("exceptionDetails").is_some() {
        return Err(
            "ブラウザの取得処理が停止しました。専用タブの移動や再読み込みを確認してください".into(),
        );
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| "CDP Runtime 応答が不正です".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::{Photos, Source};
    use std::net::TcpListener;

    #[cfg(windows)]
    #[test]
    fn rejects_an_open_chromium_profile_before_launching_a_browser() {
        use std::os::windows::fs::OpenOptionsExt;

        let profile = tempfile::tempdir().unwrap();
        ensure_profile_available(profile.path()).unwrap();
        let lock = profile.path().join("lockfile");
        fs::write(&lock, b"").unwrap();
        let chromium_lock = OpenOptions::new()
            .write(true)
            .share_mode(1)
            .open(&lock)
            .unwrap();
        let launch = Browser::launch(
            &profile.path().join("unused.exe"),
            profile.path(),
            Some(Instant::now() + Duration::from_secs(1)),
        );
        assert!(matches!(launch, Err(e) if e.to_string().contains("専用ブラウザが起動中です")));
        drop(chromium_lock);
        ensure_profile_available(profile.path()).unwrap();
        assert!(lock.exists());
    }

    #[test]
    fn accepts_only_local_browser_endpoint_components() {
        assert!(
            endpoint(b"1234\n/devtools/browser/01234567-89ab-cdef-0123-456789abcdef\n").is_ok()
        );
        for invalid in [
            "1234\nws://1.2.3.4/devtools/browser/01234567-89ab-cdef-0123-456789abcdef",
            "1234\n/devtools/page/01234567-89ab-cdef-0123-456789abcdef",
            "0\n/devtools/browser/01234567-89ab-cdef-0123-456789abcdef",
        ] {
            assert!(endpoint(invalid.as_bytes()).is_err());
        }
    }

    #[test]
    fn mock_cdp_checks_session_blocks_image_and_retries_temporary_api_error() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let mut albums = 0;
            let mut members = 0;
            let mut blocked = false;
            while let Ok(Message::Text(text)) = socket.read() {
                let call: Value = serde_json::from_str(&text).unwrap();
                let response = match call["method"].as_str().unwrap() {
                    "Target.getTargets" => {
                        json!({"targetInfos":[{"type":"page","url":"about:blank","targetId":"fixture-page"}]})
                    }
                    "Target.attachToTarget" => json!({"sessionId":"fixture-session"}),
                    "Fetch.enable" => {
                        assert_eq!(
                            call["params"]["patterns"],
                            json!([{"resourceType":"Image"},{"resourceType":"Media"}])
                        );
                        json!({})
                    }
                    "Page.navigate" => {
                        assert_eq!(call["params"]["url"], PHOTOS);
                        socket.send(Message::Text(json!({"sessionId":"fixture-session","method":"Fetch.requestPaused","params":{"requestId":"fixture-image"}}).to_string())).unwrap();
                        json!({"frameId":"fixture-frame"})
                    }
                    "Fetch.failRequest" => {
                        assert_eq!(call["params"]["requestId"], "fixture-image");
                        assert_eq!(call["params"]["errorReason"], "BlockedByClient");
                        blocked = true;
                        json!({})
                    }
                    "Runtime.evaluate" if call["params"]["returnByValue"] == true => {
                        json!({"result":{"value":"https://photos.google.com"}})
                    }
                    "Runtime.evaluate" => json!({"result":{"objectId":"fixture-adapter"}}),
                    "Runtime.callFunctionOn" => {
                        assert_eq!(call["sessionId"], "fixture-session");
                        assert_eq!(call["params"]["objectId"], "fixture-adapter");
                        let op = &call["params"]["arguments"][0]["value"];
                        let value = match op.as_str().unwrap() {
                            "verify" => json!({"data":true}),
                            "albums" => {
                                albums += 1;
                                match albums {
                                    1 => json!({"error":"TEMPORARY_HTTP"}),
                                    2 => json!({"data":{"items":[],"next":null,"total":null}}),
                                    _ => json!({"error":"SERVER_WAIT"}),
                                }
                            }
                            "members" => {
                                let argument = &call["params"]["arguments"][1]["value"];
                                assert_eq!(argument["id"], "fixture-album");
                                assert_eq!(argument["next"], "fixture-next");
                                members += 1;
                                json!({"error":"API_SCHEMA","detail":"RPC_PAYLOAD"})
                            }
                            other => panic!("unexpected adapter operation {other}"),
                        };
                        json!({"result":{"value":value}})
                    }
                    method => panic!("unexpected CDP method {method}"),
                };
                socket
                    .send(Message::Text(
                        json!({"id":call["id"],"result":response}).to_string(),
                    ))
                    .unwrap();
            }
            assert!(blocked);
            assert_eq!(albums, 3);
            assert_eq!(members, 1);
        });
        let deadline = Some(Instant::now() + Duration::from_secs(10));
        let mut cdp = Cdp::connect(port, "/devtools/browser/fixture", deadline).unwrap();
        cdp.open_photos().unwrap();
        let progress = crate::progress::Progress::quiet(deadline);
        let mut source = Photos::open(&mut cdp, "a@example.com", deadline, &progress).unwrap();
        assert!(source.albums(None).unwrap().items.is_empty());
        assert!(source
            .albums(None)
            .unwrap_err()
            .to_string()
            .contains("SERVER_WAIT"));
        assert!(source
            .members("fixture-album", Some("fixture-next"))
            .unwrap_err()
            .to_string()
            .contains("members/RPC_PAYLOAD"));
        drop(cdp);
        server.join().unwrap();
    }

    #[test]
    fn silent_cdp_socket_respects_deadline() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            assert!(socket.read().is_ok());
            thread::sleep(Duration::from_millis(400));
        });
        let deadline = Some(Instant::now() + Duration::from_millis(250));
        let mut cdp = Cdp::connect(port, "/devtools/browser/fixture", deadline).unwrap();
        assert!(cdp
            .command("Target.getTargets", json!({}), None)
            .unwrap_err()
            .to_string()
            .contains("全体の制限時間"));
        server.join().unwrap();
    }
}
