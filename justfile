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

# Run parser benchmarks (HTML, Vue and Angular)
bench:
    cargo bench -p oxc-htmlish-parser --bench parse

# Check dependency advisories, licenses and sources
deny:
    cargo deny check

# Regenerate snapshots against the pinned Prettier fixture corpus; review the diff.
conformance *dialects:
    node tasks/conformance/run.ts {{ dialects }}

# Fetch the pinned corpus without running it
conformance-clone:
    node tasks/conformance/run.ts --clone

# Remove the fetched corpus
conformance-clean:
    node tasks/conformance/run.ts --clean

# Panic and AST-invariant fuzzing over arbitrary UTF-8
fuzz-panic dialect count="10000" seed="0":
    node tasks/differential/panic.ts {{ dialect }} {{ count }} {{ seed }}

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
