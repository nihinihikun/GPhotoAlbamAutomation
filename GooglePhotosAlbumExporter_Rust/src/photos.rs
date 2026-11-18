use crate::browser::{Browser, Cdp, RequestTimeout};
use crate::progress::{Event, Progress};
use crate::Result;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Album {
    pub id: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub is_shared: Option<bool>,
    pub owner_actor_id: Option<String>,
    pub reported_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlbumPage {
    pub items: Vec<Album>,
    pub next: Option<String>,
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberPage {
    pub id: Option<String>,
    pub reported_count: Option<u64>,
    pub items: Vec<Member>,
    pub next: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub id: String,
    pub taken_ms: Option<i64>,
    pub creation_ms: Option<i64>,
    pub dedup_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub id: String,
    pub file_name: Option<String>,
    pub taken_ms: Option<i64>,
    pub creation_ms: Option<i64>,
    pub size_bytes: Option<u64>,
}

pub trait Source {
    fn verify(&mut self) -> Result<()>;
    fn albums(&mut self, next: Option<&str>) -> Result<AlbumPage>;
    fn members(&mut self, id: &str, next: Option<&str>) -> Result<MemberPage>;
    fn metadata(&mut self, album: &str, ids: &[&str]) -> Result<Vec<Metadata>>;
    fn section_due(&self) -> bool {
        false
    }
    fn renew_section(&mut self) -> Result<()> {
        Err("この接続は切り替えられません".into())
    }
}

#[derive(Debug)]
pub struct AlbumAccessDenied;

impl std::fmt::Display for AlbumAccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ALBUM_ACCESS_DENIED: アルバムの詳細に対する閲覧権限を確認できません")
    }
}

impl std::error::Error for AlbumAccessDenied {}

pub struct Photos<'a, C> {
    connection: C,
    adapter: String,
    progress: &'a Progress,
}

fn explain(code: &str) -> &'static str {
    match code {
        "LOGIN_REQUIRED" | "LOGIN_OR_ACCESS_REQUIRED" =>
            "LOGIN_OR_ACCESS_REQUIRED: SETUP.bat でログイン状態やアルバムの閲覧権限を確認してください",
        "ACCOUNT_MISMATCH" | "ACCOUNT_CHANGED" =>
            "ACCOUNT_MISMATCH: 専用ブラウザのアカウントが設定と一致しないか切り替わりました",
        "ACCOUNT_UNVERIFIED" =>
            "ACCOUNT_UNVERIFIED: Google フォト右上のアカウントとセッションを確認できません",
        "SERVER_WAIT" =>
            "SERVER_WAIT: Google フォトがリクエストを制限しているため停止しました。時間を置いて再実行してください",
        "TEMPORARY_NETWORK" | "TEMPORARY_HTTP" =>
            "RETRY_EXHAUSTED: Google フォトの通信エラーが再試行後も続いています",
        "HTTP_ERROR" => "HTTP_ERROR: Google フォトが読み取りリクエストを拒否しました",
        _ => "API_SCHEMA: Google フォトから想定した形式の応答を取得できません",
    }
}

impl<'a, C: AsMut<Cdp>> Photos<'a, C> {
    pub fn open(
        mut connection: C,
        account: &str,
        deadline: Option<Instant>,
        progress: &'a Progress,
    ) -> Result<Self> {
        let cdp = connection.as_mut();
        let ready = Instant::now() + Duration::from_secs(40);
        let ready_limit = deadline.map_or(ready, |deadline| deadline.min(ready));
        let expression = format!(
            "({})({})",
            include_str!("photos.js"),
            serde_json::to_string(account)?
        );
        let mut adapter = None;
        while Instant::now() < ready_limit {
            let origin = cdp.evaluate("location.origin".to_owned(), true)?;
            match origin["value"].as_str() {
                Some("https://accounts.google.com") => return Err(explain("LOGIN_REQUIRED").into()),
                Some("https://photos.google.com") => {
                    if adapter.is_none() {
                        let result = cdp.evaluate(expression.clone(), false)?;
                        adapter = Some(
                            result["objectId"]
                                .as_str()
                                .ok_or("CDP adapter object ID がありません")?
                                .to_owned(),
                        );
                    }
                    let object = adapter.as_deref().unwrap_or_default();
                    let result = cdp.adapter_call(object, "verify", Value::Null)?;
                    if result["data"] == true {
                        return Ok(Self {
                            connection,
                            adapter: object.to_owned(),
                            progress,
                        });
                    }
                    let code = result["error"].as_str().unwrap_or("API_SCHEMA");
                    if code != "ACCOUNT_UNVERIFIED" {
                        return Err(explain(code).into());
                    }
                }
                Some("null") => {}
                _ => return Err(explain("LOGIN_REQUIRED").into()),
            }
            thread::sleep(Duration::from_millis(350));
        }
        Err(explain("ACCOUNT_UNVERIFIED").into())
    }

