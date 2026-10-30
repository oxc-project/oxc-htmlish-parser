# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
