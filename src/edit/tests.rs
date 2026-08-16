use std::fs;

use tempfile::tempdir;

use super::*;

#[tokio::test]
async fn preview_and_apply_cover_text_and_file_operations() {
    let temp = tempdir().expect("temporary directory");
    fs::write(temp.path().join("sample.txt"), "one\ntwo\n").expect("sample file");

    let replace = Change::ReplaceText {
        path: "sample.txt".to_owned(),
        old_text: Some("one".to_owned()),
        new_text: Some("ONE".to_owned()),
    };
    let diffs = preview_changes(temp.path(), std::slice::from_ref(&replace))
        .await
        .expect("replace preview");
    assert_eq!(diffs[0].edit_type, EditType::ReplaceText);
    assert!(diffs[0].diff.contains("-one"));
    apply_changes(temp.path(), std::slice::from_ref(&replace))
        .await
        .expect("replace apply");
    assert_eq!(
        fs::read_to_string(temp.path().join("sample.txt")).expect("updated file"),
        "ONE\ntwo\n"
    );

    let range = Change::ReplaceRange {
        path: "sample.txt".to_owned(),
        start_line: Some(2),
        end_line: Some(2),
        new_text: Some("TWO".to_owned()),
    };
    apply_changes(temp.path(), std::slice::from_ref(&range))
        .await
        .expect("range apply");
    let before = Change::InsertBefore {
        path: "sample.txt".to_owned(),
        anchor: Some("TWO".to_owned()),
        text: Some("before\n".to_owned()),
    };
    let after = Change::InsertAfter {
        path: "sample.txt".to_owned(),
        anchor_after: Some("TWO".to_owned()),
        text: Some("\nafter".to_owned()),
    };
    apply_changes(temp.path(), &[before, after])
        .await
        .expect("insert apply");
    assert!(
        fs::read_to_string(temp.path().join("sample.txt"))
            .expect("inserted file")
            .contains("before")
    );

    let create = Change::Create {
        path: "nested/new.txt".to_owned(),
        text: Some("new".to_owned()),
    };
    preview_changes(temp.path(), std::slice::from_ref(&create))
        .await
        .expect("create preview");
    apply_changes(temp.path(), std::slice::from_ref(&create))
        .await
        .expect("create apply");
    let append = Change::Append {
        path: "nested/new.txt".to_owned(),
        text: Some("line".to_owned()),
    };
    apply_changes(temp.path(), std::slice::from_ref(&append))
        .await
        .expect("append apply");

    let rename = Change::Rename {
        path: "nested/new.txt".to_owned(),
        new_path: Some("nested/renamed.txt".to_owned()),
    };
    preview_changes(temp.path(), std::slice::from_ref(&rename))
        .await
        .expect("rename preview");
    apply_changes(temp.path(), std::slice::from_ref(&rename))
        .await
        .expect("rename apply");
    let delete = Change::Delete {
        path: "nested/renamed.txt".to_owned(),
    };
    apply_changes(temp.path(), std::slice::from_ref(&delete))
        .await
        .expect("delete apply");
    assert!(!temp.path().join("nested/renamed.txt").exists());
}

#[test]
fn edit_store_actions_are_single_use() {
    let mut store = EditStore::new();
    let pending = store.create("workspace".to_owned(), Vec::new(), Vec::new());
    assert!(store.take(&pending.id).is_ok());
    assert!(matches!(
        store.take(&pending.id),
        Err(EditError::UnknownAction(_))
    ));
}
