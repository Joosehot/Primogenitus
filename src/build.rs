//! Build the word network once from the dictionary, then freeze it.
//!
//! 1. every word of every part of speech becomes a base node (its
//!    familiarity, a verb's sentence frames, a noun's animacy, an adjective's
//!    position marker)
//! 2. every variation of a word becomes its own node, with a line to its base
//! 3. the dictionary's lines between senses become lines between words
//!    (synonyms, is a, has part, opposite, similar, derived, ...), each
//!    marked with which sense of the word it came from
//! 4. the parser reads every example sentence and adds the lines it finds
//!    (does, is done to, describes)

use crate::config::Config;
use crate::morph;
use crate::network::{Form, Kind, Line, Network, Node, Pos, UNKNOWN};
use crate::parser;
use crate::wordnet;
use anyhow::Result;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

fn kind_of(sym: &str) -> Option<Kind> {
    use Kind::*;
    Some(match sym {
        "@" | "@i" => Hypernym,
        "~" | "~i" => Hyponym,
        "%p" => HasPart,
        "#p" => PartOf,
        "%m" => HasMember,
        "#m" => MemberOf,
        "%s" => MadeOf,
        "#s" => SubstanceOf,
        "=" => Attribute,
        "+" => Derived,
        "!" => Antonym,
        "&" | "$" | "^" => Similar,
        "*" => Entails,
        ">" => Causes,
        "\\" | "<" => Pertains,
        _ => return None,
    })
}

fn reverse(kind: Kind) -> Kind {
    match kind {
        Kind::SubjectOf => Kind::HasSubject,
        Kind::ObjectOf => Kind::HasObject,
        Kind::Modifies => Kind::ModifiedBy,
        Kind::Eats => Kind::EatenBy,
        Kind::LivesIn => Kind::HomeOf,
        Kind::UsedFor => Kind::UseOf,
        Kind::MadeOf => Kind::SubstanceOf,
        Kind::HasPart => Kind::PartOf,
        k => k,
    }
}

/// The definition part of a gloss: before its examples and before a second definition.
fn definition(gloss: &str) -> &str {
    let d = gloss.split('"').next().unwrap_or("");
    d.split(';').next().unwrap_or("").trim()
}

/// A word never tagged in use has its senses in no particular order ("AI":
/// Army Intelligence first, artificial intelligence second). Its senses are
/// put in order: a sense named by common words first (artificial
/// intelligence, not Army Intelligence), then the more familiar; ties keep
/// their order.
fn order_untagged(wn: &mut crate::wordnet::WordNet) {
    let fam: HashMap<(Pos, String), u32> = wn.index.iter().map(|(k, e)| (k.clone(), e.tagsense)).collect();
    for ((pos, lemma), entry) in wn.index.iter_mut() {
        if entry.tagsense > 0 || entry.offsets.len() < 2 {
            continue;
        }
        // a sense named by common words before one named by names (Army
        // Intelligence), then by how familiar its words are
        let score = |off: &u32| -> (bool, u32) {
            let others: Vec<&String> = wn.synsets[&(*pos, *off)].words.iter().map(|(w, _)| w).filter(|w| w.to_lowercase() != *lemma).collect();
            let common = others.is_empty() || others.iter().any(|w| !w.chars().any(|c| c.is_uppercase()));
            let f = others.iter().map(|w| fam.get(&(*pos, w.to_lowercase())).copied().unwrap_or(0)).max().unwrap_or(0);
            (common, f)
        };
        entry.offsets.sort_by_key(|o| std::cmp::Reverse(score(o)));
    }
}

/// How often each word is used in the dictionary's own texts.
fn gloss_words(wn: &crate::wordnet::WordNet) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in wn.synsets.values() {
        for w in s.gloss.to_lowercase().split(|c: char| !(c.is_alphanumeric() || c == '-')) {
            if !w.is_empty() {
                *counts.entry(w.to_string()).or_default() += 1;
            }
        }
    }
    counts
}

