//! ブログモード(タグを意識しない文章)⇔コーディングモード(生HTML)の
//! 相互変換。
//!
//! ブログ側の記法(Markdown風の小さな部分集合):
//! - ブロック: 見出し(`# `〜`###### `)・段落・箇条書き(`- `)・番号付き(`1. `)・
//!   引用(`> `)・コードブロック(```` ``` ````、言語名つき可)・区切り線(`---`)・
//!   表(`| a | b |`+区切り行`| --- | --- |`)
//! - インライン: `**太字**`・`*斜体*`・`` `コード` ``・`[文字](URL)`・`![代替](URL)`
//!
//! 逆変換(HTML→ブログ)は簡易DOMパーサー経由で、上記に対応するタグに加え、
//! `div`/`section`/`span`等の汎用タグも辿って文章を取り出す(`head`/`script`/`style`
//! は捨てる)。このコンバーターが生成したHTMLを開き直しても崩れない(ラウンド
//! トリップ)ことを必須要件とする(`CLAUDE.md`参照)。任意のHTMLの見た目まで
//! 再現するものではなく、対応外の構造(入れ子リスト・複雑な表等)は平坦化される。
//!
//! 安全性: `javascript:`/`data:`/`vbscript:`のURLはリンク/画像にしない。

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
pub enum EditorError {
    #[error("変換エラー: {0}")]
    Conversion(String),
}

// ============================================================
// 共通: エスケープ
// ============================================================

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

fn unescape_html(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn safe_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    !l.is_empty() && !(l.starts_with("javascript:") || l.starts_with("data:") || l.starts_with("vbscript:"))
}

// ============================================================
// ブログ → HTML
// ============================================================

