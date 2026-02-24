//! Safe HTML renderer for egui.
//!
//! Converts ammonia-sanitized HTML fragments into styled egui labels
//! with block-level layout (paragraphs, headings, lists, blockquotes,
//! preformatted code) and inline formatting (bold, italic, code, links,
//! underline, strikethrough).

use egui::{text::LayoutJob, Color32, FontFamily, FontId, RichText, Stroke, TextFormat, Ui};
use scraper::{ElementRef, Html, Node};

const BASE_SIZE: f32 = 14.0;

struct Palette {
    text: Color32,
    link: Color32,
    code_fg: Color32,
    code_bg: Color32,
    quote_fg: Color32,
}

impl Palette {
    fn from_ui(ui: &Ui) -> Self {
        Self {
            text: ui.visuals().text_color(),
            link: Color32::from_rgb(0x64, 0x95, 0xED),
            code_fg: Color32::from_rgb(0xE0, 0xB0, 0x60),
            code_bg: Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 0x0A),
            quote_fg: Color32::from_rgb(0x90, 0xA8, 0xC0),
        }
    }
}

/// Render sanitized HTML into the given [`Ui`].
///
/// Returns `true` if visible content was produced, `false` if the HTML
/// contained no renderable text (caller should fall back to plain text).
pub fn render_html(ui: &mut Ui, html: &str) -> bool {
    let safe_html = ammonia::clean(html);
    let doc = Html::parse_fragment(&safe_html);
    let pal = Palette::from_ui(ui);
    let mut ctx = Ctx::new(&pal);
    ctx.walk_elem(ui, doc.root_element());
    ctx.flush(ui);

    if !ctx.links.is_empty() {
        ui.add_space(6.0);
        for (label, url) in &ctx.links {
            ui.hyperlink_to(RichText::new(label).size(13.0).underline(), url);
        }
    }

    ctx.rendered
}

// ---------------------------------------------------------------------------

struct ListLvl {
    ordered: bool,
    idx: u32,
}

struct Ctx<'p> {
    pal: &'p Palette,
    job: LayoutJob,
    // Inline style nesting counters (saturating-sub on exit).
    bold: u32,
    italic: u32,
    code: u32,
    uline: u32,
    strike: u32,
    // Link accumulation.
    link_href: Option<String>,
    link_text: String,
    // Block context.
    heading: u8,
    pre: bool,
    bq: u32,
    lists: Vec<ListLvl>,
    // Layout state.
    gap: bool,
    rendered: bool,
    links: Vec<(String, String)>,
}

impl<'p> Ctx<'p> {
    fn new(pal: &'p Palette) -> Self {
        Self {
            pal,
            job: LayoutJob::default(),
            bold: 0,
            italic: 0,
            code: 0,
            uline: 0,
            strike: 0,
            link_href: None,
            link_text: String::new(),
            heading: 0,
            pre: false,
            bq: 0,
            lists: Vec::new(),
            gap: false,
            rendered: false,
            links: Vec::new(),
        }
    }

    // -- flush / block helpers ---------------------------------------------

    fn flush(&mut self, ui: &mut Ui) {
        if self.job.text.is_empty() {
            return;
        }
        self.rendered = true;
        let mut job = std::mem::take(&mut self.job);
        job.wrap.max_width = ui.available_width();
        ui.label(job);
    }

    fn blk(&mut self, ui: &mut Ui) {
        self.flush(ui);
        if self.gap {
            ui.add_space(4.0);
        }
        self.gap = true;
    }

    // -- text formatting ---------------------------------------------------

    fn fmt(&self) -> TextFormat {
        let sz = match self.heading {
            1 => 24.0,
            2 => 20.0,
            3 => 18.0,
            4 => 16.0,
            5 => 15.0,
            6 => BASE_SIZE,
            _ if self.bold > 0 => BASE_SIZE + 0.5,
            _ => BASE_SIZE,
        };
        let fam = if self.code > 0 || self.pre {
            FontFamily::Monospace
        } else {
            FontFamily::Proportional
        };
        let col = if self.link_href.is_some() {
            self.pal.link
        } else if self.code > 0 {
            self.pal.code_fg
        } else if self.bq > 0 {
            self.pal.quote_fg
        } else {
            self.pal.text
        };
        let bg = if self.code > 0 && !self.pre {
            self.pal.code_bg
        } else {
            Color32::TRANSPARENT
        };
        TextFormat {
            font_id: FontId::new(sz, fam),
            color: col,
            background: bg,
            italics: self.italic > 0,
            underline: if self.link_href.is_some() || self.uline > 0 {
                Stroke::new(1.0, col)
            } else {
                Stroke::NONE
            },
            strikethrough: if self.strike > 0 {
                Stroke::new(1.0, col)
            } else {
                Stroke::NONE
            },
            line_height: Some(sz * 1.5),
            ..Default::default()
        }
    }

