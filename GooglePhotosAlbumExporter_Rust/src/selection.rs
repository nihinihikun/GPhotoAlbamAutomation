use crate::photos::Album;
use crate::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub all: bool,
    indices: Vec<usize>,
}

impl Selection {
    pub fn all() -> Self {
        Self {
            all: true,
            indices: Vec::new(),
        }
    }

    pub fn albums<'a>(&self, albums: &'a [Album]) -> Result<Vec<&'a Album>> {
        if self.all && self.indices.is_empty() {
            return Ok(albums.iter().collect());
        }
        if self.all || self.indices.is_empty() {
            return Err("アルバムの選択結果が不正です".into());
        }
        let mut indices = self.indices.clone();
        indices.sort_unstable();
        if indices.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("アルバムの選択結果に重複があります".into());
        }
        indices
            .into_iter()
            .map(|index| {
                albums
                    .get(index)
                    .ok_or_else(|| "選択したアルバムが一覧にありません".into())
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("アルバムの選択をキャンセルしました")
    }
}

impl std::error::Error for Cancelled {}

#[cfg(windows)]
pub fn choose(albums: &[Album]) -> Result<Selection> {
    use serde::Serialize;
    use std::fs;
    use std::process::Command;

    #[derive(Serialize)]
    struct Choice<'a> {
        title: Option<&'a str>,
        reported_count: Option<u64>,
    }

    let files = tempfile::tempdir()?;
    let script = files.path().join("select-albums.ps1");
    let input = files.path().join("albums.json");
    let output = files.path().join("selection.json");
    fs::write(&script, include_bytes!("../select-albums.ps1"))?;
    let choices: Vec<_> = albums
        .iter()
        .map(|album| Choice {
            title: album.title.as_deref(),
            reported_count: album.reported_count,
        })
        .collect();
    fs::write(&input, serde_json::to_vec(&choices)?)?;
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-STA",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-InputFile")
        .arg(input)
        .arg("-OutputFile")
        .arg(&output)
        .status()?;
    if !status.success() {
        return Err(format!("アルバムの選択画面を開けませんでした ({status})").into());
    }
    serde_json::from_slice::<Option<Selection>>(&fs::read(output)?)?.ok_or_else(|| Cancelled.into())
}

#[cfg(not(windows))]
pub fn choose(_: &[Album]) -> Result<Selection> {
    Err("アルバムの選択画面は Windows で利用できます".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_indices_and_selects_distinct_same_title_albums() {
        let albums: Vec<Album> = serde_json::from_str(
            r#"[{"id":"a","title":"旅行"},{"id":"b","title":"旅行"},{"id":"c","title":"空"}]"#,
        )
        .unwrap();
        let select = |json| -> Result<Vec<&str>> {
            let selection: Selection = serde_json::from_str(json)?;
            Ok(selection
                .albums(&albums)?
                .into_iter()
                .map(|album| album.id.as_str())
                .collect())
        };
        assert_eq!(
            select(r#"{"all":true,"indices":[]}"#).unwrap(),
            ["a", "b", "c"]
        );
        assert_eq!(select(r#"{"all":false,"indices":[1]}"#).unwrap(), ["b"]);
        for invalid in [
            r#"{"all":false,"indices":[]}"#,
            r#"{"all":false,"indices":[1,1]}"#,
            r#"{"all":false,"indices":[3]}"#,
            r#"{"all":true,"indices":[0]}"#,
        ] {
            assert!(select(invalid).is_err());
        }
    }
}
