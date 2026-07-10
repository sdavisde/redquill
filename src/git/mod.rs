//! Runs `git` commands on PATH and parses porcelain/diff output into typed
//! structs. Owns all interaction with the git CLI, respecting the user's
//! git config. No TUI types leak in here.