/// How often each word stands alone after "a"/"an" in the dictionary's own
/// texts ("an apple;", "a stone that", not "a water bird"): evidence that it counts.
fn count_evidence(wn: &crate::wordnet::WordNet) -> HashMap<String, usize> {
    const AFTER: &[&str] = &["of", "that", "with", "is", "in", "and", "or", "which", "to", "for", "on", "at", "from",
                             "by", "as", "was", "has", "having", "used", "made", "containing", "filled"];
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in wn.synsets.values() {
        let text = s.gloss.to_lowercase();
        let words: Vec<&str> = text.split_whitespace().collect();
        for k in 0..words.len().saturating_sub(1) {
            if words[k] != "a" && words[k] != "an" {
                continue;
            }
            let w = words[k + 1];
            let bare = w.trim_end_matches(|c: char| ";,.)\"".contains(c));
            let ends = bare.len() < w.len() || k + 2 == words.len() || AFTER.contains(&words[k + 2]);
            if ends && !bare.is_empty() {
                *counts.entry(bare.to_string()).or_default() += 1;
            }
        }
    }
    counts
}

pub struct Report {
    pub nodes: usize,
    pub base: usize,
    pub lines: usize,
    pub example_lines: usize,
    pub examples: usize,
    pub steps: Vec<(String, f64)>,
}

