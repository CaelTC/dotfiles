use assert_cmd::Command;
use tempfile::TempDir;

mod common;
use common::{add_remote_branch, make_remote};

fn init_project(tmp: &std::path::Path) -> std::path::PathBuf {
    let remote = make_remote(tmp);
    let parent = tmp.join("project");
    Command::cargo_bin("wt")
        .unwrap()
        .arg("init")
        .arg(&remote)
        .arg(&parent)
        .assert()
        .success();
    parent
}

/// Set up a parent where `feature/x` exists on origin and is checked out as a
/// worktree tracking origin. Removing this branch is clean: no dirty files,
/// no unpushed commits, upstream is set.
fn setup_clean_tracked(tmp: &std::path::Path) -> std::path::PathBuf {
    let remote = make_remote(tmp);
    add_remote_branch(&remote, "feature/x", "main");
    let parent = tmp.join("project");
    Command::cargo_bin("wt")
        .unwrap()
        .arg("init")
        .arg(&remote)
        .arg(&parent)
        .assert()
        .success();
    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["new", "feature/x"])
        .assert()
        .success();
    parent
}

fn wt_new(parent: &std::path::Path, branch: &str) {
    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(parent)
        .args(["new", branch])
        .assert()
        .success();
}

#[test]
fn rm_removes_worktree_branch_and_empty_parent_dirs() {
    let tmp = TempDir::new().unwrap();
    let parent = setup_clean_tracked(tmp.path());

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["rm", "feature/x"])
        .assert()
        .success();

    assert!(
        !parent.join("feature").join("x").exists(),
        "feature/x/ worktree dir should be gone"
    );
    assert!(
        !parent.join("feature").exists(),
        "empty feature/ parent dir should be cleaned up"
    );

    let branches = String::from_utf8(
        std::process::Command::new("git")
            .current_dir(parent.join(".bare"))
            .args(["for-each-ref", "--format=%(refname:short)", "refs/heads/"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(
        !branches.contains("feature/x"),
        "local branch feature/x should be deleted, got:\n{branches}"
    );
}

#[test]
fn rm_refuses_on_dirty_without_force() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());
    wt_new(&parent, "feature/x");

    let wt = parent.join("feature").join("x");
    std::fs::write(wt.join("README.md"), "modified\n").unwrap();

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["rm", "feature/x"])
        .assert()
        .failure();

    assert!(wt.exists(), "worktree should not have been removed");
    assert_eq!(
        std::fs::read_to_string(wt.join("README.md")).unwrap(),
        "modified\n",
        "dirty file should remain"
    );
}

#[test]
fn rm_force_overrides_dirty() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());
    wt_new(&parent, "feature/x");

    let wt = parent.join("feature").join("x");
    std::fs::write(wt.join("README.md"), "modified\n").unwrap();

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["rm", "feature/x", "--force"])
        .assert()
        .success();

    assert!(!wt.exists(), "worktree should be gone with --force");
}

#[test]
fn rm_refuses_on_unpushed_without_force() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());
    // wt new creates a new branch with no upstream; that is "unpushed" per spec.
    wt_new(&parent, "feature/x");

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["rm", "feature/x"])
        .assert()
        .failure();

    assert!(
        parent.join("feature").join("x").exists(),
        "worktree should remain"
    );
}

#[test]
fn rm_force_overrides_unpushed() {
    let tmp = TempDir::new().unwrap();
    let parent = init_project(tmp.path());
    wt_new(&parent, "feature/x");

    Command::cargo_bin("wt")
        .unwrap()
        .current_dir(&parent)
        .args(["rm", "feature/x", "--force"])
        .assert()
        .success();

    assert!(!parent.join("feature").exists());
}
