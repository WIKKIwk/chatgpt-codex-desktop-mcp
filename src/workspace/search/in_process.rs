use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkFinish, SinkMatch,
};

use super::{GlobMatcher, SearchError, SearchOptions, cap_text, truncate_chars};
use crate::workspace::DenyRules;

#[allow(clippy::too_many_arguments)]
pub(super) fn search(
    workspace_root: &Path,
    start_path: &Path,
    deny_rules: &DenyRules,
    max_read_bytes: usize,
    max_output_bytes: usize,
    options: &SearchOptions,
    include: Option<&GlobMatcher>,
    exclude: Option<&GlobMatcher>,
) -> Result<String, SearchError> {
    let matcher = build_matcher(options)?;
    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .before_context(options.context_lines)
        .after_context(options.context_lines)
        .binary_detection(BinaryDetection::quit(0))
        .build();
    let files = match super::index::candidate_files(
        workspace_root,
        start_path,
        deny_rules,
        max_read_bytes,
        options,
        include,
        exclude,
    )? {
        Some(files) => files,
        None => {
            super::walker::collect_files(workspace_root, start_path, deny_rules, include, exclude)
        }
    };

    let mut output = Vec::new();
    let mut matched = 0;
    let mut truncated = false;
    for path in files {
        let relative_path = super::relative_display_path(workspace_root, &path);
        let remaining = options.max_matches.saturating_sub(matched);
        if remaining == 0 {
            break;
        }
        let mut sink = OutputSink::new(
            relative_path,
            remaining,
            max_output_bytes.saturating_sub(output_bytes(&output)),
        );
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let limited = file.take(max_read_bytes.saturating_add(1) as u64);
        if searcher
            .search_reader(&matcher, limited, &mut sink)
            .is_err()
        {
            continue;
        }
        matched += sink.matched;
        truncated |= sink.truncated;
        output.extend(sink.lines);
        if matched >= options.max_matches || truncated {
            break;
        }
    }

    let text = output.join("\n");
    if truncated || text.len() > max_output_bytes {
        Ok(format!(
            "{}\n[output truncated]\n",
            cap_text(text, max_output_bytes)
        ))
    } else {
        Ok(text)
    }
}

fn build_matcher(options: &SearchOptions) -> Result<grep_regex::RegexMatcher, SearchError> {
    let mut builder = RegexMatcherBuilder::new();
    builder
        .fixed_strings(true)
        .case_insensitive(!options.case_sensitive);
    builder
        .build(&options.pattern)
        .map_err(|error| SearchError::Matcher(error.to_string()))
}

fn output_bytes(lines: &[String]) -> usize {
    lines.iter().map(|line| line.len() + 1).sum()
}

struct OutputSink {
    path: String,
    remaining_matches: usize,
    matched: usize,
    lines: Vec<String>,
    context_start: Option<u64>,
    context_end: Option<u64>,
    output_bytes: usize,
    max_output_bytes: usize,
    truncated: bool,
}

impl OutputSink {
    fn new(path: String, remaining_matches: usize, max_output_bytes: usize) -> Self {
        Self {
            path,
            remaining_matches,
            matched: 0,
            lines: Vec::new(),
            context_start: None,
            context_end: None,
            output_bytes: 0,
            max_output_bytes,
            truncated: false,
        }
    }

    fn push_line(&mut self, line: String) {
        self.output_bytes = self.output_bytes.saturating_add(line.len() + 1);
        self.lines.push(line);
        if self.output_bytes > self.max_output_bytes {
            self.truncated = true;
        }
    }

    fn flush_context(&mut self) {
        let (Some(start), Some(end)) = (self.context_start.take(), self.context_end.take()) else {
            return;
        };
        self.push_line(format!("... {}:{}-{} (context)", self.path, start, end));
    }

    fn matched_line(&mut self, mat: &SinkMatch<'_>) {
        self.flush_context();
        let line = mat.line_number().unwrap_or_default();
        let text = String::from_utf8_lossy(mat.bytes());
        let text = text.trim_end_matches(['\r', '\n']);
        self.push_line(format!(
            "{}:{}: {}",
            self.path,
            line,
            truncate_chars(text, 300)
        ));
        self.matched += 1;
    }

    fn context_line(&mut self, context: &SinkContext<'_>) {
        let line = context.line_number().unwrap_or_default();
        if self.context_end.is_some_and(|end| end + 1 == line) {
            self.context_end = Some(line);
        } else {
            self.flush_context();
            self.context_start = Some(line);
            self.context_end = Some(line);
        }
    }
}

impl Sink for OutputSink {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.matched >= self.remaining_matches {
            return Ok(false);
        }
        self.matched_line(mat);
        Ok(!self.truncated)
    }

    fn context(
        &mut self,
        _searcher: &Searcher,
        context: &SinkContext<'_>,
    ) -> Result<bool, Self::Error> {
        self.context_line(context);
        Ok(!self.truncated)
    }

    fn context_break(&mut self, _searcher: &Searcher) -> Result<bool, Self::Error> {
        self.flush_context();
        Ok(!self.truncated)
    }

    fn binary_data(
        &mut self,
        _searcher: &Searcher,
        _binary_byte_offset: u64,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    fn finish(&mut self, _searcher: &Searcher, _finish: &SinkFinish) -> Result<(), Self::Error> {
        self.flush_context();
        Ok(())
    }
}
