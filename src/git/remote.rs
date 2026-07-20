//! Fixed-argv construction of the sanctioned remote operations
//! (fetch / pull / push, plus push's publish flavor for unpublished
//! branches).
//!
//! Security is enforced *by construction* here, not by discipline at the call
//! site: [`RemoteOp`] is a closed enum whose only argument vector comes from
//! [`RemoteOp::args`], a hard-coded `&'static [&'static str]`. There is no way
//! to attach an extra flag (so `--force` can never appear), no shell is ever
//! invoked (the child is spawned via [`std::process::Command`] with an
//! explicit argv), and no user-controlled string is interpolated. Every child
//! runs with `GIT_TERMINAL_PROMPT=0` so a credential prompt fails fast in the
//! background instead of blocking a worker thread on a read that never returns.

use std::path::Path;
use std::process::Command;

/// One of the sanctioned remote operations.
///
/// Deliberately a closed enum with no payload: an operation cannot carry
/// caller-supplied arguments, so no variant can smuggle in `--force` or any
/// other flag. The argument vector is fixed per variant (see
/// [`RemoteOp::args`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOp {
    /// `git fetch` — update remote-tracking refs without touching the tree.
    Fetch,
    /// `git pull` — fetch then integrate (fast-forward or merge).
    Pull,
    /// `git push` — publish local commits to the upstream (never `--force`).
    Push,
    /// `git push --set-upstream origin HEAD` — first push of a branch with no
    /// upstream configured, creating the same-named remote branch and setting
    /// it as upstream. `HEAD` is git's own refspec for the current branch, so
    /// the argv stays fixed — no branch name is ever interpolated — and it is
    /// still never `--force`.
    Publish,
}

impl RemoteOp {
    /// The fixed git argument vector for this operation. Hard-coded and
    /// argument-free by design: never `--force`, never anything caller-supplied.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            RemoteOp::Fetch => &["fetch"],
            RemoteOp::Pull => &["pull"],
            RemoteOp::Push => &["push"],
            RemoteOp::Publish => &["push", "--set-upstream", "origin", "HEAD"],
        }
    }

    /// A short human-readable label (`"fetch"`, `"pull"`, `"push"`,
    /// `"publish"`) for the running indicator and the completion summary.
    pub fn label(self) -> &'static str {
        match self {
            RemoteOp::Fetch => "fetch",
            RemoteOp::Pull => "pull",
            RemoteOp::Push => "push",
            RemoteOp::Publish => "publish",
        }
    }

    /// The full command line as shown in the command log, e.g. `git fetch`.
    pub fn command_line(self) -> String {
        format!("git {}", self.args().join(" "))
    }
}

/// Builds the `git` [`Command`] for `op`, to be run at the repository `root`.
///
/// The command is a fixed argument vector (no shell, no user-controlled
/// interpolation), never carries `--force`, and sets `GIT_TERMINAL_PROMPT=0`
/// in the child environment so a credential prompt fails fast in the
/// background rather than hanging the worker thread on a blocked read.
pub fn remote_command(op: RemoteOp, root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root);
    cmd.args(op.args());
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

// -- PR/MR head-ref fetch -------------------------------------------------
//
// A second, narrower write ceiling lives alongside the plain remote ops
// above: fetching a PR/MR's head commit into a redquill-managed branch
// needs a *forced* ref update (the PR author can rewrite history), which
// `RemoteOp` deliberately can never express. [`PrRef`] is the closed type
// that confines forcing to exactly one namespace.

/// The short-name prefix every redquill-managed PR/MR review branch lives
/// under (`redquill/pr/<n>`). The only namespace [`PrRef`]'s forced fetch
/// and [`delete_managed_pr_branch_command`] are ever able to write to.
pub const MANAGED_PR_BRANCH_PREFIX: &str = "redquill/pr";

/// Which forge's special-ref naming convention names a PR/MR's head commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrRefKind {
    /// GitHub: `refs/pull/<n>/head`.
    GitHub,
    /// GitLab: `refs/merge-requests/<n>/head`.
    GitLab,
}

/// A PR/MR head reference: a provider's special-ref pattern plus an integer
/// PR/MR number — never a caller-supplied ref string.
///
/// Every refspec this type can produce names a source ref built purely from
/// `kind` and `number` (see [`PrRef::source_ref`]) on one side, and
/// `refs/heads/redquill/pr/<number>` (see [`PrRef::managed_ref`]) on the
/// other. There is no constructor or method on this type that accepts an
/// arbitrary ref string, so no `PrRef` value can ever produce a forced
/// refspec — or a branch delete, see [`delete_managed_pr_branch_command`] —
/// naming anything outside that one managed branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrRef {
    kind: PrRefKind,
    number: u64,
}

