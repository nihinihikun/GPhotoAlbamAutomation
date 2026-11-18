use crate::config;
use crate::control::Cancelled;
use crate::photos::{Album, AlbumAccessDenied, Member, Metadata, Source};
use crate::progress::{AlbumResult, Event, Progress};
use crate::selection::Selection;
use crate::{now, Result};
use rand::random;
use serde::{de::IgnoredAny, Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::{Builder, NamedTempFile, TempDir};

const SCHEMA: u32 = 2;
const MAX_ROW: u64 = 64 * 1024;
const METADATA_BATCH: usize = 40;
const MAX_BATCH_BYTES: u64 = MAX_ROW * METADATA_BATCH as u64;
const ALBUM_STEM_LIMIT: usize = 255 - ".jsonl".len();

#[derive(Debug)]
struct SectionEnded(u64);

impl std::fmt::Display for SectionEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("専用ブラウザの切り替え時刻に達しました")
    }
}

impl std::error::Error for SectionEnded {}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Media {
    id: String,
    position: u64,
    file_name: Option<String>,
    taken_unix_milliseconds: Option<i64>,
    creation_unix_milliseconds: Option<i64>,
    mime_type: Option<String>,
    size_bytes: Option<u64>,
    dedup_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    account: String,
    album: Album,
    collected_unix_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Scan {
    count: u64,
    reported_count: Option<u64>,
    membership_sha256: String,
}

impl Scan {
    fn count_verified(&self) -> bool {
        self.reported_count == Some(self.count)
    }

    fn count_shortfall(&self) -> bool {
        self.reported_count
            .is_some_and(|reported| self.count < reported)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Summary {
    scan: Scan,
    missing_file_name: u64,
    missing_taken_time: u64,
    data_sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "record", content = "data")]
enum Row {
    Album(Header),
    Media(Media),
    End(Summary),
}

struct Saved {
    path: PathBuf,
    header: Header,
    summary: Summary,
}

pub struct Export {
    account: String,
    output: PathBuf,
    work: PathBuf,
    started: u64,
    base: String,
    _lock: File,
    saved: Vec<Saved>,
    list_total: Option<u64>,
    enumerated: Option<usize>,
    selected: Option<usize>,
    all_selected: bool,
    listing_rechecked: bool,
    reused: usize,
    unavailable: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Success,
    Partial,
    Failed,
}

pub struct Outcome {
    pub status: Status,
    pub report: PathBuf,
    pub data: Option<PathBuf>,
    pub album_directory: Option<PathBuf>,
}

#[derive(Serialize)]
struct AlbumFile<'a> {
    album_id: &'a str,
    file: String,
    sha256: String,
}

impl Export {
    pub fn new(output: &Path, account: &str) -> Result<Self> {
        config::create_directory(output).map_err(|e| format!("output_dir: {e}"))?;
        let output = fs::canonicalize(output)?;
        let lock = config::lock_file(&output.join(".gpae-output.lock"))?;
        let work = output.join(format!(
            ".gpae-checkpoints-{:x}",
            Sha256::digest(account.as_bytes())
        ));
        config::create_directory(&work)?;
        let started = now()?;
        Ok(Self {
            account: account.to_owned(),
            output,
            work,
            started,
            base: format!("albums_{started}_{:016x}", random::<u64>()),
            _lock: lock,
            saved: Vec::new(),
            list_total: None,
            enumerated: None,
            selected: None,
            all_selected: true,
            listing_rechecked: false,
            reused: 0,
            unavailable: Vec::new(),
        })
    }

    #[cfg(test)]
    pub fn collect(&mut self, source: &mut impl Source, progress: &Progress) -> Result<()> {
        self.collect_selected(source, progress, |_| Ok(Selection::all()))
    }

