//! Bounded local diagnostic event log. No telemetry or hardware identifiers.
use crate::preferences::data_dir;
use std::{
    fs, io,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy)]
pub enum Log {
    Monitor,
    Ui,
}

impl Log {
    fn filename(self) -> &'static str {
        match self {
            Self::Monitor => "monitor.log",
            Self::Ui => "ui.log",
        }
    }
    pub fn event(self, category: &str, message: &str) -> io::Result<()> {
        let dir = data_dir()?;
        fs::create_dir_all(&dir)?;
        append(&dir, self.filename(), category, message)
    }
}

fn single_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(300)
        .collect()
}

fn append(dir: &Path, name: &str, category: &str, message: &str) -> io::Result<()> {
    let path = dir.join(name);
    if fs::metadata(&path).is_ok_and(|file| file.len() >= MAX_BYTES) {
        let previous = dir.join(format!("{name}.previous"));
        fs::rename(&path, previous)?;
    }
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(
        file,
        "{time} [{}] {}",
        single_line(category),
        single_line(message)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_lines_cannot_inject_new_records() {
        assert_eq!(single_line("hello\nworld\t!"), "hello world !");
        assert_eq!(single_line(&"a".repeat(500)).len(), 300);
    }
}