impl PrRef {
    pub fn new(kind: PrRefKind, number: u64) -> Self {
        PrRef { kind, number }
    }

    /// The PR/MR number this ref targets.
    pub fn number(&self) -> u64 {
        self.number
    }

    /// The provider's special ref that resolves to this PR/MR's head
    /// commit (e.g. `refs/pull/42/head`).
    pub fn source_ref(&self) -> String {
        match self.kind {
            PrRefKind::GitHub => format!("refs/pull/{}/head", self.number),
            PrRefKind::GitLab => format!("refs/merge-requests/{}/head", self.number),
        }
    }

    /// The managed branch's short name (`redquill/pr/<n>`).
    pub fn managed_branch(&self) -> String {
        format!("{MANAGED_PR_BRANCH_PREFIX}/{}", self.number)
    }

    /// The managed branch's full ref (`refs/heads/redquill/pr/<n>`).
    pub fn managed_ref(&self) -> String {
        format!("refs/heads/{}", self.managed_branch())
    }

    /// The forced refspec (`+<source>:<managed>`) fetched for this PR's
    /// head — forced so a re-fetch after the author rewrites history (a
    /// force-push, a rebase) updates the managed branch in place instead of
    /// failing non-fast-forward. Always writes exactly
    /// [`PrRef::managed_ref`]; see the type doc for why no other ref is
    /// reachable.
    fn forced_refspec(&self) -> String {
        format!("+{}:{}", self.source_ref(), self.managed_ref())
    }
}