fn find_sub(chars: &[char], from: usize, pat: &[char]) -> Option<usize> {
    if pat.is_empty() {
        return None;
    }
    let mut i = from;
    while i + pat.len() <= chars.len() {
        if &chars[i..i + pat.len()] == pat {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `chars[i] == '['`から`[文字](URL)`を読む。(文字, URL, 次の位置)
fn parse_link(chars: &[char], i: usize) -> Option<(String, String, usize)> {
    let close = chars.get(i + 1..)?.iter().position(|&c| c == ']')? + i + 1;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = chars.get(close + 2..)?.iter().position(|&c| c == ')')? + close + 2;
    let text: String = chars[i + 1..close].iter().collect();
    let url: String = chars[close + 2..end].iter().collect();
    Some((text, url.trim().to_string(), end + 1))
}

/// 強調の中身として妥当か(空でなく、前後が空白でない。`a * b * c`を誤変換しない)。
fn emphasis_ok(inner: &[char]) -> bool {
    match (inner.first(), inner.last()) {
        (Some(f), Some(l)) => !f.is_whitespace() && !l.is_whitespace(),
        _ => false,
    }
}

fn inline_to_html(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '`' => {
                if let Some(end) = find_sub(&chars, i + 1, &['`']) {
                    if end > i + 1 {
                        let code: String = chars[i + 1..end].iter().collect();
                        out.push_str(&format!("<code>{}</code>", escape_text(&code)));
                        i = end + 1;
                        continue;
                    }
                }
            }
            '!' if chars.get(i + 1) == Some(&'[') => {
                if let Some((alt, url, end)) = parse_link(&chars, i + 1) {
                    if safe_url(&url) {
                        out.push_str(&format!("<img src=\"{}\" alt=\"{}\">", escape_attr(&url), escape_attr(&alt)));
                        i = end;
                        continue;
                    }
                }
            }
            '[' => {
                if let Some((t, url, end)) = parse_link(&chars, i) {
                    if safe_url(&url) {
                        out.push_str(&format!("<a href=\"{}\">{}</a>", escape_attr(&url), inline_to_html(&t)));
                        i = end;
                        continue;
                    }
                }
            }
            '*' => {
                if chars.get(i + 1) == Some(&'*') {
                    if let Some(end) = find_sub(&chars, i + 2, &['*', '*']) {
                        if emphasis_ok(&chars[i + 2..end]) {
                            let inner: String = chars[i + 2..end].iter().collect();
                            out.push_str(&format!("<strong>{}</strong>", inline_to_html(&inner)));
                            i = end + 2;
                            continue;
                        }
                    }
                } else if let Some(end) = find_sub(&chars, i + 1, &['*']) {
                    if emphasis_ok(&chars[i + 1..end]) {
                        let inner: String = chars[i + 1..end].iter().collect();
                        out.push_str(&format!("<em>{}</em>", inline_to_html(&inner)));
                        i = end + 1;
                        continue;
                    }
                }
            }
            _ => {}
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

enum Block {
    Heading(usize, String),
    Para(Vec<String>),
    Ul(Vec<String>),
    Ol(Vec<String>),
    Quote(Vec<String>),
    Code(String, String),
    Hr,
    Table(Vec<Vec<String>>),
}

fn heading(t: &str) -> Option<(usize, &str)> {
    let n = t.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&n) && t[n..].starts_with(' ') {
        Some((n, t[n..].trim()))
    } else {
        None
    }
}

fn ordered(t: &str) -> Option<&str> {
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && t[digits..].starts_with(". ") {
        Some(t[digits + 2..].trim())
    } else {
        None
    }
}

fn is_block_start(t: &str) -> bool {
    t.starts_with("```")
        || heading(t).is_some()
        || t.starts_with("- ")
        || ordered(t).is_some()
        || t.starts_with('>')
        || t.starts_with('|')
        || t == "---"
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_separator_row(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim_matches(':');
            !c.is_empty() && c.chars().all(|ch| ch == '-')
        })
}

fn parse_blocks(text: &str) -> Vec<Block> {
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.is_empty() {
            i += 1;
            continue;
        }
        if let Some(lang) = t.strip_prefix("```") {
            let lang = lang.trim().to_string();
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            i += 1; // 閉じのフェンス
            blocks.push(Block::Code(lang, code.join("\n")));
        } else if let Some((lvl, rest)) = heading(t) {
            blocks.push(Block::Heading(lvl, rest.to_string()));
            i += 1;
        } else if t == "---" {
            blocks.push(Block::Hr);
            i += 1;
        } else if t.starts_with("- ") {
            let mut items = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with("- ") {
                items.push(lines[i].trim()[2..].trim().to_string());
                i += 1;
            }
            blocks.push(Block::Ul(items));
        } else if ordered(t).is_some() {
            let mut items = Vec::new();
            while i < lines.len() {
                match ordered(lines[i].trim()) {
                    Some(rest) => items.push(rest.to_string()),
                    None => break,
                }
                i += 1;
            }
            blocks.push(Block::Ol(items));
        } else if t.starts_with('>') {
            let mut q = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('>') {
                let l = lines[i].trim()[1..].trim_start();
                q.push(l.to_string());
                i += 1;
            }
            blocks.push(Block::Quote(q));
        } else if t.starts_with('|') {
            let start = i;
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('|') {
                rows.push(split_row(lines[i]));
                i += 1;
            }
            if rows.len() >= 2 && is_separator_row(&rows[1]) {
                rows.remove(1);
                blocks.push(Block::Table(rows));
            } else {
                // 区切り行が無ければ表ではなく段落として扱う
                blocks.push(Block::Para(lines[start..i].iter().map(|l| l.trim().to_string()).collect()));
            }
        } else {
            let mut para = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l.is_empty() || (!para.is_empty() && is_block_start(l)) {
                    break;
                }
                para.push(l.to_string());
                i += 1;
            }
            blocks.push(Block::Para(para));
        }
    }
    blocks
}

