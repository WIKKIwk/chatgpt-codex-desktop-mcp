use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

use notify::{RecursiveMode, Watcher};

use super::{GlobMatcher, SearchError, SearchOptions};
use crate::workspace::DenyRules;

const MAX_CACHED_INDEXES: usize = 8;
const TRIGRAM_BYTES: usize = 3;

pub(super) fn candidate_files(
    workspace_root: &Path,
    start_path: &Path,
    deny_rules: &DenyRules,
    max_read_bytes: usize,
    options: &SearchOptions,
    include: Option<&GlobMatcher>,
    exclude: Option<&GlobMatcher>,
) -> Result<Option<Vec<PathBuf>>, SearchError> {
    if !is_indexable(options) {
        return Ok(None);
    }

    let key = IndexKey {
        workspace_root: workspace_root.to_path_buf(),
        start_path: start_path.to_path_buf(),
        include: options.include.clone(),
        exclude: options.exclude.clone(),
        deny_patterns: deny_rules.patterns().to_vec(),
    };
    let cache = INDEXES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut indexes = cache
        .lock()
        .map_err(|_| SearchError::Io(io::Error::other("search index is unavailable")))?;
    if !indexes.contains_key(&key)
        && indexes.len() >= MAX_CACHED_INDEXES
        && let Some(oldest_key) = indexes.keys().next().cloned()
    {
        indexes.remove(&oldest_key);
    }
    let index = indexes.entry(key).or_default();
    index.refresh(
        workspace_root,
        start_path,
        deny_rules,
        max_read_bytes,
        include,
        exclude,
    );
    Ok(Some(
        index.candidates(&options.pattern, options.case_sensitive),
    ))
}

fn is_indexable(options: &SearchOptions) -> bool {
    options.pattern.len() >= TRIGRAM_BYTES
        && options.pattern.is_ascii()
        && !options.pattern.contains('\n')
}

static INDEXES: OnceLock<Mutex<HashMap<IndexKey, SearchIndex>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IndexKey {
    workspace_root: PathBuf,
    start_path: PathBuf,
    include: Option<String>,
    exclude: Option<String>,
    deny_patterns: Vec<String>,
}

#[derive(Debug)]
struct SearchIndex {
    files: HashMap<PathBuf, FileRecord>,
    original_postings: HashMap<u32, Vec<PathBuf>>,
    folded_postings: HashMap<u32, Vec<PathBuf>>,
    dirty: Arc<AtomicBool>,
    watcher: Option<notify::RecommendedWatcher>,
    initialized: bool,
}

impl Default for SearchIndex {
    fn default() -> Self {
        Self {
            files: HashMap::new(),
            original_postings: HashMap::new(),
            folded_postings: HashMap::new(),
            dirty: Arc::new(AtomicBool::new(true)),
            watcher: None,
            initialized: false,
        }
    }
}

impl SearchIndex {
    fn refresh(
        &mut self,
        workspace_root: &Path,
        start_path: &Path,
        deny_rules: &DenyRules,
        max_read_bytes: usize,
        include: Option<&GlobMatcher>,
        exclude: Option<&GlobMatcher>,
    ) {
        self.ensure_watcher(workspace_root);
        if self.initialized && self.watcher.is_some() && !self.dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        let paths =
            super::walker::collect_files(workspace_root, start_path, deny_rules, include, exclude);
        let previous = std::mem::take(&mut self.files);
        let mut next = HashMap::with_capacity(paths.len());
        for path in paths {
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let stamp = FileStamp::from_metadata(&metadata);
            if let Some(record) = previous.get(&path)
                && record.can_reuse(&stamp, max_read_bytes)
            {
                next.insert(path, record.clone());
                continue;
            }
            if let Some(record) = FileRecord::read(&path, stamp, max_read_bytes) {
                next.insert(path, record);
            }
        }

        let changed = next.len() != previous.len()
            || next.iter().any(|(path, record)| {
                previous.get(path).is_none_or(|old| {
                    old.stamp != record.stamp || old.read_limit != record.read_limit
                })
            });
        self.files = next;
        if changed || self.original_postings.is_empty() {
            self.rebuild_postings();
        }
        self.initialized = true;
    }

