use crate::{control::Control, Result};
use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum AlbumResult {
    Saved,
    Reused,
    Skipped,
}

pub enum Event {
    Phase(&'static str),
    Listed(usize),
    AlbumTotal(usize),
    AlbumStart(Option<u64>),
    Media(u64, Option<u64>),
    AlbumDone(AlbumResult),
    Waiting(&'static str),
    Activity(&'static str),
    Paused(bool),
    Renewing(bool),
}

#[derive(Default)]
pub struct Progress {
    sender: Option<Sender<Event>>,
    pub control: Control,
}

impl Progress {
    pub fn quiet(deadline: Option<Instant>) -> Self {
        Self {
            sender: None,
            control: Control::new(deadline),
        }
    }

    pub fn track<T>(
        started: Instant,
        deadline: Option<Instant>,
        work: impl FnOnce(&Self) -> Result<T>,
    ) -> Result<T> {
        thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            thread::Builder::new().spawn_scoped(scope, move || {
                display(receiver, started, Duration::from_secs(10), |line| {
                    let _ = writeln!(io::stdout().lock(), "{line}");
                });
            })?;
            work(&Self {
                sender: Some(sender),
                control: Control::new(deadline),
            })
        })
    }

    pub fn emit(&self, event: Event) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(event);
        }
    }
}

struct State {
    started: Instant,
    updated: Instant,
    phase: &'static str,
    detail: &'static str,
    listed: Option<usize>,
    total: Option<usize>,
    saved: usize,
    skipped: usize,
    album_started: Option<Instant>,
    media: u64,
    media_total: Option<u64>,
    samples: usize,
    sample_time: Duration,
    paused_at: Option<Instant>,
    section: usize,
    renewing: bool,
}

impl State {
    fn new(started: Instant) -> Self {
        Self {
            started,
            updated: started,
            phase: "専用ブラウザを準備中",
            detail: "",
            listed: None,
            total: None,
            saved: 0,
            skipped: 0,
            album_started: None,
            media: 0,
            media_total: None,
            samples: 0,
            sample_time: Duration::ZERO,
            paused_at: None,
            section: 1,
            renewing: false,
        }
    }

    fn update(&mut self, event: Event, now: Instant) -> bool {
        match event {
            Event::Waiting(detail) => {
                self.detail = detail;
                return false;
            }
            Event::Phase(phase) => {
                self.phase = phase;
                self.detail = "";
                self.album_started = None;
                self.paused_at = None;
                self.renewing = false;
            }
            Event::Listed(count) => self.listed = Some(count),
            Event::AlbumTotal(total) => {
                self.phase = "アルバムを取得中";
                self.detail = "";
                self.total = Some(total);
            }
            Event::AlbumStart(total) => {
                self.album_started = Some(now);
                self.media = 0;
                self.media_total = total;
                self.detail = "";
            }
            Event::Media(count, total) => {
                self.media = count;
                self.media_total = total;
            }
            Event::AlbumDone(result) => {
                let album_started = self.album_started.take();
                if matches!(result, AlbumResult::Skipped) {
                    self.skipped += 1;
                } else {
                    self.saved += 1;
                }
                if let (AlbumResult::Saved, Some(started)) = (result, album_started) {
                    self.samples += 1;
                    self.sample_time += now.duration_since(started);
                }
                self.detail = "";
            }
            Event::Activity(detail) => self.detail = detail,
            Event::Paused(true) => self.paused_at = Some(now),
            Event::Paused(false) => {
                if let (Some(started), Some(paused)) =
                    (&mut self.album_started, self.paused_at.take())
                {
                    *started += now.duration_since(paused);
                }
                self.paused_at = None;
            }
            Event::Renewing(active) => {
                self.renewing = active;
                self.detail = "";
                if !active {
                    self.section += 1;
                }
            }
        }
        self.updated = now;
        matches!(
            event,
            Event::Phase(_) | Event::AlbumTotal(_) | Event::Paused(_) | Event::Renewing(_)
        )
    }

