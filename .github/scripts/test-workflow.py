#!/usr/bin/env python3
"""Run a workflow's shell steps locally, without Docker or a runner.

This is a test harness, not a GitHub Actions implementation. It reads the
`run:` blocks straight out of the workflow file and executes them with the
environment a runner would provide, so what gets tested is the text that
ships rather than a retyped copy of it. That is enough to catch the bugs
these workflows actually have — shell quoting, tag parsing, a condition
written the wrong way round.

What it does NOT do: containers, `uses:` steps, matrices, services, caches,
or the full expression language. Steps that use an action are reported as
skipped; the working tree stands in for whatever they would have produced.
For real runner fidelity, `act` is the tool.

Nothing ever leaves your machine. `gh` is replaced with a shim that reports
the command it was handed and exits without calling GitHub, and `git` with one
that passes everything through except `push`. So a run that reaches the
publish step cannot touch a real release, and one that reaches the changelog
job cannot create a branch on the remote.

Local branches and worktrees that a run does create are removed afterwards,
the same way files it writes are.

One job at a time, named as the first argument; there is no `needs` between
them here, which is why each job in release-notes.yml resolves the tag itself.

Usage:
    .github/scripts/test-workflow.py notes                      # dispatch, newest release
    .github/scripts/test-workflow.py notes --tag v0.8.9         # dispatch, one tag
    .github/scripts/test-workflow.py notes --event push --tag v0.8.9
    .github/scripts/test-workflow.py notes --publish            # exercise the gh path
    .github/scripts/test-workflow.py notes --repository me/my-fork  # prove the fork guard
    .github/scripts/test-workflow.py changelog --event push --tag v0.8.9

Requires yq (brew install yq) to turn the workflow YAML into JSON.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DEFAULT_WORKFLOW = ".github/workflows/release-notes.yml"

# `${{ github.event_name }}` and friends. Only the contexts these workflows
# actually use are resolved; anything else is reported rather than guessed at,
# so the harness can never quietly test something other than what runs in CI.
EXPR_RE = re.compile(r"\$\{\{\s*(?P<expr>.+?)\s*\}\}")

GREEN, RED, YELLOW, DIM, RESET = "\033[32m", "\033[31m", "\033[33m", "\033[2m", "\033[0m"


def color(text: str, c: str) -> str:
    return f"{c}{text}{RESET}" if sys.stdout.isatty() else text


def repo_root() -> Path:
    out = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
        check=True,
    )
    return Path(out.stdout.strip())


def origin_slug(root: Path) -> str:
    """Best guess at owner/repo, so the default run matches the real one."""
    try:
        url = subprocess.run(
            ["git", "-C", str(root), "remote", "get-url", "origin"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
    except subprocess.CalledProcessError:
        return "owner/repo"
    match = re.search(r"[:/]([^/:]+/[^/]+?)(?:\.git)?$", url)
    return match.group(1) if match else "owner/repo"


def default_branch(root: Path) -> str:
    """The branch origin/HEAD points at, which is what GitHub calls default."""
    try:
        ref = subprocess.run(
            ["git", "-C", str(root), "symbolic-ref", "refs/remotes/origin/HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        return ref.rsplit("/", 1)[-1]
    except subprocess.CalledProcessError:
        return "main"


def load_workflow(path: Path) -> dict:
    """Parse the workflow via yq, which is the only external dependency."""
    if not shutil.which("yq"):
        sys.exit("yq not found. Install it (brew install yq) or use act instead.")
    out = subprocess.run(
        ["yq", "-o=json", str(path)], capture_output=True, text=True
    )
    if out.returncode != 0:
        sys.exit(f"yq could not parse {path}:\n{out.stderr.strip()}")
    return json.loads(out.stdout)


class Context:
    """The slice of the GitHub expression contexts these workflows read."""

    def __init__(
        self,
        event: str,
        repository: str,
        ref_name: str,
        inputs: dict,
        default_branch: str = "main",
    ):
        self.github = {
            "event_name": event,
            "repository": repository,
            "ref_name": ref_name,
            "ref_type": "tag" if event == "push" else "branch",
            "token": "fake-token-for-local-run",
            # The event payload, as far as these workflows read into it.
            "event": {"repository": {"default_branch": default_branch}},
        }
        self.inputs = inputs
        self.steps: dict[str, dict[str, str]] = {}

    def lookup(self, path: str):
        """Resolve a dotted context reference, e.g. steps.resolve.outputs.tag.

        Walks nested dicts, so a deeper payload path such as
        github.event.repository.default_branch resolves without a special
        case for every depth.
        """
        parts = path.split(".")
        roots = {"github": self.github, "inputs": self.inputs, "steps": self.steps}
        if parts[0] not in roots:
            raise KeyError(path)
        value = roots[parts[0]]
        for part in parts[1:]:
            if not isinstance(value, dict) or part not in value:
                # A step output read before that step has run is legitimately
                # empty. Only the documented 4-part shape is treated that way,
                # so a mistyped path still surfaces as unsupported rather than
                # silently resolving to "".
                if parts[0] == "steps" and len(parts) == 4 and parts[2] == "outputs":
                    return ""
                raise KeyError(path)
            value = value[part]
        return value

    def substitute(self, value: str) -> str:
        """Expand every ${{ ... }} in a string to its context value."""

        def replace(match: re.Match[str]) -> str:
            expr = match.group("expr")
            try:
                return str(self.lookup(expr))
            except KeyError:
                print(
                    color(f"    ! unsupported expression: {expr}", YELLOW),
                    file=sys.stderr,
                )
                return ""

        return EXPR_RE.sub(replace, value)

    def evaluate(self, condition: str) -> bool:
        """Evaluate an `if:` expression.

        Handles the subset these workflows use: context references, string
        literals, ==, !=, && , || and !. The expression is translated to
        Python and evaluated with no builtins in scope. Input comes from a
        workflow file in this repo, not from anything a third party controls.
        """
        expr = EXPR_RE.sub(lambda m: m.group("expr"), condition).strip()

        # Stash string literals before touching identifiers: without this the
        # identifier pass rewrites the *contents* of 'workflow_dispatch' and
        # 'sgtaziz/lian-li-linux', and every comparison silently becomes False.
        literals: list[str] = []

        def stash(match: re.Match[str]) -> str:
            literals.append(match.group(0))
            return f"__LIT{len(literals) - 1}__"

        expr = re.sub(r"'[^']*'|\"[^\"]*\"", stash, expr)

        def to_literal(match: re.Match[str]) -> str:
            path = match.group(0)
            if path.startswith("__LIT") or path in ("true", "false", "and", "or", "not"):
                return path
            try:
                value = self.lookup(path)
            except KeyError:
                print(color(f"    ! unsupported in if: {path}", YELLOW), file=sys.stderr)
                return "False"
            if isinstance(value, bool):
                return str(value)
            # An empty string is falsey in both languages, which is what an
            # unset input should be.
            return repr(str(value))

        expr = expr.replace("&&", " and ").replace("||", " or ")
        expr = re.sub(r"!(?!=)", " not ", expr)
        expr = re.sub(r"[A-Za-z_][A-Za-z0-9_.]*", to_literal, expr)
        expr = re.sub(r"__LIT(\d+)__", lambda m: literals[int(m.group(1))], expr)
        try:
            return bool(eval(expr, {"__builtins__": {}}, {}))  # noqa: S307
        except Exception as exc:  # pragma: no cover - surfaced to the user
            print(color(f"    ! could not evaluate: {condition} ({exc})", YELLOW))
            return False


def untracked(root: Path) -> set[str]:
    """Paths git currently considers untracked, as a set of repo-relative names."""
    # ls-files does not C-quote unusual paths the way `status --porcelain`
    # does, and -z makes the split unambiguous.
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--others", "--exclude-standard", "-z"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return {name for name in out.split("\0") if name}


def local_branches(root: Path) -> set[str]:
    """Local branch names, so a run's leftovers can be told from yours."""
    out = subprocess.run(
        ["git", "-C", str(root), "for-each-ref", "--format=%(refname:short)",
         "refs/heads"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return set(out.splitlines())


def worktree_branches(root: Path, workdir: Path) -> set[str]:
    """Branches checked out in worktrees this run created under `workdir`.

    Attribution by who made the branch, rather than by "it was not there when
    we started": a branch you create in another terminal while a run is going
    is not the harness's to delete. `git worktree list` still reports a
    worktree whose directory has already gone, which is what makes this
    readable after the temporary directory is removed and before the prune.
    """
    out = subprocess.run(
        ["git", "-C", str(root), "worktree", "list", "--porcelain"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    branches: set[str] = set()
    mine = False
    ours = workdir.resolve()
    for line in out.splitlines():
        if line.startswith("worktree "):
            path = Path(line[len("worktree ") :]).resolve()
            mine = path == ours or ours in path.parents
        elif line.startswith("branch ") and mine:
            branches.add(line[len("branch ") :].removeprefix("refs/heads/"))
    return branches


def make_gh_shim(bindir: Path) -> None:
    """Put a fake `gh` first on PATH so nothing can reach GitHub.

    It answers as though nothing exists yet — no release, no pull request — so
    the workflow takes its create branches, which are the interesting ones.
    The trace goes to stderr because the workflow captures `gh pr list` in a
    command substitution: on stdout it would read as "a pull request is
    already open" and the create path would never run.
    """
    shim = bindir / "gh"
    shim.write_text(
        "#!/usr/bin/env bash\n"
        'echo "[shim] gh $*" >&2\n'
        'if [ "$1" = "release" ] && [ "$2" = "view" ]; then\n'
        '  echo "[shim] pretending no release exists yet" >&2\n'
        "  exit 1\n"
        "fi\n"
        "exit 0\n"
    )
    shim.chmod(0o755)


def make_git_shim(bindir: Path) -> None:
    """Put a `git` first on PATH that will not push.

    Everything else runs for real: the workflow reads history, adds worktrees
    and makes a commit on a throwaway branch, and all of that has to work for
    the test to mean anything. Only the one irreversible, outward-facing verb
    is stubbed.
    """
    real = shutil.which("git")
    if real is None:  # pragma: no cover - git is required to get this far
        sys.exit("git is not on PATH")
    shim = bindir / "git"
    shim.write_text(
        "#!/usr/bin/env bash\n"
        "# Find the subcommand: the first argument that is neither a global\n"
        "# option nor the value of one, so `git -C dir push` is still a push.\n"
        "sub=\"\"\n"
        "skip=0\n"
        'for arg in "$@"; do\n'
        '  if [ "$skip" = 1 ]; then skip=0; continue; fi\n'
        '  case "$arg" in\n'
        "    -C|-c|--git-dir|--work-tree|--namespace|--exec-path) skip=1 ;;\n"
        "    -*) ;;\n"
        '    *) sub="$arg"; break ;;\n'
        "  esac\n"
        "done\n"
        'if [ "$sub" = "push" ]; then\n'
        '  echo "[shim] refused: git $*" >&2\n'
        "  exit 0\n"
        "fi\n"
        f'exec {real} "$@"\n'
    )
    shim.chmod(0o755)


def run_step(step: dict, ctx: Context, root: Path, workdir: Path) -> bool:
    """Execute one step. Returns False if it failed."""
    name = step.get("name") or step.get("uses") or "(unnamed)"

    if "uses" in step:
        print(color(f"  ~ {name}", DIM))
        print(color("    skipped: action steps are not run locally", DIM))
        return True

    if "if" in step and not ctx.evaluate(str(step["if"])):
        print(color(f"  ~ {name}", DIM))
        print(color(f"    skipped by if: {step['if']}", DIM))
        return True

    out_file = workdir / "step_output"
    summary_file = workdir / "step_summary"
    runner_temp = workdir / "runner-temp"
    runner_temp.mkdir(exist_ok=True)
    out_file.write_text("")

    env = dict(os.environ)
    env["PATH"] = f"{workdir / 'bin'}:{env['PATH']}"
    env.update(
        {
            "GITHUB_OUTPUT": str(out_file),
            "GITHUB_STEP_SUMMARY": str(summary_file),
            "GITHUB_REF_NAME": ctx.github["ref_name"],
            "GITHUB_REPOSITORY": ctx.github["repository"],
            "GITHUB_EVENT_NAME": ctx.github["event_name"],
            "GITHUB_WORKSPACE": str(root),
            # The changelog job puts its worktrees and its generated file
            # here. Pointing it at the throwaway directory is what keeps them
            # out of the repository.
            "RUNNER_TEMP": str(runner_temp),
            "CI": "true",
        }
    )
    for key, value in (step.get("env") or {}).items():
        env[key] = ctx.substitute(str(value))

    print(color(f"  > {name}", GREEN))
    proc = subprocess.run(
        ["bash", "-e", "-o", "pipefail", "-c", step["run"]],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
    )
    for line in (proc.stdout + proc.stderr).splitlines():
        print(f"    {line}")

    # Feed `key=value` lines back into the steps context for later steps.
    step_id = step.get("id")
    if step_id:
        outputs = {}
        for line in out_file.read_text().splitlines():
            if "=" in line:
                key, _, value = line.partition("=")
                outputs[key] = value
        if outputs:
            # Stored under an "outputs" key because that is the shape the
            # expression `steps.<id>.outputs.<name>` walks. Flattening it here
            # makes every such reference resolve to "" instead of failing.
            ctx.steps[step_id] = {"outputs": outputs}
            print(color(f"    outputs: {outputs}", DIM))

    if proc.returncode != 0:
        print(color(f"    FAILED (exit {proc.returncode})", RED))
        return False
    return True


def main() -> int:
    root = repo_root()
    parser = argparse.ArgumentParser(
        description="Run a workflow's shell steps locally.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        # Keeps the docstring's own indentation, and its heading: .strip()
        # alone dedents the first example and leaves the others sitting four
        # spaces in, which is what RawDescriptionHelpFormatter then prints.
        epilog="Usage:" + __doc__.split("Usage:")[1].split("Requires yq")[0].rstrip(),
    )
    parser.add_argument(
        "-w", "--workflow", default=DEFAULT_WORKFLOW, help=f"default: {DEFAULT_WORKFLOW}"
    )
    parser.add_argument(
        "job", nargs="?", help="job to run (default: the only one in the workflow)"
    )
    parser.add_argument(
        "--event",
        default="workflow_dispatch",
        choices=["workflow_dispatch", "push"],
        help="event to simulate (default: workflow_dispatch)",
    )
    parser.add_argument("--tag", default="", help="tag input, or the pushed tag")
    parser.add_argument(
        "--publish", action="store_true", help="set the publish input to true"
    )
    parser.add_argument(
        "--repository",
        default=origin_slug(root),
        help="owner/repo to run as; change it to test a repository guard",
    )
    parser.add_argument(
        "--default-branch",
        default=default_branch(root),
        help="branch a release is allowed to come from (default: origin/HEAD)",
    )
    args = parser.parse_args()

    workflow_path = root / args.workflow
    if not workflow_path.exists():
        sys.exit(f"No such workflow: {workflow_path}")
    workflow = load_workflow(workflow_path)

    jobs = workflow.get("jobs", {})
    job_id = args.job or (list(jobs)[0] if len(jobs) == 1 else None)
    if job_id not in jobs:
        named = f"No such job: {job_id}. " if args.job else "Name a job. "
        sys.exit(f"{named}Available: {', '.join(jobs)}")
    job = jobs[job_id]

    ctx = Context(
        event=args.event,
        repository=args.repository,
        # On a tag push the ref name IS the tag, which is what the workflow reads.
        ref_name=args.tag if args.event == "push" else "main",
        inputs={"tag": args.tag, "publish": args.publish},
        default_branch=args.default_branch,
    )

    print(f"workflow : {args.workflow}")
    print(f"job      : {job_id}")
    print(f"event    : {args.event}")
    print(f"repo     : {args.repository}")
    print(f"branch   : {args.default_branch}")
    print(f"inputs   : tag={args.tag!r} publish={args.publish}")
    print()

    if "if" in job and not ctx.evaluate(str(job["if"])):
        print(color(f"job skipped by its if: {job['if']}", YELLOW))
        print(color("(this is a result, not a failure)", DIM))
        return 0

    # A runner works in a throwaway checkout; here the steps run against your
    # real tree, so anything they write (notes.md, for one) would be left
    # behind. Snapshot first and remove only what this run actually created —
    # never a file that was already sitting there untracked.
    before = untracked(root)
    branches_before = local_branches(root)
    failed = False
    # Bound outside the `with` so the cleanup below can still name the
    # directory the run worked in after it has been removed.
    workdir: Path | None = None

    try:
        with tempfile.TemporaryDirectory(prefix="test-workflow-") as tmp:
            workdir = Path(tmp)
            bindir = workdir / "bin"
            bindir.mkdir()
            make_gh_shim(bindir)
            make_git_shim(bindir)

            for step in job.get("steps", []):
                if not run_step(step, ctx, root, workdir):
                    failed = True
                    break

            summary = workdir / "step_summary"
            if not failed and summary.exists() and summary.read_text().strip():
                print()
                print(color("--- job summary ---", DIM))
                print(summary.read_text().rstrip())
    finally:
        created = sorted(untracked(root) - before)
        for name in created:
            path = root / name
            if path.is_file():
                path.unlink()

        # The worktrees themselves went with the temporary directory; this
        # clears the administrative files git keeps for them. The branch the
        # changelog job cuts is real and local, so it has to go too — but only
        # a branch one of this run's own worktrees created, and only if it was
        # not already there beforehand. Read before the prune, which is what
        # forgets the worktrees this asks about.
        mine = worktree_branches(root, workdir) if workdir else set()
        subprocess.run(["git", "-C", str(root), "worktree", "prune"], check=False)
        new_branches = sorted(mine & (local_branches(root) - branches_before))
        for branch in new_branches:
            subprocess.run(
                ["git", "-C", str(root), "branch", "-D", branch],
                check=False,
                capture_output=True,
            )

        leftovers = created + new_branches
        if leftovers:
            print()
            print(color(f"cleaned up: {', '.join(leftovers)}", DIM))

    print()
    if failed:
        print(color("workflow FAILED", RED))
        return 1
    print(color("workflow passed", GREEN))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