    pub fn collect_selected(
        &mut self,
        source: &mut impl Source,
        progress: &Progress,
        select: impl FnOnce(&[Album]) -> Result<Selection>,
    ) -> Result<()> {
        progress.emit(Event::Phase("アルバム一覧を取得中"));
        source.verify()?;
        let (albums, total) = list(source, |count| progress.emit(Event::Listed(count)))?;
        self.list_total = total;
        self.enumerated = Some(albums.len());
        let selection = select(&albums)?;
        let selected = selection.albums(&albums)?;
        self.selected = Some(selected.len());
        self.all_selected = selection.all;
        progress.emit(Event::AlbumTotal(selected.len()));
        for album in &selected {
            let id = &album.id;
            let mut previous_section = None;
            loop {
                if source.section_due() {
                    progress.emit(Event::Renewing(true));
                    source.renew_section()?;
                    verify_listing(source, &selected, total, self.all_selected)?;
                    progress.emit(Event::Renewing(false));
                }
                progress.control.checkpoint(progress)?;
                progress.emit(Event::AlbumStart(album.reported_count));
                let result = match self.collect_album(source, album, progress) {
                    Ok(result) => result,
                    Err(e) if e.is::<AlbumAccessDenied>() => {
                        self.unavailable.push(id.clone());
                        AlbumResult::Skipped
                    }
                    Err(e) if e.is::<Cancelled>() => return Err(e),
                    Err(e) => match e.downcast_ref::<SectionEnded>() {
                        Some(SectionEnded(count)) => {
                            if previous_section.is_some_and(|previous| *count <= previous) {
                                return Err(format!(
                                    "アルバム {id}: 専用ブラウザを切り替えても {count} 件より先に進めませんでした"
                                ).into());
                            }
                            previous_section = Some(*count);
                            continue;
                        }
                        None => return Err(format!("アルバム {id}: {e}").into()),
                    },
                };
                progress.emit(Event::AlbumDone(result));
                break;
            }
        }
        progress.control.checkpoint(progress)?;
        if source.section_due() {
            progress.emit(Event::Renewing(true));
            source.renew_section()?;
            progress.emit(Event::Renewing(false));
        }
        progress.emit(Event::Phase("アルバム一覧を再確認中"));
        verify_listing(source, &selected, total, self.all_selected)?;
        source.verify()?;
        progress.control.checkpoint(progress)?;
        self.listing_rechecked = true;
        Ok(())
    }

    fn collect_album(
        &mut self,
        source: &mut impl Source,
        album: &Album,
        progress: &Progress,
    ) -> Result<AlbumResult> {
        let key = format!("{:x}", Sha256::digest(album.id.as_bytes()));
        let path = self.work.join(format!("{key}.jsonl"));
        let batches = self.work.join(format!(".batches-{key}"));
        let existing = config::check_file(&path)?;
        if existing {
            match read_checkpoint(&path, |_| Ok(())) {
                Ok((header, summary))
                    if header.account == self.account && same_album(&header.album, album) =>
                {
                    let current = scan_members(source, album, progress, |_, _, seen, total| {
                        progress.emit(Event::Media(seen, total));
                        Ok(())
                    })?;
                    if current == summary.scan {
                        self.reused += 1;
                        self.saved.push(Saved {
                            path,
                            header,
                            summary,
                        });
                        if !current.count_shortfall() {
                            remove_batches(&batches);
                        }
                        return Ok(AlbumResult::Reused);
                    }
                    progress.emit(Event::Media(0, album.reported_count));
                }
                Ok(_) | Err(_) => {}
            }
        }

        let header = Header {
            version: SCHEMA,
            account: self.account.clone(),
            album: album.clone(),
            collected_unix_seconds: now()?,
        };
        let mut staging = NamedTempFile::new_in(&self.work)?;
        let mut rows = BufWriter::new(staging.as_file_mut());
        let mut digest = Sha256::new();
        let mut position = 0;
        let mut missing_file_name = 0;
        let mut missing_taken_time = 0;
        config::create_directory(&batches)?;
        append_row(&mut rows, &mut digest, &Row::Album(header.clone()))?;
        let scan = scan_members(source, album, progress, |source, members, _, total| {
            progress.emit(Event::Media(position, total));
            for chunk in members.chunks(METADATA_BATCH) {
                progress.control.checkpoint(progress)?;
                if source.section_due() {
                    return Err(SectionEnded(position).into());
                }
                let ids: Vec<_> = chunk.iter().map(|member| member.id.as_str()).collect();
                let mut details = batch_metadata(source, &album.id, &ids, &batches)?;
                for member in chunk {
                    position += 1;
                    let details = details.remove(&member.id);
                    let media = Media {
                        id: member.id.clone(),
                        position,
                        file_name: details
                            .as_ref()
                            .and_then(|item| item.file_name.clone())
                            .filter(|name| !name.is_empty()),
                        taken_unix_milliseconds: details
                            .as_ref()
                            .and_then(|item| item.taken_ms)
                            .or(member.taken_ms),
                        creation_unix_milliseconds: details
                            .as_ref()
                            .and_then(|item| item.creation_ms)
                            .or(member.creation_ms),
                        mime_type: None,
                        size_bytes: details.as_ref().and_then(|item| item.size_bytes),
                        dedup_key: member.dedup_key.clone(),
                    };
                    missing_file_name += u64::from(media.file_name.is_none());
                    missing_taken_time += u64::from(media.taken_unix_milliseconds.is_none());
                    append_row(&mut rows, &mut digest, &Row::Media(media))?;
                }
                progress.emit(Event::Media(position, total));
            }
            Ok(())
        })?;
        let summary = Summary {
            scan,
            missing_file_name,
            missing_taken_time,
            data_sha256: format!("{:x}", digest.finalize()),
        };
        let count_shortfall = summary.scan.count_shortfall();
        serde_json::to_writer(&mut rows, &Row::End(summary.clone()))?;
        rows.write_all(b"\n")?;
        rows.flush()?;
        drop(rows);
        staging.as_file().sync_all()?;
        read_checkpoint(staging.path(), |_| Ok(()))?;
        config::check_file(&path)?;
        staging.persist(&path)?;
        self.saved.push(Saved {
            path,
            header,
            summary,
        });
        if !count_shortfall {
            remove_batches(&batches);
        }
        Ok(AlbumResult::Saved)
    }

