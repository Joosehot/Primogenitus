//! The parser: words to structure, in three places.
//!
//! - At build time it reads the dictionary's example sentences ("the dog
//!   barked all night") and turns them into lines between words: the dog
//!   does "bark", "feed" is done to the dog, "loyal" describes a friend.
//!   This is where the meaning beyond the dictionary's own lines comes from.
//! - It reads the input word.
//! - It checks every finished sentence against the grammar, independently of
//!   how the sentence was made: a sentence that does not parse is never given.

use crate::config::Config;
use crate::lexicon::{self, Closed};
use crate::network::{Form, Kind, Network, Pos, ATTRIBUTIVE_ONLY};
use anyhow::{bail, Result};

pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

fn lemmas_as(net: &Network, tok: &str, pos: Pos, forms: &[Form]) -> Vec<u32> {
    if lexicon::closed(tok).is_some() {
        return Vec::new();
    }
    let mut out: Vec<u32> = net
        .lookup(tok)
        .iter()
        .map(|&i| net.node(i))
        .filter(|n| n.pos == pos && forms.contains(&n.form) && !n.proper)
        .map(|n| n.lemma)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn nouns(net: &Network, tok: &str) -> Vec<u32> {
    lemmas_as(net, tok, Pos::Noun, &[Form::Base, Form::Plural])
}

fn adjs(net: &Network, tok: &str) -> Vec<u32> {
    lemmas_as(net, tok, Pos::Adj, &[Form::Base])
}

fn adverb_only(net: &Network, tok: &str) -> bool {
    let all = net.lookup(tok);
    !all.is_empty() && all.iter().all(|&i| net.node(i).pos == Pos::Adv)
}

/// The head noun of the chunk starting at i (determiners, adjectives, nouns),
/// or None when the chunk does not start a noun phrase.
fn chunk_head(net: &Network, t: &[String], mut i: usize) -> Option<u32> {
    if i < t.len() && lexicon::closed(&t[i]) == Some(Closed::Det) {
        i += 1;
    }
    let mut head = None;
    while i < t.len() && lexicon::closed(&t[i]).is_none() {
        let n = nouns(net, &t[i]);
        if let Some(&h) = n.first() {
            head = Some(h);
        } else if adjs(net, &t[i]).is_empty() {
            break;
        }
        i += 1;
    }
    head
}

/// Lines read from one example sentence of a sense whose words are `own`.
pub fn example_lines(net: &Network, own: &[u32], pos: Pos, example: &str) -> Vec<(u32, Kind, u32)> {
    let t = tokens(example);
    let mut out = Vec::new();
    for i in 0..t.len() {
        if lexicon::closed(&t[i]).is_some() {
            continue;
        }
        let hits: Vec<u32> = net
            .lookup(&t[i])
            .iter()
            .map(|&k| net.node(k))
            .filter(|n| n.pos == pos && own.contains(&n.lemma))
            .map(|n| n.lemma)
            .collect();
        let Some(&me) = hits.first() else { continue };
        match pos {
            Pos::Verb => {
                // the subject: the noun before, past auxiliaries, "not" and adverbs
                let mut j = i;
                let mut passive = false;
                while j > 0 {
                    j -= 1;
                    match lexicon::closed(&t[j]) {
                        Some(Closed::Aux) => {
                            if matches!(t[j].as_str(), "is" | "are" | "was" | "were" | "been" | "be" | "being") {
                                passive = true;
                            }
                            continue;
                        }
                        Some(Closed::Not) => continue,
                        Some(_) => break,
                        None if adverb_only(net, &t[j]) => continue,
                        None => {
                            if let Some(&s) = nouns(net, &t[j]).first() {
                                let form = net.lookup(&t[i]).iter().map(|&k| net.node(k)).find(|n| n.lemma == me).map(|n| n.form);
                                if passive && matches!(form, Some(Form::PastPart) | Some(Form::Past)) {
                                    out.push((s, Kind::ObjectOf, me));
                                } else {
                                    out.push((s, Kind::SubjectOf, me));
                                }
                            }
                            break;
                        }
                    }
                }
                // the object: the noun phrase right after
                let mut k = i + 1;
                while k < t.len() && (adverb_only(net, &t[k]) || lexicon::closed(&t[k]) == Some(Closed::Not)) {
                    k += 1;
                }
                if !passive {
                    if let Some(o) = chunk_head(net, &t, k) {
                        out.push((o, Kind::ObjectOf, me));
                    }
                }
            }
            Pos::Noun => {
                if i > 0 {
                    if let Some(&a) = adjs(net, &t[i - 1]).first() {
                        if nouns(net, &t[i - 1]).is_empty() {
                            out.push((a, Kind::Modifies, me));
                        }
                    }
                }
                // what the noun does: the verb after it (past auxiliaries)
                let mut k = i + 1;
                let mut aux = None;
                while k < t.len() && matches!(lexicon::closed(&t[k]), Some(Closed::Aux) | Some(Closed::Not)) {
                    aux = Some(t[k].clone());
                    k += 1;
                }
                if k < t.len() && lexicon::closed(&t[k]).is_none() {
                    let be = aux.as_deref().map(|a| matches!(a, "is" | "are" | "was" | "were" | "been" | "be")).unwrap_or(false);
                    let first = net.lookup(&t[k]).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Verb && !n.proper);
                    if let Some(n) = first {
                        match (aux.is_some(), be, n.form) {
                            (false, _, Form::ThirdSg | Form::Past) => out.push((me, Kind::SubjectOf, n.lemma)),
                            (true, true, Form::PastPart | Form::Past) => out.push((me, Kind::ObjectOf, n.lemma)),
                            (true, true, Form::Ing) => out.push((me, Kind::SubjectOf, n.lemma)),
                            (true, false, Form::Base | Form::PastPart) => out.push((me, Kind::SubjectOf, n.lemma)),
                            _ => {}
                        }
                    }
                }
                // what is done to it: a verb right before its noun phrase ("walked the old dog")
                let mut j = i;
                while j > 0 {
                    let p = &t[j - 1];
                    let det = lexicon::closed(p) == Some(Closed::Det);
                    let adj = lexicon::closed(p).is_none() && !adjs(net, p).is_empty() && nouns(net, p).is_empty();
                    if det || adj {
                        j -= 1;
                    } else {
                        break;
                    }
                }
                if j > 0 && j < i + 1 && lexicon::closed(&t[j - 1]).is_none() {
                    let v = lemmas_as(net, &t[j - 1], Pos::Verb, &[Form::Base, Form::ThirdSg, Form::Past, Form::Ing]);
                    let det_before = j >= 2 && lexicon::closed(&t[j - 2]) == Some(Closed::Det);
                    if let (Some(&v), false) = (v.first(), det_before) {
                        out.push((me, Kind::ObjectOf, v));
                    }
                }
            }
            Pos::Adj => {
                if let Some(n) = chunk_head(net, &t, i + 1) {
                    out.push((me, Kind::Modifies, n));
                } else if i >= 2 && matches!(t[i - 1].as_str(), "is" | "are" | "was" | "were") {
                    if let Some(&n) = nouns(net, &t[i - 2]).first() {
                        out.push((me, Kind::Modifies, n));
                    }
                }
            }
            Pos::Adv => {}
        }
    }
    out.retain(|(a, _, b)| a != b);
    out
}

