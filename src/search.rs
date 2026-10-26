//! Library search: a small query language compiled to SQL over the media
//! table and its FTS5 index, with typo-tolerant tag matching.
//!
//! `beach -blurry tag:sunset -tag:draft folder:Holidays/2024 root:nas kind:video`
//!
//! Free words are matched fuzzily (prefix and near-miss tags) and drive
//! ranking; prefixed terms are strict filters.

use anyhow::Result;
use rusqlite::{Connection, types::Value};
use serde::Deserialize;

/// What a listing or a bulk operation is filtered by: the structured filters
/// from the sidebar plus an optional free-text query.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub q: Option<String>,
    pub root_id: Option<i64>,
    pub folder: Option<String>,
    #[serde(default)]
    pub recursive: bool,
    pub kind: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Query {
    pub free: Vec<String>,
    pub exclude: Vec<String>,
    pub tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub folders: Vec<String>,
    pub roots: Vec<String>,
    pub kind: Option<&'static str>,
}

struct Token {
    prefix: Option<String>,
    text: String,
    negated: bool,
}

/// Splits a query into terms, honouring double quotes and backslash escapes so
/// `tag:"sunny day"` and `-tag:blurry` both work. A '-' only negates when it
/// opens a term: "well-known" is one word.
fn tokenize(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let (mut text, mut prefix) = (String::new(), None::<String>);
    let (mut negated, mut at_start, mut quote, mut escape) = (false, true, false, false);

    let mut flush = |text: &mut String, prefix: &mut Option<String>, negated: &mut bool| {
        if !text.is_empty() {
            out.push(Token { prefix: prefix.take(), text: std::mem::take(text), negated: *negated });
        }
        text.clear();
        *prefix = None;
        *negated = false;
    };

    for c in input.chars() {
        if c.is_control() && c != '\t' {
            continue;
        }
        if escape {
            escape = false;
            text.push(c);
            at_start = false;
            continue;
        }
        match c {
            '\\' => escape = true,
            '"' => {
                if !text.is_empty() && !quote && prefix.is_none() {
                    flush(&mut text, &mut prefix, &mut negated);
                }
                quote = !quote;
                at_start = false;
            }
            _ if quote => text.push(c),
            '-' if at_start => {
                negated = true;
                at_start = false;
            }
            ':' if prefix.is_none() && !text.is_empty() => {
                prefix = Some(std::mem::take(&mut text).to_lowercase());
            }
            ' ' | '\t' => {
                flush(&mut text, &mut prefix, &mut negated);
                at_start = true;
            }
            _ => {
                text.push(c);
                at_start = false;
            }
        }
    }
    flush(&mut text, &mut prefix, &mut negated);
    out
}

/// Unknown prefixes stay free words rather than being dropped: someone
/// searching "ratio:16" means it as text, and silently discarding it would
/// return a confidently wrong result.
pub fn parse(raw: &str) -> Query {
    let mut q = Query::default();
    for t in tokenize(raw) {
        let text = t.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        match t.prefix.as_deref() {
            Some("tag") => {
                let tag = text.to_lowercase();
                if t.negated { q.exclude_tags.push(tag) } else { q.tags.push(tag) }
            }
            Some("folder" | "in") => q.folders.push(text.trim_matches('/').to_string()),
            Some("root") => q.roots.push(text),
            Some("kind" | "type") => {
                q.kind = match text.to_lowercase().as_str() {
                    "image" | "img" | "photo" | "picture" => Some("image"),
                    "video" | "clip" | "movie" => Some("video"),
                    _ => q.kind,
                }
            }
            prefix => {
                let word = match prefix {
                    Some(p) => format!("{p}:{text}"),
                    None => text,
                };
                if t.negated { q.exclude.push(word) } else { q.free.push(word) }
            }
        }
    }
    q
}

/// A WHERE clause with its parameters, plus the ranking expression when the
/// query has free words to rank by.
pub struct Compiled {
    pub join: String,
    pub conditions: Vec<String>,
    pub params: Vec<Value>,
    pub rank: Option<String>,
}

impl Compiled {
    pub fn where_sql(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.conditions.join(" AND "))
        }
    }
}

