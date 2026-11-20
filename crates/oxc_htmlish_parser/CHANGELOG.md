# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.9](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.8...oxc-htmlish-parser-v0.0.9) - 2026-11-20

### Added

- *(parser/angular)* mark spread in nested pipe ([#39](https://github.com/oxc-project/oxc-htmlish-parser/pull/39))

### Fixed

- *(parser/angular)* read character references in an expression as its compiler does ([#40](https://github.com/oxc-project/oxc-htmlish-parser/pull/40))

## [0.0.8](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.7...oxc-htmlish-parser-v0.0.8) - 2026-11-20

### Added

- *(parser/angular)* parse Angular nested pipe ([#35](https://github.com/oxc-project/oxc-htmlish-parser/pull/35))

### Fixed

- *(parser)* read escaped quote in Angular attr ([#37](https://github.com/oxc-project/oxc-htmlish-parser/pull/37))
- *(parser/angular)* report unclosed ICU at the end ([#36](https://github.com/oxc-project/oxc-htmlish-parser/pull/36))

## [0.0.7](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.6...oxc-htmlish-parser-v0.0.7) - 2026-11-17

### Added

- *(parser)* parse more Angular syntaxes ([#30](https://github.com/oxc-project/oxc-htmlish-parser/pull/30))

## [0.0.6](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.5...oxc-htmlish-parser-v0.0.6) - 2026-11-17

### Added

- *(parser)* parse Angular microsyntax and expressions ([#27](https://github.com/oxc-project/oxc-htmlish-parser/pull/27))

## [0.0.5](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.4...oxc-htmlish-parser-v0.0.5) - 2026-11-16

### Added

- *(parser)* implement Angular dialect ([#25](https://github.com/oxc-project/oxc-htmlish-parser/pull/25))

## [0.0.4](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.3...oxc-htmlish-parser-v0.0.4) - 2026-11-13

### Added

- *(parser)* split Vue v-for value as AST ([#23](https://github.com/oxc-project/oxc-htmlish-parser/pull/23))

### Fixed

- *(parser)* read v-pre with argument and slot dots ([#24](https://github.com/oxc-project/oxc-htmlish-parser/pull/24))

### Other

- bump Angular's oracle to 22.2.5, keep quirk shapes inside a start tag ([#19](https://github.com/oxc-project/oxc-htmlish-parser/pull/19))
- check attribute invariants, tighten the differential's pending ([#17](https://github.com/oxc-project/oxc-htmlish-parser/pull/17))

## [0.0.3](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.2...oxc-htmlish-parser-v0.0.3) - 2026-11-05

### Fixed

- *(parser)* read Vue's raw text tags by its tokenizer's case rule ([#13](https://github.com/oxc-project/oxc-htmlish-parser/pull/13))

## [0.0.2](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.1...oxc-htmlish-parser-v0.0.2) - 2026-11-02

### Added

- *(parser)* add Xhtml dialect ([#11](https://github.com/oxc-project/oxc-htmlish-parser/pull/11))

## [0.0.1](https://github.com/oxc-project/oxc-htmlish-parser/compare/oxc-htmlish-parser-v0.0.0...oxc-htmlish-parser-v0.0.1) - 2026-10-30

### Other

- consolidate docs into AGENTS.md ([#10](https://github.com/oxc-project/oxc-htmlish-parser/pull/10))
- release v0.0.0 ([#7](https://github.com/oxc-project/oxc-htmlish-parser/pull/7))

## [0.0.0](https://github.com/oxc-project/oxc-htmlish-parser/releases/tag/oxc-htmlish-parser-v0.0.0) - 2026-10-29

### Added

- *(parser)* add ParserOptions::template_placeholder for template placeholders in tag names ([#4](https://github.com/oxc-project/oxc-htmlish-parser/pull/4))
- *(parser)* parse end tags of omitted html, head and body ([#3](https://github.com/oxc-project/oxc-htmlish-parser/pull/3))
- parse Angular control flow blocks
- add differential fuzz against angular-html-parser and vue
- initial htmlish parser with html and vue dialects

### Fixed

- read Angular block names with Angular's whitespace
- read Vue and Angular templates as their compilers do
- self-close on `/>` after an unquoted value in Angular
- follow WHATWG's optional end tags in HTML, `--!>` and script escapes everywhere
- report quotes in attribute names, read `</` without a name as a bogus comment everywhere
- read names, end tags and Angular interpolations as decided against the fuzz findings
- read raw text ends, empty comments and CDATA as the HTML tokenizer does

### Other

- *(parser)* pass self-closing elements in one step when closing implicitly ([#5](https://github.com/oxc-project/oxc-htmlish-parser/pull/5))
- *(parser)* keep the scanning helpers inlined past the dialect checks ([#2](https://github.com/oxc-project/oxc-htmlish-parser/pull/2))
- point README at AGENTS.md, and match what is not parsed yet
- close what a second fresh reader still had to guess
- settle what a fresh reader of the decision procedure had to guess
- hold DIVERGENCES.md to its fixtures and quirks, and probe every reader
- name each divergence by its concern and lead with what we do
- cosmetic changes
- differential fuzz by dialect table, with mutations, files and invariant checks
- split the builder by construct and keep parser parts under `parser/`
- split Angular blocks into their own module and name dialect switches
- record element name prefixes as part of the name
- [**breaking**] store the AST as one node list linked by NodeId
- look up the tag table once per element
