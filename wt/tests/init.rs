use assert_cmd::Command;
use tempfile::TempDir;

mod common;
use common::{git, make_remote};

#[test]
fn init_clones_bare_and_checks_out_default_branch() {
    let tmp = TempDir::new().unwrap();
    let remote = make_remote(tmp.path());
    let parent = tmp.path().join("project");

    Command::cargo_bin("wt")
        .unwrap()
        .arg("init")
        .arg(&remote)
        .arg(&parent)
        .assert()
        .success();

    let bare = parent.join(".bare");
    assert!(bare.is_dir(), ".bare/ should exist as a directory");
    assert_eq!(
        git(&bare, &["config", "--get", "core.bare"]),
        "true",
        "core.bare should be true"
    );

    let dotgit = parent.join(".git");
    assert!(dotgit.is_file(), ".git should be a file");
    let contents = std::fs::read_to_string(&dotgit).unwrap();
    assert!(
        contents.contains("gitdir:") && contents.contains(".bare"),
        ".git should point to ./.bare, got: {contents:?}"
    );

    let main_wt = parent.join("main");
    assert!(main_wt.is_dir(), "main/ worktree should exist");
    assert!(
        main_wt.join("README.md").is_file(),
        "main/ should contain the seeded README"
    );

    let main_head = git(&main_wt, &["rev-parse", "HEAD"]);
    let remote_head = git(&remote, &["rev-parse", "refs/heads/main"]);
    assert_eq!(
        main_head, remote_head,
        "main worktree HEAD should match remote main"
    );
}

#[test]
fn init_refuses_to_clobber_non_empty_parent() {
    let tmp = TempDir::new().unwrap();
    let remote = make_remote(tmp.path());
    let parent = tmp.path().join("project");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::write(parent.join("preexisting.txt"), "do not touch\n").unwrap();

    Command::cargo_bin("wt")
        .unwrap()
        .arg("init")
        .arg(&remote)
        .arg(&parent)
        .assert()
        .failure();

    assert!(
        parent.join("preexisting.txt").is_file(),
        "preexisting file should remain untouched"
    );
    assert!(
        !parent.join(".bare").exists(),
        ".bare should not have been created"
    );
}
