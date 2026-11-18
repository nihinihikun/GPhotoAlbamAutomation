use super::*;
use crate::photos::{AlbumPage, MemberPage};
use serde_json::Value;

struct Fixture {
    total: Option<u64>,
    second_error: Option<&'static str>,
    denied: bool,
    wrong_account: bool,
    first_members: u64,
    second_has_first: bool,
    duplicate_media: bool,
    broken_cursor: bool,
    count_adjustment: i64,
    missing: bool,
    metadata_calls: usize,
    fail_metadata_at: Option<usize>,
    section_after_metadata: Option<usize>,
    section_renewals: usize,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            total: Some(2),
            second_error: None,
            denied: false,
            wrong_account: false,
            first_members: 3,
            second_has_first: false,
            duplicate_media: false,
            broken_cursor: false,
            count_adjustment: 0,
            missing: false,
            metadata_calls: 0,
            fail_metadata_at: None,
            section_after_metadata: None,
            section_renewals: 0,
        }
    }
}

impl Source for Fixture {
    fn section_due(&self) -> bool {
        self.section_after_metadata
            .is_some_and(|limit| self.metadata_calls >= limit)
    }

    fn renew_section(&mut self) -> Result<()> {
        self.section_renewals += 1;
        self.section_after_metadata = None;
        Ok(())
    }

    fn verify(&mut self) -> Result<()> {
        if self.wrong_account {
            Err("ACCOUNT_MISMATCH".into())
        } else {
            Ok(())
        }
    }

    fn albums(&mut self, next: Option<&str>) -> Result<AlbumPage> {
        let first = next.is_none();
        Ok(AlbumPage {
            items: vec![Album {
                id: if first { "a" } else { "b" }.to_owned(),
                title: Some("同じアルバム名".to_owned()),
                url: None,
                is_shared: Some(!first),
                owner_actor_id: None,
                reported_count: Some(if first {
                    self.first_members
                        .checked_add_signed(self.count_adjustment)
                        .unwrap()
                } else {
                    u64::from(self.second_has_first)
                }),
            }],
            total: self.total,
            next: if first || self.broken_cursor {
                Some("second".to_owned())
            } else {
                None
            },
        })
    }

    fn members(&mut self, id: &str, next: Option<&str>) -> Result<MemberPage> {
        if id == "b" {
            if self.denied {
                return Err(AlbumAccessDenied.into());
            }
            if let Some(error) = self.second_error {
                return Err(error.into());
            }
            return Ok(MemberPage {
                id: Some(id.to_owned()),
                reported_count: Some(u64::from(self.second_has_first)),
                items: if self.second_has_first {
                    vec![Member {
                        id: "photo-0".to_owned(),
                        taken_ms: Some(1_700_000_000_000),
                        creation_ms: None,
                        dedup_key: None,
                    }]
                } else {
                    vec![]
                },
                next: None,
            });
        }
        let start = next.map(str::parse::<u64>).transpose()?.unwrap_or_default();
        let end = (start + 250).min(self.first_members);
        Ok(MemberPage {
            id: Some(id.to_owned()),
            reported_count: Some(
                self.first_members
                    .checked_add_signed(self.count_adjustment)
                    .unwrap(),
            ),
            items: (start..end)
                .map(|index| Member {
                    id: format!("photo-{}", if self.duplicate_media { 0 } else { index }),
                    taken_ms: Some(1_700_000_000_000 + index as i64),
                    creation_ms: None,
                    dedup_key: None,
                })
                .collect(),
            next: (end < self.first_members).then(|| end.to_string()),
        })
    }

    fn metadata(&mut self, _album: &str, ids: &[&str]) -> Result<Vec<Metadata>> {
        assert!(ids.len() <= 40);
        self.metadata_calls += 1;
        if self.fail_metadata_at == Some(self.metadata_calls) {
            return Err("NETWORK_TIMEOUT".into());
        }
        Ok(ids
            .iter()
            .filter(|_| !self.missing)
            .map(|id| Metadata {
                id: (*id).to_owned(),
                file_name: Some("same.jpg".to_owned()),
                taken_ms: None,
                creation_ms: None,
                size_bytes: Some(123),
            })
            .collect())
    }
}

fn json_file(path: &Path) -> Value {
    serde_json::from_reader(File::open(path).unwrap()).unwrap()
}

