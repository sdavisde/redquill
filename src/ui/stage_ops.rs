//! The staging seam between the TUI and git: [`StageOps`] is the small
//! trait the [`super::App`] drives staging through, implemented by
//! [`GitRunner`] for real sessions and by a recording fake in unit tests.
//! [`build_review`] assembles everything a review session needs from one
//! `StageOps` — parsed [`FileDiff`]s, the raw patches they came from
//! (needed later to construct hunk/line patches), and which paths currently
//! have staged changes.

use std::collections::HashMap;
use std::process::Command;

use thiserror::Error;

use crate::diff::{DiffParseError, FileChangeKind, FileDiff};
use crate::forge::{
    self, CredentialChecker, ForgeError, GhCredentialChecker, GlabCredentialChecker, ProviderKind,
    ProviderResolution, PullRequest, ResolutionCache, Thread, UnresolvedReason,
};
use crate::git::{
    BranchStatus, ChangeKind, CommitLogEntry, CommitLogRange, CommitSummary, DiffTarget,
    FileStatus, GitError, GitRunner, LocalBranch, PrRef, RawFilePatch, StashEntry, StatusCode,
    WorktreeEntry,
};
use crate::search::{FileCandidate, merge_candidates};

/// Errors produced while building a [`ReviewSnapshot`].
#[derive(Debug, Error)]
pub enum ReviewError {
    /// Running or parsing git failed.
    #[error(transparent)]
    Git(#[from] GitError),
    /// A raw patch's hunks could not be parsed.
    #[error(transparent)]
    Parse(#[from] DiffParseError),
}

/// A `Send` closure that rebuilds a [`ReviewSnapshot`] off the render thread,
/// so the periodic working-tree poll doesn't block the event loop on git I/O.
/// Only backends that can cross a thread boundary produce one (see
/// [`StageOps::async_review_builder`]): the production [`GitRunner`] does;
/// non-`Send` test fakes and git-less contexts return `None` and stay on the
/// synchronous refresh path.
pub type AsyncReviewBuilder =
    Box<dyn Fn(&DiffTarget) -> Result<ReviewSnapshot, ReviewError> + Send>;

/// A `Send` closure fetching one page of the commit-log read model
/// (`count` commits starting `skip` back from the tip) off the render
/// thread, for the git panel's History tab. The same indirection as
/// [`AsyncReviewBuilder`] and for the same reason: `App` itself is not
/// `Send`, but a cloned [`GitRunner`] handle is.
pub type AsyncCommitLogFetcher =
    Box<dyn Fn(u32, u32) -> Result<Vec<CommitLogEntry>, GitError> + Send>;

/// A `Send` closure fetching a resolved `base..head` commit range off the
/// render thread, for the Review launcher's Commits-tab ahead-of-base
/// source. Same indirection as [`AsyncCommitLogFetcher`] and for the same
/// reason — the closure itself is what crosses the thread boundary, since
/// `App` is not `Send`.
pub type AsyncCommitLogRangeFetcher =
    Box<dyn Fn(&CommitLogRange) -> Result<Vec<CommitLogEntry>, GitError> + Send>;

/// A `Send` closure fetching the fuzzy file finder's candidate list off the
/// render thread, for the finder's single-flight background load. Same
/// indirection as [`AsyncReviewBuilder`]/[`AsyncCommitLogFetcher`] and for
/// the same reason.
pub type AsyncFileCandidatesFetcher = Box<dyn Fn() -> Result<Vec<FileCandidate>, GitError> + Send>;

/// A `Send` closure running the Review launcher's Pull Requests tab's full
/// provider-resolve-then-list pipeline off the render thread. Unlike the
/// other `Async*Fetcher` aliases this doesn't return a `Result`: a
/// [`PrFetchOutcome`] already represents every failure mode as a value the
/// tab can render directly, so there's nothing left for an outer `Result`
/// to add.
pub type AsyncPrListFetcher = Box<dyn Fn() -> PrFetchOutcome + Send>;

/// Everything the Review launcher's Pull Requests tab's load can resolve
/// to: a listing, or one of the degraded states that gives the user a
/// specific, actionable next step rather than a blank tab. `origin` having
/// no forge remote at all degrades the same way as the four CLI-specific
/// states below — never a bare error, since the tab always has *something*
/// concrete to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrFetchOutcome {
    /// No `origin` remote, or its URL doesn't parse to a hostname at all.
    NoForgeRemote,
    /// Neither/both CLIs hold credentials for the host.
    Unresolved {
        hostname: String,
        reason: UnresolvedReason,
    },
    /// The resolved CLI isn't on `PATH`.
    CliMissing { cli: &'static str, hostname: String },
    /// The CLI is present but not authenticated for this host.
    Unauthenticated { cli: &'static str, hostname: String },
    /// The CLI ran and exited non-zero (or its output didn't parse) for a
    /// reason other than missing auth.
    ListFailed { message: String },
    /// A successful listing — `repo_label` is the "org/repo" slug parsed
    /// from `origin`'s URL, falling back to the hostname when the slug
    /// can't be extracted, for the tab's zero-results empty state.
    Loaded {
        repo_label: String,
        prs: Vec<PullRequest>,
    },
}

/// Everything the background PR-checkout fetch needs, resolved on the render
/// thread and handed to the fetcher: which PR (a closed [`PrRef`]), its base
/// branch, its managed branch/worktree location, whether a worktree already
/// exists (a reopen), and the head SHA last fetched (to detect an author
/// push). Plain data so it crosses the thread boundary alongside the fetcher.
#[derive(Debug, Clone)]
pub struct PrCheckoutRequest {
    /// The closed PR head reference — the only thing that can name a forced
    /// refspec, and only ever the `redquill/pr/<n>` namespace.
    pub pr_ref: PrRef,
    /// The PR's base branch name (e.g. `main`), plain-fetched so
    /// `origin/<base_ref>` resolves for the review's diff base.
    pub base_ref: String,
    /// The managed branch short name (`redquill/pr/<n>`).
    pub managed_branch: String,
    /// The managed worktree's resolved path.
    pub worktree_path: std::path::PathBuf,
    /// Whether that worktree already exists (a reopen) vs. a first checkout.
    pub worktree_exists: bool,
    /// The head SHA persisted from the last fetch, compared against a fresh
    /// peek to decide whether the author pushed new commits. `None` on a
    /// first checkout (or when prior state was lost).
    pub stored_head_sha: Option<String>,
}

/// The outcome of a background PR checkout: either a ready worktree (with the
/// freshly-fetched head SHA and whether the author pushed since last time),
/// or a failure that left local state untouched — carrying the stale worktree
/// path when a prior checkout still exists for the user to review offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrCheckoutOutcome {
    /// The managed branch is fetched and the worktree is ready at
    /// `worktree_path`. `moved` is true when the fetched head differs from
    /// the stored one (an author push), so the caller can report demotions.
    Ready {
        head_sha: String,
        moved: bool,
        worktree_path: std::path::PathBuf,
    },
    /// The fetch failed; nothing local was destroyed. `stale_worktree` is
    /// `Some` when a prior checkout still exists that the user may review as
    /// a clearly-labeled stale session.
    Failed {
        message: String,
        stale_worktree: Option<std::path::PathBuf>,
    },
}

/// A `Send` closure running a whole [`PrCheckoutRequest`] off the render
/// thread — the network fetches plus the local worktree add/remove — and
/// returning a [`PrCheckoutOutcome`] the render thread finishes into a review
/// session. Like [`AsyncPrListFetcher`] it returns a value that already
/// encodes every failure mode, so there is no outer `Result`.
pub type AsyncPrCheckoutFetcher = Box<dyn Fn(PrCheckoutRequest) -> PrCheckoutOutcome + Send>;

/// A `Send` closure fetching one PR's existing comment threads off the render
/// thread, for the review session's thread overlay. Takes the PR number and
/// returns the imported threads or a one-line diagnostic (the tab/banner
/// surfaces "comments unavailable" on `Err`). Same cloned-`GitRunner`-handle
/// indirection as the other `Async*` aliases.
pub type AsyncThreadFetcher = Box<dyn Fn(u64) -> Result<Vec<Thread>, String> + Send>;

/// A `Send` closure running one whole submit sequence — the reviews-endpoint
/// POST plus the sequential follow-ups — off the render thread, returning the
/// [`SubmitReport`] the render thread finishes into per-item published marks
/// and a status line. The batch is fully resolved on the render thread (see
/// [`super::forge_submit`]); the closure only runs it. Like the other `Async*`
/// aliases it returns a value that already encodes every failure mode, so
/// there is no outer `Result`.
pub type AsyncForgeSubmitter = Box<dyn Fn(forge::SubmitBatch) -> forge::SubmitReport + Send>;

/// The git operations the TUI needs for staging and refresh, kept behind a
/// trait so [`super::App`]'s staging logic is unit-testable without a real
/// repository. [`GitRunner`] is the production implementation. Read-model
/// methods default to erroring (or returning empty/`None`) so navigation-only
/// fakes need not implement them.
pub trait StageOps {
    /// Raw per-file patches for `target` (see [`GitRunner::diff`]).
    fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError>;
    /// Parsed porcelain status for every changed path (see
    /// [`GitRunner::status`]).
    fn status(&self) -> Result<Vec<FileStatus>, GitError>;
    /// A `Send` snapshot builder for the async working-tree poll, or `None`
    /// for backends that can't cross a thread boundary. The default returns
    /// `None`, keeping non-`Send` fakes (and git-less contexts) on the
    /// synchronous path; [`GitRunner`] overrides it by cloning itself into the
    /// closure (it is a cheap `PathBuf` handle).
    fn async_review_builder(&self) -> Option<AsyncReviewBuilder> {
        None
    }
    /// Stages `path` in its entirety (see [`GitRunner::stage_file`]).
    fn stage_file(&self, path: &str) -> Result<(), GitError>;
    /// Unstages `path` (see [`GitRunner::unstage_file`]).
    fn unstage_file(&self, path: &str) -> Result<(), GitError>;
    /// Applies `patch` to the index only (see [`GitRunner::apply_cached`]).
    fn apply_cached(&self, patch: &str) -> Result<(), GitError>;
    /// Reverses `patch` against the index only (see
    /// [`GitRunner::unapply_cached`]).
    fn unapply_cached(&self, patch: &str) -> Result<(), GitError>;
    /// Reads an untracked file's working-tree content (repo-relative
    /// `path`), for synthesizing its all-added diff. `None` if unreadable.
    fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>>;
    /// Reads a file's content at a git revision spec (see
    /// [`GitRunner::show_file`]), for sourcing whole-file content the diff
    /// pane highlights syntactically. `None` on any failure.
    fn show_file(&self, spec: &str) -> Option<String>;
    /// Reads the current branch / upstream / ahead-behind state (see
    /// [`GitRunner::status_with_branch`]); the panel treats this as
    /// best-effort.
    fn branch_status(&self) -> Result<BranchStatus, GitError> {
        Err(GitError::Parse("branch status unavailable".into()))
    }
    /// Reads the stash list, newest first (see [`GitRunner::stash_list`]).
    /// The default returns an empty list.
    fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        Ok(Vec::new())
    }
    /// Reads a one-line summary of the tip commit (see
    /// [`GitRunner::last_commit`]).
    fn last_commit(&self) -> Result<Option<CommitSummary>, GitError> {
        Ok(None)
    }
    /// Reads the local branches (see [`GitRunner::branch_list`]); the
    /// switcher treats this as unavailable rather than crashing.
    fn branch_list(&self) -> Result<Vec<LocalBranch>, GitError> {
        Err(GitError::Parse("branch list unavailable".into()))
    }
    /// Reads every worktree (see [`GitRunner::worktree_list`]).
    fn worktree_list(&self) -> Result<Vec<WorktreeEntry>, GitError> {
        Err(GitError::Parse("worktree list unavailable".into()))
    }
    /// Lists the managed PR/MR review branches (`refs/heads/redquill/pr/*`,
    /// see [`GitRunner::managed_pr_branches`]) — the driver for the Pull
    /// Requests tab's finished-review detection and cleanup. The default
    /// returns an empty list, keeping navigation-only fakes and git-less
    /// contexts free of finished-review candidates rather than erroring.
    fn managed_pr_branches(&self) -> Result<Vec<LocalBranch>, GitError> {
        Ok(Vec::new())
    }
    /// Removes a managed review worktree (see [`GitRunner::worktree_remove`]).
    /// Must be called through a backend rooted *outside* the worktree being
    /// removed (see [`super::app::App::review_origin_ops`]'s doc).
    fn worktree_remove(&self, path: &std::path::Path) -> Result<(), GitError> {
        let _ = path;
        Err(GitError::Parse("worktree remove unavailable".into()))
    }
    /// Prunes stale worktree administrative records (see
    /// [`GitRunner::worktree_prune`]).
    fn worktree_prune(&self) -> Result<(), GitError> {
        Err(GitError::Parse("worktree prune unavailable".into()))
    }
    /// Deletes a managed PR/MR review branch (`redquill/pr/<number>`, see
    /// [`GitRunner::delete_managed_pr_branch`]) — structurally confined to the
    /// managed namespace. The default errors, keeping git-less contexts and
    /// navigation-only fakes off the deletion path.
    fn delete_managed_pr_branch(&self, number: u64) -> Result<(), GitError> {
        let _ = number;
        Err(GitError::Parse("managed branch delete unavailable".into()))
    }
    /// Switches the working tree to branch `name` (see
    /// [`GitRunner::switch_branch`]).
    fn switch_branch(&self, name: &str) -> Result<(), GitError> {
        let _ = name;
        Err(GitError::Parse("branch switch unavailable".into()))
    }
    /// The repository's common git directory (see
    /// [`GitRunner::git_common_dir`]) — the shared administrative directory
    /// every linked worktree resolves review paths through.
    fn git_common_dir(&self) -> Result<std::path::PathBuf, GitError> {
        Err(GitError::Parse("git common dir unavailable".into()))
    }
    /// Resolves `--review`'s default base ref (see [`GitRunner::default_base`]):
    /// the branch `origin/HEAD` points to, else `main`, else `master`.
    fn default_base(&self) -> Result<String, GitError> {
        Err(GitError::Parse("default base unavailable".into()))
    }
    /// Creates a managed review worktree at `path`, checked out to `branch`
    /// (see [`GitRunner::worktree_add`]).
    fn worktree_add(&self, path: &std::path::Path, branch: &str) -> Result<(), GitError> {
        let _ = (path, branch);
        Err(GitError::Parse("worktree add unavailable".into()))
    }
    /// Builds the `git commit -m <message>` [`Command`] the commit gesture
    /// spawns on the background poller — returned as a `Command` rather
    /// than run here so the caller can execute it off the render thread
    /// (see [`crate::git::commit_command`] for the fixed-argv contract). The
    /// default returns `None`: backend-less contexts degrade to a footer
    /// message, and a fake *can* opt in with a synthetic command to drive
    /// the full spawn → poll → command-log pipeline without git.
    fn commit_command(&self, message: &str) -> Option<Command> {
        let _ = message;
        None
    }
    /// Reads one page of the commit-log read model (see
    /// [`GitRunner::commit_log`]), synchronously — the fallback the History
    /// tab's fetch takes when [`StageOps::async_commit_log_fetcher`] returns
    /// `None`.
    fn commit_log(&self, count: u32, skip: u32) -> Result<Vec<CommitLogEntry>, GitError> {
        let _ = (count, skip);
        Err(GitError::Parse("commit log unavailable".into()))
    }
    /// A `Send` closure fetching a commit-log page off the render thread
    /// (see [`AsyncCommitLogFetcher`]). The default returns `None`, keeping
    /// non-`Send` fakes (and git-less contexts) on the synchronous
    /// [`StageOps::commit_log`] path; [`GitRunner`] overrides it the same way
    /// it overrides [`StageOps::async_review_builder`].
    fn async_commit_log_fetcher(&self) -> Option<AsyncCommitLogFetcher> {
        None
    }
    /// Reads the commits ahead of `range.base` (see
    /// [`GitRunner::commit_log_range`]), synchronously — the fallback the
    /// Review launcher's Commits tab takes when
    /// [`StageOps::async_commit_log_range_fetcher`] returns `None`.
    fn commit_log_range(&self, range: &CommitLogRange) -> Result<Vec<CommitLogEntry>, GitError> {
        let _ = range;
        Err(GitError::Parse("commit log range unavailable".into()))
    }
    /// A `Send` closure fetching a commit-log range off the render thread
    /// (see [`AsyncCommitLogRangeFetcher`]). The default returns `None`,
    /// keeping non-`Send` fakes (and git-less contexts) on the synchronous
    /// [`StageOps::commit_log_range`] path; [`GitRunner`] overrides it the
    /// same way it overrides [`StageOps::async_commit_log_fetcher`].
    fn async_commit_log_range_fetcher(&self) -> Option<AsyncCommitLogRangeFetcher> {
        None
    }
    /// Reads the fuzzy file finder's candidate list: tracked (`git
    /// ls-files`) plus untracked-but-unignored files, merged via
    /// [`crate::search::merge_candidates`]; the finder treats this as
    /// unavailable rather than crashing.
    fn list_files(&self) -> Result<Vec<FileCandidate>, GitError> {
        Err(GitError::Parse("file list unavailable".into()))
    }
    /// A `Send` closure fetching the file-candidate list off the render
    /// thread (see [`AsyncFileCandidatesFetcher`]). The default returns
    /// `None`, keeping non-`Send` fakes (and git-less contexts) on the
    /// synchronous [`StageOps::list_files`] path; [`GitRunner`] overrides it
    /// the same way it overrides [`StageOps::async_review_builder`].
    fn async_file_candidates_fetcher(&self) -> Option<AsyncFileCandidatesFetcher> {
        None
    }
    /// Reads `path`'s blob SHA on `branch` (see [`GitRunner::blob_sha`]),
    /// for capturing at accept time and re-checking at reconciliation time;
    /// accept/reconcile degrade to recording no blob SHA rather than
    /// crashing.
    fn blob_sha(&self, branch: &str, path: &str) -> Result<Option<String>, GitError> {
        let _ = (branch, path);
        Err(GitError::Parse("blob sha unavailable".into()))
    }
    /// Resolves `origin`'s forge provider and lists its open PRs,
    /// synchronously — the fallback the Review launcher's Pull Requests tab
    /// takes when [`StageOps::async_pr_list_fetcher`] returns `None`. The
    /// default degrades to [`PrFetchOutcome::NoForgeRemote`] (a git-less or
    /// navigation-only fake has no origin to resolve), matching every other
    /// read-model default's "unavailable, not a panic" contract.
    fn list_open_prs(&self) -> PrFetchOutcome {
        PrFetchOutcome::NoForgeRemote
    }
    /// A `Send` closure running [`StageOps::list_open_prs`]'s pipeline off
    /// the render thread (see [`AsyncPrListFetcher`]). The default returns
    /// `None`, keeping non-`Send` fakes (and git-less contexts) on the
    /// synchronous path; [`GitRunner`] overrides it the same way it
    /// overrides [`StageOps::async_review_builder`].
    fn async_pr_list_fetcher(&self) -> Option<AsyncPrListFetcher> {
        None
    }
    /// A `Send` closure running a [`PrCheckoutRequest`] off the render thread
    /// (see [`AsyncPrCheckoutFetcher`]). The default returns `None`, keeping
    /// non-`Send` fakes (and git-less contexts) on a synchronous fallback via
    /// [`StageOps::pr_checkout`]; [`GitRunner`] overrides it by cloning
    /// itself into the closure.
    fn async_pr_checkout_fetcher(&self) -> Option<AsyncPrCheckoutFetcher> {
        None
    }
    /// Runs a [`PrCheckoutRequest`] synchronously — the fallback the PR
    /// checkout flow takes when [`StageOps::async_pr_checkout_fetcher`]
    /// returns `None`. The default degrades to a failure with no stale
    /// fallback (a git-less/navigation-only fake can't fetch anything),
    /// matching every other read-model default's "unavailable, not a panic"
    /// contract.
    fn pr_checkout(&self, request: PrCheckoutRequest) -> PrCheckoutOutcome {
        let _ = request;
        PrCheckoutOutcome::Failed {
            message: "PR checkout unavailable (no git backend)".to_string(),
            stale_worktree: None,
        }
    }
    /// `origin`'s forge hostname (`git remote get-url origin` parsed to a
    /// host), for stamping a PR review's forge metadata. The default returns
    /// `None`; [`GitRunner`] overrides it.
    fn origin_hostname(&self) -> Option<String> {
        None
    }
    /// `origin`'s `owner/repo` slug (`git remote get-url origin` parsed), for
    /// the submit modal's `#N on host/org/repo` target line. The default
    /// returns `None`; [`GitRunner`] overrides it.
    fn origin_repo_slug(&self) -> Option<String> {
        None
    }
    /// A `Send` closure fetching one PR's comment threads off the render
    /// thread (see [`AsyncThreadFetcher`]), dispatched by `provider` to the
    /// GitHub review-comments read or the GitLab discussions read. The default
    /// returns `None`, keeping non-`Send` fakes (and git-less contexts)
    /// thread-overlay-free; [`GitRunner`] overrides it by cloning itself into
    /// the closure and resolving `origin`'s `owner/repo` slug for the
    /// resolution overlay.
    fn async_thread_fetcher(
        &self,
        _provider: crate::review::store::ForgeProviderKind,
    ) -> Option<AsyncThreadFetcher> {
        None
    }
    /// The forge provider a prior background PR list already resolved for
    /// `origin`'s host, peeked (never re-resolved) so the render-thread
    /// checkout dispatch can pick the right special-ref kind without spawning
    /// a credential check. The default returns `None`; [`GitRunner`] overrides
    /// it by reading the process-lifetime resolution cache.
    fn resolved_pr_provider(&self) -> Option<ProviderKind> {
        None
    }
    /// A `Send` closure that publishes one previewed batch to PR `number`,
    /// anchoring any file-level comments to `head_sha` (see
    /// [`AsyncForgeSubmitter`]). The default returns `None`, keeping non-`Send`
    /// fakes and git-less contexts off the live-write path entirely — the only
    /// path that ever runs a forge write. [`GitRunner`] overrides it.
    fn async_forge_submitter(
        &self,
        _number: u64,
        _head_sha: String,
    ) -> Option<AsyncForgeSubmitter> {
        None
    }
}

impl StageOps for GitRunner {
    fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        GitRunner::diff(self, target)
    }

