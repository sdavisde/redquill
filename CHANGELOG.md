# Changelog

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