/// ブログモードの文章をHTMLへ変換する。
pub fn blog_to_html(text: &str) -> Result<String, EditorError> {
    let mut html = String::new();
    for block in parse_blocks(text) {
        match block {
            Block::Heading(n, s) => html.push_str(&format!("<h{n}>{}</h{n}>\n", inline_to_html(&s))),
            Block::Para(lines) => {
                let joined = lines.iter().map(|l| inline_to_html(l)).collect::<Vec<_>>().join("<br>\n");
                html.push_str(&format!("<p>{joined}</p>\n"));
            }
            Block::Ul(items) => push_list(&mut html, "ul", &items),
            Block::Ol(items) => push_list(&mut html, "ol", &items),
            Block::Quote(lines) => {
                html.push_str("<blockquote>\n");
                for para in lines.split(|l| l.is_empty()).filter(|p| !p.is_empty()) {
                    let joined = para.iter().map(|l| inline_to_html(l)).collect::<Vec<_>>().join("<br>\n");
                    html.push_str(&format!("<p>{joined}</p>\n"));
                }
                html.push_str("</blockquote>\n");
            }
            Block::Code(lang, code) => {
                let class = if lang.is_empty() { String::new() } else { format!(" class=\"language-{}\"", escape_attr(&lang)) };
                html.push_str(&format!("<pre><code{class}>{}</code></pre>\n", escape_text(&code)));
            }
            Block::Hr => html.push_str("<hr>\n"),
            Block::Table(rows) => {
                html.push_str("<table>\n");
                for (r, row) in rows.iter().enumerate() {
                    if r == 0 {
                        html.push_str("<thead>\n");
                    } else if r == 1 {
                        html.push_str("<tbody>\n");
                    }
                    let tag = if r == 0 { "th" } else { "td" };
                    let cells: String = row.iter().map(|c| format!("<{tag}>{}</{tag}>", inline_to_html(c))).collect();
                    html.push_str(&format!("<tr>{cells}</tr>\n"));
                    if r == 0 {
                        html.push_str("</thead>\n");
                    }
                }
                if rows.len() > 1 {
                    html.push_str("</tbody>\n");
                }
                html.push_str("</table>\n");
            }
        }
    }
    Ok(html)
}

fn push_list(html: &mut String, tag: &str, items: &[String]) {
    html.push_str(&format!("<{tag}>
"));
    for item in items {
        html.push_str(&format!("  <li>{}</li>
", inline_to_html(item)));
    }
    html.push_str(&format!("</{tag}>
"));
}

// ============================================================
// HTML → ブログ(簡易DOM)
// ============================================================

#[derive(Debug)]
enum Node {
    Text(String),
    Elem { name: String, attrs: Vec<(String, String)>, children: Vec<Node> },
}

const VOID: &[&str] = &["br", "hr", "img", "meta", "link", "input", "area", "base", "col", "source", "wbr"];

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = s.chars().collect();
    let mut attrs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        let ks = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            i += 1;
        }
        if ks == i {
            break;
        }
        let key: String = chars[ks..i].iter().collect::<String>().to_ascii_lowercase();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                i += 1;
                let vs = i;
                while i < chars.len() && chars[i] != q {
                    i += 1;
                }
                val = chars[vs..i].iter().collect();
                i += 1;
            } else {
                let vs = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                val = chars[vs..i].iter().collect();
            }
        }
        attrs.push((key, unescape_html(&val)));
    }
    attrs
}

type Frame = (String, Vec<(String, String)>, Vec<Node>);

fn close_top(stack: &mut Vec<Frame>) {
    if let Some((name, attrs, children)) = stack.pop() {
        if let Some(parent) = stack.last_mut() {
            parent.2.push(Node::Elem { name, attrs, children });
        }
    }
}

fn parse_html(html: &str) -> Vec<Node> {
    let mut stack: Vec<Frame> = vec![(String::new(), vec![], vec![])];
    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map(|p| p + 3).unwrap_or(rest.len());
        } else if rest.starts_with('<') {
            let Some(rel) = rest.find('>') else {
                stack.last_mut().unwrap().2.push(Node::Text(rest.to_string()));
                break;
            };
            let inner = &rest[1..rel];
            i += rel + 1;
            if inner.starts_with('!') || inner.starts_with('?') {
                continue;
            }
            if let Some(close) = inner.strip_prefix('/') {
                let name = close.trim().to_ascii_lowercase();
                if let Some(pos) = stack.iter().rposition(|f| f.0 == name) {
                    if pos > 0 {
                        while stack.len() > pos {
                            close_top(&mut stack);
                        }
                    }
                }
                continue;
            }
            let self_closing = inner.trim_end().ends_with('/');
            let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
            let name = inner[..name_end].to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            let attrs = parse_attrs(&inner[name_end..]);
            if name == "script" || name == "style" {
                let lower = html[i..].to_ascii_lowercase();
                let end = lower.find(&format!("</{name}")).unwrap_or(lower.len());
                let body = html[i..i + end].to_string();
                i += end;
                i += html[i..].find('>').map(|p| p + 1).unwrap_or(html.len() - i);
                stack.last_mut().unwrap().2.push(Node::Elem { name, attrs, children: vec![Node::Text(body)] });
            } else if VOID.contains(&name.as_str()) || self_closing {
                stack.last_mut().unwrap().2.push(Node::Elem { name, attrs, children: vec![] });
            } else {
                stack.push((name, attrs, vec![]));
            }
        } else {
            let next = rest.find('<').unwrap_or(rest.len());
            stack.last_mut().unwrap().2.push(Node::Text(rest[..next].to_string()));
            i += next;
        }
    }
    while stack.len() > 1 {
        close_top(&mut stack);
    }
    stack.pop().map(|f| f.2).unwrap_or_default()
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn is_inline(name: &str) -> bool {
    matches!(
        name,
        "strong" | "b" | "em" | "i" | "code" | "a" | "img" | "br" | "span" | "u" | "small" | "mark" | "sup" | "sub" | "abbr" | "font" | "s" | "del" | "ins"
    )
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

fn text_content(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| match n {
            Node::Text(t) => unescape_html(t),
            Node::Elem { children, .. } => text_content(children),
        })
        .collect()
}