    fn status(&self) -> Result<Vec<FileStatus>, GitError> {
        GitRunner::status(self)
    }

    fn stage_file(&self, path: &str) -> Result<(), GitError> {
        GitRunner::stage_file(self, path)
    }

    fn unstage_file(&self, path: &str) -> Result<(), GitError> {
        GitRunner::unstage_file(self, path)
    }

    fn apply_cached(&self, patch: &str) -> Result<(), GitError> {
        GitRunner::apply_cached(self, patch)
    }

    fn unapply_cached(&self, patch: &str) -> Result<(), GitError> {
        GitRunner::unapply_cached(self, patch)
    }

    fn blob_sha(&self, branch: &str, path: &str) -> Result<Option<String>, GitError> {
        GitRunner::blob_sha(self, branch, path)
    }

    fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root().join(path)).ok()
    }

    fn show_file(&self, spec: &str) -> Option<String> {
        GitRunner::show_file(self, spec)
    }

    fn branch_status(&self) -> Result<BranchStatus, GitError> {
        Ok(GitRunner::status_with_branch(self)?.branch)
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        GitRunner::stash_list(self)
    }

    fn last_commit(&self) -> Result<Option<CommitSummary>, GitError> {
        GitRunner::last_commit(self)
    }

    fn branch_list(&self) -> Result<Vec<LocalBranch>, GitError> {
        GitRunner::branch_list(self)
    }

    fn worktree_list(&self) -> Result<Vec<WorktreeEntry>, GitError> {
        GitRunner::worktree_list(self)
    }

    fn managed_pr_branches(&self) -> Result<Vec<LocalBranch>, GitError> {
        GitRunner::managed_pr_branches(self)
    }

    fn worktree_remove(&self, path: &std::path::Path) -> Result<(), GitError> {
        GitRunner::worktree_remove(self, path)
    }

    fn worktree_prune(&self) -> Result<(), GitError> {
        GitRunner::worktree_prune(self)
    }

    fn delete_managed_pr_branch(&self, number: u64) -> Result<(), GitError> {
        GitRunner::delete_managed_pr_branch(self, number)
    }

    fn switch_branch(&self, name: &str) -> Result<(), GitError> {
        GitRunner::switch_branch(self, name)
    }

    fn git_common_dir(&self) -> Result<std::path::PathBuf, GitError> {
        GitRunner::git_common_dir(self)
    }

    fn default_base(&self) -> Result<String, GitError> {
        GitRunner::default_base(self)
    }

    fn worktree_add(&self, path: &std::path::Path, branch: &str) -> Result<(), GitError> {
        GitRunner::worktree_add(self, path, branch)
    }

    fn commit_command(&self, message: &str) -> Option<Command> {
        Some(crate::git::commit_command(message, self.root()))
    }

    fn async_review_builder(&self) -> Option<AsyncReviewBuilder> {
        // `GitRunner` is a `Clone` `PathBuf` handle, so cloning it into a
        // `Send` closure lets the periodic poll run `build_review` on a
        // background thread without touching `App`'s non-`Send` state.
        let runner = self.clone();
        Some(Box::new(move |target| build_review(&runner, target)))
    }

    fn commit_log(&self, count: u32, skip: u32) -> Result<Vec<CommitLogEntry>, GitError> {
        GitRunner::commit_log(self, count, skip)
    }

    fn async_commit_log_fetcher(&self) -> Option<AsyncCommitLogFetcher> {
        // Same cloned-handle trick as `async_review_builder`.
        let runner = self.clone();
        Some(Box::new(move |count, skip| runner.commit_log(count, skip)))
    }

    fn commit_log_range(&self, range: &CommitLogRange) -> Result<Vec<CommitLogEntry>, GitError> {
        GitRunner::commit_log_range(self, range)
    }

    fn async_commit_log_range_fetcher(&self) -> Option<AsyncCommitLogRangeFetcher> {
        // Same cloned-handle trick as `async_review_builder`.
        let runner = self.clone();
        Some(Box::new(move |range: &CommitLogRange| {
            runner.commit_log_range(range)
        }))
    }

    fn list_files(&self) -> Result<Vec<FileCandidate>, GitError> {
        let tracked = GitRunner::ls_files(self)?;
        let untracked = GitRunner::ls_files_untracked(self)?;
        Ok(merge_candidates(tracked, untracked))
    }

    fn async_file_candidates_fetcher(&self) -> Option<AsyncFileCandidatesFetcher> {
        // Same cloned-handle trick as `async_review_builder`.
        let runner = self.clone();
        Some(Box::new(move || {
            let tracked = runner.ls_files()?;
            let untracked = runner.ls_files_untracked()?;
            Ok(merge_candidates(tracked, untracked))
        }))
    }

    fn list_open_prs(&self) -> PrFetchOutcome {
        pr_fetch_outcome(self)
    }

    fn async_pr_list_fetcher(&self) -> Option<AsyncPrListFetcher> {
        // Same cloned-handle trick as `async_review_builder`.
        let runner = self.clone();
        Some(Box::new(move || pr_fetch_outcome(&runner)))
    }

    fn pr_checkout(&self, request: PrCheckoutRequest) -> PrCheckoutOutcome {
        pr_checkout_fetch(self, request)
    }

    fn async_pr_checkout_fetcher(&self) -> Option<AsyncPrCheckoutFetcher> {
        // Same cloned-handle trick as `async_review_builder`.
        let runner = self.clone();
        Some(Box::new(move |request| pr_checkout_fetch(&runner, request)))
    }

    fn origin_hostname(&self) -> Option<String> {
        let url = self.origin_url().ok().flatten()?;
        forge::parse_origin_hostname(&url)
            .ok()
            .map(|h| h.as_str().to_string())
    }

    fn origin_repo_slug(&self) -> Option<String> {
        let url = self.origin_url().ok().flatten()?;
        forge::parse_origin_repo_slug(&url)
    }

    fn async_thread_fetcher(
        &self,
        provider: crate::review::store::ForgeProviderKind,
    ) -> Option<AsyncThreadFetcher> {
        use crate::review::store::ForgeProviderKind;
        // Same cloned-handle trick as `async_review_builder`. GitHub's
        // review-comments read needs the `owner/repo` slug for its (best-effort)
        // resolution overlay — a missing slug just leaves threads unresolved;
        // GitLab's discussions read infers the project from the working
        // directory, so it needs no slug.
        let runner = self.clone();
        Some(Box::new(move |number| match provider {
            ForgeProviderKind::GitLab => {
                forge::fetch_discussions(number).map_err(|e| e.to_string())
            }
            ForgeProviderKind::GitHub => {
                let slug = runner
                    .origin_url()
                    .ok()
                    .flatten()
                    .and_then(|url| forge::parse_origin_repo_slug(&url));
                forge::fetch_review_threads(slug.as_deref(), number).map_err(|e| e.to_string())
            }
        }))
    }

    fn resolved_pr_provider(&self) -> Option<ProviderKind> {
        match PR_PROVIDER_RESOLUTION.peek()? {
            ProviderResolution::Resolved(kind) => Some(kind),
            ProviderResolution::Unresolved { .. } => None,
        }
    }

    fn async_forge_submitter(&self, number: u64, head_sha: String) -> Option<AsyncForgeSubmitter> {
        // The real live-write path: build the GitHub executor from the typed
        // PR number + head SHA and run the sequence over it. Agents never
        // reach this (fakes return the default `None`); it exists only for the
        // human dogfood.
        Some(Box::new(move |batch| {
            let executor = forge::GhSubmitExecutor::new(number, head_sha.clone());
            forge::run_submit_sequence(&batch, &executor)
        }))
    }
}