/// The input word as a base noun.
pub fn read_topic(net: &Network, word: &str) -> Result<u32> {
    let w = word.trim().to_lowercase().replace('_', " ");
    let ns: Vec<u32> = net
        .lookup(&w)
        .iter()
        .map(|&i| net.node(i))
        .filter(|n| n.pos == Pos::Noun && !n.proper)
        .map(|n| n.lemma)
        .collect();
    if let Some(&n) = ns.first() {
        if !net.node(n).sure {
            bail!("\"{w}\" is in the network, but its number cannot be known from its spelling (singular or plural?), so no sentence about it can be sure to be grammatical");
        }
        return Ok(n);
    }
    if !net.lookup(&w).is_empty() {
        bail!("\"{w}\" is in the network but not as a noun; give a noun (a thing, an animal, a person)");
    }
    let near = closest(net, &w);
    bail!("\"{w}\" is not in the network (did you mean \"{near}\"?)")
}

pub fn closest(net: &Network, w: &str) -> String {
    let mut best = (usize::MAX, String::new());
    for k in net.words() {
        if k.len().abs_diff(w.len()) > 2 {
            continue;
        }
        let d = edit_distance(w, k);
        if d < best.0 || (d == best.0 && k < best.1.as_str()) {
            best = (d, k.to_string());
        }
    }
    best.1
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    prev[b.len()]
}

// ---------------------------------------------------------------- the check

#[derive(Clone, Copy, PartialEq, Debug)]
enum Num {
    Sg,
    Pl,
}

