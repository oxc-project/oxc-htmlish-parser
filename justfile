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
