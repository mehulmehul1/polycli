use serde::Serialize;
use serde_json::json;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct JsonlEventLogger {
    writer: Arc<Mutex<BufWriter<File>>>,
    path: PathBuf,
}

impl JsonlEventLogger {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn log<T: Serialize>(&self, kind: &str, payload: &T) {
        let line = json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "kind": kind,
            "payload": payload,
        });

        if let Ok(mut writer) = self.writer.lock() {
            let _ = serde_json::to_writer(&mut *writer, &line);
            let _ = writer.write_all(b"\n");
            let _ = writer.flush();
        }
    }
}
