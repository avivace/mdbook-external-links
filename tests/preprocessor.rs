//! End-to-end tests: run the built binary the way mdBook does, through the
//! `supports` subcommand and the JSON `[context, book]` protocol on stdin/stdout.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_mdbook-external-links2");

fn chapter(name: &str, content: &str, sub_items: Vec<Value>) -> Value {
    json!({
        "Chapter": {
            "name": name,
            "content": content,
            "number": null,
            "sub_items": sub_items,
            "path": format!("{name}.md"),
            "source_path": format!("{name}.md"),
            "parent_names": [],
        }
    })
}

/// Runs the preprocessor over a book with the given items and returns the output book.
fn run(items: Vec<Value>) -> Value {
    let context = json!({
        "root": ".",
        "config": { "book": { "src": "src" } },
        "renderer": "html",
        "mdbook_version": mdbook_preprocessor::MDBOOK_VERSION,
    });
    let input = json!([context, { "items": items }]);

    let mut child = Command::new(BIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn preprocessor");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "preprocessor exited with {}",
        output.status
    );
    serde_json::from_slice(&output.stdout).expect("preprocessor did not output a valid book")
}

fn content(item: &Value) -> &str {
    item["Chapter"]["content"].as_str().unwrap()
}

#[test]
fn supports_html_renderer() {
    let status = Command::new(BIN)
        .args(["supports", "html"])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn rewrites_links_in_nested_chapters() {
    let book = run(vec![
        chapter(
            "parent",
            "[out](https://example.com)",
            vec![chapter(
                "child",
                "[in](./parent.md) and [out](https://example.org)",
                vec![],
            )],
        ),
        json!("Separator"),
    ]);

    let parent = &book["items"][0];
    assert_eq!(
        content(parent),
        r#"<a href="https://example.com" title="" target="_blank">out</a>"#
    );
    let child = &parent["Chapter"]["sub_items"][0];
    assert_eq!(
        content(child),
        r#"[in](./parent.md) and <a href="https://example.org" title="" target="_blank">out</a>"#
    );
    assert_eq!(book["items"][1], json!("Separator"));
}

#[test]
fn leaves_other_preprocessors_syntax_alone() {
    // Syntax that other preprocessors running after this one rely on must survive:
    // admonition fences (definition lists), `^_^` (superscript), `[[...]]` (wikilinks).
    let source = "Intro.\n\n:::tip Title\n\nBody with <kbd>^_^</kbd>.\n\n:::\n\n[[TOC]]";
    let book = run(vec![chapter("page", source, vec![])]);
    let out = content(&book["items"][0]);

    assert!(out.contains("\n:::tip Title\n"), "{out}");
    assert!(!out.contains(": ::"), "{out}");
    assert!(!out.contains("<sup>"), "{out}");
    assert!(!out.contains("<a"), "{out}");
}