    pub fn finish(self, error: Option<&str>) -> Result<Outcome> {
        let mut missing_names = 0;
        let mut missing_times = 0;
        let mut unverified_counts = 0;
        let mut count_mismatches = Vec::new();
        let mut missing_titles = 0;
        let mut memberships = 0;
        for album in &self.saved {
            missing_names += album.summary.missing_file_name;
            missing_times += album.summary.missing_taken_time;
            let scan = &album.summary.scan;
            unverified_counts += u64::from(!scan.count_verified());
            if scan.count_shortfall() {
                count_mismatches.push(json!({
                    "album_id": album.header.album.id,
                    "reported_count": scan.reported_count,
                    "membership_count": scan.count,
                }));
            }
            missing_titles += u64::from(album.header.album.title.is_none());
            memberships += album.summary.scan.count;
        }
        let selected_complete = error.is_none()
            && self.listing_rechecked
            && self.selected == Some(self.saved.len())
            && self.unavailable.is_empty()
            && unverified_counts == 0
            && missing_titles == 0
            && missing_names == 0
            && missing_times == 0;
        let overall_complete = selected_complete
            && self.all_selected
            && self.list_total == Some(self.saved.len() as u64);
        let status = if overall_complete || (!self.all_selected && selected_complete) {
            Status::Success
        } else if !self.saved.is_empty() || (self.listing_rechecked && self.enumerated == Some(0)) {
            Status::Partial
        } else {
            Status::Failed
        };
        let finished = now()?;
        let validation = json!({
            "album_count_reported":self.list_total,
            "album_count_enumerated":self.enumerated,
            "album_count_selected":self.selected,
            "selection_mode":self.selected.map(|_| if self.all_selected { "all" } else { "selected" }),
            "album_list_rechecked":self.listing_rechecked,
            "album_count_saved":self.saved.len(),
            "membership_count_saved":memberships,
            "albums_with_unverified_member_count":unverified_counts,
            "album_member_count_mismatches":count_mismatches,
            "missing_album_title_count":missing_titles,
            "missing_filename_count":missing_names,
            "missing_timestamp_count":missing_times,
            "resumed_albums":self.reused,
            "access_denied_album_ids":self.unavailable,
            "selected_completeness_verified":selected_complete,
            "overall_completeness_verified":overall_complete,
            "scope":if self.all_selected {
                "albums returned by the Photos album listing; hidden or omitted albums are outside observable coverage"
            } else {
                "albums explicitly selected from the Photos album listing"
            },
        });
        let data_name = (status != Status::Failed).then(|| format!("{}.json", self.base));
        let staged = data_name
            .as_ref()
            .map(|_| self.stage_export(status, finished, &validation, error))
            .transpose()?;
        let checksum = staged
            .as_ref()
            .map(|file| hash_file(file.path()))
            .transpose()?;
        let (album_directory, album_files) = self.stage_album_files()?;
        let report_path = self.output.join(format!("{}.report.json", self.base));
        let mut report = NamedTempFile::new_in(&self.output)?;
        serde_json::to_writer_pretty(
            report.as_file_mut(),
            &json!({
                "schema_version":SCHEMA,
                "status":status,
                "account":self.account,
                "started_unix_seconds":self.started,
                "finished_unix_seconds":finished,
                "validation":validation,
                "error":error,
                "json_file":data_name,
                "sha256":checksum,
                "album_directory":album_directory.as_ref().map(|_| &self.base),
                "album_files":album_files,
                "album_backup_compatible_json":null,
                "compatibility_note":"Album Backup does not preserve album IDs, ownership, missing metadata, or ambiguous matches. The GPAE schema preserves these distinctions.",
            }),
        )?;
        report.as_file().sync_all()?;
        let data = if let (Some(file), Some(name)) = (staged, data_name) {
            let path = self.output.join(name);
            file.persist_noclobber(&path)?;
            Some(path)
        } else {
            None
        };
        if let Err(e) = report.persist_noclobber(&report_path) {
            if let Some(path) = &data {
                let _ = fs::remove_file(path);
            }
            return Err(e.into());
        }
        let album_directory = album_directory.map(TempDir::into_path);
        if !count_mismatches.is_empty() {
            eprintln!(
                "申告件数より取得件数が少ないアルバムが{}件ありました。差はレポートに記録しました。",
                count_mismatches.len()
            );
        }
        if self.listing_rechecked && self.unavailable.is_empty() && count_mismatches.is_empty() {
            for saved in &self.saved {
                if let Err(e) = fs::remove_file(&saved.path) {
                    eprintln!(
                        "チェックポイントを削除できません ({}): {e}",
                        saved.path.display()
                    );
                }
            }
            let _ = fs::remove_dir(&self.work);
        }
        Ok(Outcome {
            status,
            report: report_path,
            data,
            album_directory,
        })
    }