    fn ensure_watcher(&mut self, workspace_root: &Path) {
        if self.watcher.is_some() {
            return;
        }
        let dirty = Arc::clone(&self.dirty);
        let Ok(mut watcher) =
            notify::recommended_watcher(move |_event: notify::Result<notify::Event>| {
                dirty.store(true, Ordering::Release);
            })
        else {
            return;
        };
        if watcher
            .watch(workspace_root, RecursiveMode::Recursive)
            .is_ok()
        {
            self.watcher = Some(watcher);
        }
    }

    fn rebuild_postings(&mut self) {
        self.original_postings.clear();
        self.folded_postings.clear();
        for (path, record) in &self.files {
            for trigram in &record.original_trigrams {
                self.original_postings
                    .entry(*trigram)
                    .or_default()
                    .push(path.clone());
            }
            for trigram in &record.folded_trigrams {
                self.folded_postings
                    .entry(*trigram)
                    .or_default()
                    .push(path.clone());
            }
        }
        for posting in self
            .original_postings
            .values_mut()
            .chain(self.folded_postings.values_mut())
        {
            posting.sort();
        }
    }

    fn candidates(&self, pattern: &str, case_sensitive: bool) -> Vec<PathBuf> {
        let bytes = if case_sensitive {
            pattern.as_bytes().to_vec()
        } else {
            pattern
                .bytes()
                .map(|byte| byte.to_ascii_lowercase())
                .collect()
        };
        let wanted = trigrams(&bytes);
        let postings = if case_sensitive {
            &self.original_postings
        } else {
            &self.folded_postings
        };
        let seed = wanted
            .iter()
            .filter_map(|trigram| postings.get(trigram))
            .min_by_key(|posting| posting.len());
        let mut candidates = seed.cloned().unwrap_or_default();
        if seed.is_some() {
            candidates.retain(|path| {
                wanted.iter().all(|trigram| {
                    postings
                        .get(trigram)
                        .is_some_and(|posting| posting.binary_search(path).is_ok())
                })
            });
        }
        if !case_sensitive {
            candidates.extend(
                self.files
                    .iter()
                    .filter_map(|(path, record)| record.has_non_ascii.then_some(path.clone())),
            );
        }
        candidates.sort();
        candidates.dedup();
        candidates
    }
}

#[derive(Debug, Clone)]
struct FileRecord {
    stamp: FileStamp,
    read_limit: usize,
    has_non_ascii: bool,
    original_trigrams: Vec<u32>,
    folded_trigrams: Vec<u32>,
}

impl FileRecord {
    fn read(path: &Path, stamp: FileStamp, max_read_bytes: usize) -> Option<Self> {
        let mut bytes = Vec::new();
        File::open(path)
            .ok()?
            .take(max_read_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        bytes.truncate(max_read_bytes);
        let folded = bytes
            .iter()
            .map(|byte| byte.to_ascii_lowercase())
            .collect::<Vec<_>>();
        Some(Self {
            stamp,
            read_limit: max_read_bytes,
            has_non_ascii: bytes.iter().any(|byte| !byte.is_ascii()),
            original_trigrams: trigrams(&bytes),
            folded_trigrams: trigrams(&folded),
        })
    }

    fn can_reuse(&self, stamp: &FileStamp, max_read_bytes: usize) -> bool {
        self.stamp == *stamp && self.read_limit >= max_read_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

fn trigrams(bytes: &[u8]) -> Vec<u32> {
    if bytes.len() < TRIGRAM_BYTES {
        return Vec::new();
    }
    let mut trigrams = bytes
        .windows(TRIGRAM_BYTES)
        .map(|window| ((window[0] as u32) << 16) | ((window[1] as u32) << 8) | window[2] as u32)
        .collect::<Vec<_>>();
    trigrams.sort_unstable();
    trigrams.dedup();
    trigrams
}