/// Noun phrases starting at i: (end, number).
fn noun_phrases(net: &Network, cfg: &Config, t: &[String], i: usize) -> Vec<(usize, Num)> {
    let mut out = Vec::new();
    let (det, j) = match t.get(i).map(|s| s.as_str()) {
        Some("a") | Some("an") => (Some(t[i].as_str()), i + 1),
        Some("the") => (Some("the"), i + 1),
        _ => (None, i),
    };
    for with_adj in [false, true] {
        let mut k = j;
        if with_adj {
            let ok = t.get(k).map(|w| {
                net.lookup(w).iter().any(|&q| {
                    let n = net.node(q);
                    n.pos == Pos::Adj && n.form == Form::Base && n.marker != crate::network::PREDICATIVE_ONLY && n.marker != crate::network::POSTNOMINAL
                })
            });
            if ok != Some(true) || lexicon::closed(&t[k]).is_some() {
                continue;
            }
            k += 1;
        }
        // the noun, possibly several words long
        for len in 1..=4 {
            if k + len > t.len() {
                break;
            }
            let text = t[k..k + len].join(" ");
            if len == 1 && lexicon::closed(&text).is_some() {
                continue;
            }
            for &q in net.lookup(&text) {
                let n = net.node(q);
                if n.pos != Pos::Noun || n.proper || !n.sure {
                    continue;
                }
                let num = match n.form {
                    Form::Base => Num::Sg,
                    Form::Plural => Num::Pl,
                    _ => continue,
                };
                let first = &t[if with_adj { j } else { k }];
                let mass = net.node(n.lemma).mass;
                let ok = (!mass || (det == Some("the") && num == Num::Sg)) && match det {
                    Some("a") | Some("an") => num == Num::Sg && lexicon::article(first, &cfg.article) == det.unwrap(),
                    Some(_) => true,
                    None => num == Num::Pl,
                };
                if ok && !out.contains(&(k + len, num)) {
                    out.push((k + len, num));
                }
            }
        }
    }
    out
}

fn verb_frames(net: &Network, w: &str, want: &[Form]) -> Vec<(u64, Form)> {
    if lexicon::closed(w).is_some() {
        return Vec::new();
    }
    net.lookup(w)
        .iter()
        .map(|&q| net.node(q))
        .filter(|n| n.pos == Pos::Verb && n.sure && want.contains(&n.form))
        .map(|n| (net.node(n.lemma).frames, n.form))
        .collect()
}

pub const INTRANSITIVE: u64 = (1 << 1) | (1 << 2);
pub const TRANSITIVE: u64 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11);

fn agrees(num: Num, form: Form) -> bool {
    match form {
        Form::Past => true,
        Form::ThirdSg => num == Num::Sg,
        Form::Base => num == Num::Pl,
        _ => false,
    }
}

/// The grammar every sentence must parse under:
///   S  -> NP VP "."
///   NP -> ("a" | "an" | "the")? Adj? Noun     (a/an: singular, its sound; none: plural)
///   VP -> Verb(intransitive) | Verb(transitive) NP | Be Adj | Be NP | Have NP
/// with the verb agreeing with the subject's number.
pub fn check(net: &Network, cfg: &Config, sentence: &str) -> Result<String> {
    let s = sentence.trim();
    if !s.ends_with('.') {
        bail!("does not end with a period");
    }
    if !s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        bail!("does not start with a capital letter");
    }
    let t = tokens(&s[..s.len() - 1]);
    for (end, num) in noun_phrases(net, cfg, &t, 0) {
        let Some(v) = t.get(end) else { continue };
        let rest = end + 1;
        let full = |e: usize| e == t.len();
        // the copula and "have"
        let be = match v.as_str() {
            "is" | "was" => Some(Num::Sg),
            "are" | "were" => Some(Num::Pl),
            _ => None,
        };
        if let Some(bn) = be {
            if bn == num {
                if rest + 1 == t.len()
                    && net.lookup(&t[rest]).iter().any(|&q| {
                        let n = net.node(q);
                        n.pos == Pos::Adj && n.form == Form::Base && n.marker != ATTRIBUTIVE_ONLY && lexicon::closed(&t[rest]).is_none()
                    })
                {
                    return Ok("NP Be Adj".into());
                }
                if noun_phrases(net, cfg, &t, rest).iter().any(|&(e, n2)| full(e) && n2 == num) {
                    return Ok("NP Be NP".into());
                }
            }
            continue;
        }
        let have = match v.as_str() {
            "has" => Some(Num::Sg),
            "have" => Some(Num::Pl),
            _ => None,
        };
        if let Some(hn) = have {
            if hn == num && noun_phrases(net, cfg, &t, rest).iter().any(|&(e, _)| full(e)) {
                return Ok("NP Have NP".into());
            }
            continue;
        }
        for (frames, form) in verb_frames(net, v, &[Form::Base, Form::ThirdSg, Form::Past]) {
            if !agrees(num, form) {
                continue;
            }
            if full(rest) && frames & INTRANSITIVE != 0 {
                return Ok("NP Verb".into());
            }
            if frames & TRANSITIVE != 0 && noun_phrases(net, cfg, &t, rest).iter().any(|&(e, _)| full(e)) {
                return Ok("NP Verb NP".into());
            }
        }
    }
    bail!("does not parse under the grammar")
}
