"""Small helpers shared by bench/run and bench/validate.

Nothing clever here: load a task folder, make a throwaway worktree at the
task's base commit, run a shell command, list what changed.
"""
import json
import os
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def load_tasks(tasks_dir, only=()):
    """Every folder under tasks_dir that has an expect.json, optionally filtered by id."""
    tasks = []
    for name in sorted(os.listdir(tasks_dir)):
        d = os.path.join(tasks_dir, name)
        if not os.path.isfile(os.path.join(d, "expect.json")):
            continue
        if only and name not in only:
            continue
        with open(os.path.join(d, "expect.json")) as f:
            expect = json.load(f)
        with open(os.path.join(d, "issue.md")) as f:
            issue = f.read()
        with open(os.path.join(d, "base_sha")) as f:
            base_sha = f.read().strip()
        tasks.append({"id": name, "dir": d, "expect": expect, "issue": issue, "base_sha": base_sha})
    return tasks


def sh(cmd, cwd, timeout=600, env=None):
    """Run a shell command, return (exit_code, combined_output). A timeout is exit code 124."""
    try:
        p = subprocess.run(cmd, shell=True, cwd=cwd, env=env, timeout=timeout,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        return p.returncode, p.stdout
    except subprocess.TimeoutExpired as e:
        return 124, (e.stdout or "") if isinstance(e.stdout, str) else ""


def git(repo, *args):
    p = subprocess.run(["git", "-C", repo, *args], stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, text=True)
    return p.returncode, p.stdout


class Worktree:
    """A fresh checkout of repo at sha. Use as a context manager so it always gets cleaned up."""

    def __init__(self, repo, sha):
        self.repo, self.sha = repo, sha
        self.tmp = tempfile.mkdtemp(prefix="zerd-bench-")
        self.path = os.path.join(self.tmp, "wt")

    def __enter__(self):
        rc, out = git(self.repo, "worktree", "add", "--detach", self.path, self.sha)
        if rc != 0:
            raise RuntimeError("worktree add failed: " + out)
        return self

    def __exit__(self, *exc):
        git(self.repo, "worktree", "remove", "--force", self.path)
        subprocess.run(["rm", "-rf", self.tmp])


def changed_files(wt_path, base_sha):
    """Files that differ from base, including brand new untracked ones."""
    git(wt_path, "add", "-A")
    _, out = git(wt_path, "diff", "--cached", "--name-only", base_sha)
    return [l for l in out.splitlines() if l.strip()]


def run_hidden(task, wt_path):
    """Run the task's hidden test inside the worktree. Returns True if it passes."""
    script = os.path.join(task["dir"], "hidden_test", "test.sh")
    if not os.path.isfile(script):
        return None
    rc, _ = sh("sh " + script, wt_path, timeout=task["expect"].get("timeout_s", 600))
    return rc == 0


def run_suite(task, wt_path):
    """Run the repo's existing suite. Returns True if it passes."""
    cmd = task["expect"].get("suite_cmd")
    if not cmd:
        return True
    rc, _ = sh(cmd, wt_path, timeout=task["expect"].get("timeout_s", 600))
    return rc == 0
