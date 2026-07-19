# Changelog

## [0.15.0](https://github.com/sdavisde/redquill/compare/v0.14.0...v0.15.0) (2026-07-18)


### Features

* close the git panel with Esc and reach staging and search from it ([b92f5b0](https://github.com/sdavisde/redquill/commit/b92f5b0189d7512a3969642734b73665f78d1b78))
* edit and delete annotations from the diff view ([ed879d0](https://github.com/sdavisde/redquill/commit/ed879d0a79ac5bdc456a2cb689c37057cf9fa56b))
* stage, accept, and defer files from the git panel ([8b60128](https://github.com/sdavisde/redquill/commit/8b601283b669c53c87fc569a682e7a5df715fedb))

## [0.14.0](https://github.com/sdavisde/redquill/compare/v0.13.0...v0.14.0) (2026-07-18)


### Features

* curated common-workflows header resolves live to keys ([684e9d9](https://github.com/sdavisde/redquill/commit/684e9d9bc406361fff74297e687b5643b5762066))
* derive which-key prefixes and continuations from the keymap table ([e82355f](https://github.com/sdavisde/redquill/commit/e82355f129ac9ee1a086b23b5abf24d9b49e6f0c))
* shrink the help overlay's height cap from ~4/5 to ~3/5 ([ed680d4](https://github.com/sdavisde/redquill/commit/ed680d46771c64e8eb5fb1ea8f152f39723207ae))
* split the ? help overlay into This context / All keys tabs ([f534d00](https://github.com/sdavisde/redquill/commit/f534d00c77d01a9511e939bbf622b9e82a0446e0))
* which-key popup for pending g/z prefixes ([4912ca1](https://github.com/sdavisde/redquill/commit/4912ca1267e9365d05a5ea97fc5aeb0e8cb9e752))


### Bug Fixes

* help-overlay scrollbar thumb now reaches the track bottom ([2f1b589](https://github.com/sdavisde/redquill/commit/2f1b5894e684caf394dcd00e7a1ee2872235ec6f))

## [0.13.0](https://github.com/sdavisde/redquill/compare/v0.12.0...v0.13.0) (2026-07-18)


### Features

* add [keys.global] config section for Scope::Global remapping ([23555f9](https://github.com/sdavisde/redquill/commit/23555f9f8d580e46db31273a6a09ac83b2bd7af2))
* add CommitLogRange git-layer query for ahead-of-base commits ([833e0c3](https://github.com/sdavisde/redquill/commit/833e0c36fde6bd8fafdc874352a25af2a30fb037))
* global R opens the Review launcher, refresh moves to r ([c2dc66d](https://github.com/sdavisde/redquill/commit/c2dc66dee8005b3a7ca5149ba9bb529b399707f3))
* render the Review launcher modal ([496678f](https://github.com/sdavisde/redquill/commit/496678f34ff53c944ba487a5115c1ce3e5a1b02c))
* Review launcher Commits tab lists commits and opens read-only view ([9478b47](https://github.com/sdavisde/redquill/commit/9478b47dd680f2abe5ec4c6999127d22cd375908))
* wire the Review launcher's Branches tab, retire Mode::ReviewBranch ([08eac70](https://github.com/sdavisde/redquill/commit/08eac70a8835fa28ce3396150957665739bd2188))

## [0.12.0](https://github.com/sdavisde/redquill/compare/v0.11.1...v0.12.0) (2026-07-17)


### Features

* copy annotations to clipboard on quit ([4322f5a](https://github.com/sdavisde/redquill/commit/4322f5a21418e23dfe69b2cc94a9e3d4ae6469b8))
* file-tree git panel with icons, guides, bottom-pinned stashes ([53ee171](https://github.com/sdavisde/redquill/commit/53ee171e8da7640af9f7f1fde415d359ffdd132d))
* git-log-style graph rail and right-aligned sha on History tab ([056f188](https://github.com/sdavisde/redquill/commit/056f188517bb8efa22a099782b810dc4056b0445))

## [0.11.1](https://github.com/sdavisde/redquill/compare/v0.11.0...v0.11.1) (2026-07-17)


### Bug Fixes

* allow diff lines to wrap ([b0cf06b](https://github.com/sdavisde/redquill/commit/b0cf06b08b7f063b39a0e2880f2f8bf07f7bfc5d))

## [0.11.0](https://github.com/sdavisde/redquill/compare/v0.10.0...v0.11.0) (2026-07-17)


### Features

* **review:** accept/defer tri-state for review sessions ([df102c3](https://github.com/sdavisde/redquill/commit/df102c3126939bc62b1ffe615b60694366875c1f))
* **review:** in-app review-branch modal ([9bc8ba4](https://github.com/sdavisde/redquill/commit/9bc8ba4010121bdd37811e6145d553d1b0426be2))
* **review:** persist annotations across pause/resume, emit once on finish ([94caeca](https://github.com/sdavisde/redquill/commit/94caeca6965ce8e7f4195f406964d4b98814f17d))
* **review:** persist review progress across sessions, self-invalidate on change ([425cbd1](https://github.com/sdavisde/redquill/commit/425cbd1fec019ceacf0ef0fd2d65e7025311e408))
* **review:** S toggle parity, accepted-files panel, guarded remote writes ([1d9d188](https://github.com/sdavisde/redquill/commit/1d9d188d8fd193f14ab20670da2f4c8609138125))
* **ui:** polish review banner and end-review modal (dogfood feedback) ([e7d83dd](https://github.com/sdavisde/redquill/commit/e7d83dd5287ad1d2fb9c655852b283782eac9d34))

## [0.10.0](https://github.com/sdavisde/redquill/compare/v0.9.0...v0.10.0) (2026-07-16)


### Features

* config layer — TOML config for layout, search, editor, LSP, and keymaps (spec 07) ([e9c7804](https://github.com/sdavisde/redquill/commit/e9c7804e876e795a541bd4bb532fd7ab03f699c1))
* **config:** [keys.&lt;mode&gt;] modal-panel remapping (spec 07 unit 4, task 5) ([613bd35](https://github.com/sdavisde/redquill/commit/613bd3596c890101670d106e9420d7d7cf77ed29))
* **config:** [keys.diff]/[keys.panel] main-keymap remapping (spec 07 unit 4) ([e5656ea](https://github.com/sdavisde/redquill/commit/e5656eadef3f9a93a2ff5dd38255e82718bca31d))
* **config:** [lsp] per-language server overrides (spec 07 unit 3) ([a91f11b](https://github.com/sdavisde/redquill/commit/a91f11bca558ab5d0be5c67431008b71fd08b1c9))
* **config:** config loading infrastructure + [layout]/[search] (spec 07 unit 1) ([a6cf0e1](https://github.com/sdavisde/redquill/commit/a6cf0e1e3fc8ecdea294d5b8bd1dbdba42b768cd))
* **config:** editor templating and presets ([editor] section) ([c02d833](https://github.com/sdavisde/redquill/commit/c02d833435d0391adae104b4565dc63220d6c1b9))


### Bug Fixes

* resolve release workflow issue ([be56bda](https://github.com/sdavisde/redquill/commit/be56bdac089e5a7a5b24954514e64a345d317212))

## [0.9.0](https://github.com/sdavisde/redquill/compare/v0.8.0...v0.9.0) (2026-07-15)


### Features

* **ui:** enable kitty keyboard protocol for modifier disambiguation ([1a8e4a9](https://github.com/sdavisde/redquill/commit/1a8e4a957ca8ec42de9a4610ab46da7843b60de4))
* **ui:** open file in external editor at cursor line via g&lt;Space&gt; ([fd1fe1b](https://github.com/sdavisde/redquill/commit/fd1fe1b6c09a3fb610c4e62074785692194046fb))
* **ui:** shift-enter newline + desktop-style keys in compose/commit modals ([a3a142c](https://github.com/sdavisde/redquill/commit/a3a142c7c19852014397951db0ec531253bcb413))
* **ui:** soft-wrap and cursor-following scroll in compose/commit modals ([99912ea](https://github.com/sdavisde/redquill/commit/99912eaf00c660f0deacad1206b046ae8d7335d5))
* **ui:** word/line/doc motions in compose text buffer ([4e9f88b](https://github.com/sdavisde/redquill/commit/4e9f88b53cde74aea717c0e3db8777861ba2e8d1))

## [0.8.0](https://github.com/sdavisde/redquill/compare/v0.7.0...v0.8.0) (2026-07-15)


### Features

* **ui:** two-focus model for Project Search (Esc/Tab/`/`) ([e4990ca](https://github.com/sdavisde/redquill/commit/e4990ca660844557cfa14087cb7861efbeb12ba5))

## [0.7.0](https://github.com/sdavisde/redquill/compare/v0.6.0...v0.7.0) (2026-07-15)


### Features

* **annotate:** group annotation output by diff source with a Reviewing: line ([55fa2b9](https://github.com/sdavisde/redquill/commit/55fa2b98f130fc5222ea02afbca12da804dc907e))
* **git:** add DiffTarget capability triple (is_live/staging_mode/supports_code_intel) ([611ee3b](https://github.com/sdavisde/redquill/commit/611ee3bb10083f68e599f9891f436a770ad7cca5))
* **git:** add DiffTarget::Commit and the commit-log read model ([7ac9a91](https://github.com/sdavisde/redquill/commit/7ac9a91f28f01396deaa0de1202c1a57a426522f))
* **ui:** add git panel History tab and commit view ([d0eed59](https://github.com/sdavisde/redquill/commit/d0eed59c23bcfa8cfaec48f9df7aa0c3556a70d0))
* **ui:** expand vim motions for faster diff navigation ([11eb279](https://github.com/sdavisde/redquill/commit/11eb2794e3af82fc22c657908ba31e12d4febe71))
* **ui:** publish unpublished branches with P instead of failing a plain push ([532b1e2](https://github.com/sdavisde/redquill/commit/532b1e213617c3ae7b7b464993d4e0e30904c3ea))
* **ui:** show a keyed welcome state instead of a blank empty diff ([82e4b08](https://github.com/sdavisde/redquill/commit/82e4b08cc5329e3825b0f23acf06804c1161713e))


### Bug Fixes

* clamp git panel size ([c938956](https://github.com/sdavisde/redquill/commit/c938956fa2320da410c4d39cdbaf30cb80e70e83))
* **ui:** gate LSP code-intel on DiffTarget::supports_code_intel ([b7284fc](https://github.com/sdavisde/redquill/commit/b7284fc29bebf3608afc8453d236c7ab8982932f))

## [0.6.0](https://github.com/sdavisde/redquill/compare/v0.5.0...v0.6.0) (2026-07-13)


### Features

* **ui:** add scrolloff to line motions and reveal hunk/file jumps at viewport top ([b7bfb8c](https://github.com/sdavisde/redquill/commit/b7bfb8c566195b7dc693b12120b3ceee96cf3f47))
* **ui:** blend cursor-row highlight with diff tints and bold gutter line numbers ([6119754](https://github.com/sdavisde/redquill/commit/6119754d3d23c712718b643e093803d9a5867cb7))
* **ui:** commit staged changes from the git panel ([897a501](https://github.com/sdavisde/redquill/commit/897a50145bcfbd6ea818791c40af4267322aba04))


### Bug Fixes

* allow viewing files after they've been staged ([35177c3](https://github.com/sdavisde/redquill/commit/35177c387afd6b6b1610db945d28a3058fbf67d5))
* **ui:** keep files in stable path order when staged ([3fcd923](https://github.com/sdavisde/redquill/commit/3fcd92389feed1e15023fab2349ee148cba2bbbf))

## [0.5.0](https://github.com/sdavisde/redquill/compare/v0.4.1...v0.5.0) (2026-07-12)


### Features

* **ui:** filter the keybind help overlay with / search ([e093d19](https://github.com/sdavisde/redquill/commit/e093d1970ebc8d674eb141350a3c74ffaefae39d))
* **ui:** hide the git panel until opened with backtick ([19252c7](https://github.com/sdavisde/redquill/commit/19252c7264149d66b1991cea9e4bddbbf3651947))
* **ui:** jump to the top/bottom of the diff with vim-style gg and G ([0093b18](https://github.com/sdavisde/redquill/commit/0093b1880367cb40b07478d116b62921a2a19485))
* **ui:** show context-sensitive key hints in the footer ([52aa593](https://github.com/sdavisde/redquill/commit/52aa59391c8de0a43c670d7937bb09f5dabde1f4))
* **ui:** size the diff gutter to fit the largest line number ([1fa97ae](https://github.com/sdavisde/redquill/commit/1fa97aee0c2df73f266fd607eb69f6eebd27cb10))
* **ui:** visually separate annotations and file headers from diff content ([162afbd](https://github.com/sdavisde/redquill/commit/162afbd169b525c8b73051225fd6d0e30f08fe42))

## [0.4.1](https://github.com/sdavisde/redquill/compare/v0.4.0...v0.4.1) (2026-07-12)


### Bug Fixes

* pin bare-remote test fixtures to main regardless of host git config ([4ddfb04](https://github.com/sdavisde/redquill/commit/4ddfb043f73afa4b9ec17b0f7ab30b13a8bd733e))

## [0.4.0](https://github.com/sdavisde/redquill/compare/v0.3.0...v0.4.0) (2026-07-12)


### Features

* **diff:** auto-refresh the diff from the working tree, plus `R` to reload ([30f5a6d](https://github.com/sdavisde/redquill/commit/30f5a6d333bbbbfa1e9ce13c8e0a1d7f74ea57ab))
* **ui:** add branch/worktree switcher modal shell ([95c7878](https://github.com/sdavisde/redquill/commit/95c787815cb88d7e68da753cb833711ac900b83c))
* **ui:** follow the git panel cursor in the diff view ([b93dd79](https://github.com/sdavisde/redquill/commit/b93dd79d79a7e7d5939ef1357feacb357fe46f9c))
* **ui:** make the help overlay shorter and scrollable ([ee0558f](https://github.com/sdavisde/redquill/commit/ee0558f70852c6f88a640afafa27eaa59fbd6a5e))
* **ui:** quit with `q` from the git panel; keep it inert over overlays ([af8a7a6](https://github.com/sdavisde/redquill/commit/af8a7a67c61d7f8afda599b2e57800dcaea001f0))
* **ui:** switch branches and re-root onto worktrees from the switcher ([fd7c769](https://github.com/sdavisde/redquill/commit/fd7c769cf11c1dd1951fa3acb6fb02fc743e1a33))


### Performance Improvements

* **diff:** poll the working tree off the render thread ([9169151](https://github.com/sdavisde/redquill/commit/916915196709e05403f65e52835394b354a326c2))

## [0.3.0](https://github.com/sdavisde/redquill/compare/v0.2.0...v0.3.0) (2026-07-11)


### Features

* add async remote ops (fetch/pull/push) and command log pane ([0bc7c71](https://github.com/sdavisde/redquill/commit/0bc7c71ea1b2ccd811620e5b9a9c264156af8dfc))
* add branch, ahead/behind, and stash read models to git module ([9db8e72](https://github.com/sdavisde/redquill/commit/9db8e72860802897d68365c9905cfdb318201c8a))
* add git panel focus and keyboard navigation ([1f22a97](https://github.com/sdavisde/redquill/commit/1f22a97fb225c769c524f0bee8ba7ec1f9ac5ef3))
* add side-by-side diff view ([ce150a7](https://github.com/sdavisde/redquill/commit/ce150a71cde5366b63aa73d04c3822af69eff473))
* add transport-agnostic background-task poller ([c91dc50](https://github.com/sdavisde/redquill/commit/c91dc50658a3b363c7b82cd13ede91dd87780f03))
* render git panel with branch header and sectioned display ([7ed21e4](https://github.com/sdavisde/redquill/commit/7ed21e4a85fe2e5ed481ec912ddf9aeb3ce7c3a5))

## [0.2.0](https://github.com/sdavisde/redquill/compare/v0.1.0...v0.2.0) (2026-07-10)


### Features

* add annotation model and markdown serialization ([a9b0b86](https://github.com/sdavisde/redquill/commit/a9b0b86faaa662278b10ec7f52f471372a3f0f32))
* add annotation UI with compose modal, inline display, and list panel ([59fa3ed](https://github.com/sdavisde/redquill/commit/59fa3ed3edb95203e014b26937b7dffb9ba91586))
* add diff model with hunk parsing and word-level intra-line diff ([4cb01d6](https://github.com/sdavisde/redquill/commit/4cb01d6f37d89e3299b25f2a1ee3f7854abd411d))
* add git module for status and per-file diff retrieval ([0150343](https://github.com/sdavisde/redquill/commit/0150343c75ed9e9288df357309bd5c264f25a571))
* add index staging plumbing with hunk and line granularity ([841d2e6](https://github.com/sdavisde/redquill/commit/841d2e6f351c1bfe6d87eef984390149b2102ee9))
* add LSP client with server lifecycle and definition/references/hover ([abb93b8](https://github.com/sdavisde/redquill/commit/abb93b8f8ac905ccb37689a72730e238b93cc951))
* add LSP peek overlays with go-to-definition, references, and hover ([1d04334](https://github.com/sdavisde/redquill/commit/1d04334b9336de110f53da506fc938c2b01b6cb0))
* add ratatui diff viewer with sidebar, navigation, and stderr rendering ([99e889c](https://github.com/sdavisde/redquill/commit/99e889ca26937b0ac3998b6441602136c5e598af))
* add staging UI with hunk/line granularity and staging panel ([89d7524](https://github.com/sdavisde/redquill/commit/89d7524416872e77ff78e58b791d6c63468a32cf))
* add tree-sitter syntax highlighting engine ([bd09faf](https://github.com/sdavisde/redquill/commit/bd09faf501dba88c11f43d23c6366c5ced3569c7))
* scaffold module layout and CLI parsing ([0faf0d5](https://github.com/sdavisde/redquill/commit/0faf0d595de81e7d4f2263f99514b1b7ad57a9a6))
* wire syntax highlighting into diff view and add search ([38a4b56](https://github.com/sdavisde/redquill/commit/38a4b56354ea39e0633dee9d075e4c78f2423ff3))