    fn stage_album_files(&self) -> Result<(Option<TempDir>, Vec<AlbumFile<'_>>)> {
        if self.saved.is_empty() {
            return Ok((None, Vec::new()));
        }
        // Reserve the run directory exclusively. Dropping it rolls back files until the
        // report is published.
        let directory = Builder::new()
            .prefix(&self.base)
            .rand_bytes(0)
            .tempdir_in(&self.output)?;
        let mut albums: Vec<_> = self.saved.iter().collect();
        albums.sort_unstable_by(|a, b| a.header.album.id.cmp(&b.header.album.id));
        let mut names = HashSet::new();
        let mut pending = Vec::new();
        let mut files = Vec::with_capacity(albums.len());
        for album in albums {
            let stem = album_filename_stem(album.header.album.title.as_deref());
            let name = format!("{stem}.jsonl");
            if let Some(file) = create_album_file(directory.path(), &name, &mut names)? {
                files.push(write_album_file(
                    album,
                    file,
                    directory.path(),
                    &self.base,
                    name,
                )?);
            } else {
                pending.push((album, stem));
            }
        }
        // Give every available plain name priority over any generated suffix.
        for (album, stem) in pending {
            for number in 2.. {
                let suffix = format!(" ({number})");
                let name = format!(
                    "{}{suffix}.jsonl",
                    truncate_filename(&stem, ALBUM_STEM_LIMIT - suffix.len())
                );
                if let Some(file) = create_album_file(directory.path(), &name, &mut names)? {
                    files.push(write_album_file(
                        album,
                        file,
                        directory.path(),
                        &self.base,
                        name,
                    )?);
                    break;
                }
            }
        }
        Ok((Some(directory), files))
    }