/// How many known tags one typed word may stand in for. Enough to absorb a
/// typo without one vague word dragging in half the library.
const EXPANSIONS: usize = 3;
const SIMILARITY: f64 = 0.3;

pub fn compile(conn: &Connection, filter: &Filter) -> Result<Compiled> {
    let mut c = Compiled { join: String::new(), conditions: Vec::new(), params: Vec::new(), rank: None };
    let bind = |c: &mut Compiled, v: Value| {
        c.params.push(v);
        format!("?{}", c.params.len())
    };

    if let Some(root) = filter.root_id {
        let p = bind(&mut c, root.into());
        c.conditions.push(format!("m.root_id = {p}"));
        let folder = filter.folder.clone().unwrap_or_default();
        if !filter.recursive {
            let p = bind(&mut c, folder.into());
            c.conditions.push(format!("m.folder = {p}"));
        } else if !folder.is_empty() {
            let eq = bind(&mut c, folder.clone().into());
            let like = bind(&mut c, format!("{}/%", like_escape(&folder)).into());
            c.conditions.push(format!("(m.folder = {eq} OR m.folder LIKE {like} ESCAPE '\\')"));
        }
    }
    if let Some(kind) = filter.kind.as_deref().filter(|k| matches!(*k, "image" | "video")) {
        let p = bind(&mut c, kind.to_string().into());
        c.conditions.push(format!("m.kind = {p}"));
    }

    let q = parse(filter.q.as_deref().unwrap_or(""));

    if let Some(kind) = q.kind {
        let p = bind(&mut c, kind.to_string().into());
        c.conditions.push(format!("m.kind = {p}"));
    }
    for tag in q.tags {
        let p = bind(&mut c, tag.into());
        c.conditions.push(format!("EXISTS (SELECT 1 FROM media_tags t WHERE t.media_id = m.id AND t.tag = {p})"));
    }
    for tag in q.exclude_tags {
        let p = bind(&mut c, tag.into());
        c.conditions.push(format!("NOT EXISTS (SELECT 1 FROM media_tags t WHERE t.media_id = m.id AND t.tag = {p})"));
    }
    for folder in q.folders {
        let eq = bind(&mut c, folder.clone().into());
        let like = bind(&mut c, format!("{}/%", like_escape(&folder)).into());
        c.conditions.push(format!(
            "(m.folder = {eq} COLLATE NOCASE OR m.folder LIKE {like} ESCAPE '\\')"
        ));
    }
    for root in q.roots {
        let p = bind(&mut c, root.into());
        c.conditions.push(format!("m.root_id IN (SELECT id FROM roots WHERE name = {p} COLLATE NOCASE)"));
    }
    for word in q.exclude {
        // Exclusion is literal rather than fuzzy: "not this" means that word,
        // and also dropping its near-neighbours would remove results nobody
        // asked to lose.
        let p = bind(&mut c, phrase(&word, false).into());
        c.conditions.push(format!("m.id NOT IN (SELECT rowid FROM media_fts WHERE media_fts MATCH {p})"));
    }

    if !q.free.is_empty() {
        let known = known_tags(conn)?;
        let groups: Vec<String> = q
            .free
            .iter()
            .map(|word| {
                let mut variants = vec![phrase(word, true)];
                variants.extend(expand(&known, word).into_iter().map(|t| phrase(&t, false)));
                format!("({})", variants.join(" OR "))
            })
            .collect();
        // Words are OR-ed for matching and their AND-ness is left to bm25,
        // which scores a file hitting every word far above one hitting a
        // single word — so a near miss still surfaces instead of vanishing.
        let p = bind(&mut c, groups.join(" OR ").into());
        c.join = "JOIN media_fts ON media_fts.rowid = m.id".into();
        c.conditions.push(format!("media_fts MATCH {p}"));
        // Column weights: title, filename, tags, description, folder.
        c.rank = Some("bm25(media_fts, 10.0, 6.0, 8.0, 1.0, 2.0)".into());
    }
    Ok(c)
}