/// The one-line diagnostic a [`GitError`] contributes to a
/// [`PrCheckoutOutcome::Failed`]: a `Command` error's first stderr line
/// (git's stderr is often multi-line; the session shows one actionable
/// line), or the error's own `Display` otherwise. Mirrors
/// [`forge_error_headline`].
fn git_error_headline(e: &GitError) -> String {
    match e {
        GitError::Command { stderr, .. } if !stderr.trim().is_empty() => stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(stderr)
            .trim()
            .to_string(),
        other => other.to_string(),
    }
}

/// Runs a whole [`PrCheckoutRequest`] against `runner` (rooted at the repo,
/// outside any review worktree): the network fetches and the local worktree
/// add/remove, returning a [`PrCheckoutOutcome`] the render thread finishes
/// into a review session. Only ever called off the render thread.
///
/// A first checkout force-fetches the PR head into the managed branch, then
/// adds the worktree. A reopen first *peeks* the remote head into
/// `FETCH_HEAD` (git refuses to force-update the managed branch while it is
/// checked out): an unchanged head reuses the worktree as-is; a moved head
/// recreates it (remove → forced update → re-add). A failed fetch destroys
/// nothing and, when a prior worktree survives, offers it as a stale
/// fallback — except in the narrow window after a moved-head worktree
/// removal, where a subsequent fetch failure (right after a *successful*
/// peek proved connectivity, so vanishingly unlikely) has no worktree left
/// to fall back to; a retry then takes the first-checkout path.
fn pr_checkout_fetch(runner: &GitRunner, request: PrCheckoutRequest) -> PrCheckoutOutcome {
    if !request.worktree_exists {
        if let Err(e) = runner.fetch_pr_head(&request.pr_ref) {
            return PrCheckoutOutcome::Failed {
                message: git_error_headline(&e),
                stale_worktree: None,
            };
        }
        // Best-effort: `origin/<base>` may already resolve; a genuine
        // unresolved base surfaces later as a reroot/diff error.
        let _ = runner.fetch_base_ref(&request.base_ref);
        let head_sha = runner
            .commit_sha_of(&request.managed_branch)
            .ok()
            .flatten()
            .unwrap_or_default();
        return match super::review_session::ensure_review_worktree(runner, &request.managed_branch)
        {
            Ok(path) => PrCheckoutOutcome::Ready {
                head_sha,
                moved: false,
                worktree_path: path,
            },
            Err(e) => PrCheckoutOutcome::Failed {
                message: git_error_headline(&e),
                stale_worktree: None,
            },
        };
    }

    let remote_head = match runner.peek_pr_head(&request.pr_ref) {
        Ok(sha) => sha,
        Err(e) => {
            return PrCheckoutOutcome::Failed {
                message: git_error_headline(&e),
                stale_worktree: Some(request.worktree_path.clone()),
            };
        }
    };
    let moved = request.stored_head_sha.as_deref() != Some(remote_head.as_str());
    if !moved {
        let _ = runner.fetch_base_ref(&request.base_ref);
        return PrCheckoutOutcome::Ready {
            head_sha: remote_head,
            moved: false,
            worktree_path: request.worktree_path,
        };
    }

    // The author pushed: recreate the worktree so the forced managed-branch
    // update (refused while the branch is checked out) can run.
    if let Err(e) = runner.worktree_remove(&request.worktree_path) {
        return PrCheckoutOutcome::Failed {
            message: git_error_headline(&e),
            stale_worktree: Some(request.worktree_path.clone()),
        };
    }
    let _ = runner.worktree_prune();
    if let Err(e) = runner.fetch_pr_head(&request.pr_ref) {
        return PrCheckoutOutcome::Failed {
            message: git_error_headline(&e),
            stale_worktree: None,
        };
    }
    let _ = runner.fetch_base_ref(&request.base_ref);
    match super::review_session::ensure_review_worktree(runner, &request.managed_branch) {
        Ok(path) => PrCheckoutOutcome::Ready {
            head_sha: remote_head,
            moved: true,
            worktree_path: path,
        },
        Err(e) => PrCheckoutOutcome::Failed {
            message: git_error_headline(&e),
            stale_worktree: None,
        },
    }
}

