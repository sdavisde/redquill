# Changelog

## [0.21.0](https://github.com/sdavisde/redquill/compare/v0.20.1...v0.21.0) (2026-07-30)


### Features

* **config:** remappable thread, submit, result, and cleanup modal keys ([#51](https://github.com/sdavisde/redquill/issues/51)) ([039d119](https://github.com/sdavisde/redquill/commit/039d119cf51a0cd50d59984d5f06acd207002544))
* **ui:** announce the GitHub fallback when the forge provider is unresolved ([#44](https://github.com/sdavisde/redquill/issues/44)) ([62f3853](https://github.com/sdavisde/redquill/commit/62f3853602b75f91e21832feae91efd2ecdb830c))
* **ui:** bracket tab nav, deduped modal footers, leaner hint strips ([218c1bd](https://github.com/sdavisde/redquill/commit/218c1bd798c01c153a9696e3ce6ff4582aad4ad7))
* **ui:** compose multi-line review summaries ([#48](https://github.com/sdavisde/redquill/issues/48)) ([b771d6a](https://github.com/sdavisde/redquill/commit/b771d6aaf27e2e250f96719d4c277a105db6d7e5))
* **ui:** drop switch-tab hints from the footer strip ([d0e1fec](https://github.com/sdavisde/redquill/commit/d0e1fecaf563590cb01c5b4242c23cb50dbd0528))
* **ui:** edit the review summary in place, with a visible cursor ([bb00e70](https://github.com/sdavisde/redquill/commit/bb00e702c88328e0becd700c7ad017eb116f4c90))
* **ui:** make the submit modal scrollable with overflow indicators ([#46](https://github.com/sdavisde/redquill/issues/46)) ([96bb1ef](https://github.com/sdavisde/redquill/commit/96bb1ef05536443a497d72c1c17893935440d7b9))
* **ui:** match PR filter against author and branch ([#42](https://github.com/sdavisde/redquill/issues/42)) ([8f7838b](https://github.com/sdavisde/redquill/commit/8f7838b4e87d1a9689bcaba9f37648d71017848d))
* **ui:** name reply targets in the submit preview ([#47](https://github.com/sdavisde/redquill/issues/47)) ([b8614c5](https://github.com/sdavisde/redquill/commit/b8614c5d02b0bd62edaa502a8e574a2846e63006))
* **ui:** per-entry selection in the finished-review cleanup ([#50](https://github.com/sdavisde/redquill/issues/50)) ([9d1bf50](https://github.com/sdavisde/redquill/commit/9d1bf504ad0c7201dc8e6391badd2ff8aacf43e2))
* **ui:** PR description overlay in the picker and review session ([#45](https://github.com/sdavisde/redquill/issues/45)) ([452b498](https://github.com/sdavisde/redquill/commit/452b49819cae60276bf0df05ffe1be847142aadc))
* **ui:** refresh the PR list on demand and on launcher reopen ([411f2a0](https://github.com/sdavisde/redquill/commit/411f2a0b4f393a31bc293fa780783c6f3a99d5ab))
* **ui:** show a per-item result view when a submit partially fails ([#49](https://github.com/sdavisde/redquill/issues/49)) ([b10c547](https://github.com/sdavisde/redquill/commit/b10c547954d916f754b12189e9e081eb009480bd))
* **ui:** show relative timestamps in PR picker rows ([aaad184](https://github.com/sdavisde/redquill/commit/aaad184fb09687a9c2da5ea80880bfb035d51df6))


### Bug Fixes

* **forge:** raise the GitHub PR list cap and surface truncation ([#43](https://github.com/sdavisde/redquill/issues/43)) ([7c0e4b8](https://github.com/sdavisde/redquill/commit/7c0e4b88d39d2eef768d931529f215fe582a5c77))
* reduce duplication in ci pipeline ([458d133](https://github.com/sdavisde/redquill/commit/458d133141050521a1cad8f2af108f240c2c5fc7))
* **ui:** clamp thread-view scrolling to content ([681b35c](https://github.com/sdavisde/redquill/commit/681b35cf6c8ff0c1d32c111151fddf50a23b3833))
* **ui:** clamp thread-view scrolling to content ([d6fd071](https://github.com/sdavisde/redquill/commit/d6fd0712eb811731732db43261cf6651d57e1148))
* **ui:** drop stub-era copy now that PR review is live ([bc52124](https://github.com/sdavisde/redquill/commit/bc52124dab17157cc1b29a738d56b03dc26f7326))
* **ui:** drop stub-era copy now that PR review is live ([2f9b19a](https://github.com/sdavisde/redquill/commit/2f9b19aa805679ac5f8526e12f7fd51eab74febe))
* **ui:** keep the PR title across manual refresh in review sessions ([f0ad09d](https://github.com/sdavisde/redquill/commit/f0ad09d7ec82da617abf76eb4aceebcf0ad38014))
* **ui:** scope launcher footer hints to their tab and thread keys to review sessions ([318b91d](https://github.com/sdavisde/redquill/commit/318b91d29173e1796bc9a079e39105460d5b0070))
* **ui:** scope launcher footer hints to their tab and thread keys to review sessions ([c519873](https://github.com/sdavisde/redquill/commit/c5198738aa0c5034e21ee7dbd49a20b09ff2d38d))

## [0.20.1](https://github.com/sdavisde/redquill/compare/v0.20.0...v0.20.1) (2026-07-30)


### Bug Fixes

* **ui:** keep file-row counts and status visible when the path is long ([dd56785](https://github.com/sdavisde/redquill/commit/dd5678590c8573ff96bb6b8cbde2dca9b0eaf51f))


### Performance Improvements

* **ui:** cache highlights per blob so switching views stops re-highlighting ([7230105](https://github.com/sdavisde/redquill/commit/72301050d16eb22ffb8a34ba29710bc10700f90b))
* **ui:** highlight only what is on screen, not the whole review ([a73e627](https://github.com/sdavisde/redquill/commit/a73e627bb31ab3d4ab28161e884798c97371d64c))

## [0.20.0](https://github.com/sdavisde/redquill/compare/v0.19.0...v0.20.0) (2026-07-29)


### Features

* **ui:** drop the end-review hint from the banner, promote gx to the footer ([28313c8](https://github.com/sdavisde/redquill/commit/28313c88c2655cae9809ef76241641f34a4c9df7))
* **ui:** gx opens the branch or commit too, labelled by what it opens ([4795361](https://github.com/sdavisde/redquill/commit/4795361300fd6afa2ba0f01225487b0b14049a08))
* **ui:** name the PR in the review banner and open it with gx ([3b67af5](https://github.com/sdavisde/redquill/commit/3b67af56b65ba97f28915e25be78c0ce48c41346))


### Bug Fixes

* **ui:** show gx only in a forge PR review, not any review session ([cefecd2](https://github.com/sdavisde/redquill/commit/cefecd2556edb825b00d798528ba1c41f69f0def))

## [0.19.0](https://github.com/sdavisde/redquill/compare/v0.18.0...v0.19.0) (2026-07-29)


### Features

* **ui:** restore a file's changes from the diff view and git panel ([130a8bb](https://github.com/sdavisde/redquill/commit/130a8bbc1953e09d91b53717e60a6373bafb5530))


### Bug Fixes

* **ui:** keep the annotation's classification visible in the compose title ([eaa3fc1](https://github.com/sdavisde/redquill/commit/eaa3fc16b2ba258123f8b357a0cfb726447edadd))

## [0.18.0](https://github.com/sdavisde/redquill/compare/v0.17.2...v0.18.0) (2026-07-28)


### Features

* **ui:** make the diff view's scrolloff configurable, default 10 ([eb88d0b](https://github.com/sdavisde/redquill/commit/eb88d0be8713f5d9f3c8bc19c36598d535566ecf))


### Bug Fixes

* **ui:** land panel-focused files at the top and sync the panel cursor ([0767e73](https://github.com/sdavisde/redquill/commit/0767e73746f8aeffa120c012dd92ab017ca0b632))

## [0.17.2](https://github.com/sdavisde/redquill/compare/v0.17.1...v0.17.2) (2026-07-20)


### Bug Fixes

* **annotate,forge:** carry a context line's opposite-side number into GitLab positions ([e47bd0c](https://github.com/sdavisde/redquill/commit/e47bd0c525841fdd5fa9fb219304a4da06226eda))
* **forge:** explain 401/403 submit failures with a next step ([9e595be](https://github.com/sdavisde/redquill/commit/9e595bee14b9c9e51b44d94c7e1999aa2c0f26d6))
* **forge:** explain 401/403 submit failures with a next step ([e07856d](https://github.com/sdavisde/redquill/commit/e07856d5a7a4832452ef76ae6a692b6fba8e0aff))
* **forge:** pin GitLab diff_refs at review open instead of fetching at submit ([ae3e46b](https://github.com/sdavisde/redquill/commit/ae3e46b1d44a47cc5dab88ad7204dc4514531c12))
* **gitlab:** account for created drafts per item and skip them on resubmit ([3ce3a8e](https://github.com/sdavisde/redquill/commit/3ce3a8e4283b98b2ae3686a3133e950932770a45))
* **ui:** report pending drafts honestly in the submit outcome message ([2e691cd](https://github.com/sdavisde/redquill/commit/2e691cd312fdac9efd3cdc3ab8d93bd621dbd9bf))

## [0.17.1](https://github.com/sdavisde/redquill/compare/v0.17.0...v0.17.1) (2026-07-20)


### Bug Fixes

* **gitlab:** send explicit JSON Content-Type on submit POSTs ([2a8b171](https://github.com/sdavisde/redquill/commit/2a8b171eb008107babf528d9384415b2da746cb3))
* **gitlab:** send explicit JSON Content-Type on submit POSTs ([237038f](https://github.com/sdavisde/redquill/commit/237038f22983760667916f9386a16c42bd3b16b4))

## [0.17.0](https://github.com/sdavisde/redquill/compare/v0.16.0...v0.17.0) (2026-07-20)


### Features

* add closed-type PR/MR head-ref fetch to the git layer ([fc549c3](https://github.com/sdavisde/redquill/commit/fc549c38a0782dd87a9b018112ea179997a512ce))
* add forge module scaffolding with ForgeProvider trait ([1c4b69d](https://github.com/sdavisde/redquill/commit/1c4b69d2c02ec778c1f686060cc7aa5aad46b777))
* add GitLab MR listing and detail reads ([ef13dd2](https://github.com/sdavisde/redquill/commit/ef13dd2c2decf3601ba6e4d5526883a04f089908))
* add the Pull Requests tab to the Review launcher ([ad0ee58](https://github.com/sdavisde/redquill/commit/ad0ee581cf172a9fbd639d8cd12c60c9471a52e6))
* bump review-state schema to v3 with optional forge metadata ([5386847](https://github.com/sdavisde/redquill/commit/53868477f25c8dfeb3990f7dc4a2e9f6fdd9472f))
* cover every Pull Requests tab render state with UI tests ([db481ea](https://github.com/sdavisde/redquill/commit/db481ea1188844535ffe5a8e6e5cd6839a18bffc))
* draft replies to imported PR threads (persisted, in the notes panel) ([cb62a8b](https://github.com/sdavisde/redquill/commit/cb62a8b565c3a5004aec2a9761026ee97554a35d))
* finished-review detection + persisted PR title ([e85262d](https://github.com/sdavisde/redquill/commit/e85262dfae4d89c34bb7d400e86feadd052699b5))
* **forge:** GitHub review payload builder ([34df4f0](https://github.com/sdavisde/redquill/commit/34df4f05647102ab022aa75fe496e71e199b8cb3))
* **forge:** GitHub thread fetch and read-only overlay store ([5181fc4](https://github.com/sdavisde/redquill/commit/5181fc402b70823c515b4f60152ab151ca2f5c72))
* **forge:** overlay thread resolution state and paginate review comments ([b859664](https://github.com/sdavisde/redquill/commit/b8596645befcc4b4cb45a0e043aa57c9dad2e5d4))
* **forge:** pure PR review-thread model with root/reply ordering ([9d67408](https://github.com/sdavisde/redquill/commit/9d6740818b9de69531c0ba559f8947f671edbadc))
* GitLab review submit via draft notes with a visible fallback ([9f6ef5d](https://github.com/sdavisde/redquill/commit/9f6ef5d845fea2df2e82ec23c795e05277caa14a))
* hide a published annotation whose forge copy is already on screen ([f8b2006](https://github.com/sdavisde/redquill/commit/f8b2006eb7a6a153bc29945d0927e84e74f727b0))
* implement the glab credential-lookup checker ([4e4ccbb](https://github.com/sdavisde/redquill/commit/4e4ccbbd02ee2ea5b697dbddecbd6fae8c677bee))
* import GitLab discussion threads into the shared thread model ([aa1861c](https://github.com/sdavisde/redquill/commit/aa1861cff2b566285d91659b7e334ee3d30709be))
* PR checkout into a worktree-backed review session ([802fc90](https://github.com/sdavisde/redquill/commit/802fc908187199086316b23a11bd25b8b090036d))
* PRs-tab cleanup modal for finished reviews ([7817a57](https://github.com/sdavisde/redquill/commit/7817a57765b6a287b08a24fbc903b56777bfb268))
* published-state completion for annotations and draft replies ([2365649](https://github.com/sdavisde/redquill/commit/2365649662c7d9636a17979e3148b98c77f1e3e7))
* re-surface the "comments unavailable" notice + Unit 3 journey proof ([dcadb7f](https://github.com/sdavisde/redquill/commit/dcadb7f10ea936baf36bba7ee6c410c73f815b03))
* render imported PR threads and drafted replies inline in the diff ([79feee5](https://github.com/sdavisde/redquill/commit/79feee52acb9c9a8b1d5569e6ede36cbaa02b5d5))
* submit-review modal and publish driver (GitHub) ([b54c67d](https://github.com/sdavisde/redquill/commit/b54c67d2cd2f0aab14828f2c377d5437c43e75a1))
* **ui:** imported comment-thread overlay for PR review sessions ([58f2941](https://github.com/sdavisde/redquill/commit/58f2941750ba2e38510cd6c47139ea93f34e67a3))
* wire GitLab through the PR-list and checkout flows ([6748895](https://github.com/sdavisde/redquill/commit/6748895e40c02a213e1933467727e6155c2236df))


### Bug Fixes

* prevent forge review-submit 422s (empty COMMENT review, one-line span, request-changes) ([d234018](https://github.com/sdavisde/redquill/commit/d234018da7a680161c00d311f31791366a1bfa2f))
* stop PR-review refresh from duplicating draft annotations and replies ([8d38d99](https://github.com/sdavisde/redquill/commit/8d38d99632135d7634be608639b944f09631cc65))

## [0.16.0](https://github.com/sdavisde/redquill/compare/v0.15.0...v0.16.0) (2026-07-19)


### Features

* annotation list, staging/accepted panel, and switcher adopt the shared `/` filter ([cf5aa33](https://github.com/sdavisde/redquill/commit/cf5aa331182d966d347a58c4db221d94a96c0909))
* git panel (both tabs) consumes the shared motion layer ([4ae4add](https://github.com/sdavisde/redquill/commit/4ae4adda517fd9c052c63c13b776aa6063f182ec))
* modal list contexts consume the shared motion layer ([dd626c8](https://github.com/sdavisde/redquill/commit/dd626c88460c4c037cfc02239b9251ce982225ed))
* Review launcher (both tabs) adopts the shared `/` filter ([3115286](https://github.com/sdavisde/redquill/commit/31152865e6d46bcea9695948c5a78ca41478d266))
* Review launcher (both tabs) consumes the shared motion layer ([4414546](https://github.com/sdavisde/redquill/commit/4414546658c9ff28c54dfed3847742f08471234a))
* shared `/` list-filter component ([684d29d](https://github.com/sdavisde/redquill/commit/684d29d458c604e3c0b104bfcdf239a9bb4c26f6))
* shared motion-set layer (src/ui/motion.rs) ([78a6c2c](https://github.com/sdavisde/redquill/commit/78a6c2c463ae902893bfb4aa53928700027d353e))


### Performance Improvements

* add a 5k-row list-filter re-rank budget tripwire ([7ce3eb3](https://github.com/sdavisde/redquill/commit/7ce3eb373fe1754e93098ccb71dc5d308144af1f))

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
