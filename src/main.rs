//! CLI entry point: argument parsing and wiring for redquill.
//!
//! Owns the working-tree default, `--staged`, ref-range, and `-o file`
//! flags described in the README, and wires parsed args into the rest of
//! the crate.
//!
//! If stderr is a terminal, launches the interactive TUI (which renders to
//! stderr, keeping stdout free for annotation markdown emitted on quit). If
//! stderr is not a terminal — e.g. piped, or running under a test harness —
//! falls back to the plain-text summary so redquill stays scriptable.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use clap::Parser;

use redquill::annotate::render_markdown;
use redquill::config;
use redquill::diff::FileDiff;
use redquill::git::{ChangeKind, DiffTarget, GitRunner, sanitize_branch_dir_name};
use redquill::ui::{
    self, App, EditorConfigTier, EditorLaunch, QuitOutcome, build_review,
    resolve_editor_config_tier,
};

/// redquill: a terminal UI for reviewing agentic code changes.
///
/// Reviews the working tree by default. Pass a ref range to review a
/// commit range instead, or `--staged` to review the index.
#[derive(Parser, Debug)]
#[command(name = "redquill", version, about, long_about = None)]
struct Cli {
    /// Ref range to review (e.g. `main..HEAD`). Defaults to the working
    /// tree when omitted. Mutually exclusive with `--staged` and `--review`.
    #[arg(conflicts_with_all = ["staged", "review"])]
    range: Option<String>,

    /// Review the staged index instead of the working tree. Mutually
    /// exclusive with `--review`.
    #[arg(long, conflicts_with = "review")]
    staged: bool,

    /// Review a local branch (spec 08 Unit 1) inside its own managed
    /// worktree instead of the working tree. redquill creates (or reuses) a
    /// worktree at `<git-common-dir>/redquill/worktrees/<sanitized-branch>`,
    /// then shows the `base...branch` (three-dot) diff rooted there, so LSP
    /// navigation and `g<Space>` operate on the branch's real files.
    /// Mutually exclusive with the positional range and `--staged`.
    #[arg(long, value_name = "BRANCH", conflicts_with_all = ["staged", "range"])]
    review: Option<String>,

    /// Base ref for `--review`'s three-dot diff. Defaults to the branch
    /// `origin/HEAD` points to, else `main`, else `master`.
    #[arg(long, value_name = "REF", requires = "review")]
    base: Option<String>,

    /// Also write emitted annotations to this file, in addition to stdout.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: Option<PathBuf>,

    /// Editor `g<Space>` opens in the diff viewer, e.g. `"nvim"` or `"code
    /// --wait"`. Overrides `$VISUAL`/`$EDITOR`; falls back to `nvim` if none
    /// of the three are set.
    #[arg(long, value_name = "CMD")]
    editor: Option<String>,
}

/// Fully resolved configuration derived from parsed CLI arguments.
struct RunConfig {
    /// Ref range to diff, if any; `None` means the working tree.
    range: Option<String>,
    /// Whether to review the staged index instead of the working tree.
    staged: bool,
    /// `--review <branch>`, if passed: start a branch review session
    /// instead of any of the above (see [`Cli::review`]).
    review: Option<String>,
    /// `--base <ref>`, if passed: overrides `--review`'s default base
    /// resolution (see [`GitRunner::default_base`]).
    base: Option<String>,
    /// Optional file to additionally write annotations to, alongside stdout.
    output: Option<PathBuf>,
    /// The `--editor` flag, if passed; highest-precedence tier of
    /// [`resolve_editor`].
    editor: Option<String>,
}

impl From<Cli> for RunConfig {
    fn from(cli: Cli) -> Self {
        RunConfig {
            range: cli.range,
            staged: cli.staged,
            review: cli.review,
            base: cli.base,
            output: cli.output,
            editor: cli.editor,
        }
    }
}

impl RunConfig {
    /// Resolves the non-review CLI flags into the diff target to inspect.
    /// Never called when `--review` is set — [`resolve_session`] branches
    /// on that before this is reached, since a review target additionally
    /// requires resolving a base ref and ensuring a worktree exists (I/O
    /// this pure function deliberately doesn't do).
    fn diff_target(&self) -> DiffTarget {
        match &self.range {
            Some(range) => DiffTarget::Range(range.clone()),
            None if self.staged => DiffTarget::Staged,
            None => DiffTarget::WorkingTree,
        }
    }
}