/// Builds `git fetch origin +<special-ref>:refs/heads/redquill/pr/<n>` for
/// `pr_ref` — the only forced fetch this codebase ever runs, and
/// structurally confined to the `redquill/pr/` namespace by [`PrRef`]
/// itself (see its doc). The refspec sits after a `--` separator so it can
/// never be misread as an option. `GIT_TERMINAL_PROMPT=0`, matching every
/// other git invocation.
pub fn pr_fetch_command(pr_ref: &PrRef, root: &Path) -> Command {
    let refspec = pr_ref.forced_refspec();
    let mut cmd = Command::new("git");
    cmd.current_dir(root);
    cmd.args(["fetch", "origin", "--", refspec.as_str()]);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// Builds a plain (never forced) `git fetch origin <base>:refs/remotes/origin/<base>`,
/// so `origin/<base>` resolves for the PR's diff base (e.g.
/// `DiffTarget::Review`'s `base...branch`). `base` is placed after a `--`
/// separator so a base name that happens to start with `-` can never be
/// read as an option. `GIT_TERMINAL_PROMPT=0`, matching every other git
/// invocation.
pub fn base_fetch_command(base: &str, root: &Path) -> Command {
    let refspec = format!("{base}:refs/remotes/origin/{base}");
    let mut cmd = Command::new("git");
    cmd.current_dir(root);
    cmd.args(["fetch", "origin", "--", refspec.as_str()]);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// Builds `git branch -D redquill/pr/<n>` — force-delete, because a managed
/// PR branch is unrelated history fetched from another repo and will
/// essentially never be "merged" from this repo's perspective, so a plain
/// `-d` would always refuse. Force is safe here only because deletion is
/// structurally confined to the `redquill/pr/` namespace: the branch name
/// is built from `number` (a `u64`) through [`MANAGED_PR_BRANCH_PREFIX`],
/// never a caller-supplied string, so this can never delete anything
/// outside that namespace.
pub fn delete_managed_pr_branch_command(number: u64, root: &Path) -> Command {
    let branch = format!("{MANAGED_PR_BRANCH_PREFIX}/{number}");
    let mut cmd = Command::new("git");
    cmd.current_dir(root);
    cmd.args(["branch", "-D", branch.as_str()]);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    const ALL_OPS: [RemoteOp; 4] = [
        RemoteOp::Fetch,
        RemoteOp::Pull,
        RemoteOp::Push,
        RemoteOp::Publish,
    ];

    #[test]
    fn each_variant_has_its_fixed_argv() {
        assert_eq!(RemoteOp::Fetch.args(), &["fetch"]);
        assert_eq!(RemoteOp::Pull.args(), &["pull"]);
        assert_eq!(RemoteOp::Push.args(), &["push"]);
        assert_eq!(
            RemoteOp::Publish.args(),
            &["push", "--set-upstream", "origin", "HEAD"]
        );
    }

    #[test]
    fn labels_and_command_lines_are_plain_git_invocations() {
        assert_eq!(RemoteOp::Fetch.label(), "fetch");
        assert_eq!(RemoteOp::Pull.label(), "pull");
        assert_eq!(RemoteOp::Push.label(), "push");
        assert_eq!(RemoteOp::Publish.label(), "publish");
        assert_eq!(RemoteOp::Fetch.command_line(), "git fetch");
        assert_eq!(RemoteOp::Pull.command_line(), "git pull");
        assert_eq!(RemoteOp::Push.command_line(), "git push");
        assert_eq!(
            RemoteOp::Publish.command_line(),
            "git push --set-upstream origin HEAD"
        );
    }

    #[test]
    fn no_variant_can_carry_force_or_a_force_refspec() {
        for op in ALL_OPS {
            let args = op.args();
            // The argv is fixed per variant (see each_variant_has_its_fixed_argv);
            // this pins the security property directly: no force flag and no
            // `+`-prefixed (force-push) refspec can ever appear.
            assert!(
                !args
                    .iter()
                    .any(|a| a.contains("force") || *a == "-f" || a.starts_with('+')),
                "{op:?} args must never force: {args:?}"
            );
        }
    }

    #[test]
    fn remote_command_spawns_git_with_the_fixed_argv_at_root() {
        let root = PathBuf::from("/tmp/redquill-remote-test");
        let cmd = remote_command(RemoteOp::Push, &root);
        assert_eq!(cmd.get_program(), OsStr::new("git"));
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(args, vec![OsStr::new("push")]);
        assert_eq!(cmd.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn remote_command_disables_the_terminal_prompt() {
        let cmd = remote_command(RemoteOp::Fetch, Path::new("."));
        let prompt = cmd
            .get_envs()
            .find(|(k, _)| *k == OsStr::new("GIT_TERMINAL_PROMPT"))
            .and_then(|(_, v)| v);
        assert_eq!(prompt, Some(OsStr::new("0")));
    }

    #[test]
    fn remote_command_never_sets_a_force_argument() {
        for op in ALL_OPS {
            let cmd = remote_command(op, Path::new("."));
            let has_force = cmd
                .get_args()
                .any(|a| a == OsStr::new("--force") || a == OsStr::new("-f"));
            assert!(!has_force, "{op:?} must never pass a force flag");
        }
    }

    // -- PrRef / pr_fetch_command --------------------------------------------

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn source_ref_follows_each_providers_special_ref_pattern() {
        let gh = PrRef::new(PrRefKind::GitHub, 42);
        assert_eq!(gh.source_ref(), "refs/pull/42/head");
        let gl = PrRef::new(PrRefKind::GitLab, 7);
        assert_eq!(gl.source_ref(), "refs/merge-requests/7/head");
    }

    #[test]
    fn managed_branch_and_ref_are_scoped_to_the_prefix_for_any_kind_or_number() {
        for kind in [PrRefKind::GitHub, PrRefKind::GitLab] {
            for number in [1_u64, 42, 100_000] {
                let pr_ref = PrRef::new(kind, number);
                assert_eq!(pr_ref.managed_branch(), format!("redquill/pr/{number}"));
                assert_eq!(
                    pr_ref.managed_ref(),
                    format!("refs/heads/redquill/pr/{number}")
                );
            }
        }
    }

    #[test]
    fn pr_fetch_command_argv_is_fixed_shape() {
        let pr_ref = PrRef::new(PrRefKind::GitHub, 1);
        let cmd = pr_fetch_command(&pr_ref, Path::new("/repo"));
        assert_eq!(cmd.get_program(), OsStr::new("git"));
        assert_eq!(
            args_of(&cmd),
            vec![
                "fetch",
                "origin",
                "--",
                "+refs/pull/1/head:refs/heads/redquill/pr/1",
            ]
        );
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/repo")));
    }

    #[test]
    fn pr_fetch_command_forced_refspec_only_ever_names_the_managed_branch() {
        // Property check across many kinds/numbers: the destination side of
        // the forced refspec is always exactly `refs/heads/redquill/pr/<n>`
        // — never any other ref — and the source side is always that
        // provider's special ref for the same number, never anything
        // caller-supplied.
        for kind in [PrRefKind::GitHub, PrRefKind::GitLab] {
            for number in [1_u64, 2, 999, 123_456] {
                let pr_ref = PrRef::new(kind, number);
                let cmd = pr_fetch_command(&pr_ref, Path::new("."));
                let args = args_of(&cmd);
                let refspec = args.last().expect("refspec arg present");
                let expected_source = match kind {
                    PrRefKind::GitHub => format!("refs/pull/{number}/head"),
                    PrRefKind::GitLab => format!("refs/merge-requests/{number}/head"),
                };
                let expected = format!("+{expected_source}:refs/heads/redquill/pr/{number}");
                assert_eq!(refspec, &expected);
                assert!(refspec.starts_with('+'), "the head fetch must be forced");
                assert!(
                    refspec.ends_with(&format!("refs/heads/redquill/pr/{number}")),
                    "forced refspec must write only the managed branch: {refspec:?}"
                );
            }
        }
    }

    #[test]
    fn pr_fetch_command_disables_the_terminal_prompt() {
        let pr_ref = PrRef::new(PrRefKind::GitHub, 1);
        let cmd = pr_fetch_command(&pr_ref, Path::new("."));
        let prompt = cmd
            .get_envs()
            .find(|(k, _)| *k == OsStr::new("GIT_TERMINAL_PROMPT"))
            .and_then(|(_, v)| v);
        assert_eq!(prompt, Some(OsStr::new("0")));
    }

    // -- base_fetch_command ---------------------------------------------------

    #[test]
    fn base_fetch_command_argv_is_fixed_shape() {
        let cmd = base_fetch_command("main", Path::new("/repo"));
        assert_eq!(cmd.get_program(), OsStr::new("git"));
        assert_eq!(
            args_of(&cmd),
            vec!["fetch", "origin", "--", "main:refs/remotes/origin/main"]
        );
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/repo")));
    }

    #[test]
    fn base_fetch_command_is_never_forced() {
        let cmd = base_fetch_command("main", Path::new("."));
        let has_force = args_of(&cmd)
            .iter()
            .any(|a| a.starts_with('+') || a == "--force" || a == "-f");
        assert!(!has_force, "base fetch must never be forced");
    }

    #[test]
    fn base_fetch_command_places_the_refspec_after_a_separator_guarding_option_injection() {
        // A base name beginning with `-` must never be readable as an
        // option: the `--` separator (present in the fixed argv shape) is
        // what guarantees that, independent of the base name's content.
        let cmd = base_fetch_command("--upload-pack=evil", Path::new("."));
        let args = args_of(&cmd);
        let sep_idx = args.iter().position(|a| a == "--").expect("-- present");
        assert_eq!(sep_idx, 2, "the -- separator must precede the refspec");
        assert_eq!(args.len(), 4, "refspec must be the only arg after --");
    }

    #[test]
    fn base_fetch_command_disables_the_terminal_prompt() {
        let cmd = base_fetch_command("main", Path::new("."));
        let prompt = cmd
            .get_envs()
            .find(|(k, _)| *k == OsStr::new("GIT_TERMINAL_PROMPT"))
            .and_then(|(_, v)| v);
        assert_eq!(prompt, Some(OsStr::new("0")));
    }

    // -- delete_managed_pr_branch_command ---------------------------------------

    #[test]
    fn delete_managed_pr_branch_command_argv_is_fixed_shape() {
        let cmd = delete_managed_pr_branch_command(1, Path::new("/repo"));
        assert_eq!(cmd.get_program(), OsStr::new("git"));
        assert_eq!(args_of(&cmd), vec!["branch", "-D", "redquill/pr/1"]);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/repo")));
    }

    #[test]
    fn delete_managed_pr_branch_command_only_ever_names_the_managed_prefix() {
        for number in [1_u64, 42, 999_999] {
            let cmd = delete_managed_pr_branch_command(number, Path::new("."));
            let args = args_of(&cmd);
            let branch = args.last().expect("branch arg present");
            assert_eq!(branch, &format!("redquill/pr/{number}"));
            assert!(branch.starts_with("redquill/pr/"));
        }
    }

    #[test]
    fn delete_managed_pr_branch_command_disables_the_terminal_prompt() {
        let cmd = delete_managed_pr_branch_command(1, Path::new("."));
        let prompt = cmd
            .get_envs()
            .find(|(k, _)| *k == OsStr::new("GIT_TERMINAL_PROMPT"))
            .and_then(|(_, v)| v);
        assert_eq!(prompt, Some(OsStr::new("0")));
    }
}
