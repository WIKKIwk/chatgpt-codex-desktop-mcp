use super::*;
use std::fs;
use tempfile::tempdir;

fn deny_rules() -> DenyRules {
    DenyRules::new(&[]).expect("deny rules")
}

#[test]
fn search_supports_case_matching_and_context() {
    let temp = tempdir().expect("temporary directory");
    fs::write(
        temp.path().join("sample.rs"),
        "before\nNeedle line\nafter\n",
    )
    .expect("sample file");
    let options = SearchOptions {
        pattern: "needle".to_owned(),
        case_sensitive: false,
        context_lines: 1,
        max_matches: 10,
        include: Some("*.rs".to_owned()),
        exclude: None,
    };

    let output = search_text_files(
        temp.path(),
        temp.path(),
        &deny_rules(),
        10_000,
        10_000,
        &options,
    )
    .expect("search output");
    assert!(output.contains("sample.rs:2: Needle line"));
    assert!(output.contains("(context)"));
}

#[test]
fn search_from_subdirectory_keeps_workspace_relative_paths() {
    let temp = tempdir().expect("temporary directory");
    fs::create_dir(temp.path().join("src")).expect("src");
    fs::write(temp.path().join("src/main.rs"), "needle\n").expect("source");
    let options = SearchOptions {
        pattern: "needle".to_owned(),
        case_sensitive: true,
        context_lines: 0,
        max_matches: 10,
        include: None,
        exclude: None,
    };

    let output = search_text_files(
        temp.path(),
        &temp.path().join("src"),
        &deny_rules(),
        10_000,
        10_000,
        &options,
    )
    .expect("search output");
    assert_eq!(output, "src/main.rs:1: needle");
}

#[test]
fn find_and_tree_skip_ignored_directories() {
    let temp = tempdir().expect("temporary directory");
    fs::create_dir(temp.path().join("src")).expect("src");
    fs::create_dir(temp.path().join("node_modules")).expect("ignored");
    fs::write(temp.path().join("src/main.rs"), "fn main() {}").expect("source");
    fs::write(temp.path().join("node_modules/hidden.rs"), "hidden").expect("ignored file");
    let rules = deny_rules();

    let found = find_files(temp.path(), temp.path(), &rules, "*.rs", 10).expect("find");
    assert_eq!(found, vec!["src/main.rs"]);
    let tree = project_tree(temp.path(), temp.path(), &rules, 3, 10_000).expect("tree");
    assert!(tree.contains("📁 src/"));
    assert!(tree.contains("📄 main.rs"));
    assert!(!tree.contains("node_modules"));
}

#[test]
fn search_honors_gitignore_and_keeps_workspace_relative_paths() {
    let temp = tempdir().expect("temporary directory");
    fs::create_dir(temp.path().join("src")).expect("src");
    fs::create_dir(temp.path().join(".git")).expect("git metadata");
    fs::write(temp.path().join(".gitignore"), "ignored.rs\nignored-dir/\n").expect("gitignore");
    fs::write(temp.path().join("visible.rs"), "needle\n").expect("visible");
    fs::write(temp.path().join("ignored.rs"), "needle\n").expect("ignored");
    fs::create_dir(temp.path().join("ignored-dir")).expect("ignored directory");
    fs::write(temp.path().join("ignored-dir/hidden.rs"), "needle\n").expect("ignored file");
    let options = SearchOptions {
        pattern: "needle".to_owned(),
        case_sensitive: true,
        context_lines: 0,
        max_matches: 10,
        include: Some("*.rs".to_owned()),
        exclude: None,
    };

    let output = search_text_files(
        temp.path(),
        temp.path(),
        &deny_rules(),
        10_000,
        10_000,
        &options,
    )
    .expect("search output");

    assert_eq!(output, "visible.rs:1: needle");
}

#[test]
fn search_stops_at_match_and_output_limits() {
    let temp = tempdir().expect("temporary directory");
    fs::write(
        temp.path().join("sample.rs"),
        "needle one\nneedle two\nneedle three\n",
    )
    .expect("sample file");
    let options = SearchOptions {
        pattern: "needle".to_owned(),
        case_sensitive: true,
        context_lines: 0,
        max_matches: 2,
        include: None,
        exclude: None,
    };

    let output = search_text_files(
        temp.path(),
        temp.path(),
        &deny_rules(),
        10_000,
        20,
        &options,
    )
    .expect("search output");

    assert!(output.contains("[output truncated]"));
    assert_eq!(output.matches("sample.rs:").count(), 1);
}

#[test]
fn repeated_indexed_search_refreshes_changed_and_new_files() {
    let temp = tempdir().expect("temporary directory");
    let first = temp.path().join("first.rs");
    fs::write(&first, "needle\n").expect("first file");
    let options = SearchOptions {
        pattern: "needle".to_owned(),
        case_sensitive: false,
        context_lines: 0,
        max_matches: 10,
        include: Some("*.rs".to_owned()),
        exclude: None,
    };
    let rules = deny_rules();

    let initial = search_text_files(temp.path(), temp.path(), &rules, 10_000, 10_000, &options)
        .expect("initial search");
    assert_eq!(initial, "first.rs:1: needle");

    fs::write(&first, "changed content\n").expect("changed file");
    fs::write(temp.path().join("second.rs"), "NEEDLE\n").expect("new file");
    let refreshed = search_text_files(temp.path(), temp.path(), &rules, 10_000, 10_000, &options)
        .expect("refreshed search");
    assert_eq!(refreshed, "second.rs:1: NEEDLE");
}
