# Changelog

All notable changes are recorded here. This file is generated from
conventional commits by [git-cliff](https://git-cliff.org) via
[release-plz](https://release-plz.dev).

## [1.0.1] - 2026-07-30

### Documentation

- Correct the stale parts of the spec, and stop quoting exact counts
- Correct the README for 1.0, and make its examples doctests


## [1.0.0] - 2026-07-30

### Bug Fixes

- Truncate a label to its column instead of overrunning it

### Build System

- Make the bundled font a default of neovision

### Documentation

- Bring the demo, README and spec up to date before 1.0
- Path the cross-module intra-doc links through the crate root

### Features

- Make the panel geometry configurable, and drop the old aliases
- Only buttons claim accelerators
- Arrows walk straight through a cluster
- A radio caret moves without choosing, and record why
- Add an interactive embedded-graphics host
- Add radio and checkbox clusters
- Draw a scrollbar on a list that scrolls
- Make how far Enter reaches configurable, conservative by default
- Add a default button, and give Space a uniform meaning
- Give buttons labels, roles and actions
- Add hotkeys, and make FormTheme actually reachable
- Add a Text entry field
- Animate the README demo, and colour the terminal caret
- Add a pixel-framebuffer host and the VGA face

### Refactor

- Rename FormTheme to Theme


## [0.1.0] - 2026-07-30

### Documentation

- State what FormTheme cursor is actually for ([#6](https://github.com/lokkju/neovision/pull/6))

### Features

- Add neovision cell UI toolkit