/// Caches the Pull Requests tab's provider resolution for the process
/// lifetime (a single process only ever reviews one repo, and hence one
/// `origin` hostname) — the ladder's own credential-check subprocesses
/// never re-run once a resolution lands.
static PR_PROVIDER_RESOLUTION: ResolutionCache = ResolutionCache::new();

/// Resolves `origin`'s forge provider and lists its open PRs, collapsing
/// every failure mode into a [`PrFetchOutcome`] the tab can render
/// directly. Only ever called off the render thread (via
/// [`AsyncPrListFetcher`] in production; directly, synchronously, in a
/// git-less-caller fallback) since the credential-check and listing steps
/// both spawn subprocesses with multi-second timeouts.
fn pr_fetch_outcome(runner: &GitRunner) -> PrFetchOutcome {
    let Ok(Some(url)) = runner.origin_url() else {
        return PrFetchOutcome::NoForgeRemote;
    };
    let Ok(hostname) = forge::parse_origin_hostname(&url) else {
        return PrFetchOutcome::NoForgeRemote;
    };
    let resolution = PR_PROVIDER_RESOLUTION.get_or_resolve(
        &hostname,
        &GhCredentialChecker,
        &GlabCredentialChecker,
    );
    match resolution {
        ProviderResolution::Unresolved { hostname, reason } => {
            PrFetchOutcome::Unresolved { hostname, reason }
        }
        ProviderResolution::Resolved(ProviderKind::GitLab) => list_outcome(
            &url,
            &hostname,
            "glab",
            forge::list_open_mrs(),
            &GlabCredentialChecker,
        ),
        ProviderResolution::Resolved(ProviderKind::GitHub) => list_outcome(
            &url,
            &hostname,
            "gh",
            forge::list_open_prs(),
            &GhCredentialChecker,
        ),
    }
}

