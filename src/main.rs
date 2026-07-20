//! CLI entry point: argument parsing and wiring for redquill.
//!
//! Owns the working-tree default, `--staged`, ref-range, and `-o file`
//! flags described in the README, and wires parsed args into the rest of
//! the crate.
//!
//! If stderr is a terminal, launches the interactive TUI (which renders to
//! stderr). On quit, emitted annotations are copied to the system clipboard,
//! with stdout as the fallback sink when no clipboard is available (see
//! [`present_annotations`]). If stderr is not a terminal — e.g. piped, or
//! running under a test harness — falls back to the plain-text summary so
//! redquill stays scriptable.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::PathBuf;

use clap::Parser;

use redquill::annotate::render_markdown;
use redquill::config;
use redquill::diff::FileDiff;
use redquill::git::{ChangeKind, DiffTarget, GitRunner};
use redquill::review::store;
use redquill::ui::{
    self, App, EditorConfigTier, EditorLaunch, QuitOutcome, build_review, ensure_review_worktree,
    load_reconciled_review_state, resolve_editor_config_tier, resolve_review_base,
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

    /// Review a local branch inside its own managed worktree instead of the
    /// working tree. redquill creates (or reuses) a worktree at
    /// `<git-common-dir>/redquill/worktrees/<sanitized-branch>`, then shows
    /// the `base...branch` (three-dot) diff rooted there, so LSP navigation
    /// and `g<Space>` operate on the branch's real files. Mutually exclusive
    /// with the positional range and `--staged`.
    #[arg(long, value_name = "BRANCH", conflicts_with_all = ["staged", "range"])]
    review: Option<String>,

    /// Base ref for `--review`'s three-dot diff. Defaults to the branch
    /// `origin/HEAD` points to, else `main`, else `master`.
    #[arg(long, value_name = "REF", requires = "review")]
    base: Option<String>,

    /// Also write emitted annotations to this file, in addition to copying
    /// them to the clipboard.
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
    /// Optional file to additionally write annotations to, alongside the
    /// clipboard copy.
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

    let base = resolve_review_base(&discovered, config.base.as_deref())?;
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

/// Resolves `<git-common-dir>/redquill/review-state.json`'s path for this
/// repository and garbage-collects entries whose branch no longer exists,
/// before returning it. Runs on every launch, unconditionally — even
/// outside a review session — since another, already-paused review's entry
/// should still get cleaned up opportunistically the next time redquill
/// runs at all, not only the next time that specific branch is reviewed
/// again. Best-effort throughout: a `git_common_dir`/`branch_list` failure
/// degrades to skipping GC for this launch (never fails the launch itself —
/// this is housekeeping, not something worth blocking a session over), and
/// a GC save failure is silently retried on the next launch the same way.
fn gc_review_state(discovered: &GitRunner) -> Option<PathBuf> {
    let common_dir = discovered.git_common_dir().ok()?;
    let path = common_dir.join("redquill").join("review-state.json");
    let Ok(branches) = discovered.branch_list() else {
        return Some(path);
    };
    let existing: HashSet<String> = branches.into_iter().map(|b| b.name).collect();
    let mut state = store::load(&path);
    if store::gc(&mut state, &existing) {
        let _ = store::save(&path, &state);
        // Best-effort: clears stale worktree admin records for any managed
        // worktree whose branch just got GC'd. A failure here is not worth
        // surfacing — it's the same harmless-clutter case
        // `App::finish_review`'s own prune call already treats this way.
        let _ = discovered.worktree_prune();
    }
    Some(path)
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
/// from `crate::config::EditorConfig` by `ui::resolve_editor_config_tier`),
/// then `$VISUAL`, then `$EDITOR`, then `"nvim"`. Takes every tier as an
/// explicit arg (rather than reading `std::env::var`/resolving config
/// itself) so precedence is unit-testable without mutating process-global
/// env state or touching real config paths; `run_tui` reads the real env
/// vars and config at the one call site. Empty or whitespace-only strings
/// at the flag/`$VISUAL`/`$EDITOR` tiers are treated as unset and fall
/// through to the next — an exported `EDITOR=""` shouldn't silently break
/// `g<Space>`. A config-tier template always wins over
/// `$VISUAL`/`$EDITOR`/`"nvim"` and carries no such "empty is unset"
/// allowance: it's already been validated non-empty by
/// `crate::config::EditorConfig::from_value`.
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
    // Runs on every launch, review session or not — GC'ing *other* paused
    // reviews' stale entries shouldn't wait for the next time that specific
    // branch happens to be reviewed again. Must happen before `discovered`
    // is potentially moved into `app.set_review_origin_ops` below.
    let review_state_path = gc_review_state(&discovered);
    // `discovered` is rooted at the caller's cwd (the user's own checkout),
    // *outside* any managed review worktree `resolve_session` might create —
    // kept alive (cloned into `resolve_session`) so a review session's
    // finish gesture can remove that worktree through a backend that isn't
    // rooted inside the very directory being removed.
    let (runner, target) = resolve_session(discovered.clone(), config)?;
    let snapshot = build_review(&runner, &target)?;

    let mut app = App::with_git(snapshot, target.clone(), Box::new(runner.clone()));
    if let DiffTarget::Review { branch, .. } = &target {
        // Load + reconcile this branch's persisted progress before the
        // first render, so `Accepted`/`Deferred` files start collapsed and a
        // stale `Accepted` file starts marked `ChangedSinceAccepted` and
        // un-collapsed from the very first frame.
        if let Some(state_path) = &review_state_path {
            let (states, blob_shas, annotations, replies) =
                load_reconciled_review_state(&runner, state_path, branch);
            app.set_review_states(states, blob_shas);
            app.set_review_state_path(state_path.clone());
            // Restored before the first render: annotations reattach to
            // their recorded anchors verbatim, so a resumed session's
            // annotation list and in-diff markers already reflect them on
            // the very first frame.
            app.restore_review_annotations(annotations);
            app.restore_review_replies(replies);
        }
        app.set_review_origin_ops(Box::new(discovered));
    }
    app.set_repo_root(runner.root().to_path_buf());
    // Config loads exactly once, here, before the first render — there is no
    // reload path. Warnings (missing file is silent and yields none; a
    // syntax error or an invalid entry each yield one) are handed to the app
    // for its dismissible status-line notice; never printed to stdout.
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
        present_annotations(&markdown, app.annotations.len(), config.output.as_deref())?;
    }

    Ok(())
}