#[test]
fn selected_export_skips_unselected_same_title_album_and_reports_scope() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = Fixture {
        total: None,
        duplicate_media: true,
        second_has_first: true,
        ..Fixture::default()
    };
    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    run.collect_selected(&mut source, &Progress::default(), |albums| {
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].title, albums[1].title);
        Ok(serde_json::from_str(r#"{"all":false,"indices":[1]}"#)?)
    })
    .unwrap();
    assert_eq!(source.metadata_calls, 1);
    let outcome = run.finish(None).unwrap();
    assert_eq!(outcome.status, Status::Success);
    let report = json_file(&outcome.report);
    assert_eq!(report["validation"]["album_count_enumerated"], 2);
    assert_eq!(report["validation"]["album_count_selected"], 1);
    assert_eq!(report["validation"]["selection_mode"], "selected");
    assert_eq!(report["validation"]["selected_completeness_verified"], true);
    assert_eq!(report["validation"]["overall_completeness_verified"], false);
    let data = json_file(&outcome.data.unwrap());
    assert_eq!(data["albums"].as_array().unwrap().len(), 1);
    assert_eq!(data["albums"][0]["album_id"], "b");
    let directory = outcome.album_directory.unwrap();
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    assert!(directory.join("同じアルバム名.jsonl").is_file());
}

#[test]
fn publishes_named_jsonl_files_for_distinct_album_ids_and_keeps_combined_json() {
    let directory = tempfile::tempdir().unwrap();
    let mut source = Fixture::default();
    let mut export = Export::new(directory.path(), "a@example.com").unwrap();
    export.collect(&mut source, &Progress::default()).unwrap();
    let outcome = export.finish(None).unwrap();
    let data = json_file(outcome.data.as_ref().unwrap());
    let report = json_file(&outcome.report);
    let folder = outcome.album_directory.as_ref().unwrap();
    assert_eq!(
        report["album_directory"],
        folder.file_name().unwrap().to_str().unwrap()
    );
    let files = report["album_files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    for (index, (id, name)) in [
        ("a", "同じアルバム名.jsonl"),
        ("b", "同じアルバム名 (2).jsonl"),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = &files[index];
        let path = directory.path().join(entry["file"].as_str().unwrap());
        assert_eq!(entry["album_id"], id);
        assert_eq!(entry["sha256"], hash_file(&path).unwrap());
        assert_eq!(fs::canonicalize(&path).unwrap(), folder.join(name));
        let mut rows = Vec::new();
        let (header, summary) = read_checkpoint(&path, |item| {
            rows.push(serde_json::to_value(item)?);
            Ok(())
        })
        .unwrap();
        assert_eq!(header.album.id, id);
        assert_eq!(summary.scan.count as usize, rows.len());
        assert_eq!(data["albums"][index]["items"], json!(rows));
    }
}

#[test]
fn keeps_existing_report_and_rolls_back_new_files_if_report_cannot_be_published() {
    let directory = tempfile::tempdir().unwrap();
    let mut export = Export::new(directory.path(), "a@example.com").unwrap();
    export
        .collect(&mut Fixture::default(), &Progress::default())
        .unwrap();
    let expected_report = directory
        .path()
        .join(format!("{}.report.json", export.base));
    fs::write(&expected_report, b"keep").unwrap();
    let expected_data = directory.path().join(format!("{}.json", export.base));
    let expected_folder = directory.path().join(&export.base);
    assert!(export.finish(None).is_err());
    assert_eq!(fs::read(&expected_report).unwrap(), b"keep");
    assert!(!expected_data.exists());
    assert!(!expected_folder.exists());
}

#[test]
fn keeps_valid_album_titles_and_makes_windows_reserved_names_safe() {
    for (title, expected) in [
        ("旅行 2026 🌸", "旅行 2026 🌸"),
        ("../A:B?C*", ".._A_B_C_"),
        ("CON.txt", "_CON.txt"),
        ("LPT².旅行", "_LPT².旅行"),
        ("家族 . ", "家族"),
        (" . ", "無題のアルバム"),
    ] {
        assert_eq!(album_filename_stem(Some(title)), expected);
    }
    assert_eq!(album_filename_stem(None), "無題のアルバム");
    let reserved_after_trim = format!("COM1{}x", " ".repeat(250));
    assert_eq!(album_filename_stem(Some(&reserved_after_trim)), "_COM1");
    let long_title = format!("a{}", "🌸".repeat(240));
    let stem = album_filename_stem(Some(&long_title));
    assert!(long_title.starts_with(&stem));
    let units = if cfg!(windows) {
        stem.encode_utf16().count()
    } else {
        stem.len()
    };
    assert_eq!(units, ALBUM_STEM_LIMIT);
    #[cfg(windows)]
    assert_eq!(
        album_filename_stem(Some(&"旅".repeat(100))),
        "旅".repeat(100)
    );
}

