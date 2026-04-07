//! List mock recordings

use crate::ui;
use std::path::PathBuf;

pub fn list_recordings(dir: Option<String>) -> anyhow::Result<()> {
  let recordings_dir =
    dir.unwrap_or_else(|| std::env::var("RECORDINGS_DIR").unwrap_or_else(|_| "mocks/recordings".to_string()));

  println!(
    "{}",
    ui::info(&format!("Recordings directory: {}", ui::path(&recordings_dir)))
  );
  println!();

  let path = PathBuf::from(&recordings_dir);
  if !path.exists() {
    println!("{}", ui::warning("Recordings directory does not exist"));
    return Ok(());
  }

  let read_dir_entries = std::fs::read_dir(&path)?;

  let mut entries = read_dir_entries
    .filter_map(Result::ok)
    .filter(|e| {
      e.file_type().is_ok_and(|ft| ft.is_file())
        && e
          .path()
          .extension()
          .and_then(|ext| ext.to_str())
          .is_some_and(|ext| ext == "json" || ext == "yaml" || ext == "yml")
    })
    .collect::<Vec<_>>();

  if entries.is_empty() {
    println!("{}", ui::info("No recordings found"));
    return Ok(());
  }

  entries.sort_by(|a, b| {
    let a_time = a.metadata().ok().and_then(|m| m.modified().ok());
    let b_time = b.metadata().ok().and_then(|m| m.modified().ok());
    b_time.cmp(&a_time) // Reverse order (newest first)
  });

  println!(
    "{}",
    ui::header(&format!("Found {} recording(s)", ui::number(entries.len())))
  );

  for entry in &entries {
    let path = entry.path();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
    let size = entry.metadata().ok().map_or(0, |m| m.len());

    println!();
    println!("{}", ui::list_item(&ui::emphasis(name)));
    println!("{}", ui::kv("  Size", &ui::format_bytes(size)));

    if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
      if let Ok(duration) = modified.duration_since(std::time::SystemTime::UNIX_EPOCH) {
        use chrono::{DateTime, Utc};
        #[allow(clippy::cast_possible_wrap)]
        let datetime = DateTime::<Utc>::from_timestamp(duration.as_secs() as i64, 0);
        if let Some(dt) = datetime {
          println!(
            "{}",
            ui::kv("  Modified", &dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
          );
        }
      }
    }
  }

  println!();
  println!("{}", ui::kv("Total", &ui::number(entries.len())));

  Ok(())
}