fn wrap_marker(marker: &str, inner: String) -> String {
    let t = inner.trim();
    if t.is_empty() {
        return inner;
    }
    let lead = &inner[..inner.len() - inner.trim_start().len()];
    let trail = &inner[inner.trim_end().len()..];
    format!("{lead}{marker}{t}{marker}{trail}")
}

fn render_inline(nodes: &[&Node]) -> String {
    let mut out = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(&collapse_ws(&unescape_html(t))),
            Node::Elem { name, attrs, children } => {
                let kids: Vec<&Node> = children.iter().collect();
                match name.as_str() {
                    "br" => out.push('\n'),
                    "strong" | "b" => out.push_str(&wrap_marker("**", render_inline(&kids))),
                    "em" | "i" => out.push_str(&wrap_marker("*", render_inline(&kids))),
                    "code" => out.push_str(&wrap_marker("`", text_content(children))),
                    "a" => {
                        let inner = render_inline(&kids);
                        match attr(attrs, "href") {
                            Some(h) if safe_url(h) && !inner.trim().is_empty() => out.push_str(&format!("[{}]({h})", inner.trim())),
                            _ => out.push_str(&inner),
                        }
                    }
                    "img" => {
                        if let Some(src) = attr(attrs, "src").filter(|s| safe_url(s)) {
                            out.push_str(&format!("![{}]({src})", attr(attrs, "alt").unwrap_or("")));
                        }
                    }
                    "script" | "style" => {}
                    _ => out.push_str(&render_inline(&kids)),
                }
            }
        }
    }
    out
}

/// 行ごとに前後の空白を除き、先頭/末尾の空行を落とす。
fn tidy(s: &str) -> String {
    s.split('\n').map(str::trim).collect::<Vec<_>>().join("\n").trim_matches('\n').to_string()
}

fn flush_inline(out: &mut Vec<String>, buf: &mut Vec<&Node>) {
    if !buf.is_empty() {
        let s = tidy(&render_inline(buf));
        if !s.is_empty() {
            out.push(s);
        }
        buf.clear();
    }
}

fn cell_text(children: &[Node]) -> String {
    let kids: Vec<&Node> = children.iter().collect();
    tidy(&render_inline(&kids)).replace('\n', " ")
}

fn collect_rows(nodes: &[Node], rows: &mut Vec<Vec<String>>) {
    for n in nodes {
        if let Node::Elem { name, children, .. } = n {
            if name == "tr" {
                let cells = children
                    .iter()
                    .filter_map(|c| match c {
                        Node::Elem { name, children, .. } if name == "th" || name == "td" => Some(cell_text(children)),
                        _ => None,
                    })
                    .collect();
                rows.push(cells);
            } else {
                collect_rows(children, rows);
            }
        }
    }
}

