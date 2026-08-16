use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

pub const FILE_TRUNCATION_MARKER: &str = "\n[file truncated]\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFileResult {
    pub content: String,
    pub truncated: bool,
}

pub fn list_directory(path: &Path) -> io::Result<String> {
    let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    if entries.is_empty() {
        return Ok("(empty)".to_owned());
    }

    entries
        .into_iter()
        .map(|entry| {
            let prefix = if entry.file_type()?.is_dir() {
                "dir "
            } else {
                "file "
            };
            Ok(format!("{prefix}{}", entry.file_name().to_string_lossy()))
        })
        .collect::<io::Result<Vec<_>>>()
        .map(|lines| lines.join("\n"))
}

pub fn read_file_capped(path: &Path, max_bytes: usize) -> io::Result<ReadFileResult> {
    let mut bytes = Vec::with_capacity(max_bytes.saturating_add(1));
    File::open(path)?
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;

    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
    }
    let mut content = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        content.push_str(FILE_TRUNCATION_MARKER);
    }

    Ok(ReadFileResult { content, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn list_directory_is_sorted_and_labels_entries() {
        let temp = tempdir().expect("temporary directory");
        fs::create_dir(temp.path().join("z-dir")).expect("directory");
        fs::write(temp.path().join("a.txt"), "content").expect("file");

        assert_eq!(
            list_directory(temp.path()).expect("directory listing"),
            "file a.txt\ndir z-dir"
        );
    }

    #[test]
    fn read_file_capped_marks_truncation_without_reading_the_whole_file() {
        let temp = tempdir().expect("temporary directory");
        let path = temp.path().join("sample.txt");
        fs::write(&path, "0123456789").expect("file");

        assert_eq!(
            read_file_capped(&path, 4).expect("read result"),
            ReadFileResult {
                content: "0123\n[file truncated]\n".to_owned(),
                truncated: true,
            }
        );
        assert_eq!(
            read_file_capped(&path, 20).expect("read result"),
            ReadFileResult {
                content: "0123456789".to_owned(),
                truncated: false,
            }
        );
    }
}
