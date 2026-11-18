#![forbid(unsafe_code)]

mod browser;
mod config;
mod control;
mod export;
mod photos;
mod progress;
mod selection;

use photos::Source;
use progress::{Event, Progress};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn run(config: &config::Config, all: bool) -> Result<i32> {
    // Resolve and check both directory trees before creating either one.
    config::check_directories(&config.output_dir)?;
    let profile = browser::prepare(config)?;
    let mut export = export::Export::new(&config.output_dir, &config.account)?;
    let started = Instant::now();
    let deadline = (config.timeout_seconds != 0)
        .then(|| started + Duration::from_secs(config.timeout_seconds));
    let result = Progress::track(started, deadline, |progress| {
        let result = (|| -> Result<()> {
            let executable = browser::find(config)?;
            let mut source =
                photos::Sections::open(&executable, &profile, &config.account, progress)?;
            export.collect_selected(&mut source, progress, |albums| {
                let selection = if all {
                    selection::Selection::all()
                } else {
                    progress.emit(Event::Phase("画面でアルバムを選択してください"));
                    let waiting = Instant::now();
                    let choice = selection::choose(albums);
                    progress.control.exclude_wait(waiting.elapsed());
                    choice?
                };
                progress.control.start_keyboard();
                Ok(selection)
            })
        })();
        if result
            .as_ref()
            .is_err_and(|error| error.is::<selection::Cancelled>())
        {
            return Ok(None);
        }
        let cancelled = progress.control.finish();
        let error = if cancelled {
            Some(control::Cancelled.to_string())
        } else {
            result.err().map(|e| e.to_string())
        };
        progress.emit(Event::Phase("結果をファイルに保存中"));
        Ok(Some((export.finish(error.as_deref())?, error, cancelled)))
    })?;
    let Some((outcome, error, cancelled)) = result else {
        println!("アルバムの選択をキャンセルしました");
        return Ok(3);
    };
    if let Some(error) = error {
        eprintln!("{error}");
    }
    if let Some(data) = outcome.data {
        println!("JSON: {}", data.display());
    }
    if let Some(directory) = outcome.album_directory {
        println!("アルバム別 JSONL: {}", directory.display());
    }
    println!("レポート: {}", outcome.report.display());
    if cancelled {
        println!("CANCELLED");
        return Ok(3);
    }
    match outcome.status {
        export::Status::Success => {
            println!("SUCCESS");
            Ok(0)
        }
        export::Status::Partial => {
            println!("PARTIAL");
            Ok(2)
        }
        export::Status::Failed => {
            println!("FAILED");
            Ok(1)
        }
    }
}

fn probe(config: &config::Config) -> Result<()> {
    let executable = browser::find(config)?;
    let profile = browser::prepare(config)?;
    let deadline = Some(Instant::now() + Duration::from_secs(60));
    let mut browser = browser::Browser::launch(&executable, &profile, deadline)?;
    let progress = Progress::quiet(deadline);
    let mut source = photos::Photos::open(&mut browser.cdp, &config.account, deadline, &progress)?;
    let page = source.albums(None)?;
    let mut albums = 0;
    let mut memberships = 0;
    let mut metadata_rows = 0;
    for album in page.items.iter().take(2) {
        albums += 1;
        let members = source.members(&album.id, None)?;
        if members.id.as_deref() != Some(&album.id) {
            return Err("ALBUM_ID_MISMATCH: 詳細応答が要求したアルバムと一致しません".into());
        }
        memberships += members.items.len();
        if let Some(member) = members.items.first() {
            let details = source.metadata(&album.id, &[&member.id])?;
            if details.len() > 1 || details.iter().any(|item| item.id != member.id) {
                return Err("METADATA_ID_MISMATCH: メタデータ応答の ID が一致しません".into());
            }
            metadata_rows += details
                .iter()
                .filter(|detail| {
                    detail
                        .file_name
                        .as_deref()
                        .is_some_and(|name| !name.is_empty())
                })
                .count();
        }
    }
    source.verify()?;
    println!("PROBE OK: 一覧の最初のページ {} アルバム、確認した {} アルバムの最初の所属ページ {} 項目、照合できたファイル名 {} 件",
        page.items.len(), albums, memberships, metadata_rows);
    Ok(())
}

fn main_result() -> Result<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let command = args.first().and_then(|a| a.to_str()).unwrap_or("--help");
    if matches!(command, "--help" | "-h") {
        println!(
            "GPAE {} | gpae setup|run|check|probe [--config PATH] | gpae run --all [--config PATH]",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(0);
    }
    if !matches!(command, "setup" | "run" | "check" | "probe") {
        return Err("gpae setup|run|check|probe [--config PATH] を指定してください".into());
    }
    let (path, all) = match args.as_slice() {
        [_] => (Path::new("config.json"), false),
        [_, flag] if command == "run" && flag == "--all" => (Path::new("config.json"), true),
        [_, flag, path] if flag == "--config" => (Path::new(path), false),
        [_, all, flag, path] if command == "run" && flag == "--config" && all == "--all" => {
            (Path::new(path), true)
        }
        [_, flag, path, all] if command == "run" && flag == "--config" && all == "--all" => {
            (Path::new(path), true)
        }
        _ => {
            return Err(
                "--config PATH の形式で指定してください。run は --all も指定できます".into(),
            )
        }
    };
    let config = config::load(path)?;
    match command {
        "run" => run(&config, all),
        "probe" => {
            probe(&config)?;
            Ok(0)
        }
        "setup" => {
            let executable = browser::find(&config)?;
            config::check_directories(&config.output_dir)
                .map_err(|e| format!("output_dir: {e}"))?;
            let profile = browser::prepare(&config)?;
            browser::setup(&executable, &profile)?;
            println!("専用ウィンドウで指定アカウントにログインしてください。右上の「…」メニューから「終了」を選んでブラウザを閉じ、RUN.bat を実行します。");
            Ok(0)
        }
        _ => {
            let executable = browser::find(&config)?;
            config::check_directories(&config.output_dir)
                .map_err(|e| format!("output_dir: {e}"))?;
            config::check_directories(&config.profile_dir)
                .map_err(|e| format!("profile_dir: {e}"))?;
            println!("ブラウザ: {}", executable.display());
            println!("保存先: {}", config.output_dir.display());
            println!("専用プロファイル: {}", config.profile_dir.display());
            Ok(0)
        }
    }
}

fn main() {
    match main_result() {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("FAILED: {e}");
            std::process::exit(1);
        }
    }
}