    fn stage_export(
        &self,
        status: Status,
        finished: u64,
        validation: &serde_json::Value,
        error: Option<&str>,
    ) -> Result<NamedTempFile> {
        let mut file = NamedTempFile::new_in(&self.output)?;
        let mut out = BufWriter::new(file.as_file_mut());
        write!(out,
            "{{\"schema_version\":{SCHEMA},\"account\":{},\"source\":\"google-photos-web-cdp\",\"status\":{},\"started_unix_seconds\":{},\"finished_unix_seconds\":{finished},\"validation\":{validation},\"error\":{},\"albums\":[",
            serde_json::to_string(&self.account)?, serde_json::to_string(&status)?, self.started, serde_json::to_string(&error)?,
        )?;
        for (index, saved) in self.saved.iter().enumerate() {
            if index != 0 {
                out.write_all(b",")?;
            }
            let album = &saved.header.album;
            let scan = &saved.summary.scan;
            write!(out,
                "{{\"album_id\":{},\"title\":{},\"url\":{},\"is_shared\":{},\"owner_actor_id\":{},\"owned_by_account\":null,\"reported_count\":{},\"membership_count\":{},\"membership_count_verified\":{},\"collected_unix_seconds\":{},\"items\":[",
                serde_json::to_string(&album.id)?, serde_json::to_string(&album.title)?,
                serde_json::to_string(&album.url)?, serde_json::to_string(&album.is_shared)?,
                serde_json::to_string(&album.owner_actor_id)?, serde_json::to_string(&scan.reported_count)?,
                scan.count, scan.count_verified(), saved.header.collected_unix_seconds,
            )?;
            let (header, summary) = read_checkpoint(&saved.path, |media| {
                if media.position != 1 {
                    out.write_all(b",")?;
                }
                serde_json::to_writer(&mut out, &media)?;
                Ok(())
            })?;
            if header.account != self.account
                || header.album != *album
                || summary.data_sha256 != saved.summary.data_sha256
            {
                return Err("確定中にチェックポイントの内容が変化しました".into());
            }
            out.write_all(b"]}")?;
        }
        out.write_all(b"]}")?;
        out.flush()?;
        drop(out);
        file.as_file().sync_all()?;
        let mut parser =
            serde_json::Deserializer::from_reader(BufReader::new(File::open(file.path())?));
        IgnoredAny::deserialize(&mut parser)?;
        parser.end()?;
        Ok(file)
    }
}

fn truncate_filename(name: &str, limit: usize) -> &str {
    let mut units = 0;
    let mut end = name.len();
    for (index, character) in name.char_indices() {
        units += if cfg!(windows) {
            character.len_utf16()
        } else {
            character.len_utf8()
        };
        if units > limit {
            end = index;
            break;
        }
    }
    name[..end].trim_end_matches([' ', '.'])
}

fn album_filename_stem(title: Option<&str>) -> String {
    let sanitized: String = title
        .unwrap_or_default()
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let stem = truncate_filename(&sanitized, ALBUM_STEM_LIMIT);
    let device = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        device.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || device
        .strip_prefix("COM")
        .or_else(|| device.strip_prefix("LPT"))
        .is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    {
        return format!("_{}", truncate_filename(stem, ALBUM_STEM_LIMIT - 1));
    }
    if stem.is_empty() {
        "無題のアルバム".to_owned()
    } else {
        stem.to_owned()
    }
}

fn create_album_file(
    directory: &Path,
    name: &str,
    names: &mut HashSet<String>,
) -> Result<Option<File>> {
    if !names.insert(name.to_lowercase()) {
        return Ok(None);
    }
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))
    {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(format!("アルバムファイル {name} を作成できません: {e}").into()),
    }
}

fn write_album_file<'a>(
    album: &'a Saved,
    mut file: File,
    directory: &Path,
    relative_directory: &str,
    name: String,
) -> Result<AlbumFile<'a>> {
    io::copy(&mut File::open(&album.path)?, &mut file)?;
    file.sync_all()?;
    drop(file);
    let path = directory.join(&name);
    let (header, summary) = read_checkpoint(&path, |_| Ok(()))?;
    if header.account != album.header.account
        || header.album != album.header.album
        || summary != album.summary
    {
        return Err("確定中にチェックポイントの内容が変化しました".into());
    }
    Ok(AlbumFile {
        album_id: &album.header.album.id,
        file: format!("{relative_directory}/{name}"),
        sha256: hash_file(&path)?,
    })
}

fn same_album(a: &Album, b: &Album) -> bool {
    a.id == b.id
        && a.title == b.title
        && a.is_shared == b.is_shared
        && a.owner_actor_id == b.owner_actor_id
        && a.reported_count == b.reported_count
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}

fn merge_count(current: &mut Option<u64>, reported: Option<u64>) -> Result<()> {
    if let Some(reported) = reported {
        if current.is_some_and(|old| old != reported) {
            return Err("COUNT_CHANGED: ページ間で API の申告件数が異なります".into());
        }
        *current = Some(reported);
    }
    Ok(())
}

fn check_count(actual: u64, reported: Option<u64>) -> Result<()> {
    if let Some(expected) = reported {
        if expected != actual {
            return Err(format!("COUNT_MISMATCH: 申告 {expected} 件、取得 {actual} 件").into());
        }
    }
    Ok(())
}

