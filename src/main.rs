use clap::{Arg, ArgMatches, Command};
use exl_lib::Exl;
use mdbook_preprocessor::{
    Preprocessor, PreprocessorContext,
    book::{Book, Chapter},
    errors::Result,
};
use semver::{Version, VersionReq};
use std::io;
use std::process;

pub fn make_app() -> Command {
    Command::new("external-links2-preprocessor")
        .about(r#"A mdbook preprocessor that adds 'target="_blank"' to anchor tags"#)
        .subcommand(
            Command::new("supports")
                .arg(Arg::new("renderer").required(true))
                .about("Check whether a renderer is supported by this preprocessor"),
        )
}

fn main() {
    let matches = make_app().get_matches();

    let preprocessor = Exl::new();
    if let Some(sub_args) = matches.subcommand_matches("supports") {
        handle_supports(&preprocessor, sub_args);
    } else if let Err(e) = handle_preprocessing(&preprocessor) {
        eprintln!("{}", e);
        process::exit(1);
    }
}

fn handle_preprocessing(pre: &dyn Preprocessor) -> Result<()> {
    let (ctx, book) = mdbook_preprocessor::parse_input(io::stdin())?;

    let book_version = Version::parse(&ctx.mdbook_version)?;
    let version_req = VersionReq::parse(mdbook_preprocessor::MDBOOK_VERSION)?;

    if !version_req.matches(&book_version) {
        eprintln!(
            "Warning: The {} plugin was built against version {} of mdbook, \
             but we're being called from version {}",
            pre.name(),
            mdbook_preprocessor::MDBOOK_VERSION,
            ctx.mdbook_version
        );
    }

    let processed_book = pre.run(&ctx, book)?;
    serde_json::to_writer(io::stdout(), &processed_book)?;

    Ok(())
}

fn handle_supports(pre: &dyn Preprocessor, sub_args: &ArgMatches) -> ! {
    let renderer = sub_args
        .get_one::<String>("renderer")
        .expect("Required argument");

    let supported = pre.supports_renderer(renderer).unwrap_or(false);

    if supported {
        process::exit(0);
    } else {
        process::exit(1);
    }
}

mod exl_lib {
    use mdbook_preprocessor::book::BookItem;
    use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
    use pulldown_cmark_to_cmark::cmark;

    use super::*;

    pub struct Exl;

    impl Exl {
        pub fn new() -> Self {
            Self
        }

        fn replace_anchors(&self, chapter: &mut Chapter) -> Result<String> {
            let mut buf = String::with_capacity(chapter.content.len());
            let mut link_stack = Vec::new();

            // Only enable the extensions mdBook itself uses: this preprocessor re-serializes
            // the whole chapter, and extensions such as superscript or definition lists
            // (part of `Options::all()` since pulldown-cmark 0.10) would rewrite the source,
            // e.g. `^_^` into `<sup>_</sup>` or `:::tip` into a definition list.
            let options = Options::ENABLE_TABLES
                | Options::ENABLE_FOOTNOTES
                | Options::ENABLE_STRIKETHROUGH
                | Options::ENABLE_TASKLISTS
                | Options::ENABLE_HEADING_ATTRIBUTES;
            let events = Parser::new_ext(&chapter.content, options).map(|e| {
                let (ev, (link_type, dest_url, title)) = match e.clone() {
                    Event::Start(Tag::Link {
                        link_type,
                        dest_url,
                        title,
                        id: _,
                    }) => {
                        link_stack.push((link_type, dest_url, title));
                        ("start", link_stack.last().unwrap().clone())
                    }
                    Event::End(TagEnd::Link) => ("end", link_stack.pop().unwrap()),
                    _ => return e,
                };

                match link_type {
                    LinkType::Shortcut
                    | LinkType::Inline
                    | LinkType::Reference
                    | LinkType::Collapsed
                    | LinkType::WikiLink { .. } => {
                        if dest_url.starts_with("http") {
                            if ev == "end" {
                                Event::Html(CowStr::from("</a>"))
                            } else {
                                Event::Html(CowStr::from(format!(
                                    r#"<a href="{dest_url}" title="{title}" target="_blank">"#
                                )))
                            }
                        } else {
                            e
                        }
                    }
                    LinkType::Email => {
                        if ev == "end" {
                            Event::Html(CowStr::from("</a>"))
                        } else {
                            Event::Html(CowStr::from(format!(r#"<a href="mailto:{dest_url}">"#)))
                        }
                    }
                    LinkType::Autolink => {
                        if ev == "end" {
                            Event::Html(CowStr::from("</a>"))
                        } else {
                            Event::Html(CowStr::from(format!(
                                r#"<a href="{dest_url}" target="_blank">"#
                            )))
                        }
                    }
                    LinkType::ReferenceUnknown => e,
                    LinkType::CollapsedUnknown => e,
                    LinkType::ShortcutUnknown => e,
                }
            });

            cmark(events, &mut buf)
                .map(|_| buf)
                .map_err(|err| anyhow::anyhow!("Markdown serialization failed: {err}"))
        }
    }

    impl Preprocessor for Exl {
        fn name(&self) -> &str {
            "external-links2"
        }

        fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book> {
            book.for_each_mut(|bi| match bi {
                BookItem::Chapter(c) => {
                    c.content = self
                        .replace_anchors(c)
                        .expect("Error converting links for chapter");
                }
                BookItem::Separator | BookItem::PartTitle(_) => {}
            });
            Ok(book)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn process(content: &str) -> String {
            let mut chapter = Chapter::new("Test", content.to_string(), "test.md", Vec::new());
            Exl::new()
                .replace_anchors(&mut chapter)
                .expect("preprocessing failed")
        }

        /// Renders markdown to HTML with only the base syntax every mdBook build has.
        /// Deliberately *without* the optional extensions, so that if the preprocessor
        /// rewrites the source through one of them (e.g. `^x^` into `<sup>x</sup>`), the
        /// rendered output differs.
        fn render(md: &str) -> String {
            let options = Options::ENABLE_TABLES
                | Options::ENABLE_FOOTNOTES
                | Options::ENABLE_STRIKETHROUGH
                | Options::ENABLE_TASKLISTS
                | Options::ENABLE_HEADING_ATTRIBUTES;
            let mut html = String::new();
            pulldown_cmark::html::push_html(&mut html, Parser::new_ext(md, options));
            html
        }

        /// Asserts that the preprocessor leaves `content` semantically unchanged. The
        /// serializer may normalize syntax (escape `_` as `\_`, change fence style, ...),
        /// so this compares rendered HTML rather than bytes.
        #[track_caller]
        fn assert_untouched(content: &str) -> String {
            let out = process(content);
            assert_eq!(
                render(&out),
                render(content),
                "source was rewritten into:\n{out}"
            );
            out
        }

        // --- Links ------------------------------------------------------------------

        #[test]
        fn inline_external_link_opens_in_new_tab() {
            assert_eq!(
                process("See [gbadev](https://gbadev.net) for more."),
                r#"See <a href="https://gbadev.net" title="" target="_blank">gbadev</a> for more."#
            );
        }

        #[test]
        fn inline_external_link_keeps_title() {
            assert_eq!(
                process(r#"[gbadev](https://gbadev.net "GBA dev")"#),
                r#"<a href="https://gbadev.net" title="GBA dev" target="_blank">gbadev</a>"#
            );
        }

        #[test]
        fn plain_http_link_opens_in_new_tab() {
            assert!(process("[x](http://example.com)").contains(r#"target="_blank""#));
        }

        #[test]
        fn reference_external_link_opens_in_new_tab() {
            let out = process("Read [the docs][docs].\n\n[docs]: https://example.com/docs\n");
            assert!(
                out.contains(
                    r#"<a href="https://example.com/docs" title="" target="_blank">the docs</a>"#
                ),
                "{out}"
            );
        }

        #[test]
        fn autolink_opens_in_new_tab() {
            assert_eq!(
                process("<https://example.com>"),
                r#"<a href="https://example.com" target="_blank">https://example.com</a>"#
            );
        }

        #[test]
        fn email_autolink_becomes_mailto() {
            assert_eq!(
                process("<someone@example.com>"),
                r#"<a href="mailto:someone@example.com">someone@example.com</a>"#
            );
        }

        #[test]
        fn formatting_inside_link_text_is_kept() {
            assert_eq!(
                process("[**bold** link](https://example.com)"),
                r#"<a href="https://example.com" title="" target="_blank">**bold** link</a>"#
            );
        }

        #[test]
        fn relative_links_are_untouched() {
            assert_untouched("[next chapter](./next.md)");
            assert_untouched("[section](#section)");
            assert_untouched("[image dir](img/foo.png)");
        }

        #[test]
        fn links_in_code_are_untouched() {
            assert_untouched("`[x](https://example.com)`");
            assert_untouched("```md\n[x](https://example.com)\n```");
        }

        #[test]
        fn chapters_without_links_are_untouched() {
            assert_untouched("# Title\n\nSome *emphasis* and **strong** text.");
        }

        // --- Regressions: markdown extensions mdBook doesn't use ----------------------
        //
        // The whole chapter is re-serialized, so any extension that mdBook itself does not
        // enable must stay off; otherwise the source is rewritten before other
        // preprocessors (and mdBook) see it. These used to break with `Options::all()`.

        #[test]
        fn admonition_after_paragraph_is_not_a_definition_list() {
            // With definition lists enabled, `:::tip` after a paragraph parses as the
            // definition `::tip` of the paragraph and is re-serialized as `: ::tip`.
            let out = assert_untouched("Some text.\n\n:::tip Title\n\nTip body.\n\n:::");
            assert!(out.contains("\n:::tip Title\n"), "{out}");
            assert!(!out.contains(": ::"), "{out}");
        }

        #[test]
        fn colon_line_after_paragraph_is_not_a_definition_list() {
            assert_untouched("Term\n: not a definition");
        }

        #[test]
        fn carets_are_not_superscript() {
            // With superscript enabled this became `<sup>\_</sup>`.
            for md in ["Yay <kbd>^_^;;</kbd>", "2^10^ is 1024"] {
                let out = assert_untouched(md);
                assert!(!out.contains("<sup>"), "{out}");
            }
        }

        #[test]
        fn tildes_are_strikethrough_not_subscript() {
            // mdBook enables strikethrough, but not subscript.
            let out = assert_untouched("H~2~O and ~~gone~~");
            assert!(!out.contains("<sub>"), "{out}");
        }

        #[test]
        fn double_brackets_are_not_wikilinks() {
            assert_untouched("[[not a link]]");
        }

        #[test]
        fn quotes_are_not_smartened() {
            // Smart punctuation is an mdBook renderer option (`output.html.smart-punctuation`);
            // a preprocessor must not apply it behind the user's back.
            let out = assert_untouched(r#"It's "quoted" -- ok..."#);
            assert!(!out.contains(['’', '“', '”', '–', '…']), "{out}");
        }

        #[test]
        fn dollar_signs_are_not_math() {
            assert_untouched("Costs $5 and $10");
        }

        #[test]
        fn heading_attributes_are_kept() {
            assert_untouched("## Introduction {#sec-intro}");
        }

        #[test]
        fn tables_are_kept() {
            let out = process("| a | b |\n|---|---|\n| 1 | 2 |");
            assert!(out.contains("|a|b|") || out.contains("| a | b |"), "{out}");
        }
    }
}
