use clap::{Arg, ArgMatches, Command};
use exl_lib::Exl;
use mdbook_preprocessor::{
    book::{Book, Chapter},
    errors::Result,
    Preprocessor, PreprocessorContext,
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
}
