# oxc-htmlish-parser tasks. Run `just` (or `just --list`) to see all recipes.

# List available recipes
_default:
    @just --list

# Run the test suite
test:
    cargo test --all-features

# Format the codebase
fmt:
    cargo fmt

# Differential fuzz against the dialect's oracle.
# Requires `npm install` in tasks/differential once, and Node >= 22.18.
differential dialect count="5000" seed="0":
    node tasks/differential/run.ts {{ dialect }} {{ count }} {{ seed }}

# One input through every reader: ours, the oracle, the norm, browsers (html) and Prettier at the pin.
probe dialect source:
    node tasks/differential/probe.ts {{ dialect }} {{ quote(source) }}

# The differential comparison on real files.
differential-files dialect +files:
    node tasks/differential/run.ts {{ dialect }} --files {{ files }}

# What a parser change does to real files: this tree against `rev`, file by file.
compare dialect rev +files:
    node tasks/differential/compare.ts {{ dialect }} {{ rev }} {{ files }}