/// The ORDER BY for a sort name. Relevance only means something when there
/// are free words; otherwise it falls back to newest first.
pub fn order_by(sort: Option<&str>, rank: Option<&str>) -> String {
    let newest = "m.added_at DESC, m.id DESC";
    match sort.unwrap_or(if rank.is_some() { "relevance" } else { "newest" }) {
        "oldest" => "m.added_at ASC, m.id ASC".into(),
        "title" => "m.title COLLATE NOCASE ASC, m.id ASC".into(),
        "title_desc" => "m.title COLLATE NOCASE DESC, m.id DESC".into(),
        "largest" => "m.size DESC, m.id DESC".into(),
        "smallest" => "m.size ASC, m.id ASC".into(),
        "longest" => "m.duration DESC, m.id DESC".into(),
        "shortest" => "m.duration ASC, m.id ASC".into(),
        "relevance" => match rank {
            Some(rank) => format!("{rank} ASC, {newest}"),
            None => newest.into(),
        },
        _ => newest.into(),
    }
}

/// A term as an FTS5 string. Quoting makes every character literal, so a
/// search for `a|b` or `"` cannot become an FTS syntax error.
fn phrase(word: &str, prefix: bool) -> String {
    let quoted = format!("\"{}\"", word.replace('"', "\"\""));
    if prefix { format!("{quoted}*") } else { quoted }
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

pub fn known_tags(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare_cached("SELECT tag, count(*) FROM media_tags GROUP BY tag")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The known tags closest to a word, best first, excluding the word itself.
fn expand(known: &[(String, i64)], word: &str) -> Vec<String> {
    let word = word.to_lowercase();
    let target = trigrams(&word);
    let mut scored: Vec<(f64, &str)> = known
        .iter()
        .filter(|(tag, _)| *tag != word)
        .map(|(tag, _)| (similarity(&target, &trigrams(tag)), tag.as_str()))
        .filter(|(score, _)| *score >= SIMILARITY)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
    scored.into_iter().take(EXPANSIONS).map(|(_, t)| t.to_string()).collect()
}

/// Tag completions: prefix matches first (most used first), then near misses.
pub fn suggest(conn: &Connection, prefix: &str, limit: usize) -> Result<Vec<String>> {
    let prefix = prefix.trim().to_lowercase();
    let known = known_tags(conn)?;
    let mut starts: Vec<&(String, i64)> = known.iter().filter(|(t, _)| t.starts_with(&prefix)).collect();
    starts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut out: Vec<String> = starts.into_iter().take(limit).map(|(t, _)| t.clone()).collect();
    if out.len() < limit {
        for tag in expand(&known, &prefix) {
            if !out.contains(&tag) && out.len() < limit {
                out.push(tag);
            }
        }
    }
    Ok(out)
}

/// Trigrams the way pg_trgm forms them: per word, padded with two spaces in
/// front and one behind, so short words and word starts still match.
fn trigrams(s: &str) -> Vec<[char; 3]> {
    let mut out = Vec::new();
    for word in s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let chars: Vec<char> = "  ".chars().chain(word.chars().flat_map(char::to_lowercase)).chain([' ']).collect();
        for w in chars.windows(3) {
            out.push([w[0], w[1], w[2]]);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn similarity(a: &[[char; 3]], b: &[[char; 3]]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.iter().filter(|t| b.binary_search(t).is_ok()).count();
    shared as f64 / (a.len() + b.len() - shared) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefixes_and_negation() {
        let q = parse(r#"beach -blurry tag:"sunny day" -tag:draft folder:/Trips/2024/ kind:clip well-known"#);
        assert_eq!(q.free, vec!["beach", "well-known"]);
        assert_eq!(q.exclude, vec!["blurry"]);
        assert_eq!(q.tags, vec!["sunny day"]);
        assert_eq!(q.exclude_tags, vec!["draft"]);
        assert_eq!(q.folders, vec!["Trips/2024"]);
        assert_eq!(q.kind, Some("video"));
    }

    #[test]
    fn unknown_prefix_is_text() {
        assert_eq!(parse("ratio:16").free, vec!["ratio:16"]);
    }

    #[test]
    fn phrases_are_literal() {
        assert_eq!(phrase(r#"a"b"#, true), r#""a""b"*"#);
    }

    #[test]
    fn typos_expand_to_tags() {
        let known = vec![("sunset".to_string(), 3), ("sunrise".to_string(), 1), ("cat".to_string(), 9)];
        assert_eq!(expand(&known, "sunet").first().map(String::as_str), Some("sunset"));
        assert!(expand(&known, "dog").is_empty());
    }
}
