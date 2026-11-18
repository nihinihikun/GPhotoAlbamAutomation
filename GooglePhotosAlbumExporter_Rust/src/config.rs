use crate::Result;
use serde::Deserialize;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Deserialize)]
pub struct Config {
    #[serde(alias = "google_account")]
    pub account: String,
    #[serde(alias = "output_directory")]
    pub output_dir: PathBuf,
    #[serde(default = "default_profile")]
    pub profile_dir: PathBuf,
    #[serde(default, alias = "chrome_path")]
    pub browser_path: Option<PathBuf>,
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(skip)]
    pub(crate) default_profile: PathBuf,
}

fn default_profile() -> PathBuf {
    PathBuf::from("chrome-profile")
}

pub fn valid_account(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && email.len() <= 254
        && local
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._%+-".contains(&c))
        && domain
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
}

pub fn load(path: &Path) -> Result<Config> {
    let path = std::env::current_dir()?.join(path);
    let data = fs::read(&path)
        .map_err(|e| format!("config.json ({}) を読み込めません: {e}", path.display()))?;
    let data = data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&data);
    let mut config: Config = serde_json::from_slice(data).map_err(|e| {
        format!(
            "config.json の JSON が不正です (行 {}, 列 {})",
            e.line(),
            e.column()
        )
    })?;
    config.account = config.account.trim().to_ascii_lowercase();
    if !valid_account(&config.account) || config.account == "your_old_account@gmail.com" {
        return Err(
            "config.json の account に対象の Google メールアドレスを入力してください".into(),
        );
    }
    if config.timeout_seconds != 0 && !(30..=86400).contains(&config.timeout_seconds) {
        return Err("timeout_seconds は 0（制限なし）または 30～86400 秒を指定してください".into());
    }
    let parent = path
        .parent()
        .ok_or("config.json の親フォルダがありません")?;
    config.default_profile = parent.join(default_profile());
    config.output_dir = resolve_path(parent, &config.output_dir, "output_dir")?;
    config.profile_dir = resolve_path(parent, &config.profile_dir, "profile_dir")?;
    config.browser_path = config
        .browser_path
        .map(|p| resolve_path(parent, &p, "browser_path"))
        .transpose()?;
    Ok(config)
}

fn resolve_path(base: &Path, path: &Path, field: &str) -> Result<PathBuf> {
    let text = path
        .as_os_str()
        .to_str()
        .ok_or("設定パスが Unicode ではありません")?;
    if text.trim().is_empty() || text.chars().any(|c| c.is_control() || c == '"') {
        return Err(format!(
            "config.json の {field} が不正です。Windows パスは D:/GooglePhotosAlbumBackup のように / で区切れます"
        ).into());
    }
    #[cfg(windows)]
    if let Some(Component::Prefix(prefix)) = path.components().next() {
        use std::path::Prefix;
        if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _)) {
            return Err(format!("config.json の {field} には通常のドライブまたは共有フォルダのパスを指定してください").into());
        }
    }
    #[cfg(windows)]
    if path.components().any(|component| match component {
        Component::Normal(name) => name.to_str().is_some_and(|name| {
            name.ends_with(['.', ' ']) || name.contains(['<', '>', ':', '|', '?', '*'])
        }),
        _ => false,
    }) {
        return Err(format!("config.json の {field} に Windows で使えない名前があります").into());
    }
    if path.components().any(|c| c == Component::ParentDir)
        || (!path.is_absolute()
            && (path.has_root() || matches!(path.components().next(), Some(Component::Prefix(_)))))
    {
        return Err(format!(
            "config.json の {field} に絶対パスか設定ファイルからの相対パスを指定してください"
        )
        .into());
    }
    Ok(base.join(path))
}

pub fn reject_link(metadata: &fs::Metadata, path: &Path) -> Result<()> {
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if reparse || metadata.file_type().is_symlink() {
        return Err(format!(
            "リンクやジャンクションを含むパスは使用できません: {}",
            path.display()
        )
        .into());
    }
    Ok(())
}

pub fn check_directories(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                reject_link(&metadata, ancestor)?;
                if !metadata.is_dir() {
                    return Err(format!("フォルダではありません: {}", ancestor.display()).into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", ancestor.display()).into()),
        }
    }
    Ok(())
}

pub fn create_directory(path: &Path) -> Result<()> {
    check_directories(path)?;
    fs::create_dir_all(path)
        .map_err(|e| format!("フォルダを作成できません ({}): {e}", path.display()))?;
    check_directories(path)
}

pub fn resolved_directory(path: &Path) -> Result<PathBuf> {
    for ancestor in path.ancestors() {
        match fs::canonicalize(ancestor) {
            Ok(base) => return Ok(base.join(path.strip_prefix(ancestor)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", ancestor.display()).into()),
        }
    }
    Err(format!("パスを解決できません: {}", path.display()).into())
}

pub fn check_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_link(&metadata, path)?;
            if !metadata.is_file() {
                return Err(format!("通常のファイルではありません: {}", path.display()).into());
            }
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn lock_file(path: &Path) -> Result<fs::File> {
    check_file(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|e| {
        format!(
            "同じ保存先または専用プロファイルで別の処理が実行中です ({}): {e}",
            path.display()
        )
    })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_report_embedded_json_tab_and_preserve_forward_slashes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        fs::write(
            &path,
            br#"{"account":"a@example.com","output_dir":"backup\test"}"#,
        )
        .unwrap();
        assert!(load(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("output_dir"));
        fs::write(
            &path,
            br#"{"google_account":"a@example.com","output_directory":"backup/test"}"#,
        )
        .unwrap();
        assert_eq!(
            load(&path).unwrap().output_dir,
            directory.path().join("backup/test")
        );
    }
}