/// Presents emitted annotations to the user on quit: copies the rendered
/// markdown to the system clipboard and prints a one-line confirmation
/// (`Copied <N> annotations to the clipboard`).
///
/// If the clipboard is unavailable — e.g. a headless or SSH session with no
/// display server — falls back to writing the markdown to stdout (the prior
/// behavior) with a note on stderr, so annotations are never lost. When
/// `output` is set (`-o <file>`), the markdown is written there regardless of
/// how the clipboard fares. An empty annotation set is a no-op.
///
/// Note: on X11 the clipboard is served by the owning process, so a value set
/// immediately before exit may not persist; macOS and Windows (the primary
/// targets) keep it after exit.
fn present_annotations(
    markdown: &str,
    count: usize,
    output: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    if let Some(path) = output {
        std::fs::write(path, markdown)?;
    }
    if count == 0 {
        return Ok(());
    }
    match copy_to_clipboard(markdown) {
        Ok(()) => {
            let noun = if count == 1 {
                "annotation"
            } else {
                "annotations"
            };
            println!("Copied {count} {noun} to the clipboard");
        }
        Err(err) => {
            eprintln!(
                "redquill: clipboard unavailable ({err}); writing annotations to stdout instead"
            );
            print!("{markdown}");
        }
    }
    Ok(())
}