fn render_blocks(nodes: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf: Vec<&Node> = Vec::new();
    for n in nodes {
        match n {
            Node::Text(_) => buf.push(n),
            Node::Elem { name, .. } if is_inline(name) => buf.push(n),
            Node::Elem { name, attrs, children } => {
                flush_inline(&mut out, &mut buf);
                let kids: Vec<&Node> = children.iter().collect();
                match name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let lvl: usize = name[1..].parse().unwrap_or(1);
                        let t = tidy(&render_inline(&kids)).replace('\n', " ");
                        if !t.is_empty() {
                            out.push(format!("{} {t}", "#".repeat(lvl)));
                        }
                    }
                    "p" => {
                        let t = tidy(&render_inline(&kids));
                        if !t.is_empty() {
                            out.push(t);
                        }
                    }
                    "ul" | "ol" => {
                        let items: Vec<String> = children
                            .iter()
                            .filter_map(|c| match c {
                                Node::Elem { name, children, .. } if name == "li" => {
                                    Some(render_blocks(children).join(" ").replace('\n', " "))
                                }
                                _ => None,
                            })
                            .collect();
                        let lines: Vec<String> = items
                            .iter()
                            .enumerate()
                            .map(|(i, t)| if name == "ol" { format!("{}. {t}", i + 1) } else { format!("- {t}") })
                            .collect();
                        if !lines.is_empty() {
                            out.push(lines.join("\n"));
                        }
                    }
                    "blockquote" => {
                        let inner = render_blocks(children).join("\n\n");
                        let quoted: Vec<String> =
                            inner.split('\n').map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") }).collect();
                        if !inner.is_empty() {
                            out.push(quoted.join("\n"));
                        }
                    }
                    "pre" => {
                        let lang = children
                            .iter()
                            .find_map(|c| match c {
                                Node::Elem { name, attrs, .. } if name == "code" => {
                                    attr(attrs, "class").and_then(|c| c.split_whitespace().find_map(|w| w.strip_prefix("language-")))
                                }
                                _ => None,
                            })
                            .unwrap_or("");
                        let code = text_content(children);
                        out.push(format!("```{lang}\n{}\n```", code.trim_matches('\n')));
                    }
                    "hr" => out.push("---".to_string()),
                    "table" => {
                        let mut rows = Vec::new();
                        collect_rows(children, &mut rows);
                        if !rows.is_empty() {
                            let cols = rows[0].len().max(1);
                            let mut lines = Vec::new();
                            for (r, row) in rows.iter().enumerate() {
                                lines.push(format!("| {} |", row.join(" | ")));
                                if r == 0 {
                                    lines.push(format!("|{}", " --- |".repeat(cols)));
                                }
                            }
                            out.push(lines.join("\n"));
                        }
                    }
                    "head" | "script" | "style" | "title" | "meta" | "link" | "noscript" | "template" => {}
                    _ => {
                        let _ = attrs;
                        out.extend(render_blocks(children));
                    }
                }
            }
        }
    }
    flush_inline(&mut out, &mut buf);
    out
}

/// HTMLをブログモードの文章へ逆変換する(対応外の構造は平坦化される)。
pub fn html_to_blog(html: &str) -> Result<String, EditorError> {
    Ok(render_blocks(&parse_html(html)).join("\n\n"))
}

/// `blog_to_html`のString引数版(Tauriコマンド/CLIコマンドいずれからも
/// そのまま使える薄いラッパー)。
pub fn editor_blog_to_html(text: String) -> Result<String, EditorError> {
    blog_to_html(&text)
}