/// Collapses one provider's list call into a [`PrFetchOutcome`], shared by the
/// GitHub and GitLab arms since the failure-mode ladder is identical: a listing
/// on success, a missing binary, and otherwise a fresh local credential check
/// disambiguating "not logged in" (the common case with an exact fix) from
/// every other failure. The CLI is already known to be on `PATH` at this point
/// (it just ran), so `has_credentials` returning `false` here means "no
/// credential", not "missing binary".
fn list_outcome(
    url: &str,
    hostname: &forge::Hostname,
    cli: &'static str,
    result: Result<Vec<PullRequest>, ForgeError>,
    checker: &dyn CredentialChecker,
) -> PrFetchOutcome {
    match result {
        Ok(prs) => {
            let repo_label =
                forge::parse_origin_repo_slug(url).unwrap_or_else(|| hostname.as_str().to_string());
            PrFetchOutcome::Loaded { repo_label, prs }
        }
        Err(ForgeError::CliNotFound { cli }) => PrFetchOutcome::CliMissing {
            cli,
            hostname: hostname.as_str().to_string(),
        },
        Err(e) => {
            if checker.has_credentials(hostname) {
                PrFetchOutcome::ListFailed {
                    message: forge_error_headline(&e),
                }
            } else {
                PrFetchOutcome::Unauthenticated {
                    cli,
                    hostname: hostname.as_str().to_string(),
                }
            }
        }
    }
}

