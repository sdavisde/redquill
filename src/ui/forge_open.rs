//! Opening what's under review on its forge (`gx`): resolves the current
//! view to a [`WebTarget`], hands it to the forge CLI on a background
//! thread so a slow or hung child can never stall the render loop, and
//! drains the outcome into the status line.
//!
//! Read-only by construction: every command this module can build is a
//! `view`/`browse` invocation carrying one already-typed value (a `u64` PR
//! number, or a ref name read back from git as its own argv element). There
//! is no path from this keypress to a forge write.
//!
//! What `gx` opens follows the view, and its footer label follows the
//! target (see [`WebTargetKind::label`]):
//!
//! | View | Target | GitHub | GitLab |
//! |---|---|---|---|
//! | PR/MR review | [`WebTarget::Pr`] | `gh pr view --web` | `glab mr view --web` |
//! | branch review | [`WebTarget::Branch`] | `gh browse --branch` | `glab repo view --branch --web` |
//! | commit view | [`WebTarget::Commit`] | `gh browse <sha>` | *unsupported* |
//!
//! GitLab has no CLI route to a commit page (`glab` has no `browse`, and
//! `repo view --branch <sha>` opens the tree at that commit rather than the
//! commit's own diff), so that one cell reports why instead of opening
//! something misleading. Everywhere with no forge counterpart at all (the
//! working tree, the index, a range) `gx` is hidden outright rather than
//! left inert — see [`super::help::binding_hidden`].

use crate::git::DiffTarget;

use super::app::App;

/// What `gx` would open from the current view. Carries the already-resolved
/// value so the background closure needs no further app state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebTarget {
    /// The PR/MR under review, by number.
    Pr(u64),
    /// The branch under review, by name — a local-branch review, which has
    /// no PR behind it.
    Branch(String),
    /// The commit being viewed, by revision spec.
    Commit(String),
}

/// [`WebTarget`] without its payload: enough to gate the binding and label
/// it, cheap enough to sit in [`super::footer::FooterFlags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebTargetKind {
    Pr,
    Branch,
    Commit,
}

impl WebTarget {
    pub fn kind(&self) -> WebTargetKind {
        match self {
            WebTarget::Pr(_) => WebTargetKind::Pr,
            WebTarget::Branch(_) => WebTargetKind::Branch,
            WebTarget::Commit(_) => WebTargetKind::Commit,
        }
    }
}

impl WebTargetKind {
    /// The footer hint's label for this target. The keymap table carries the
    /// PR wording as its static default; [`super::footer`] swaps in the
    /// branch/commit wording, the same presentation-side relabel
    /// `push`/`publish` uses.
    pub fn label(self) -> &'static str {
        match self {
            WebTargetKind::Pr => "open PR",
            WebTargetKind::Branch => "open branch",
            WebTargetKind::Commit => "open commit",
        }
    }

    /// How the status line names this target while opening it.
    fn noun(self) -> &'static str {
        match self {
            WebTargetKind::Pr => "PR",
            WebTargetKind::Branch => "branch",
            WebTargetKind::Commit => "commit",
        }
    }
}

impl App {
    /// What `gx` would open here, or `None` in a view with no forge
    /// counterpart (the working tree, the index, a range, a bare file view).
    ///
    /// A review session carrying [`App::review_forge`] is a PR/MR review and
    /// resolves to its number; one without is a local-branch review and
    /// resolves to the branch itself, since there is no PR to open.
    pub(super) fn web_target(&self) -> Option<WebTarget> {
        if let Some(forge) = &self.review_forge {
            return Some(WebTarget::Pr(forge.number));
        }
        match &self.target {
            DiffTarget::Review { branch, .. } => Some(WebTarget::Branch(branch.clone())),
            DiffTarget::Commit(sha) => Some(WebTarget::Commit(sha.clone())),
            DiffTarget::WorkingTree
            | DiffTarget::Staged
            | DiffTarget::Range(_)
            | DiffTarget::File(_) => None,
        }
    }

    /// [`App::web_target`]'s kind, for gating and labelling the binding
    /// without cloning the payload every frame.
    pub(super) fn web_target_kind(&self) -> Option<WebTargetKind> {
        self.web_target().map(|t| t.kind())
    }

    /// `gx`: opens the current view's forge counterpart in the user's
    /// browser. A no-op with a status hint in a view that has none, and
    /// single-flight — a second `gx` while one is running is ignored rather
    /// than opening a second tab.
    pub(super) fn open_in_browser(&mut self) {
        let Some(target) = self.web_target() else {
            self.set_status_message("nothing here to open on the forge");
            return;
        };
        if self.pr_web_in_flight.is_some() {
            return;
        }
        // A PR session names its own provider; outside one, follow whatever
        // a prior PR listing resolved for this host, falling back to GitHub
        // — the same peek-never-re-resolve fallback the PR checkout uses, so
        // `gx` never spawns a credential check of its own.
        let provider = self
            .review_forge
            .as_ref()
            .map(|f| f.provider)
            .or_else(|| {
                self.stage_ops()
                    .and_then(|ops| ops.resolved_pr_provider())
                    .map(super::review_launcher::store_provider)
            })
            .unwrap_or(crate::review::store::ForgeProviderKind::GitHub);
        let kind = target.kind();
        let Some(ops) = self.stage_ops() else {
            return;
        };
        let Some(opener) = ops.async_web_opener(provider, target) else {
            self.set_status_message("can't open a browser from here");
            return;
        };
        let id = self.pr_web_tasks.spawn(opener);
        self.pr_web_in_flight = Some(id);
        self.web_target_noun = kind.noun();
        self.set_status_message(format!("opening {} \u{2026}", kind.noun()));
    }

    /// Drains a completed browser open (once per event-loop tick, alongside
    /// the other pollers) into the status line. Reports success as well as
    /// failure so the "opening …" line never sits there stale. A failure
    /// carries its own already-worded message (an unpushed ref, an
    /// unsupported target, or the CLI's own diagnostic).
    pub(super) fn poll_pr_web_open(&mut self) {
        for (id, result) in self.pr_web_tasks.poll() {
            if self.pr_web_in_flight != Some(id) {
                continue;
            }
            self.pr_web_in_flight = None;
            let noun = self.web_target_noun;
            match result {
                Ok(Ok(())) => self.set_status_message(format!("opened the {noun} in your browser")),
                Ok(Err(message)) => self.set_status_message(message),
                Err(_panic) => self.set_status_message("browser open failed"),
            }
        }
    }
}

#[cfg(test)]
#[path = "forge_open_tests.rs"]
mod tests;