#[test]
fn rotates_between_batches_and_reuses_details_from_the_previous_section() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = Fixture {
        first_members: 85,
        section_after_metadata: Some(1),
        ..Fixture::default()
    };
    let mut export = Export::new(dir.path(), "a@example.com").unwrap();
    export.collect(&mut fixture, &Progress::default()).unwrap();
    assert_eq!(fixture.section_renewals, 1);
    assert_eq!(fixture.metadata_calls, 3);
    let finished = export.finish(None).unwrap();
    assert_eq!(finished.status, Status::Success);
    assert_eq!(
        json_file(&finished.data.unwrap())["albums"][0]["items"]
            .as_array()
            .unwrap()
            .len(),
        85
    );
}

#[test]
fn retries_only_unfinished_batches_inside_an_incomplete_album() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = Fixture {
        first_members: 85,
        fail_metadata_at: Some(2),
        ..Fixture::default()
    };
    let progress = Progress::default();
    let mut export = Export::new(dir.path(), "a@example.com").unwrap();
    let error = export.collect(&mut fixture, &progress).unwrap_err();
    assert!(error.to_string().contains("NETWORK_TIMEOUT"));
    assert_eq!(
        export.finish(Some(&error.to_string())).unwrap().status,
        Status::Failed
    );

    fixture.fail_metadata_at = None;
    fixture.metadata_calls = 0;
    let mut export = Export::new(dir.path(), "a@example.com").unwrap();
    export.collect(&mut fixture, &progress).unwrap();
    assert_eq!(fixture.metadata_calls, 2);
    let finished = export.finish(None).unwrap();
    assert_eq!(finished.status, Status::Success);
    let data = json_file(&finished.data.unwrap());
    assert_eq!(data["albums"][0]["items"].as_array().unwrap().len(), 85);
    assert!(!dir.path().read_dir().unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".gpae-checkpoints-")
    }));
}

#[test]
fn resumes_only_completed_album_without_duplicate_members_and_keeps_hash() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = Fixture {
        first_members: 503,
        second_error: Some("NETWORK_TIMEOUT"),
        ..Fixture::default()
    };
    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    let error = run
        .collect(&mut fixture, &Progress::default())
        .unwrap_err()
        .to_string();
    let partial = run.finish(Some(&error)).unwrap();
    assert_eq!(partial.status, Status::Partial);
    assert_eq!(
        json_file(&partial.report)["validation"]["album_count_saved"],
        1
    );
    let old_file = partial.data.unwrap();
    let old_hash = hash_file(&old_file).unwrap();

    let mut fixture = Fixture {
        first_members: 503,
        ..Fixture::default()
    };
    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    run.collect(&mut fixture, &Progress::default()).unwrap();
    assert_eq!(fixture.metadata_calls, 0);
    let finished = run.finish(None).unwrap();
    assert_eq!(finished.status, Status::Success);
    let report = json_file(&finished.report);
    assert_eq!(report["validation"]["resumed_albums"], 1);
    let path = finished.data.unwrap();
    assert_eq!(
        hash_file(&path).unwrap(),
        report["sha256"].as_str().unwrap()
    );
    let data = json_file(&path);
    assert_eq!(data["albums"][0]["title"], data["albums"][1]["title"]);
    assert_eq!(data["albums"][0]["items"][0]["file_name"], "same.jpg");
    assert_eq!(data["albums"][0]["items"][502]["position"], 503);
    assert_eq!(data["albums"][1]["items"].as_array().unwrap().len(), 0);
    assert_eq!(hash_file(&old_file).unwrap(), old_hash);
}

#[test]
fn unverified_total_missing_metadata_and_denied_shared_album_are_partial() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = Fixture {
        total: None,
        missing: true,
        denied: true,
        ..Fixture::default()
    };
    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    run.collect(&mut source, &Progress::default()).unwrap();
    let outcome = run.finish(None).unwrap();
    assert_eq!(outcome.status, Status::Partial);
    let report = json_file(&outcome.report);
    assert!(report["validation"]["album_count_reported"].is_null());
    assert_eq!(report["validation"]["missing_filename_count"], 3);
    assert_eq!(
        report["validation"]["access_denied_album_ids"],
        json!(["b"])
    );
    assert!(json_file(&outcome.data.unwrap())["albums"][0]["items"][0]["file_name"].is_null());
}