/// `html_to_blog`のString引数版。
pub fn editor_html_to_blog(html: String) -> Result<String, EditorError> {
    html_to_blog(&html)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RICH: &str = "# 見出し1\n\n## 見出し2\n\n段落に[リンク](https://example.com/a?x=1&y=2)と`code`と**太字**と*斜体*。\n2行目です。\n\n- 項目A\n- 項目B\n\n1. 手順1\n2. 手順2\n\n> 引用文\n> 続き\n\n---\n\n```rust\nfn main() { println!(\"<hi>\"); }\n```\n\n![代替テキスト](/img/a.png)\n\n| 名前 | 値 |\n| --- | --- |\n| a | 1 |\n| b | 2 |";

    #[test]
    fn roundtrip_heading_and_paragraph() {
        let blog = "# タイトル\n\n本文の段落です。**強調**もできます。";
        let html = blog_to_html(blog).unwrap();
        assert!(html.contains("<h1>タイトル</h1>"));
        assert!(html.contains("<strong>強調</strong>"));
        let back = html_to_blog(&html).unwrap();
        assert!(back.contains("# タイトル"));
        assert!(back.contains("**強調**"));
    }

    #[test]
    fn roundtrip_list() {
        let blog = "- 項目1\n- 項目2";
        let html = blog_to_html(blog).unwrap();
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>項目1</li>"));
        let back = html_to_blog(&html).unwrap();
        assert!(back.contains("- 項目1"));
        assert!(back.contains("- 項目2"));
    }

    #[test]
    fn rich_document_round_trips_exactly() {
        let html = blog_to_html(RICH).unwrap();
        let back = html_to_blog(&html).unwrap();
        assert_eq!(back, RICH, "ブログ→HTML→ブログで内容が変わった\n--- html ---\n{html}");
    }

    #[test]
    fn generated_html_is_stable_after_a_second_round() {
        let html = blog_to_html(RICH).unwrap();
        let html2 = blog_to_html(&html_to_blog(&html).unwrap()).unwrap();
        assert_eq!(html, html2);
    }

    #[test]
    fn rich_constructs_produce_expected_tags() {
        let html = blog_to_html(RICH).unwrap();
        for needle in [
            "<h2>見出し2</h2>",
            "<a href=\"https://example.com/a?x=1&amp;y=2\">リンク</a>",
            "<code>code</code>",
            "<ol>",
            "<blockquote>",
            "<hr>",
            "<pre><code class=\"language-rust\">",
            "<img src=\"/img/a.png\" alt=\"代替テキスト\">",
            "<table>",
            "<th>名前</th>",
            "<td>1</td>",
        ] {
            assert!(html.contains(needle), "{needle} が無い:\n{html}");
        }
    }

    #[test]
    fn dangerous_urls_are_not_linked() {
        let html = blog_to_html("[x](javascript:alert(1)) と ![y](data:text/html;base64,AAAA)").unwrap();
        assert!(!html.contains("<a "), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.to_lowercase().contains("href=\"javascript"));
    }

    #[test]
    fn code_blocks_and_text_are_escaped() {
        let html = blog_to_html("```\n<script>alert(1)</script>\n```\n\n<b>生のタグ</b> & 記号").unwrap();
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;b&gt;生のタグ&lt;/b&gt; &amp; 記号"));
    }

    #[test]
    fn stray_asterisks_stay_literal() {
        let html = blog_to_html("a * b * c と 5 * 6").unwrap();
        assert!(!html.contains("<em>"), "{html}");
    }

    #[test]
    fn heading_line_does_not_swallow_the_next_line() {
        let html = blog_to_html("# 題\n本文").unwrap();
        assert!(html.contains("<h1>題</h1>"));
        assert!(html.contains("<p>本文</p>"));
    }

    #[test]
    fn arbitrary_html_is_reduced_to_blog_text() {
        let html = "<!doctype html><html><head><title>t</title><style>p{color:red}</style></head><body>\
            <div class=\"x\"><h2>Hi</h2><p>a <span>b</span> <a href=\"/x\">c</a> &amp; d</p><script>bad()</script>\
            <ul><li>one <b>two</b></li></ul></div></body></html>";
        assert_eq!(html_to_blog(html).unwrap(), "## Hi\n\na b [c](/x) & d\n\n- one **two**");
    }

    #[test]
    fn broken_html_does_not_fail() {
        let back = html_to_blog("<p>未閉じ<b>太字<p>次の段落</i></div>").unwrap();
        assert!(back.contains("未閉じ"));
        assert!(back.contains("次の段落"));
    }
}