fn next_cursor(
    next: Option<String>,
    seen: &mut HashSet<String>,
    empty: bool,
) -> Result<Option<String>> {
    if let Some(token) = &next {
        if empty || token.is_empty() || token.len() > 16384 || !seen.insert(token.clone()) {
            return Err("PAGINATION_ERROR: 空の中間ページまたは無効・反復したトークンです".into());
        }
    }
    Ok(next)
}

fn list(
    source: &mut impl Source,
    mut on_page: impl FnMut(usize),
) -> Result<(Vec<Album>, Option<u64>)> {
    let mut albums = Vec::new();
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    let mut next = None;
    let mut total = None;
    loop {
        let page = source.albums(next.as_deref())?;
        merge_count(&mut total, page.total)?;
        next = next_cursor(page.next, &mut cursors, page.items.is_empty())?;
        for album in page.items {
            if !valid_id(&album.id) || !ids.insert(album.id.clone()) {
                return Err("DUPLICATE_ALBUM: 無効または重複するアルバム ID です".into());
            }
            albums.push(album);
        }
        on_page(albums.len());
        if next.is_none() {
            break;
        }
    }
    check_count(albums.len() as u64, total)?;
    Ok((albums, total))
}

fn scan_members<S: Source>(
    source: &mut S,
    album: &Album,
    progress: &Progress,
    mut on_page: impl FnMut(&mut S, &[Member], u64, Option<u64>) -> Result<()>,
) -> Result<Scan> {
    let mut next = None;
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    let mut reported_count = album.reported_count;
    let mut fingerprint = Sha256::new();
    loop {
        progress.control.checkpoint(progress)?;
        if source.section_due() {
            return Err(SectionEnded(ids.len() as u64).into());
        }
        let page = source.members(&album.id, next.as_deref())?;
        if (next.is_none() && page.id.as_deref() != Some(&album.id))
            || page.id.as_deref().is_some_and(|id| id != album.id)
        {
            return Err("ALBUM_ID_MISMATCH: 詳細応答が要求したアルバムと一致しません".into());
        }
        merge_count(&mut reported_count, page.reported_count)?;
        next = next_cursor(page.next, &mut cursors, page.items.is_empty())?;
        for member in &page.items {
            if !valid_id(&member.id) || !ids.insert(member.id.clone()) {
                return Err(
                    "DUPLICATE_MEDIA: アルバム内に無効または重複するメディア ID があります".into(),
                );
            }
            let signature = serde_json::to_vec(&(
                &member.id,
                member.taken_ms,
                member.creation_ms,
                &member.dedup_key,
            ))?;
            fingerprint.update(signature);
            fingerprint.update(b"\n");
        }
        if reported_count.is_some_and(|count| count < ids.len() as u64) {
            check_count(ids.len() as u64, reported_count)?;
        }
        let seen = ids.len() as u64;
        let total = reported_count.or_else(|| next.is_none().then_some(seen));
        on_page(source, &page.items, seen, total)?;
        if next.is_none() {
            break;
        }
    }
    let count = ids.len() as u64;
    Ok(Scan {
        count,
        reported_count,
        membership_sha256: format!("{:x}", fingerprint.finalize()),
    })
}

fn verify_listing(
    source: &mut impl Source,
    selected: &[&Album],
    total: Option<u64>,
    all: bool,
) -> Result<()> {
    let (again, again_total) = list(source, |_| {})?;
    let current: HashMap<_, _> = again.iter().map(|album| (&album.id, album)).collect();
    if (all && (total != again_total || again.len() != selected.len()))
        || selected.iter().any(|album| {
            !current
                .get(&album.id)
                .is_some_and(|current| same_album(album, current))
        })
    {
        return Err("ALBUM_LIST_CHANGED: 取得中に一覧が変化しました".into());
    }
    Ok(())
}

