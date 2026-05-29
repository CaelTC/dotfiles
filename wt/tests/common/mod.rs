use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {:?} failed in {:?}: {}",
        args,
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// Create a bare git repo populated with a single initial commit on `main`.
/// Returns the path to the bare repo (suitable as a clone URL).
pub fn make_remote(tmp: &Path) -> PathBuf {
    let work = tmp.join("seed");
    let bare = tmp.join("remote.git");

    std::fs::create_dir_all(&work).unwrap();
    Command::new("git")
        .args(["init", "-b", "main"])
        .arg(&work)
        .output()
        .unwrap();
    git(&work, &["config", "user.email", "test@example.com"]);
    git(&work, &["config", "user.name", "test"]);
    std::fs::write(work.join("README.md"), "hello\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-m", "initial"]);

    Command::new("git")
        .args(["clone", "--bare"])
        .arg(&work)
        .arg(&bare)
        .output()
        .unwrap();

    // Set HEAD on the bare repo so origin/HEAD resolution works after clone.
    git(&bare, &["symbolic-ref", "HEAD", "refs/heads/main"]);

    bare
}

/// Add a branch to a bare remote repo at the given base ref.
pub fn add_remote_branch(remote_bare: &Path, branch: &str, base: &str) {
    git(remote_bare, &["branch", branch, base]);
}
