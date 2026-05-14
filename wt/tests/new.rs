use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

mod common;
use common::{add_remote_branch, git, make_remote};

fn init_from_remote(tmp: &std::path::Path, remote: &std::path::Path) -> std::path::PathBuf {
    let parent = tmp.join("project");
    Command::cargo_bin("wt")
        .unwrap()
        .arg("init")
        .arg(remote)
        .arg(&parent)
        .assert()
        .success();
    parent
}

fn init_project(tmp: &std::path::Path) -> std::path::PathBuf {
    let remote = make_remote(tmp);
    init_from_remote(tmp, &remote)
}

#[test]
fn new_creates_worktree_for_new_branch() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["new", "feature/x"])
        .assert()
        .success();

    let wt = parent.join("feature").join("x");
    assert!(wt.is_dir(), "feature/x/ worktree should exist");
    assert_eq!(
        git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/x",
        "worktree should be on branch feature/x"
    );

    let main_head = git(&parent.join("main"), &["rev-parse", "HEAD"]);
    let feature_head = git(&wt, &["rev-parse", "HEAD"]);
    assert_eq!(
        feature_head, main_head,
        "feature/x should start at origin/HEAD (same as main)"
    );
}

#[test]
fn new_tracks_remote_only_branch() {
    let tmp = TempDir::new().unwrap();
    let remote = make_remote(tmp.path());
    add_remote_branch(&remote, "feature/x", "main");
    let parent = init_from_remote(tmp.path(), &remote);

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["new", "feature/x"])
        .assert()
        .success();

    let wt = parent.join("feature").join("x");
    assert_eq!(
        git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/x"
    );
    assert_eq!(
        git(&wt, &["rev-parse", "--abbrev-ref", "feature/x@{upstream}"]),
        "origin/feature/x",
        "local feature/x should track origin/feature/x"
    );
}

#[test]
fn new_symlinks_shared_files_from_config() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());

    std::fs::write(parent.join(".env"), "SECRET=42\n").unwrap();
    std::fs::write(
        parent.join(".bare").join("wt-config.toml"),
        "shared = [\".env\"]\n",
    )
    .unwrap();

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["new", "feature/x"])
        .assert()
        .success();

    let wt_env = parent.join("feature").join("x").join(".env");
    let meta = std::fs::symlink_metadata(&wt_env).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "feature/x/.env should be a symlink"
    );

    let link_target = std::fs::read_link(&wt_env).unwrap();
    assert!(
        link_target.is_relative(),
        "symlink target should be relative, got: {}",
        link_target.display()
    );

    let content = std::fs::read_to_string(&wt_env).unwrap();
    assert_eq!(content, "SECRET=42\n");
}

#[test]
fn new_dot_base_uses_current_worktree_head() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());
    let main_wt = parent.join("main");

    git(&main_wt, &["config", "user.email", "test@example.com"]);
    git(&main_wt, &["config", "user.name", "test"]);
    std::fs::write(main_wt.join("local.txt"), "x\n").unwrap();
    git(&main_wt, &["add", "."]);
    git(&main_wt, &["commit", "-m", "local-only"]);
    let main_head = git(&main_wt, &["rev-parse", "HEAD"]);

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&main_wt)
        .args(["new", "feature/x", "."])
        .assert()
        .success();

    let wt = parent.join("feature").join("x");
    assert_eq!(
        git(&wt, &["rev-parse", "HEAD"]),
        main_head,
        "feature/x HEAD should equal the main worktree's HEAD"
    );
}

#[test]
fn new_runs_post_create_hook_in_worktree() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());

    std::fs::write(
        parent.join(".bare").join("wt-config.toml"),
        "post_create = \"touch hook-ran.txt\"\n",
    )
    .unwrap();

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["new", "feature/x"])
        .assert()
        .success();

    let marker = parent.join("feature").join("x").join("hook-ran.txt");
    assert!(
        marker.exists(),
        "post_create hook should have created hook-ran.txt at {}",
        marker.display()
    );
}

#[test]
fn new_errors_when_branch_already_has_a_worktree() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["new", "feature/x"])
        .assert()
        .success();

    let existing = parent.join("feature").join("x").canonicalize().unwrap();
    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["new", "feature/x"])
        .assert()
        .failure()
        .stderr(contains(existing.to_str().unwrap().to_string()));
}
