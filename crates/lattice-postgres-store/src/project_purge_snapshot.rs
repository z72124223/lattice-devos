//! The exact legacy JSON map commitment, produced without retaining row payloads.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as FmtWrite;
use std::io::{self, Write};

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            write!(text, "{byte:02x}").expect("writing to String cannot fail");
            text
        })
}

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Default)]
struct HashWriter {
    digest: Sha256,
    pending: Vec<u8>,
}
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl HashWriter {
    fn update(&mut self, bytes: &[u8]) {
        if self.pending.len() + bytes.len() >= 64 * 1024 {
            self.digest.update(&self.pending);
            self.pending.clear();
            self.digest.update(bytes);
        } else {
            self.pending.extend_from_slice(bytes);
        }
    }
    fn finish(mut self) -> String {
        self.digest.update(&self.pending);
        hex(self.digest.finalize().as_slice())
    }
}

pub(super) struct SnapshotHasher {
    rows: HashWriter,
    scope: Option<HashWriter>,
    scope_suffix: Vec<u8>,
    table_count: usize,
    row_count: u64,
}
impl Write for SnapshotHasher {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.rows.write_all(bytes)?;
        if let Some(scope) = &mut self.scope {
            scope.write_all(bytes)?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl SnapshotHasher {
    pub(super) fn new(scope: Option<&Value>) -> Result<Self> {
        let mut prefix = Vec::new();
        let mut suffix = Vec::new();
        if let Some(scope) = scope {
            let fields = scope.as_object().ok_or("PROJECT_PURGE_SERIALIZATION")?;
            if !fields.contains_key("rows") {
                return Err("PROJECT_PURGE_SERIALIZATION");
            }
            prefix.push(b'{');
            let mut after_rows = false;
            for (index, (key, value)) in fields.iter().enumerate() {
                let out = if after_rows { &mut suffix } else { &mut prefix };
                if index > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key).map_err(|_| "PROJECT_PURGE_SERIALIZATION")?;
                out.push(b':');
                if key == "rows" {
                    after_rows = true;
                } else {
                    serde_json::to_writer(out, value).map_err(|_| "PROJECT_PURGE_SERIALIZATION")?;
                }
            }
            suffix.push(b'}');
        }
        let mut result = Self {
            rows: HashWriter::default(),
            scope: scope.map(|_| HashWriter::default()),
            scope_suffix: suffix,
            table_count: 0,
            row_count: 0,
        };
        if let Some(scope) = &mut result.scope {
            scope.update(&prefix);
        }
        result.bytes(b"{");
        Ok(result)
    }
    fn bytes(&mut self, value: &[u8]) {
        self.rows.update(value);
        if let Some(scope) = &mut self.scope {
            scope.update(value);
        }
    }
    pub(super) fn table(&mut self, name: &str) -> Result<()> {
        if self.table_count > 0 {
            self.bytes(b",");
        }
        serde_json::to_writer(&mut *self, name).map_err(|_| "PROJECT_PURGE_SERIALIZATION")?;
        self.bytes(b":[");
        self.table_count += 1;
        self.row_count = 0;
        Ok(())
    }
    pub(super) fn row(&mut self, row: &str) -> Result<()> {
        if self.row_count > 0 {
            self.bytes(b",");
        }
        serde_json::to_writer(&mut *self, row).map_err(|_| "PROJECT_PURGE_SERIALIZATION")?;
        self.row_count = self
            .row_count
            .checked_add(1)
            .ok_or("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
        Ok(())
    }
    pub(super) fn end_table(&mut self) {
        self.bytes(b"]");
    }
    pub(super) fn finish(mut self) -> (String, Option<String>) {
        self.bytes(b"}");
        if let Some(scope) = &mut self.scope {
            scope.update(&self.scope_suffix);
        }
        (self.rows.finish(), self.scope.map(HashWriter::finish))
    }
}

#[cfg(test)]
mod tests {
    use super::{SnapshotHasher, hex};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;

    #[test]
    fn stream_matches_legacy_map_and_all_scope_versions_byte_for_byte() {
        for rows in [
            BTreeMap::new(),
            BTreeMap::from([
                ("control.empty".to_owned(), vec![]),
                (
                    "memory.records".to_owned(),
                    vec![
                        "{\"unicode\":\"繁體🧪\",\"escape\":\"\\n\\\\\\\"\"}".to_owned(),
                        "duplicates".to_owned(),
                        "duplicates".to_owned(),
                    ],
                ),
            ]),
        ] {
            for version in ["v2", "v3", "v4"] {
                let mut scope = json!({"schema":format!("lattice.project-purge.scope.{version}"),
                    "database":"database", "project":"project", "operation":"operation",
                    "maintenanceExtensionInstalled":true,"rows":rows});
                if version != "v2" {
                    scope["registryPolicy"] = json!("MINIMAL_ATTESTATION");
                }
                if version == "v4" {
                    scope["graphSourceProof"] = json!({"targetAnalyses":2});
                }
                let mut streamed = SnapshotHasher::new(Some(&scope)).unwrap();
                for (name, values) in &rows {
                    streamed.table(name).unwrap();
                    for row in values {
                        streamed.row(row).unwrap();
                    }
                    streamed.end_table();
                }
                let (actual_rows, actual_scope) = streamed.finish();
                assert_eq!(
                    actual_rows,
                    hex(Sha256::digest(serde_json::to_vec(&rows).unwrap()).as_slice())
                );
                assert_eq!(
                    actual_scope.unwrap(),
                    hex(Sha256::digest(serde_json::to_vec(&scope).unwrap()).as_slice())
                );
            }
        }
    }
}
