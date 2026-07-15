//! Parser for `git ls-files -z`.
//!
//! Pure text-in / structs-out: [`parse_ls_files_z`] splits the NUL-delimited
//! payload into repo-relative paths. No process spawning lives here, keeping
//! it directly unit-testable with fixture bytes — mirrors
//! `crate::git::status`'s `-z` splitting convention (`input.split('\0')`,
//! empty fields dropped).

/// Parses a NUL-delimited `git ls-files -z` payload into repo-relative
/// paths, in the order git printed them. Empty fields (a trailing NUL, or an
/// empty payload) are dropped rather than yielding a spurious empty-string
/// path.
pub fn parse_ls_files_z(input: &str) -> Vec<String> {
    input
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_nul_delimited_paths() {
        let input = "src/main.rs\0src/lib.rs\0README.md\0";
        assert_eq!(
            parse_ls_files_z(input),
            vec!["src/main.rs", "src/lib.rs", "README.md"]
        );
    }

    #[test]
    fn empty_input_yields_no_paths() {
        assert!(parse_ls_files_z("").is_empty());
    }

    #[test]
    fn missing_trailing_nul_still_parses_the_last_path() {
        let input = "a.rs\0b.rs";
        assert_eq!(parse_ls_files_z(input), vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn paths_with_spaces_are_preserved_whole() {
        let input = "my file.rs\0other.rs\0";
        assert_eq!(parse_ls_files_z(input), vec!["my file.rs", "other.rs"]);
    }

    #[test]
    fn a_single_path_with_no_trailing_nul_parses() {
        assert_eq!(parse_ls_files_z("only.rs"), vec!["only.rs"]);
    }
}
