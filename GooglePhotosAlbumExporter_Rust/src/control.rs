use crate::progress::{Event, Progress};
use crate::Result;
use std::fmt;
use std::io::{self, IsTerminal};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ユーザーの操作で書き出しを中止しました")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Running,
    Paused,
    Cancelled,
    Finished,
}

#[derive(Default)]
struct State {
    mode: Mode,
    deadline: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct Control(Arc<(Mutex<State>, Condvar)>);

impl Control {
    pub fn exclude_wait(&self, elapsed: Duration) {
        let (lock, _) = &*self.0;
        let mut state = lock.lock().unwrap();
        state.deadline = state.deadline.map(|deadline| deadline + elapsed);
    }

    pub fn new(deadline: Option<Instant>) -> Self {
        Self(Arc::new((
            Mutex::new(State {
                deadline,
                ..State::default()
            }),
            Condvar::new(),
        )))
    }

    pub fn start_keyboard(&self) {
        if !io::stdin().is_terminal() {
            return;
        }
        let control = self.clone();
        match thread::Builder::new().spawn(move || {
            let stdin = io::stdin();
            let mut line = String::new();
            loop {
                line.clear();
                if !matches!(stdin.read_line(&mut line), Ok(n) if n > 0) {
                    // Release a pause if the input stream is closed.
                    control.set_mode(Mode::Running);
                    return;
                }
                let mode = match line.trim().to_ascii_lowercase().as_str() {
                    "p" => Mode::Paused,
                    "r" => Mode::Running,
                    "q" => Mode::Cancelled,
                    _ => continue,
                };
                if control.set_mode(mode) {
                    match mode {
                        Mode::Paused => println!("一時停止を受け付けました。処理中の通信が終わり次第、停止します。"),
                        Mode::Cancelled => {
                            println!("中止を受け付けました。処理中の通信が終わり次第、結果を保存します。");
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }) {
            Ok(_) => println!("P + Enter で一時停止、R + Enter で再開、Q + Enter で中止"),
            Err(e) => eprintln!("操作キーを有効にできませんでした ({e})"),
        }
    }

    fn set_mode(&self, mode: Mode) -> bool {
        let (lock, changed) = &*self.0;
        let mut state = lock.lock().unwrap();
        if matches!(state.mode, Mode::Cancelled | Mode::Finished) || state.mode == mode {
            return false;
        }
        state.mode = mode;
        changed.notify_all();
        true
    }

    pub fn finish(&self) -> bool {
        let (lock, _) = &*self.0;
        let mut state = lock.lock().unwrap();
        let cancelled = state.mode == Mode::Cancelled;
        state.mode = Mode::Finished;
        cancelled
    }

    pub fn checkpoint(&self, progress: &Progress) -> Result<Option<Instant>> {
        let (lock, changed) = &*self.0;
        let mut state = lock.lock().unwrap();
        if state.mode == Mode::Paused {
            let paused = Instant::now();
            progress.emit(Event::Paused(true));
            state = changed
                .wait_while(state, |state| state.mode == Mode::Paused)
                .unwrap();
            state.deadline = state.deadline.map(|deadline| deadline + paused.elapsed());
            if state.mode != Mode::Cancelled {
                progress.emit(Event::Paused(false));
            }
        }
        if state.mode == Mode::Cancelled {
            return Err(Cancelled.into());
        }
        if state
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err("全体の制限時間を超えました".into());
        }
        Ok(state.deadline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn pauses_and_resumes_at_the_checkpoint_and_can_cancel_while_paused() {
        let deadline = Instant::now() + Duration::from_secs(5);
        let control = Control::new(Some(deadline));
        for mode in [Mode::Running, Mode::Cancelled] {
            assert!(control.set_mode(Mode::Paused));
            let (done, finished) = mpsc::channel();
            let worker = control.clone();
            let handle = thread::spawn(move || {
                let progress = Progress::default();
                let result = worker
                    .checkpoint(&progress)
                    .map_err(|e| e.is::<Cancelled>());
                done.send(result).unwrap();
            });
            let blocked = finished.recv_timeout(Duration::from_millis(30));
            control.set_mode(mode);
            assert!(matches!(blocked, Err(mpsc::RecvTimeoutError::Timeout)));
            let result = finished.recv_timeout(Duration::from_secs(2)).unwrap();
            if mode == Mode::Running {
                assert!(result.unwrap().unwrap() > deadline);
            } else {
                assert_eq!(result, Err(true));
            }
            handle.join().unwrap();
        }
        assert!(control.finish());
    }
}