/// Resolves the git runner and diff target this invocation should operate
/// against, from the repo `discovered` at the caller's cwd. Ordinary flags
/// (working tree / `--staged` / a range) resolve immediately, reusing
/// `discovered` as-is. `--review <branch>` additionally: resolves the base
/// ref (`--base`, else [`GitRunner::default_base`]'s fallback chain),
/// ensures the branch's managed worktree exists (creating it via
/// [`GitRunner::worktree_add`], or reusing one a paused review already
/// created), and returns a runner *discovered inside that worktree* — so
/// every subsequent git call this session makes (diff, LSP root, `g<Space>`)
/// is truthfully rooted there rather than in the user's own checkout.
///
/// Every failure path (unresolved base, unknown branch, branch already
/// checked out elsewhere) returns before any worktree is created, and
/// `worktree_add` itself never retries with `--force` — so a failure here
/// leaves the user's checkout, index, and HEAD, and the filesystem, exactly
/// as they were.
fn resolve_session(
    discovered: GitRunner,
    config: &RunConfig,
) -> anyhow::Result<(GitRunner, DiffTarget)> {
    let Some(branch) = &config.review else {
        return Ok((discovered, config.diff_target()));
    };

    let base = match &config.base {
        Some(base) => base.clone(),
        None => discovered.default_base()?,
    };

    let worktree_path = ensure_review_worktree(&discovered, branch)?;
    let session_runner = GitRunner::discover_in(&worktree_path)?;

    Ok((
        session_runner,
        DiffTarget::Review {
            base,
            branch: branch.clone(),
        },
    ))
}

/// Ensures a managed worktree exists for `branch`, at
/// `<git-common-dir>/redquill/worktrees/<sanitized-branch>`, and returns its
/// path. Reuses an existing worktree (a paused review) rather than creating
/// a new one when `git worktree list` already knows about the path;
/// otherwise creates it with [`GitRunner::worktree_add`], which surfaces
/// git's own error message (unknown branch, branch checked out elsewhere,
/// ...) without side effects on failure.
fn ensure_review_worktree(runner: &GitRunner, branch: &str) -> anyhow::Result<PathBuf> {
    let common_dir = runner.git_common_dir()?;
    let dir_name = sanitize_branch_dir_name(branch);
    let worktree_path = common_dir.join("redquill").join("worktrees").join(dir_name);

    let already_registered = worktree_path.exists()
        && runner
            .worktree_list()?
            .iter()
            .any(|entry| paths_match(&entry.path, &worktree_path));

    if !already_registered {
        runner.worktree_add(&worktree_path, branch)?;
    }

    Ok(worktree_path)
}