#[test]
fn saves_a_count_shortfall_continues_and_reuses_details_when_the_missing_item_appears() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = Fixture {
        first_members: 85,
        count_adjustment: 1,
        second_has_first: true,
        ..Fixture::default()
    };
    let progress = Progress::default();
    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    run.collect(&mut source, &progress).unwrap();
    let outcome = run.finish(None).unwrap();
    assert_eq!(outcome.status, Status::Partial);
    let report = json_file(&outcome.report);
    assert_eq!(report["validation"]["album_count_saved"], 2);
    assert_eq!(
        report["validation"]["albums_with_unverified_member_count"],
        1
    );
    assert_eq!(
        report["validation"]["album_member_count_mismatches"],
        json!([{"album_id":"a", "reported_count":86, "membership_count":85}])
    );
    let data = json_file(&outcome.data.unwrap());
    assert_eq!(data["albums"][0]["items"].as_array().unwrap().len(), 85);
    assert_eq!(data["albums"][0]["membership_count_verified"], false);
    assert_eq!(data["albums"][1]["membership_count_verified"], true);

    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    source.metadata_calls = 0;
    run.collect(&mut source, &progress).unwrap();
    assert_eq!(source.metadata_calls, 0);
    assert_eq!(run.finish(None).unwrap().status, Status::Partial);

    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    source.first_members = 86;
    source.count_adjustment = 0;
    run.collect(&mut source, &progress).unwrap();
    assert_eq!(source.metadata_calls, 1);
    let outcome = run.finish(None).unwrap();
    assert_eq!(outcome.status, Status::Success);
    let data = json_file(&outcome.data.unwrap());
    assert_eq!(data["albums"][0]["items"].as_array().unwrap().len(), 86);
    assert_eq!(data["albums"][0]["membership_count_verified"], true);
}

#[test]
fn account_cursor_and_count_errors_fail_before_final_album_publication() {
    for (mut source, expected) in [
        (
            Fixture {
                wrong_account: true,
                ..Fixture::default()
            },
            "ACCOUNT_MISMATCH",
        ),
        (
            Fixture {
                broken_cursor: true,
                ..Fixture::default()
            },
            "PAGINATION_ERROR",
        ),
        (
            Fixture {
                count_adjustment: -1,
                ..Fixture::default()
            },
            "COUNT_MISMATCH",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut run = Export::new(dir.path(), "a@example.com").unwrap();
        let error = run
            .collect(&mut source, &Progress::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
        let outcome = run.finish(Some(&error)).unwrap();
        assert_eq!(outcome.status, Status::Failed);
        assert!(outcome.data.is_none());
    }
}

#[test]
fn output_error_and_lock_conflict_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    fs::write(&path, "keep").unwrap();
    assert!(Export::new(&path, "a@example.com")
        .err()
        .unwrap()
        .to_string()
        .contains("output_dir"));
    assert_eq!(fs::read_to_string(path).unwrap(), "keep");
    let first = Export::new(dir.path(), "a@example.com").unwrap();
    assert!(Export::new(dir.path(), "a@example.com").is_err());
    drop(first);
    assert!(Export::new(dir.path(), "a@example.com").is_ok());
}

#[test]
fn repeated_media_is_an_error_only_inside_one_album() {
    let directory = tempfile::tempdir().unwrap();
    let mut source = Fixture {
        second_has_first: true,
        ..Fixture::default()
    };
    let mut run = Export::new(directory.path(), "a@example.com").unwrap();
    run.collect(&mut source, &Progress::default()).unwrap();
    let outcome = run.finish(None).unwrap();
    let data = json_file(&outcome.data.unwrap());
    assert_eq!(
        data["albums"][0]["items"][0]["id"],
        data["albums"][1]["items"][0]["id"]
    );
    assert_eq!(data["validation"]["membership_count_saved"], 4);

    let mut source = Fixture {
        duplicate_media: true,
        ..Fixture::default()
    };
    let mut run = Export::new(directory.path(), "a@example.com").unwrap();
    assert!(run
        .collect(&mut source, &Progress::default())
        .unwrap_err()
        .to_string()
        .contains("DUPLICATE_MEDIA"));
}

#[test]
fn streams_twenty_thousand_members_in_bounded_pages_and_metadata_batches() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = Fixture {
        first_members: 20_000,
        ..Fixture::default()
    };
    let mut run = Export::new(dir.path(), "a@example.com").unwrap();
    run.collect(&mut source, &Progress::default()).unwrap();
    let outcome = run.finish(None).unwrap();
    assert_eq!(outcome.status, Status::Success);
    assert_eq!(
        json_file(&outcome.report)["validation"]["membership_count_saved"],
        20_000
    );
    assert_eq!(source.metadata_calls, 560);
    assert!(fs::metadata(outcome.data.unwrap()).unwrap().len() > 1_000_000);
}
