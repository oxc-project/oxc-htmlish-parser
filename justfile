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