    fn call<T: DeserializeOwned>(&mut self, op: &str, argument: Value) -> Result<T> {
        let waiting = match op {
            "albums" => "アルバム一覧の応答待ち",
            "members" => "アルバム内の一覧の応答待ち",
            "metadata" => "写真・動画の一括照会の応答待ち",
            "metadata_single" => "写真・動画の個別照会の応答待ち",
            _ => "Google フォトの応答待ち",
        };
        for attempt in 0..3 {
            let deadline = self.progress.control.checkpoint(self.progress)?;
            let cdp = self.connection.as_mut();
            cdp.set_deadline(deadline);
            self.progress.emit(Event::Waiting(waiting));
            let mut result = match cdp.adapter_call(&self.adapter, op, argument.clone()) {
                Ok(result) => result,
                Err(e) if e.is::<RequestTimeout>() && attempt < 2 => {
                    self.progress
                        .emit(Event::Waiting("ブラウザの応答が遅いため再試行待ち"));
                    thread::sleep(Duration::from_millis(750 << attempt));
                    continue;
                }
                Err(e) => return Err(e),
            };
            match result.get("error").and_then(Value::as_str) {
                None => {
                    let value = result
                        .as_object_mut()
                        .and_then(|object| object.remove("data"))
                        .ok_or(explain("API_SCHEMA"))?;
                    let data = serde_json::from_value(value).map_err(|_| explain("API_SCHEMA"))?;
                    self.progress
                        .emit(Event::Activity("Google フォトから応答を受信"));
                    if op != "verify" {
                        thread::sleep(Duration::from_millis(220));
                    }
                    return Ok(data);
                }
                Some("TEMPORARY_NETWORK" | "TEMPORARY_HTTP") if attempt < 2 => {
                    let pause = Duration::from_millis(750 << attempt);
                    if deadline.is_some_and(|deadline| Instant::now() + pause >= deadline) {
                        return Err("全体の制限時間を超えました".into());
                    }
                    self.progress
                        .emit(Event::Waiting("通信エラーのため再試行待ち"));
                    thread::sleep(pause);
                }
                Some("ALBUM_ACCESS_DENIED") => return Err(AlbumAccessDenied.into()),
                Some("API_SCHEMA") => {
                    let detail = result["detail"]
                        .as_str()
                        .filter(|detail| {
                            matches!(
                                *detail,
                                "ARRAY"
                                    | "TEXT"
                                    | "COUNT"
                                    | "TIMESTAMP"
                                    | "CURSOR"
                                    | "ID"
                                    | "RESPONSE_PREFIX"
                                    | "RESPONSE_FRAME"
                                    | "RPC_PAYLOAD"
                                    | "RPC_MISSING"
                                    | "ALBUM_LIST"
                                    | "ALBUM_ATTRIBUTES"
                                    | "ALBUM_SHARE_KEY"
                                    | "ALBUM_SHARED_FLAG"
                                    | "ALBUM_TITLE"
                                    | "ALBUM_OWNER"
                                    | "ALBUM_LIST_COUNT"
                                    | "ALBUM_MEMBERS"
                                    | "ALBUM_ID"
                                    | "ALBUM_MEMBER_COUNT"
                                    | "METADATA_ROW"
                                    | "METADATA_ROW_COUNT"
                                    | "NORMALIZATION"
                            )
                        })
                        .unwrap_or("UNKNOWN");
                    return Err(format!("{} ({op}/{detail})", explain("API_SCHEMA")).into());
                }
                Some(code) => return Err(explain(code).into()),
            }
        }
        Err(explain("TEMPORARY_NETWORK").into())
    }
}