    fn push_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if self.link_href.is_some() {
            self.link_text.push_str(s);
        }
        let f = self.fmt();
        if self.pre {
            self.job.append(s, 0.0, f);
        } else {
            let c = collapse_ws(s);
            if !c.is_empty() {
                self.job.append(&c, 0.0, f);
            }
        }
    }

    // -- tree walk ---------------------------------------------------------

    fn walk_children(&mut self, ui: &mut Ui, parent: ElementRef<'_>) {
        for child in parent.children() {
            match child.value() {
                Node::Text(t) => self.push_text(&t.text),
                Node::Element(_) => {
                    if let Some(el) = ElementRef::wrap(child) {
                        self.walk_elem(ui, el);
                    }
                }
                _ => {}
            }
        }
    }

    fn walk_elem(&mut self, ui: &mut Ui, el: ElementRef<'_>) {
        let tag = el.value().name.local.as_ref();
        if !self.enter(tag, el, ui) {
            return;
        }
        self.walk_children(ui, el);
        self.leave(tag, ui);
    }

    fn enter(&mut self, tag: &str, el: ElementRef<'_>, ui: &mut Ui) -> bool {
        match tag {
            // Skip content of these entirely.
            "style" | "script" => return false,

            // Block elements.
            "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "nav"
            | "figure" | "figcaption" | "center" | "dl" => {
                self.blk(ui);
            }
            "br" => {
                let f = self.fmt();
                self.job.append("\n", 0.0, f);
            }
            "hr" => {
                self.blk(ui);
                ui.separator();
            }

            // Headings.
            "h1" => { self.blk(ui); self.heading = 1; }
            "h2" => { self.blk(ui); self.heading = 2; }
            "h3" => { self.blk(ui); self.heading = 3; }
            "h4" => { self.blk(ui); self.heading = 4; }
            "h5" => { self.blk(ui); self.heading = 5; }
            "h6" => { self.blk(ui); self.heading = 6; }

            // Blockquote & preformatted.
            "blockquote" => { self.blk(ui); self.bq += 1; }
            "pre" => { self.blk(ui); self.pre = true; }

            // Lists.
            "ul" => {
                self.blk(ui);
                self.lists.push(ListLvl { ordered: false, idx: 0 });
            }
            "ol" => {
                self.blk(ui);
                self.lists.push(ListLvl { ordered: true, idx: 0 });
            }
            "li" => {
                self.blk(ui);
                let depth = self.lists.len();
                if let Some(lv) = self.lists.last_mut() {
                    lv.idx += 1;
                    let indent = "  ".repeat(depth);
                    let marker = if lv.ordered {
                        format!("{indent}{}. ", lv.idx)
                    } else {
                        format!("{indent}\u{2022} ")
                    };
                    let f = self.fmt();
                    self.job.append(&marker, 0.0, f);
                }
            }
            "dt" => { self.blk(ui); self.bold += 1; }
            "dd" => { self.blk(ui); }

            // Tables (best-effort plain-text layout).
            "table" => self.blk(ui),
            "tr" => {
                if !self.job.text.is_empty() {
                    let f = self.fmt();
                    self.job.append("\n", 0.0, f);
                }
            }
            "td" | "th" => {
                let needs_sep = !self.job.text.is_empty()
                    && !self.job.text.ends_with('\n');
                if needs_sep {
                    let f = self.fmt();
                    self.job.append("  \u{2502}  ", 0.0, f);
                }
                if tag == "th" {
                    self.bold += 1;
                }
            }

            // Inline styling.
            "b" | "strong" => self.bold += 1,
            "i" | "em" | "cite" => self.italic += 1,
            "code" | "kbd" | "samp" => self.code += 1,
            "u" | "ins" => self.uline += 1,
            "s" | "del" | "strike" => self.strike += 1,

            // Links.
            "a" => {
                self.link_href = el.value().attr("href").map(|s| s.to_string());
                self.link_text.clear();
            }

            // Images (placeholder).
            "img" => {
                let alt = el.value().attr("alt").unwrap_or("image");
                let mut f = self.fmt();
                f.italics = true;
                f.color = Color32::GRAY;
                self.job.append(&format!("[{alt}]"), 0.0, f);
            }

            // Everything else: transparent wrapper, just recurse.
            _ => {}
        }
        true
    }

    fn leave(&mut self, tag: &str, ui: &mut Ui) {
        match tag {
            "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "nav"
            | "figure" | "figcaption" | "center" | "dl" => {
                self.blk(ui);
            }

            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.heading = 0;
                self.blk(ui);
            }
            "blockquote" => {
                self.blk(ui);
                self.bq = self.bq.saturating_sub(1);
            }
            "pre" => {
                self.pre = false;
                self.blk(ui);
            }
            "ul" | "ol" => {
                self.lists.pop();
                self.blk(ui);
            }
            "table" => self.blk(ui),

            "dt" => self.bold = self.bold.saturating_sub(1),
            "b" | "strong" => self.bold = self.bold.saturating_sub(1),
            "i" | "em" | "cite" => self.italic = self.italic.saturating_sub(1),
            "code" | "kbd" | "samp" => self.code = self.code.saturating_sub(1),
            "u" | "ins" => self.uline = self.uline.saturating_sub(1),
            "s" | "del" | "strike" => self.strike = self.strike.saturating_sub(1),

            "a" => {
                if let Some(href) = self.link_href.take() {
                    let label = if self.link_text.trim().is_empty() {
                        href.clone()
                    } else {
                        self.link_text.trim().to_string()
                    };
                    self.links.push((label, href));
                    self.link_text.clear();
                }
            }
            "td" | "th" => {
                if tag == "th" {
                    self.bold = self.bold.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
}

/// Collapse runs of whitespace into a single space (standard HTML behaviour).
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !ws {
                out.push(' ');
                ws = true;
            }
        } else {
            out.push(ch);
            ws = false;
        }
    }
    out
}
