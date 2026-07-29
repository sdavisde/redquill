//! Opening the PR/MR under review on its forge (`gx`): hands the number to
//! `gh pr view --web` / `glab mr view --web` on a background thread, so a
//! slow or hung CLI can never stall the render loop, and drains the outcome
//! into the status line once per tick.
//!
//! Read-only by construction: the only forge command this module can build
//! is the provider's `view --web`, from a `u64` PR number — there is no path
//! from this keypress to a forge write.

use super::app::App;

impl App {
    /// `gx`: opens the PR/MR under review in the user's browser. A no-op
    /// with a status hint outside a forge PR review session (a local-branch
    /// review has no PR to open), when a previous open is still running, or
    /// when the backend can't cross a thread boundary (test fakes, git-less
    /// contexts).
    pub(super) fn open_pr_in_browser(&mut self) {
        let Some(forge) = self.review_forge.clone() else {
            self.set_status_message("no PR under review \u{2014} nothing to open");
            return;
        };
        if self.pr_web_in_flight.is_some() {
            return;
        }
        let Some(ops) = self.stage_ops() else {
            return;
        };
        let Some(opener) = ops.async_pr_web_opener(forge.provider) else {
            self.set_status_message("can't open a browser from here");
            return;
        };
        let number = forge.number;
        let id = self.pr_web_tasks.spawn(move || opener(number));
        self.pr_web_in_flight = Some(id);
        self.set_status_message(format!("opening #{number} \u{2026}"));
    }

    /// Drains a completed browser-open (once per event-loop tick, alongside
    /// the other pollers) into the status line. Reports success as well as
    /// failure so the "opening …" line never sits there stale.
    pub(super) fn poll_pr_web_open(&mut self) {
        for (id, result) in self.pr_web_tasks.poll() {
            if self.pr_web_in_flight != Some(id) {
                continue;
            }
            self.pr_web_in_flight = None;
            let number = self.review_forge.as_ref().map(|f| f.number).unwrap_or(0);
            match result {
                Ok(Ok(())) => self.set_status_message(format!("opened #{number} in your browser")),
                Ok(Err(message)) => {
                    self.set_status_message(format!("couldn't open #{number}: {message}"))
                }
                Err(_panic) => self.set_status_message("browser open failed"),
            }
        }
    }
}

#[cfg(test)]
#[path = "forge_open_tests.rs"]
mod tests;
