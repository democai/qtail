use clap::Parser;
use crossterm::event::{self, Event as CEvent, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

type Buffer = Arc<Mutex<VecDeque<String>>>;

#[derive(Parser, Debug)]
#[command(name = "qtail", about = "Quiet tail with on-demand output")]
struct Args {
    #[arg(short = 'p', long, default_value = "")]
    pattern: String,
    #[arg(short = 'n', long, default_value_t = 20)]
    lines: usize,
    #[arg(long, default_value_t = false)]
    stdout: bool,
}

#[derive(Debug)]
enum Event {
    Match(String),
    Eof,
}

fn main() {
    let args = Args::parse();
    let capacity = args.lines.max(1);
    let pattern_lower = args.pattern.to_lowercase();
    let output_target = OutputTarget::from_stdout_flag(args.stdout);
    let output_is_tty = output_target.is_terminal();

    let buffer: Buffer = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
    let (tx, rx) = mpsc::channel::<Event>();

    let reader_buffer = Arc::clone(&buffer);
    let reader_handle = thread::spawn(move || {
        reader_loop(
            io::stdin().lock(),
            reader_buffer,
            capacity,
            &pattern_lower,
            tx,
        );
    });

    let tty_file = File::open("/dev/tty").ok();
    let keyboard_enabled = tty_file.is_some();
    let mut raw_mode_enabled = false;
    if keyboard_enabled && enable_raw_mode().is_ok() {
        raw_mode_enabled = true;
    }

    let mut saw_eof = false;
    while !saw_eof {
        drain_events(
            &rx,
            &buffer,
            output_target,
            output_is_tty,
            raw_mode_enabled,
            &mut saw_eof,
        );
        if saw_eof {
            break;
        }

        if keyboard_enabled {
            if let Ok(true) = event::poll(Duration::from_millis(100))
                && let Ok(CEvent::Key(key_event)) = event::read()
            {
                if is_ctrl_c(&key_event) {
                    dump_buffer(&buffer, DumpKind::Exit, output_target, raw_mode_enabled);
                    break;
                }
                if key_event.code == KeyCode::Char(' ') {
                    dump_buffer(&buffer, DumpKind::Refresh, output_target, raw_mode_enabled);
                }
            }
        } else {
            thread::sleep(Duration::from_millis(100));
        }
    }

    if raw_mode_enabled {
        let _ = disable_raw_mode();
    }

    let _ = reader_handle.join();
}

fn reader_loop<R: BufRead>(
    mut reader: R,
    buffer: Buffer,
    capacity: usize,
    pattern_lower: &str,
    tx: Sender<Event>,
) {
    let mut raw_line = Vec::new();
    loop {
        raw_line.clear();
        let bytes_read: usize = reader.read_until(b'\n', &mut raw_line).unwrap_or_default();

        if bytes_read == 0 {
            let _ = tx.send(Event::Eof);
            break;
        }

        let line = normalize_line(&raw_line);
        push_line(&buffer, capacity, line.clone());
        if line_matches(&line, pattern_lower) {
            let _ = tx.send(Event::Match(line));
        }
    }
}

fn normalize_line(raw_line: &[u8]) -> String {
    let mut s = String::from_utf8_lossy(raw_line).into_owned();
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    s
}

fn push_line(buffer: &Buffer, capacity: usize, line: String) {
    let mut guard = buffer.lock().expect("buffer mutex poisoned");
    if guard.len() == capacity {
        guard.pop_front();
    }
    guard.push_back(line);
}

fn line_matches(line: &str, pattern_lower: &str) -> bool {
    if pattern_lower.is_empty() {
        return false;
    }
    line.to_lowercase().contains(pattern_lower)
}

fn drain_events(
    rx: &Receiver<Event>,
    buffer: &Buffer,
    output_target: OutputTarget,
    output_is_tty: bool,
    raw_mode: bool,
    saw_eof: &mut bool,
) {
    loop {
        match rx.try_recv() {
            Ok(Event::Match(line)) => print_match(&line, output_target, output_is_tty, raw_mode),
            Ok(Event::Eof) => {
                dump_buffer(buffer, DumpKind::Exit, output_target, raw_mode);
                *saw_eof = true;
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }
}

fn print_match(line: &str, output_target: OutputTarget, output_is_tty: bool, raw_mode: bool) {
    if output_is_tty {
        write_line(
            output_target,
            &format!("\x1b[2;33m[match]\x1b[0m {line}"),
            raw_mode,
        );
    } else {
        write_line(output_target, &format!("[match] {line}"), raw_mode);
    }
}

#[derive(Clone, Copy)]
enum DumpKind {
    Refresh,
    Exit,
}

fn dump_buffer(buffer: &Buffer, kind: DumpKind, output_target: OutputTarget, raw_mode: bool) {
    let lines = snapshot_buffer(buffer);
    let header = dump_header(lines.len(), kind);
    write_line(output_target, &header, raw_mode);
    for line in lines {
        write_line(output_target, &line, raw_mode);
    }
    if matches!(kind, DumpKind::Refresh) {
        write_line(output_target, "---", raw_mode);
    }
}

#[derive(Clone, Copy)]
enum OutputTarget {
    Stdout,
    Stderr,
}

impl OutputTarget {
    fn from_stdout_flag(use_stdout: bool) -> Self {
        if use_stdout {
            return Self::Stdout;
        }
        Self::Stderr
    }

    fn is_terminal(self) -> bool {
        match self {
            Self::Stdout => io::stdout().is_terminal(),
            Self::Stderr => io::stderr().is_terminal(),
        }
    }
}

fn write_line(output_target: OutputTarget, line: &str, raw_mode: bool) {
    let eol = if raw_mode { "\r\n" } else { "\n" };
    match output_target {
        OutputTarget::Stdout => {
            let mut stdout = io::stdout().lock();
            write!(stdout, "{line}{eol}").expect("failed to write to stdout");
        }
        OutputTarget::Stderr => {
            let mut stderr = io::stderr().lock();
            write!(stderr, "{line}{eol}").expect("failed to write to stderr");
        }
    }
}

fn snapshot_buffer(buffer: &Buffer) -> Vec<String> {
    let guard = buffer.lock().expect("buffer mutex poisoned");
    guard.iter().cloned().collect()
}

fn dump_header(line_count: usize, kind: DumpKind) -> String {
    match kind {
        DumpKind::Refresh => {
            format!("--- last {line_count} lines (press space again to refresh) ---")
        }
        DumpKind::Exit => format!("--- last {line_count} lines (exit) ---"),
    }
}

fn is_ctrl_c(key_event: &crossterm::event::KeyEvent) -> bool {
    key_event.code == KeyCode::Char('c') && key_event.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest_line() {
        let capacity = 2;
        let buffer: Buffer = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
        push_line(&buffer, capacity, "line 1".to_string());
        push_line(&buffer, capacity, "line 2".to_string());
        push_line(&buffer, capacity, "line 3".to_string());

        let snapshot = snapshot_buffer(&buffer);
        assert_eq!(snapshot, vec!["line 2".to_string(), "line 3".to_string()]);
    }

    #[test]
    fn ring_buffer_length_stays_bounded_under_load() {
        let capacity = 20;
        let buffer: Buffer = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));

        for i in 0..50_000 {
            push_line(&buffer, capacity, format!("line {i}"));
        }

        let guard = buffer.lock().expect("buffer mutex poisoned");
        assert_eq!(guard.len(), capacity);
        assert_eq!(guard.front().map(String::as_str), Some("line 49980"));
        assert_eq!(guard.back().map(String::as_str), Some("line 49999"));
    }

    #[test]
    fn line_match_is_case_insensitive() {
        let pattern = "error";
        assert!(line_matches("ERROR: boom", pattern));
        assert!(line_matches("Error happened", pattern));
        assert!(!line_matches("all good", pattern));
    }

    #[test]
    fn empty_pattern_disables_matching() {
        assert!(!line_matches("ERROR: boom", ""));
        assert!(!line_matches("", ""));
    }

    #[test]
    fn dump_header_uses_actual_line_count() {
        let header = dump_header(5, DumpKind::Exit);
        assert_eq!(header, "--- last 5 lines (exit) ---");
    }

    #[test]
    fn output_target_defaults_to_stderr() {
        let output = OutputTarget::from_stdout_flag(false);
        assert!(matches!(output, OutputTarget::Stderr));
    }

    #[test]
    fn output_target_switches_to_stdout_when_enabled() {
        let output = OutputTarget::from_stdout_flag(true);
        assert!(matches!(output, OutputTarget::Stdout));
    }
}