/// Copies `text` to the system clipboard, returning any backend error so the
/// caller can fall back. Kept as a thin seam around `arboard` so the fallback
/// policy lives in one place ([`present_annotations`]).
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    clipboard.set_text(text.to_owned())?;
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
    use super::{Cli, resolve_editor};
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

    // -- gc_review_state ----------------------------------------------------
    //
    // Real-git tempdir tests: `gc_review_state` is private to the binary
    // crate (not part of `redquill::ui`'s public surface), so it can only be
    // exercised from *this* crate's own `#[cfg(test)]` module, which is
    // exactly where `cargo test`'s `unittests src/main.rs` binary already
    // runs from.

    use super::gc_review_state;
    use redquill::git::GitRunner;
    use redquill::review::store;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("failed to spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(dir: &std::path::Path, rel: &str, contents: &str) {
        std::fs::write(dir.join(rel), contents).unwrap();
    }

    fn canon(path: &std::path::Path) -> std::path::PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
    }

    /// Asserts that `path` resolves inside `tmp`, so a mutating git call in
    /// this test module can never touch the host repo.
    fn assert_inside_tempdir(path: &std::path::Path, tmp: &TempDir) {
        let tmp_root = canon(tmp.path());
        let mut probe = path.to_path_buf();
        while !probe.exists() {
            match probe.parent() {
                Some(parent) => probe = parent.to_path_buf(),
                None => panic!("path {path:?} has no existing ancestor to canonicalize"),
            }
        }
        assert!(
            canon(&probe).starts_with(&tmp_root),
            "refusing to run a mutating git call outside the tempdir: {path:?}"
        );
    }

    fn repo_with_branch(name: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        assert_inside_tempdir(dir, &tmp);
        git(dir, &["init", "-q", "-b", name]);
        git(dir, &["config", "user.email", "test@redquill.invalid"]);
        git(dir, &["config", "user.name", "redquill test"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
        write(dir, "base.txt", "line one\n");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "initial"]);
        tmp
    }

    #[test]
    fn gc_review_state_drops_entries_for_deleted_branches_and_keeps_live_ones() {
        let repo = repo_with_branch("main");
        git(repo.path(), &["branch", "feature-live"]);
        // `feature-gone` is deliberately never created as a real branch —
        // simulating a previously-reviewed branch the author (or the user)
        // has since deleted.
        let runner = GitRunner::discover_in(repo.path()).unwrap();
        let common_dir = runner.git_common_dir().unwrap();
        let state_path = common_dir.join("redquill").join("review-state.json");
        assert_inside_tempdir(&state_path, &repo);

        for branch in ["feature-live", "feature-gone"] {
            store::save_review(
                &state_path,
                branch,
                store::PersistedReview {
                    base: "main".to_string(),
                    worktree_path: repo.path().join("wt").join(branch),
                    files: Default::default(),
                    annotations: Default::default(),
                    replies: Vec::new(),
                    forge: None,
                },
            )
            .unwrap();
        }

        let resolved = gc_review_state(&runner);

        assert_eq!(resolved, Some(state_path.clone()));
        let state = store::load(&state_path);
        assert!(
            state.reviews.contains_key("feature-live"),
            "GC must never touch an entry for a branch that still exists"
        );
        assert!(
            !state.reviews.contains_key("feature-gone"),
            "GC must drop an entry for a branch that no longer exists"
        );
    }

    #[test]
    fn gc_review_state_is_a_no_op_when_every_branch_still_exists() {
        let repo = repo_with_branch("main");
        git(repo.path(), &["branch", "feature"]);
        let runner = GitRunner::discover_in(repo.path()).unwrap();
        let common_dir = runner.git_common_dir().unwrap();
        let state_path = common_dir.join("redquill").join("review-state.json");
        assert_inside_tempdir(&state_path, &repo);
        store::save_review(
            &state_path,
            "feature",
            store::PersistedReview {
                base: "main".to_string(),
                worktree_path: repo.path().join("wt"),
                files: Default::default(),
                annotations: Default::default(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();
        let before = store::load(&state_path);

        gc_review_state(&runner);

        assert_eq!(store::load(&state_path), before);
    }
}