    fn line(&self, now: Instant) -> String {
        let mut line = format!(
            "[経過 {}] {}",
            clock(now.duration_since(self.started)),
            if self.paused_at.is_some() {
                "一時停止中（R + Enter で再開、Q + Enter で中止）"
            } else if self.renewing {
                "次の区間に向けて専用ブラウザを切り替え中"
            } else {
                self.phase
            }
        );
        if let Some(total) = self.total {
            let _ = write!(line, " | 区間 {}", self.section);
            let _ = write!(line, " | アルバム {}/{total} 保存", self.saved);
            if self.skipped != 0 {
                let _ = write!(line, "・{}件スキップ", self.skipped);
            }
            if let Some(started) = self.album_started {
                let current = self.saved + self.skipped + 1;
                let _ = write!(line, " | {current}件目 写真・動画 {}", self.media);
                if let Some(media_total) = self.media_total {
                    let _ = write!(line, "/{media_total}");
                }
                line.push_str(" 件確認");
                if self.samples >= 3 && self.paused_at.is_none() {
                    let remaining = total.saturating_sub(self.saved + self.skipped) as f64;
                    let average = self.sample_time.as_secs_f64() / self.samples as f64;
                    let seconds = average * remaining - now.duration_since(started).as_secs_f64();
                    if seconds > 0.0 {
                        let _ = write!(line, " | 取得の残り目安 約{}分", (seconds / 60.0).ceil());
                    } else {
                        line.push_str(" | 残り時間を再計算中");
                    }
                }
            }
        } else if let Some(listed) = self.listed {
            let _ = write!(line, " | {listed}アルバム確認");
        }
        if !self.detail.is_empty() && self.paused_at.is_none() {
            let _ = write!(line, " | {}", self.detail);
        }
        let since_update = now.duration_since(self.updated).as_secs();
        if since_update >= 10 && self.paused_at.is_none() {
            let _ = write!(line, " | 最後の更新から{since_update}秒");
        }
        line
    }
}

fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn display(
    receiver: Receiver<Event>,
    started: Instant,
    interval: Duration,
    mut output: impl FnMut(String),
) {
    let mut state = State::new(started);
    let mut refresh = started + interval;
    loop {
        let event = receiver.recv_timeout(refresh.saturating_duration_since(Instant::now()));
        let now = Instant::now();
        let immediate = match event {
            Ok(event) => state.update(event, now),
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if immediate || now >= refresh {
            output(state.line(now));
            refresh = now + interval;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_the_display_while_waiting_for_a_response() {
        let (events, inbox) = mpsc::channel();
        let (lines, output) = mpsc::channel();
        events.send(Event::AlbumTotal(4)).unwrap();
        events.send(Event::AlbumStart(Some(80))).unwrap();
        events.send(Event::Media(40, Some(80))).unwrap();
        events.send(Event::Waiting("個別照会の応答待ち")).unwrap();
        let renderer = thread::spawn(move || {
            display(inbox, Instant::now(), Duration::from_millis(20), |line| {
                let _ = lines.send(line);
            });
        });
        let wait = Duration::from_secs(2);
        assert!(output.recv_timeout(wait).unwrap().contains("0/4 保存"));
        for _ in 0..2 {
            let line = output.recv_timeout(wait).unwrap();
            assert!(line.contains("40/80 件確認"));
            assert!(line.contains("個別照会の応答待ち"));
        }
        drop(events);
        renderer.join().unwrap();
    }

    #[test]
    fn estimates_from_new_albums_and_reports_skipped_and_stale_work() {
        let started = Instant::now();
        let mut state = State::new(started);
        state.update(Event::AlbumTotal(8), started);
        for (index, result) in [
            AlbumResult::Reused,
            AlbumResult::Skipped,
            AlbumResult::Saved,
            AlbumResult::Saved,
            AlbumResult::Saved,
        ]
        .into_iter()
        .enumerate()
        {
            let at = started + Duration::from_secs(60 * index as u64);
            state.update(Event::AlbumStart(None), at);
            assert!(!state.line(at).contains("残り目安"));
            state.update(Event::AlbumDone(result), at + Duration::from_secs(60));
        }
        let at = started + Duration::from_secs(300);
        state.update(Event::AlbumStart(None), at);
        let now = at + Duration::from_secs(12);
        state.update(Event::Waiting("再試行待ち"), now);
        let line = state.line(now);
        assert!(line.contains("4/8 保存・1件スキップ"));
        assert!(line.contains("6件目 写真・動画 0 件確認"));
        assert!(line.contains("残り目安 約3分"));
        assert!(line.contains("最後の更新から12秒"));
        state.update(Event::Phase("結果をファイルに保存中"), now);
        assert!(!state.line(now).contains("6件目"));
    }
}