/// Whether `a` and `b` name the same filesystem location, canonicalizing
/// both when possible (falling back to a direct comparison for a path that
/// doesn't exist, e.g. before the first `worktree add`) — macOS tempdirs
/// live under a symlinked root, so a raw `PathBuf` comparison can spuriously
/// disagree with what `git worktree list` reports.
fn paths_match(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Prints a one-line summary per changed file for the resolved diff target.
fn run(config: &RunConfig) -> anyhow::Result<()> {
    let discovered = GitRunner::discover()?;
    let (runner, target) = resolve_session(discovered, config)?;
    let patches = runner.diff(&target)?;

    let mut printed = 0usize;
    for patch in &patches {
        let binary = if patch.is_binary { " (binary)" } else { "" };
        let diff = FileDiff::from_patch(patch)?;
        match &patch.old_path {
            Some(old) => println!("R  {old} -> {}{binary}", patch.path),
            None => println!("{}  {}{binary}", diff.kind.letter(), patch.path),
        }
        printed += 1;
    }

    // `git diff` never surfaces untracked files; pull them from status so the
    // working-tree summary is complete.
    if matches!(target, DiffTarget::WorkingTree) {
        for entry in runner.status()? {
            if entry.kind == ChangeKind::Untracked {
                println!("?  {}", entry.path);
                printed += 1;
            }
        }
    }

    if printed == 0 {
        println!("no changes");
    }
    Ok(())
}

/// Resolves the editor `g<Space>` opens, in five-tier precedence order: the
/// `--editor` flag, then `[editor]` config (`config_tier`, already resolved
/// from `crate::config::EditorConfig` by
/// `ui::resolve_editor_config_tier` — an `EditorConfigTier::UnknownPreset`
/// is *not* a config-tier hit here; `run_tui` reports it as a warning and
/// passes `EditorConfigTier::Absent` in its place instead, so this function
/// only ever sees a real hit or a miss), then `$VISUAL`, then `$EDITOR`,
/// then `"nvim"`. Takes every tier as an explicit arg (rather than reading
/// `std::env::var`/resolving config itself) so precedence is unit-testable
/// without mutating process-global env state or touching real config paths;
/// `run_tui` reads the real env vars and config at the one call site. Empty
/// or whitespace-only strings at the flag/`$VISUAL`/`$EDITOR` tiers are
/// treated as unset and fall through to the next — an exported `EDITOR=""`
/// shouldn't silently break `g<Space>` (unchanged from before this spec). A
/// config-tier template always wins over `$VISUAL`/`$EDITOR`/`"nvim"` and
/// carries no such "empty is unset" allowance: it's already been validated
/// non-empty by `crate::config::EditorConfig::from_value`.
fn resolve_editor(
    flag: Option<String>,
    config_tier: EditorConfigTier,
    visual: Option<String>,
    editor_env: Option<String>,
) -> EditorLaunch {
    if let Some(cmd) = flag.filter(|s| !s.trim().is_empty()) {
        return EditorLaunch::Command(cmd);
    }
    if let EditorConfigTier::Template(template) = config_tier {
        return EditorLaunch::Template(template);
    }
    [visual, editor_env]
        .into_iter()
        .find_map(|candidate| candidate.filter(|s| !s.trim().is_empty()))
        .map(EditorLaunch::Command)
        .unwrap_or_else(|| EditorLaunch::Command("nvim".to_string()))
}

/// Runs the interactive TUI and, on quit, emits annotations per the
/// resolved [`QuitOutcome`].
fn run_tui(config: &RunConfig) -> anyhow::Result<()> {
    let discovered = GitRunner::discover()?;
    // `discovered` is rooted at the caller's cwd (the user's own checkout),
    // *outside* any managed review worktree `resolve_session` might create —
    // kept alive (cloned into `resolve_session`) so a review session's
    // finish gesture (spec 08 Unit 2) can remove that worktree through a
    // backend that isn't rooted inside the very directory being removed.
    let (runner, target) = resolve_session(discovered.clone(), config)?;
    let snapshot = build_review(&runner, &target)?;

    let mut app = App::with_git(snapshot, target.clone(), Box::new(runner.clone()));
    if matches!(target, DiffTarget::Review { .. }) {
        app.set_review_origin_ops(Box::new(discovered));
    }
    app.set_repo_root(runner.root().to_path_buf());
    // Config loads exactly once, here, before the first render — there is no
    // reload path (docs/specs/07-spec-config-layer). Warnings (missing file
    // is silent and yields none; a syntax error or an invalid entry each
    // yield one) are handed to the app for its dismissible status-line
    // notice; never printed to stdout.
    let (loaded_config, mut config_warnings) = config::load();
    // The `[editor]` config tier is resolved *before* `loaded_config` moves
    // into `app.set_config` below. An unknown preset name is folded into
    // the same warning collection here (`ui::editor` owns the preset table
    // that names its validity against — see that module's doc for why
    // `crate::config` can't do this check itself) and then treated exactly
    // like an absent config tier for precedence purposes.
    let config_tier = match resolve_editor_config_tier(&loaded_config.editor) {
        EditorConfigTier::UnknownPreset(name) => {
            config_warnings.push(config::ConfigWarning::InvalidValue {
                section: "editor".to_string(),
                key: "preset".to_string(),
                message: format!("unknown preset \"{name}\""),
            });
            EditorConfigTier::Absent
        }
        resolved => resolved,
    };
    app.set_config(loaded_config, config_warnings);
    let visual = std::env::var_os("VISUAL").map(|s| s.to_string_lossy().into_owned());
    let editor_env = std::env::var_os("EDITOR").map(|s| s.to_string_lossy().into_owned());
    app.set_editor(resolve_editor(
        config.editor.clone(),
        config_tier,
        visual,
        editor_env,
    ));
    let outcome = ui::run(&mut app)?;

    if let QuitOutcome::Emit = outcome {
        let markdown = render_markdown(&app.annotations);
        print!("{markdown}");
        if let Some(path) = &config.output {
            std::fs::write(path, &markdown)?;
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = RunConfig::from(cli);

    if std::io::stderr().is_terminal() {
        run_tui(&config)
    } else {
        run(&config)
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, paths_match, resolve_editor};
    use clap::Parser;
    use redquill::ui::{EditorConfigTier, EditorLaunch};

    // -- Cli parsing: `--review` conflicts and `--base` gating --------------

    #[test]
    fn review_is_accepted_alone() {
        let cli = Cli::try_parse_from(["redquill", "--review", "feature"]).unwrap();
        assert_eq!(cli.review.as_deref(), Some("feature"));
        assert_eq!(cli.range, None);
        assert!(!cli.staged);
    }

    #[test]
    fn review_conflicts_with_staged() {
        let err = Cli::try_parse_from(["redquill", "--review", "feature", "--staged"])
            .expect_err("--review and --staged must conflict");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "unexpected error kind: {err}"
        );
    }

    #[test]
    fn review_conflicts_with_positional_range() {
        let err = Cli::try_parse_from(["redquill", "main..HEAD", "--review", "feature"])
            .expect_err("--review and the positional range must conflict");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn staged_still_conflicts_with_range_without_review() {
        let err = Cli::try_parse_from(["redquill", "main..HEAD", "--staged"])
            .expect_err("--staged and a positional range must still conflict");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn base_requires_review() {
        let err = Cli::try_parse_from(["redquill", "--base", "main"])
            .expect_err("--base without --review must be rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn base_is_accepted_alongside_review() {
        let cli =
            Cli::try_parse_from(["redquill", "--review", "feature", "--base", "trunk"]).unwrap();
        assert_eq!(cli.review.as_deref(), Some("feature"));
        assert_eq!(cli.base.as_deref(), Some("trunk"));
    }

    // -- paths_match ----------------------------------------------------------

    #[test]
    fn paths_match_identical_nonexistent_paths() {
        // Neither side exists, so this exercises the direct-comparison
        // fallback rather than canonicalize.
        let p = std::path::PathBuf::from("/no/such/path/anywhere");
        assert!(paths_match(&p, &p));
    }

    #[test]
    fn paths_match_distinguishes_different_nonexistent_paths() {
        assert!(!paths_match(
            std::path::Path::new("/no/such/path/a"),
            std::path::Path::new("/no/such/path/b")
        ));
    }

    #[test]
    fn flag_wins_over_everything() {
        assert_eq!(
            resolve_editor(
                Some("code --wait".to_string()),
                EditorConfigTier::Template("zed {{filename}}:{{line}}".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("code --wait".to_string())
        );
    }

    #[test]
    fn config_template_wins_when_no_flag() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Template("zed {{filename}}:{{line}}".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Template("zed {{filename}}:{{line}}".to_string())
        );
    }

    #[test]
    fn visual_wins_when_no_flag_or_config_tier() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Absent,
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("emacs".to_string())
        );
    }

    #[test]
    fn editor_env_wins_when_no_flag_config_tier_or_visual() {
        assert_eq!(
            resolve_editor(None, EditorConfigTier::Absent, None, Some("vi".to_string())),
            EditorLaunch::Command("vi".to_string())
        );
    }

    #[test]
    fn nvim_is_the_final_fallback() {
        assert_eq!(
            resolve_editor(None, EditorConfigTier::Absent, None, None),
            EditorLaunch::Command("nvim".to_string())
        );
    }

    #[test]
    fn empty_flag_falls_through_to_config_tier() {
        assert_eq!(
            resolve_editor(
                Some("   ".to_string()),
                EditorConfigTier::Template("zed {{filename}}:{{line}}".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Template("zed {{filename}}:{{line}}".to_string())
        );
    }

    #[test]
    fn empty_flag_and_absent_config_tier_falls_through_to_visual() {
        assert_eq!(
            resolve_editor(
                Some("   ".to_string()),
                EditorConfigTier::Absent,
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("emacs".to_string())
        );
    }

    #[test]
    fn empty_visual_falls_through_to_editor_env() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Absent,
                Some("".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("vi".to_string())
        );
    }

    #[test]
    fn empty_editor_env_falls_through_to_nvim() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Absent,
                None,
                Some("  \t".to_string())
            ),
            EditorLaunch::Command("nvim".to_string())
        );
    }
}