/// The one-line summary a [`ForgeError`] contributes to a
/// [`PrFetchOutcome::ListFailed`] body: a `Command` error's first stderr
/// line (stderr is often multi-line; the tab shows one actionable line, not
/// a dump), or the error's own `Display` for every other variant.
fn forge_error_headline(e: &ForgeError) -> String {
    match e {
        ForgeError::Command { stderr, .. } if !stderr.is_empty() => {
            stderr.lines().next().unwrap_or(stderr).to_string()
        }
        other => other.to_string(),
    }
}

/// One file with staged changes, as shown in the staging panel and marked
/// in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    /// The file's current path, relative to the repo root.
    pub path: String,
    /// The index-side porcelain status letter (`M`, `A`, `D`, ...).
    pub letter: char,
}

/// Everything one review pass over a diff target needs: parsed file diffs,
/// the raw patches they were built from (index-aligned with `files`; `None`
/// for synthetic untracked entries, which have no real patch), and the
/// paths that currently have staged changes.
#[derive(Debug, Clone)]
pub struct ReviewSnapshot {
    /// Every file in the diff, in display order: sorted by path
    /// (byte-wise ascending), independent of staged state, so staging or
    /// unstaging a file never moves it in the list.
    pub files: Vec<FileDiff>,
    /// The raw patch each entry of `files` was parsed from, by index.
    /// `None` for synthetic untracked entries and for fully-staged entries
    /// with no textual hunks in the staged diff (e.g. a staged deletion or
    /// binary file); a fully-staged entry with real staged hunks carries
    /// its staged `RawFilePatch` here, same as any other file.
    pub patches: Vec<Option<RawFilePatch>>,
    /// Files with staged changes, per `git status`.
    pub staged: Vec<StagedFile>,
    /// Per-path [`StagedState`] for the `●`/`±` header/sidebar markers.
    /// Missing entries default to [`StagedState::Unstaged`].
    pub staged_states: HashMap<String, StagedState>,
}

/// A single file's staged state, derived from its `git status` index-side
/// (`X`) and working-tree-side (`Y`) codes. This is the three-state marker
/// the multibuffer section header and sidebar render: `Full` → `●`,
/// `Partial` → `±`, `Unstaged` → blank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StagedState {
    /// Nothing staged for this path (working-tree-only changes, or
    /// untracked): no marker.
    #[default]
    Unstaged,
    /// Some but not all of this path's changes are staged (the index
    /// differs from `HEAD` *and* the working tree differs from the index):
    /// `±`.
    Partial,
    /// Everything is staged (the index differs from `HEAD`, the working
    /// tree matches the index): `●`.
    Full,
}

/// Derives a file's [`StagedState`] from its porcelain status. A path with
/// no staged changes is `Unstaged` (covers untracked `??` and working-tree
/// -only `.M`); a path with staged changes is `Partial` when it *also* has
/// unstaged changes (e.g. `MM`, `AM`, `RM`) and `Full` otherwise (`M.`,
/// `A.`, `D.`, `R.`, `C.`).
pub fn staged_state(status: &FileStatus) -> StagedState {
    match (status.has_staged_changes(), status.has_unstaged_changes()) {
        (false, _) => StagedState::Unstaged,
        (true, true) => StagedState::Partial,
        (true, false) => StagedState::Full,
    }
}

/// A path-keyed map of every file's [`StagedState`], for the paths that have
/// any staged changes (`Partial`/`Full`); `Unstaged` files are omitted, so a
/// missing entry means [`StagedState::Unstaged`] (its `Default`). This is
/// what `rebuild_rows` and the sidebar consume to render the `●`/`±` markers.
pub fn staged_states_from_status(status: &[FileStatus]) -> HashMap<String, StagedState> {
    status
        .iter()
        .filter_map(|s| {
            let state = staged_state(s);
            (state != StagedState::Unstaged).then(|| (s.path.clone(), state))
        })
        .collect()
}

