//! Fixed-argv construction of the three sanctioned remote operations
//! (fetch / pull / push).
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

/// One of the three sanctioned remote operations.
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
}

impl RemoteOp {
    /// The fixed git argument vector for this operation. Hard-coded and
    /// argument-free by design: never `--force`, never anything caller-supplied.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            RemoteOp::Fetch => &["fetch"],
            RemoteOp::Pull => &["pull"],
            RemoteOp::Push => &["push"],
        }
    }

    /// A short human-readable label (`"fetch"`, `"pull"`, `"push"`) for the
    /// running indicator and the completion summary.
    pub fn label(self) -> &'static str {
        self.args()[0]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn each_variant_has_its_fixed_argv() {
        assert_eq!(RemoteOp::Fetch.args(), &["fetch"]);
        assert_eq!(RemoteOp::Pull.args(), &["pull"]);
        assert_eq!(RemoteOp::Push.args(), &["push"]);
    }

    #[test]
    fn labels_and_command_lines_are_plain_git_invocations() {
        assert_eq!(RemoteOp::Fetch.label(), "fetch");
        assert_eq!(RemoteOp::Pull.label(), "pull");
        assert_eq!(RemoteOp::Push.label(), "push");
        assert_eq!(RemoteOp::Fetch.command_line(), "git fetch");
        assert_eq!(RemoteOp::Pull.command_line(), "git pull");
        assert_eq!(RemoteOp::Push.command_line(), "git push");
    }

    #[test]
    fn no_variant_can_carry_force_or_any_extra_flag() {
        for op in [RemoteOp::Fetch, RemoteOp::Pull, RemoteOp::Push] {
            let args = op.args();
            // Exactly the subcommand, nothing else — so `--force`, `-f`,
            // `+ref` refspecs, and every other flag are impossible.
            assert_eq!(args.len(), 1, "{op:?} must carry exactly one arg");
            assert!(
                !args
                    .iter()
                    .any(|a| a.contains("force") || a.starts_with('-')),
                "{op:?} args must never contain a flag: {args:?}"
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
        for op in [RemoteOp::Fetch, RemoteOp::Pull, RemoteOp::Push] {
            let cmd = remote_command(op, Path::new("."));
            let has_force = cmd
                .get_args()
                .any(|a| a == OsStr::new("--force") || a == OsStr::new("-f"));
            assert!(!has_force, "{op:?} must never pass a force flag");
        }
    }
}