impl<C: AsMut<Cdp>> Source for Photos<'_, C> {
    fn verify(&mut self) -> Result<()> {
        if self.call("verify", Value::Null)? {
            Ok(())
        } else {
            Err(explain("ACCOUNT_UNVERIFIED").into())
        }
    }

    fn albums(&mut self, next: Option<&str>) -> Result<AlbumPage> {
        self.call("albums", json!(next))
    }

    fn members(&mut self, id: &str, next: Option<&str>) -> Result<MemberPage> {
        self.call("members", json!({"id":id,"next":next}))
    }

    fn metadata(&mut self, album: &str, ids: &[&str]) -> Result<Vec<Metadata>> {
        let mut batch: Vec<Metadata> = self.call("metadata", json!(ids))?;
        for id in ids {
            let existing = batch.iter().position(|item| item.id == *id);
            if existing.is_some_and(|index| {
                batch[index]
                    .file_name
                    .as_deref()
                    .is_some_and(|name| !name.is_empty())
            }) {
                continue;
            }
            let fallback: Option<Metadata> =
                self.call("metadata_single", json!({"album":album,"id":id}))?;
            if let Some(fallback) = fallback {
                if fallback.id != *id {
                    return Err("METADATA_ID_MISMATCH: 詳細応答の ID が一致しません".into());
                }
                if let Some(index) = existing {
                    let item = &mut batch[index];
                    item.file_name = fallback.file_name;
                    item.taken_ms = item.taken_ms.or(fallback.taken_ms);
                    item.size_bytes = item.size_bytes.or(fallback.size_bytes);
                } else {
                    batch.push(fallback);
                }
            }
        }
        Ok(batch)
    }
}

pub struct Sections<'a> {
    executable: &'a Path,
    profile: &'a Path,
    account: &'a str,
    progress: &'a Progress,
    session: Option<Photos<'a, Browser>>,
    renew_at: Instant,
}

impl<'a> Sections<'a> {
    pub fn open(
        executable: &'a Path,
        profile: &'a Path,
        account: &'a str,
        progress: &'a Progress,
    ) -> Result<Self> {
        let mut source = Self {
            executable,
            profile,
            account,
            progress,
            session: None,
            renew_at: Instant::now(),
        };
        source.renew_section()?;
        Ok(source)
    }

    fn session(&mut self) -> Result<&mut Photos<'a, Browser>> {
        self.session
            .as_mut()
            .ok_or_else(|| "専用ブラウザに接続していません".into())
    }
}

impl Source for Sections<'_> {
    fn verify(&mut self) -> Result<()> {
        self.session()?.verify()
    }

    fn albums(&mut self, next: Option<&str>) -> Result<AlbumPage> {
        self.session()?.albums(next)
    }

    fn members(&mut self, id: &str, next: Option<&str>) -> Result<MemberPage> {
        self.session()?.members(id, next)
    }

    fn metadata(&mut self, album: &str, ids: &[&str]) -> Result<Vec<Metadata>> {
        self.session()?.metadata(album, ids)
    }

    fn section_due(&self) -> bool {
        Instant::now() >= self.renew_at
    }

    fn renew_section(&mut self) -> Result<()> {
        let deadline = self.progress.control.checkpoint(self.progress)?;
        let first = self.session.is_none();
        drop(self.session.take());
        let started = Instant::now();
        let browser = Browser::launch(self.executable, self.profile, deadline)?;
        if first {
            self.progress.emit(Event::Phase("アカウントを確認中"));
        }
        self.session = Some(Photos::open(
            browser,
            self.account,
            deadline,
            self.progress,
        )?);
        self.renew_at = started + Duration::from_secs(30 * 60);
        Ok(())
    }
}