pub fn build(root: &Path, cfg: &Config, log: &mut dyn FnMut(String)) -> Result<(Network, Report)> {
    let mut steps = Vec::new();
    let t = Instant::now();
    let dir = wordnet::fetch(root, &cfg.build.url, log)?;
    steps.push(("the dictionary downloaded and unpacked (or found)".to_string(), t.elapsed().as_secs_f64()));
    let t = Instant::now();
    let mut wn = wordnet::read(&dir)?;
    order_untagged(&mut wn);
    steps.push((format!("the dictionary read: {} senses, {} words", wn.synsets.len(), wn.index.len()), t.elapsed().as_secs_f64()));

    // 1. base nodes, in the index's (sorted) order
    let t = Instant::now();
    let counted = count_evidence(&wn);
    let seen = gloss_words(&wn);
    let mut nodes: Vec<Node> = Vec::new();
    let mut base_id: HashMap<(Pos, String), u32> = HashMap::new();
    for ((pos, lemma), entry) in &wn.index {
        let first = &wn.synsets[&(*pos, entry.offsets[0])];
        let (written, marker) = first
            .words
            .iter()
            .find(|(w, _)| w.to_lowercase() == *lemma)
            .cloned()
            .unwrap_or_else(|| (lemma.clone(), String::new()));
        let text = written.replace('_', " ");
        let mut frames = 0u64;
        if *pos == Pos::Verb {
            for off in entry.offsets.iter().take(cfg.build.frame_senses) {
                let s = &wn.synsets[&(*pos, *off)];
                let w = s.words.iter().position(|(w, _)| w.to_lowercase() == *lemma).map(|k| k as u8 + 1).unwrap_or(0);
                for &(f, wn_) in &s.frames {
                    if wn_ == 0 || wn_ == w {
                        frames |= 1u64 << f;
                    }
                }
            }
        }
        // an acronym ("AI", "TV", "DNA") is a common word, not a name
        let acronym = !text.contains(' ') && text.len() <= 4 && text.len() >= 2 && text.chars().all(|c| c.is_ascii_uppercase());
        let proper = !acronym && text.chars().any(|c| c.is_uppercase());
        let head = first.words.first().map(|(w, _)| w.to_lowercase() == *lemma).unwrap_or(false);
        let multi = text.contains(' ');
        let plain = acronym || text.chars().all(|c| c.is_ascii_lowercase() || c == ' ' || c == '-');
        let sure = plain
            && !proper
            && match pos {
                Pos::Noun => !morph::plural_looking(&text),
                Pos::Verb | Pos::Adj => !multi,
                Pos::Adv => !multi,
            };
        let mass = *pos == Pos::Noun && {
                let a = counted.get(lemma.as_str()).copied().unwrap_or(0);
                let plural = morph::forms(&text, Pos::Noun, wn.exc.get(&(Pos::Noun, lemma.clone())).map(|v| v.as_slice()).unwrap_or(&[]))
                    .first()
                    .map(|f| seen.get(f.0.as_str()).copied().unwrap_or(0))
                    .unwrap_or(0);
                (cfg.build.mass_lexfiles.contains(&first.lexfile) && a < cfg.build.count_evidence)
                    // never "a flesh", never "fleshes", and a word the texts do use: uncountable
                    || (a == 0 && plural == 0 && seen.get(lemma.as_str()).copied().unwrap_or(0) >= cfg.build.mass_seen && !text.contains(' '))
            };
        let id = nodes.len() as u32;
        nodes.push(Node {
            text,
            pos: *pos,
            form: Form::Base,
            lemma: id,
            fam: entry.tagsense.min(u16::MAX as u32) as u16,
            proper,
            frames,
            animate: *pos == Pos::Noun && matches!(first.lexfile, 5 | 18),
            mass,
            head,
            marker: match marker.as_str() {
                "(a)" => crate::network::ATTRIBUTIVE_ONLY,
                "(p)" => crate::network::PREDICATIVE_ONLY,
                "(ip)" => crate::network::POSTNOMINAL,
                _ => 0,
            },
            sure,
        });
        base_id.insert((*pos, lemma.clone()), id);
    }
    let base = nodes.len();

    // 2. the variations
    let mut forms: Vec<Vec<u32>> = vec![Vec::new(); base];
    for b in 0..base {
        let (text, pos, proper, fam, sure_base) = {
            let n = &nodes[b];
            (n.text.clone(), n.pos, n.proper, n.fam, n.sure)
        };
        if proper {
            continue;
        }
        let key = (pos, text.to_lowercase().replace(' ', "_"));
        let irregular: Vec<String> = wn.exc.get(&key).cloned().unwrap_or_default();
        for (ftext, form, sure) in morph::forms(&text, pos, &irregular) {
            let id = nodes.len() as u32;
            nodes.push(Node {
                text: ftext,
                pos,
                form,
                lemma: b as u32,
                fam,
                proper: false,
                frames: 0,
                animate: false,
                marker: 0,
                mass: false,
                sure: sure && sure_base,
                head: false,
            });
            forms[b].push(id);
        }
    }
    forms.resize(nodes.len(), Vec::new());
    steps.push((format!("{} nodes: {} words and {} variations", nodes.len(), base, nodes.len() - base), t.elapsed().as_secs_f64()));

    // 3. the dictionary's lines
    let t = Instant::now();
    let mut lines: Vec<Vec<Line>> = vec![Vec::new(); nodes.len()];
    let rank_of = |pos: Pos, lemma: &str, off: u32| -> u8 {
        wn.index
            .get(&(pos, lemma.to_string()))
            .and_then(|e| e.offsets.iter().position(|&o| o == off))
            .map(|k| k.min(255) as u8)
            .unwrap_or(255)
    };
    let is_proper: Vec<bool> = nodes.iter().map(|n| n.proper).collect();
    let words_of = |pos: Pos, off: u32| -> Vec<(u32, String)> {
        wn.synsets[&(pos, off)]
            .words
            .iter()
            .filter_map(|(w, _)| {
                let l = w.to_lowercase();
                let &id = base_id.get(&(pos, l.clone()))?;
                let acronym = w.len() <= 4 && w.len() >= 2 && w.chars().all(|c| c.is_ascii_uppercase());
                let proper_sense = !acronym && w.chars().any(|c| c.is_uppercase()) && !is_proper[id as usize];
                (!proper_sense).then_some((id, l))
            })
            .collect()
    };
    for (&(pos, off), s) in &wn.synsets {
        let own = words_of(pos, off);
        for (a, la) in &own {
            let r = rank_of(pos, la, off);
            for (b, lb) in &own {
                if a != b {
                    lines[*a as usize].push(Line { to: *b, kind: Kind::Synonym, rank: r, to_rank: rank_of(pos, lb, off) });
                }
            }
        }
        for p in &s.ptrs {
            let Some(kind) = kind_of(&p.sym) else { continue };
            if !wn.synsets.contains_key(&p.target) {
                continue;
            }
            let target = words_of(p.target.0, p.target.1);
            if p.src == 0 && p.tgt == 0 {
                for (a, la) in &own {
                    let r = rank_of(pos, la, off);
                    for (b, lb) in target.iter().take(cfg.build.semantic_targets) {
                        let tr = rank_of(p.target.0, lb, p.target.1);
                        lines[*a as usize].push(Line { to: *b, kind, rank: r, to_rank: tr });
                    }
                }
            } else if let (Some((a, la)), Some((b, lb))) = (own.get(p.src as usize - 1), target.get(p.tgt as usize - 1)) {
                let r = rank_of(pos, la, off);
                let tr = rank_of(p.target.0, lb, p.target.1);
                lines[*a as usize].push(Line { to: *b, kind, rank: r, to_rank: tr });
            }
        }
    }
    for b in 0..base {
        for &f in &forms[b] {
            lines[f as usize].push(Line { to: b as u32, kind: Kind::FormOf, rank: 0, to_rank: 0 });
        }
    }
    steps.push(("the dictionary's lines between senses laid between words".to_string(), t.elapsed().as_secs_f64()));

    // 4. the parser reads the example sentences
    let t = Instant::now();
    let mut net = Network::new(nodes, Vec::new(), forms);
    let mut found: BTreeMap<(u32, Kind, u32), u8> = BTreeMap::new();
    let mut examples = 0usize;
    for (&(pos, off), s) in &wn.synsets {
        let own = words_of(pos, off);
        if own.is_empty() {
            continue;
        }
        let ids: Vec<u32> = own.iter().map(|(i, _)| *i).collect();
        for ex in s.gloss.split('"').skip(1).step_by(2) {
            examples += 1;
            for (a, kind, b) in parser::example_lines(&net, &ids, pos, ex) {
                let me = own.iter().find(|(i, _)| *i == a || *i == b).map(|(_, l)| l.as_str()).unwrap_or(&own[0].1);
                let r = rank_of(pos, me, off);
                let e = found.entry((a, kind, b)).or_insert(r);
                *e = (*e).min(r);
            }
        }
    }
    let example_lines = found.len();
    steps.push((format!("{examples} example sentences parsed: {example_lines} lines (does, is done to, describes)"), t.elapsed().as_secs_f64()));

    // 5. the parser reads every definition: what it eats, where it lives,
    //    what it is used for and made of, what it has; and each base word keeps
    //    its definition (its most common sense) to quote
    let t = Instant::now();
    let mut from_defs: BTreeMap<(u32, Kind, u32), u8> = BTreeMap::new();
    let mut defs: HashMap<u32, String> = HashMap::new();
    for (&(pos, off), s) in &wn.synsets {
        let own = words_of(pos, off);
        if own.is_empty() {
            continue;
        }
        let def = definition(&s.gloss);
        for (id, lemma) in &own {
            if rank_of(pos, lemma, off) == 0 && !def.is_empty() {
                defs.entry(*id).or_insert_with(|| def.to_string());
            }
        }
        if pos != Pos::Noun {
            continue;
        }
        let ids: Vec<u32> = own.iter().map(|(i, _)| *i).collect();
        for (a, kind, b) in parser::definition_lines(&net, &ids, def) {
            let me = own.iter().find(|(i, _)| *i == a).map(|(_, l)| l.as_str()).unwrap_or(&own[0].1);
            let r = rank_of(pos, me, off);
            let e = from_defs.entry((a, kind, b)).or_insert(r);
            *e = (*e).min(r);
        }
    }
    let definition_lines = from_defs.len();
    for (k, r) in from_defs {
        found.entry(k).or_insert(r);
    }
    net.defs = defs;
    for (&(a, kind, b), &r) in &found {
        lines[a as usize].push(Line { to: b, kind, rank: r, to_rank: UNKNOWN });
        lines[b as usize].push(Line { to: a, kind: reverse(kind), rank: r, to_rank: UNKNOWN });
    }
    steps.push((format!("{} definitions parsed: {definition_lines} lines (eats, lives in, used for, made of, has)", net.defs.len()), t.elapsed().as_secs_f64()));

    let t = Instant::now();
    for l in lines.iter_mut() {
        l.sort_by_key(|x| (x.to, x.kind, x.rank));
        l.dedup_by_key(|x| (x.to, x.kind));
    }
    net.lines = lines;
    let report = Report {
        nodes: net.nodes.len(),
        base,
        lines: net.line_count(),
        example_lines,
        examples,
        steps: {
            steps.push(("lines sorted".to_string(), t.elapsed().as_secs_f64()));
            steps
        },
    };
    Ok((net, report))
}