fn batch_metadata(
    source: &mut impl Source,
    album: &str,
    ids: &[&str],
    directory: &Path,
) -> Result<HashMap<String, Metadata>> {
    let key = Sha256::digest(serde_json::to_vec(ids)?);
    let path = directory.join(format!("{key:x}.json"));
    if config::check_file(&path)? {
        let mut bytes = Vec::new();
        File::open(&path)?
            .take(MAX_BATCH_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 <= MAX_BATCH_BYTES && bytes.len() >= 32 {
            let (checksum, data) = bytes.split_at(32);
            if Sha256::digest(data).as_slice() == checksum {
                if let Ok(items) = serde_json::from_slice::<Vec<Metadata>>(data) {
                    if let Ok(values) = validate_metadata(ids, items) {
                        return Ok(values);
                    }
                }
            }
        }
    }
    let items = source.metadata(album, ids)?;
    let data = serde_json::to_vec(&items)?;
    let values = validate_metadata(ids, items)?;
    if data.len() as u64 + 32 <= MAX_BATCH_BYTES {
        let mut staged = NamedTempFile::new_in(directory)?;
        staged.write_all(&Sha256::digest(&data))?;
        staged.write_all(&data)?;
        staged.as_file().sync_all()?;
        staged.persist(&path)?;
    }
    Ok(values)
}

fn validate_metadata(ids: &[&str], items: Vec<Metadata>) -> Result<HashMap<String, Metadata>> {
    let requested: HashSet<_> = ids.iter().copied().collect();
    let mut values = HashMap::new();
    for item in items {
        if !requested.contains(item.id.as_str()) || values.insert(item.id.clone(), item).is_some() {
            return Err(
                "METADATA_ID_MISMATCH: メタデータ応答に要求外または重複した ID があります".into(),
            );
        }
    }
    Ok(values)
}

fn remove_batches(path: &Path) {
    if config::check_directories(path).is_ok() {
        if let Err(e) = fs::remove_dir_all(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("照会の途中データを片付けられませんでした ({e})");
            }
        }
    }
}

fn append_row(out: &mut impl Write, digest: &mut Sha256, row: &Row) -> Result<()> {
    let mut data = serde_json::to_vec(row)?;
    data.push(b'\n');
    if data.len() as u64 >= MAX_ROW {
        return Err("メタデータ行が大きすぎます".into());
    }
    out.write_all(&data)?;
    digest.update(data);
    Ok(())
}

fn read_checkpoint(
    path: &Path,
    mut on_item: impl FnMut(Media) -> Result<()>,
) -> Result<(Header, Summary)> {
    if !config::check_file(path)? {
        return Err("チェックポイントがありません".into());
    }
    let mut reader = BufReader::new(File::open(path)?);
    let mut data = String::new();
    let mut header = None;
    let mut count = 0;
    let mut missing_file_name = 0;
    let mut missing_taken_time = 0;
    let mut ids = HashSet::new();
    let mut digest = Sha256::new();
    loop {
        data.clear();
        let length = reader.by_ref().take(MAX_ROW).read_line(&mut data)?;
        if length == 0 || length as u64 >= MAX_ROW || !data.ends_with('\n') {
            return Err("チェックポイントの行が不完全です".into());
        }
        match serde_json::from_str::<Row>(&data)? {
            Row::Album(value) if header.is_none() && value.version == SCHEMA => {
                if !valid_id(&value.album.id) {
                    return Err("チェックポイントのアルバム ID が不正です".into());
                }
                header = Some(value);
                digest.update(data.as_bytes());
            }
            Row::Media(value) if header.is_some() => {
                count += 1;
                if value.position != count || !valid_id(&value.id) || !ids.insert(value.id.clone())
                {
                    return Err("チェックポイントの項目順序または ID が不正です".into());
                }
                missing_file_name += u64::from(value.file_name.is_none());
                missing_taken_time += u64::from(value.taken_unix_milliseconds.is_none());
                digest.update(data.as_bytes());
                on_item(value)?;
            }
            Row::End(value) if header.is_some() => {
                if !reader.fill_buf()?.is_empty()
                    || value.scan.count != count
                    || value.missing_file_name != missing_file_name
                    || value.missing_taken_time != missing_taken_time
                    || value.data_sha256 != format!("{:x}", digest.finalize())
                {
                    return Err("チェックポイントの件数または SHA-256 が一致しません".into());
                }
                let header = header.ok_or("チェックポイントのヘッダーがありません")?;
                if header.album.reported_count.is_some()
                    && header.album.reported_count != value.scan.reported_count
                {
                    return Err("チェックポイントの申告件数が一致しません".into());
                }
                if value
                    .scan
                    .reported_count
                    .is_some_and(|reported| count > reported)
                {
                    check_count(count, value.scan.reported_count)?;
                }
                return Ok((header, value));
            }
            _ => return Err("チェックポイントのレコード順序が不正です".into()),
        }
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = BufReader::new(File::open(path)?);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 32768];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests;
