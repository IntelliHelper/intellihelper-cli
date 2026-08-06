//! `INTELLIHELPER_HOME` override tests in an isolated binary so `intellihelper_home()`'s
//! process-wide `OnceLock` initializes from the overridden env var.

use std::path::PathBuf;

#[test]
fn intellihelper_home_override_path_helpers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let intellihelper_home = tmp.path().to_path_buf();
    unsafe {
        std::env::set_var("INTELLIHELPER_HOME", &intellihelper_home);
    }

    assert_eq!(
        intellihelper_pager::util::pager_toml_path(),
        intellihelper_home.join("pager.toml")
    );
    assert_eq!(
        intellihelper_pager::util::display_intellihelper_home_prefix(),
        "$INTELLIHELPER_HOME"
    );
    assert_eq!(
        intellihelper_pager::util::display_user_intellihelper_path("config.toml"),
        "$INTELLIHELPER_HOME/config.toml"
    );

    let memory_path = intellihelper_home.join("memory/MEMORY.md");
    assert_eq!(
        intellihelper_pager::util::abbreviate_path(&memory_path.display().to_string()),
        "$INTELLIHELPER_HOME/memory/MEMORY.md"
    );

    // Copy-toast paths follow the same abbreviation convention, so a custom
    // $INTELLIHELPER_HOME outside $HOME still displays short.
    assert_eq!(
        intellihelper_pager::clipboard::display_copy_path(&intellihelper_home.join("last-copy.txt")),
        "$INTELLIHELPER_HOME/last-copy.txt"
    );

    assert!(intellihelper_pager::util::is_under_user_intellihelper_home(&memory_path));
    assert!(!intellihelper_pager::util::is_under_user_intellihelper_home(
        PathBuf::from("/tmp/other").as_path()
    ));
}