/// Maps a porcelain index-side [`StatusCode`] to the [`FileChangeKind`] used
/// for a fully-staged file's synthetic (header-only) section, so its header
/// shows the right change-kind letter.
fn kind_from_staged_code(code: StatusCode) -> FileChangeKind {
    match code {
        StatusCode::Added => FileChangeKind::Added,
        StatusCode::Deleted => FileChangeKind::Deleted,
        StatusCode::Renamed => FileChangeKind::Renamed,
        StatusCode::Copied => FileChangeKind::Copied,
        // Modified/TypeChange/anything else display as a modification.
        _ => FileChangeKind::Modified,
    }
}

/// The staged-file list derived from parsed porcelain status.
pub fn staged_from_status(status: &[FileStatus]) -> Vec<StagedFile> {
    status
        .iter()
        .filter(|s| s.has_staged_changes())
        .map(|s| StagedFile {
            path: s.path.clone(),
            letter: s.staged.letter(),
        })
        .collect()
}

/// Builds a [`ReviewSnapshot`] for `target`: the diff's parsed files plus,
/// for the working tree, synthetic all-added entries for untracked files
/// (`git diff` never surfaces those), and the staged-file list from status.
pub fn build_review(
    ops: &dyn StageOps,
    target: &DiffTarget,
) -> Result<ReviewSnapshot, ReviewError> {
    let raw_patches = ops.diff(target)?;
    let status = ops.status()?;

    let mut files = Vec::with_capacity(raw_patches.len());
    let mut patches = Vec::with_capacity(raw_patches.len());
    for patch in raw_patches {
        files.push(FileDiff::from_patch(&patch)?);
        patches.push(Some(patch));
    }

    if target.is_live() {
        // Fully-staged files have no working-tree diff at all, so their
        // real content only exists in the staged (`--staged`) diff. Fetch
        // it once, indexed by path, so the synthesis loop below can give
        // them real hunks instead of an empty header-only placeholder.
        let staged_patches: HashMap<String, RawFilePatch> = ops
            .diff(&DiffTarget::Staged)?
            .into_iter()
            .map(|patch| (patch.path.clone(), patch))
            .collect();

        for entry in &status {
            if entry.kind != ChangeKind::Untracked {
                continue;
            }
            // Unreadable (permissions, race with deletion, ...): skip
            // rather than fail the whole review session.
            let Some(bytes) = ops.read_worktree_file(&entry.path) else {
                continue;
            };
            let file = match String::from_utf8(bytes) {
                Ok(content) => FileDiff::synthetic_added(entry.path.clone(), &content),
                Err(_) => FileDiff {
                    path: entry.path.clone(),
                    old_path: None,
                    kind: FileChangeKind::Added,
                    is_binary: true,
                    hunks: Vec::new(),
                },
            };
            files.push(file);
            patches.push(None);
        }

        // Fully-staged files never appear in the working-tree diff (their
        // changes are all in the index), yet the review must keep them as
        // sections so unstaging is one `S` on a header, and expanding shows
        // their (staged) content rather than nothing. Union them in from
        // the staged diff fetched above, falling back to a header-only
        // placeholder when there's no textual staged patch; the path sort
        // below places them, like every other entry, by path.
        for entry in &status {
            if staged_state(entry) != StagedState::Full {
                continue;
            }
            if files.iter().any(|f| f.path == entry.path) {
                continue;
            }
            match staged_patches.get(&entry.path) {
                // The staged diff has real hunks for this path (the common
                // case): parse them so the file is expandable and shows its
                // (staged) content, not just a header.
                Some(patch) => {
                    files.push(FileDiff::from_patch(patch)?);
                    patches.push(Some(patch.clone()));
                }
                // No staged patch (e.g. a staged deletion of a file with no
                // textual hunks, or a binary file): fall back to the
                // header-only placeholder so the section still exists.
                None => {
                    files.push(FileDiff {
                        path: entry.path.clone(),
                        old_path: entry.orig_path.clone(),
                        kind: kind_from_staged_code(entry.staged),
                        is_binary: false,
                        hunks: Vec::new(),
                    });
                    patches.push(None);
                }
            }
        }
    }

    // One flat list in a stable, status-independent order: sort every entry
    // by path (byte-wise), whatever source it came from. This is what keeps
    // a file from teleporting when staging flips it between the diff-parsed
    // and fully-staged-synthesized sources — only its marker and section
    // content change, never its position. `patches` is index-aligned with
    // `files`, so the two are sorted together.
    let mut entries: Vec<(FileDiff, Option<RawFilePatch>)> =
        files.into_iter().zip(patches).collect();
    entries.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    let (files, patches) = entries.into_iter().unzip();

    Ok(ReviewSnapshot {
        files,
        patches,
        staged: staged_from_status(&status),
        staged_states: staged_states_from_status(&status),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ChangeKind;

    /// A porcelain status entry with the given record kind, index-side (`X`)
    /// and working-tree-side (`Y`) codes.
    fn status(
        kind: ChangeKind,
        staged: StatusCode,
        unstaged: StatusCode,
        path: &str,
    ) -> FileStatus {
        FileStatus {
            kind,
            staged,
            unstaged,
            path: path.to_string(),
            orig_path: None,
        }
    }

    fn ordinary(staged: StatusCode, unstaged: StatusCode) -> FileStatus {
        status(ChangeKind::Ordinary, staged, unstaged, "f.rs")
    }

    #[test]
    fn unstaged_when_working_tree_only_modification() {
        // ` M`: modified in the working tree, nothing staged.
        let s = ordinary(StatusCode::Unmodified, StatusCode::Modified);
        assert_eq!(staged_state(&s), StagedState::Unstaged);
    }

    #[test]
    fn full_when_staged_modification_only() {
        // `M.`: staged modification, clean working tree.
        let s = ordinary(StatusCode::Modified, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn partial_when_both_staged_and_unstaged_modification() {
        // `MM`: staged and then edited again.
        let s = ordinary(StatusCode::Modified, StatusCode::Modified);
        assert_eq!(staged_state(&s), StagedState::Partial);
    }

    #[test]
    fn full_when_staged_addition() {
        // `A.`: newly added and fully staged.
        let s = ordinary(StatusCode::Added, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn partial_when_added_then_modified() {
        // `AM`: staged add plus a subsequent unstaged edit.
        let s = ordinary(StatusCode::Added, StatusCode::Modified);
        assert_eq!(staged_state(&s), StagedState::Partial);
    }

    #[test]
    fn full_when_staged_deletion() {
        // `D.`: staged deletion.
        let s = ordinary(StatusCode::Deleted, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn unstaged_when_untracked() {
        // `??`: untracked, counts as unstaged working-tree changes.
        let s = status(
            ChangeKind::Untracked,
            StatusCode::Unmodified,
            StatusCode::Untracked,
            "new.rs",
        );
        assert_eq!(staged_state(&s), StagedState::Unstaged);
    }

    #[test]
    fn full_when_staged_rename() {
        // `R.`: staged rename, clean working tree.
        let mut s = status(
            ChangeKind::RenamedOrCopied,
            StatusCode::Renamed,
            StatusCode::Unmodified,
            "new/name.rs",
        );
        s.orig_path = Some("old/name.rs".to_string());
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn partial_when_renamed_then_modified() {
        // `RM`: staged rename plus a subsequent unstaged edit.
        let s = status(
            ChangeKind::RenamedOrCopied,
            StatusCode::Renamed,
            StatusCode::Modified,
            "new/name.rs",
        );
        assert_eq!(staged_state(&s), StagedState::Partial);
    }

    #[test]
    fn full_when_staged_copy() {
        // `C.`: staged copy.
        let s = status(
            ChangeKind::RenamedOrCopied,
            StatusCode::Copied,
            StatusCode::Unmodified,
            "copy.rs",
        );
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn unstaged_when_no_changes_on_either_side() {
        let s = ordinary(StatusCode::Unmodified, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Unstaged);
    }

    #[test]
    fn states_map_omits_unstaged_and_keys_partial_full_by_path() {
        let entries = vec![
            ordinary(StatusCode::Unmodified, StatusCode::Modified), // f.rs unstaged
            status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Unmodified,
                "full.rs",
            ),
            status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Modified,
                "partial.rs",
            ),
            status(
                ChangeKind::Untracked,
                StatusCode::Unmodified,
                StatusCode::Untracked,
                "new.rs",
            ),
        ];
        let map = staged_states_from_status(&entries);
        // Unstaged (`f.rs`) and untracked (`new.rs`) are omitted.
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("full.rs"), Some(&StagedState::Full));
        assert_eq!(map.get("partial.rs"), Some(&StagedState::Partial));
        assert_eq!(map.get("f.rs"), None);
        assert_eq!(map.get("new.rs"), None);
    }

    #[test]
    fn kind_from_staged_code_maps_letters() {
        assert_eq!(
            kind_from_staged_code(StatusCode::Added),
            FileChangeKind::Added
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Deleted),
            FileChangeKind::Deleted
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Renamed),
            FileChangeKind::Renamed
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Copied),
            FileChangeKind::Copied
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Modified),
            FileChangeKind::Modified
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::TypeChange),
            FileChangeKind::Modified
        );
    }

    /// A minimal [`StageOps`] fake for [`build_review`]: `diff` is
    /// target-aware (separate working-tree and staged patch lists, as a
    /// real backend's would be), `status` is fixed, and every other
    /// operation is an unused no-op.
    #[derive(Default)]
    struct Fake {
        working_tree_diff: Vec<RawFilePatch>,
        staged_diff: Vec<RawFilePatch>,
        status: Vec<FileStatus>,
    }

    impl StageOps for Fake {
        fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
            match target {
                DiffTarget::Staged => Ok(self.staged_diff.clone()),
                _ => Ok(self.working_tree_diff.clone()),
            }
        }

        fn status(&self) -> Result<Vec<FileStatus>, GitError> {
            Ok(self.status.clone())
        }

        fn stage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn unstage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn apply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn unapply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }

        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
    }

    /// A single-hunk raw patch for `path`, matching the minimal shape
    /// `FileDiff::from_patch` needs to parse a non-empty hunk list.
    fn one_hunk_patch(path: &str) -> RawFilePatch {
        RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: format!(
                "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
            ),
            is_binary: false,
        }
    }

    #[test]
    fn fully_staged_file_gets_hunks_from_the_staged_diff() {
        // `x.rs` is fully staged: it has no working-tree diff (its changes
        // are all in the index), but it does have a staged one.
        // `build_review` should carry the real staged hunks so the file
        // stays expandable.
        let fake = Fake {
            working_tree_diff: Vec::new(),
            staged_diff: vec![one_hunk_patch("x.rs")],
            status: vec![status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Unmodified,
                "x.rs",
            )],
        };

        let review = build_review(&fake, &DiffTarget::WorkingTree).unwrap();

        let idx = review
            .files
            .iter()
            .position(|f| f.path == "x.rs")
            .expect("x.rs must still appear as a section");
        assert!(
            !review.files[idx].hunks.is_empty(),
            "expanding a fully-staged file must show its staged hunks, not an empty section"
        );
        assert!(
            review.patches[idx].is_some(),
            "a fully-staged file with a real staged patch must carry it, enabling hunk/line addressing"
        );
    }

    #[test]
    fn fully_staged_file_without_a_staged_patch_falls_back_to_a_header_only_placeholder() {
        // No staged patch is found for `deleted.rs` (e.g. a staged deletion
        // with no textual hunks, or a binary file): the section must still
        // exist (so unstaging stays reachable) but degrades to the old
        // header-only placeholder rather than erroring.
        let fake = Fake {
            working_tree_diff: Vec::new(),
            staged_diff: Vec::new(),
            status: vec![status(
                ChangeKind::Ordinary,
                StatusCode::Deleted,
                StatusCode::Unmodified,
                "deleted.rs",
            )],
        };

        let review = build_review(&fake, &DiffTarget::WorkingTree).unwrap();

        let idx = review
            .files
            .iter()
            .position(|f| f.path == "deleted.rs")
            .expect("deleted.rs must still appear as a section");
        assert!(review.files[idx].hunks.is_empty());
        assert!(review.patches[idx].is_none());
    }

    #[test]
    fn staged_target_does_not_fetch_the_staged_diff_again() {
        // When `target` is already `Staged`, `build_review` must not issue
        // a second `diff(&Staged)` call — that extra fetch only exists to
        // backfill fully-staged sections in a *working-tree* review.
        let fake = Fake {
            working_tree_diff: Vec::new(),
            staged_diff: vec![one_hunk_patch("y.rs")],
            status: vec![status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Unmodified,
                "y.rs",
            )],
        };

        let review = build_review(&fake, &DiffTarget::Staged).unwrap();

        // `diff(&Staged)` already returns `y.rs` as the primary diff, so it
        // must appear exactly once, not duplicated by the fully-staged
        // synthesis path.
        let count = review.files.iter().filter(|f| f.path == "y.rs").count();
        assert_eq!(count, 1);
    }

    // -- forge_error_headline -------------------------------------------

    #[test]
    fn command_error_headline_is_the_first_stderr_line() {
        let e = crate::forge::ForgeError::Command {
            cli: "gh",
            command: "pr list".to_string(),
            code: "1".to_string(),
            stderr: "not logged in\nrun `gh auth login`".to_string(),
        };
        assert_eq!(forge_error_headline(&e), "not logged in");
    }

    #[test]
    fn command_error_with_empty_stderr_falls_back_to_display() {
        let e = crate::forge::ForgeError::Command {
            cli: "gh",
            command: "pr list".to_string(),
            code: "1".to_string(),
            stderr: String::new(),
        };
        assert_eq!(forge_error_headline(&e), e.to_string());
    }

    #[test]
    fn non_command_error_headline_is_the_display_string() {
        let e = crate::forge::ForgeError::CliNotFound { cli: "gh" };
        assert_eq!(forge_error_headline(&e), e.to_string());
    }
}
