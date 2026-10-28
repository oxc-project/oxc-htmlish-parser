//! `DIVERGENCES.md` against what it names (AGENTS.md "Deciding a difference"):
//! every entry names its deciding step, and every one above the kept section a fixture,
//! every fixture it names exists, its `runner:` names and `tasks/differential/dialects.ts`'s quirks are the same set,
//! and every `DIVERGENCES.md#slug` the sources cite is an entry.

use std::{collections::BTreeSet, fs, path::Path};

const KEPT: &str = "## Kept";

/// The backticked `key: value` items on a line (`` `fixture: a`, `fixture: b` ``).
fn items<'a>(line: &'a str, key: &str) -> impl Iterator<Item = &'a str> {
    let prefix = format!("`{key}: ");
    line.split(", ").filter_map(move |item| item.strip_prefix(&prefix)?.strip_suffix('`'))
}

/// GitHub's heading anchor.
fn slug(title: &str) -> String {
    title
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// The quoted keys of the `quirks` and `mutationQuirks` objects in `dialects.ts`.
fn quirk_names(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut depth = None;
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("quirks: {") || line.starts_with("mutationQuirks: {") {
            depth = Some(0);
            continue;
        }
        let Some(open) = depth.as_mut() else { continue };
        if *open == 0 && line.starts_with('}') {
            depth = None;
            continue;
        }
        if *open == 0
            && let Some(rest) = line.strip_prefix('"')
            && let Some((name, _)) = rest.split_once("\":")
        {
            names.insert(name.to_owned());
        }
        *open += line.matches(['{', '[', '(']).count();
        *open -= line.matches(['}', ']', ')']).count().min(*open);
    }
    names
}

#[test]
fn divergences() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = crate_dir.join("../..");
    let doc = fs::read_to_string(root.join("DIVERGENCES.md")).unwrap();
    let mut problems = vec![];

    let mut slugs = BTreeSet::new();
    let mut runners = BTreeSet::new();
    let (drops, kept) = doc.split_once(KEPT).expect("the kept section");
    for (section, is_kept) in [(drops, false), (kept, true)] {
        for entry in section.split("\n### ").skip(1) {
            let (title, body) = entry.split_once('\n').unwrap();
            slugs.insert(slug(title));
            let lines = || body.lines().take_while(|line| !line.starts_with("## "));
            let fixtures: Vec<&str> = lines().flat_map(|line| items(line, "fixture")).collect();
            runners.extend(lines().flat_map(|line| items(line, "runner")).map(str::to_owned));
            if !lines().any(|line| line.starts_with("`decided by: ")) {
                problems.push(format!("{title}: no `decided by:`"));
            }
            if !is_kept && fixtures.is_empty() {
                problems.push(format!("{title}: no `fixture:`"));
            }
            for fixture in fixtures {
                if !crate_dir.join("tests/fixtures").join(fixture).is_file() {
                    problems.push(format!("{title}: no fixture {fixture}"));
                }
            }
        }
    }

    let quirks =
        quirk_names(&fs::read_to_string(root.join("tasks/differential/dialects.ts")).unwrap());
    for name in runners.difference(&quirks) {
        problems.push(format!("`runner: {name}` is no quirk in dialects.ts"));
    }
    for name in quirks.difference(&runners) {
        problems.push(format!("quirk {name:?} in dialects.ts has no `runner:` entry"));
    }

    let mut sources = vec![crate_dir.join("src")];
    let mut files = vec![];
    while let Some(dir) = sources.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.extend(
        ["dialects.ts", "probe.ts", "run.ts"].map(|f| root.join("tasks/differential").join(f)),
    );
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for cited in source.split("DIVERGENCES.md#").skip(1) {
            let cited: String =
                cited.chars().take_while(|&c| c.is_alphanumeric() || c == '-').collect();
            if !slugs.contains(&cited) {
                problems.push(format!("{}: DIVERGENCES.md#{cited} is no entry", path.display()));
            }
        }
    }

    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
