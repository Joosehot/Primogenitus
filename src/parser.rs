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
        // "a kind of fish": the fish is meant
        if lexicon::partitive(&t[i]) && t.get(i + 1).map(|w| w == "of").unwrap_or(false) {
            return chunk_head(net, t, i + 2);
        }
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

/// A noun phrase's head from t[i] and where the phrase ends, any noun (names too).
fn chunk(net: &Network, t: &[String], mut i: usize) -> Option<(u32, usize)> {
    if i < t.len() && lexicon::closed(&t[i]) == Some(Closed::Det) {
        i += 1;
    }
    let mut head = None;
    while i < t.len() && lexicon::closed(&t[i]).is_none() {
        if lexicon::partitive(&t[i]) && t.get(i + 1).map(|w| w == "of").unwrap_or(false) {
            return chunk(net, t, i + 2);
        }
        let n: Vec<u32> = net
            .lookup(&t[i])
            .iter()
            .map(|&q| net.node(q))
            .filter(|n| n.pos == Pos::Noun && matches!(n.form, Form::Base | Form::Plural))
            .map(|n| n.lemma)
            .collect();
        if let Some(&h) = n.iter().find(|&&h| !net.node(h).proper).or(n.first()) {
            head = Some(h);
        } else if adjs(net, &t[i]).is_empty() {
            break;
        }
        i += 1;
    }
    head.map(|h| (h, i))
}

/// The heads of a noun phrase and of the ones joined to it by "and" / "or".
fn heads(net: &Network, t: &[String], i: usize) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = i;
    while let Some((h, end)) = chunk(net, t, at) {
        out.push(h);
        match t.get(end).map(|w| w.as_str()) {
            Some("and") | Some("or") => at = end + 1,
            _ => break,
        }
    }
    out
}

/// Lines read from a sense's definition, for each of its words `own`:
/// what it eats ("feeds on fish", "flesh-eating"), where it lives ("living
/// in water", "found in Asia", "native to Africa", "aquatic"), what it is used
/// for ("used for pounding", "used to cut"), what it is made of ("made of
/// flour"), what it has ("having scales").
pub fn definition_lines(net: &Network, own: &[u32], def: &str) -> Vec<(u32, Kind, u32)> {
    let t = tokens(def);
    let mut found: Vec<(Kind, u32)> = Vec::new();
    for i in 0..t.len() {
        let w = t[i].as_str();
        let next = t.get(i + 1).map(|x| x.as_str());
        if let Some(food) = w.strip_suffix("-eating").or_else(|| w.strip_suffix("-feeding")) {
            found.extend(nouns(net, food).into_iter().take(1).map(|n| (Kind::Eats, n)));
        }
        if let Some(home) = w.strip_suffix("-dwelling") {
            found.extend(nouns(net, home).into_iter().take(1).map(|n| (Kind::LivesIn, n)));
        }
        if let Some(place) = lexicon::habitat(w) {
            found.extend(net.base(place, Pos::Noun).map(|n| (Kind::LivesIn, n)));
        }
        match (w, next) {
            ("feeds" | "feeding" | "feed" | "preys" | "preying" | "prey" | "subsisting" | "subsists", Some("on")) => {
                found.extend(heads(net, &t, i + 2).into_iter().map(|n| (Kind::Eats, n)));
            }
            ("eats" | "eating" | "eat" | "devours" | "devouring", _) => {
                found.extend(heads(net, &t, i + 1).into_iter().map(|n| (Kind::Eats, n)));
            }
            _ => {}
        }
        match (w, next) {
            ("living" | "lives" | "live" | "found" | "growing" | "grows" | "dwelling" | "occurring" | "breeding" | "nesting", Some("in" | "on" | "near")) => {
                found.extend(heads(net, &t, i + 2).into_iter().map(|n| (Kind::LivesIn, n)));
            }
            ("native" | "indigenous" | "endemic", Some("to")) => found.extend(heads(net, &t, i + 2).into_iter().map(|n| (Kind::LivesIn, n))),
            ("inhabiting" | "inhabits", _) => found.extend(heads(net, &t, i + 1).into_iter().map(|n| (Kind::LivesIn, n))),
            ("used", Some("as")) => {
                // "used as a cutting instrument": what it is used for is the cutting
                let mut k = i + 2;
                if t.get(k).map(|x| lexicon::closed(x) == Some(Closed::Det)).unwrap_or(false) {
                    k += 1;
                }
                if let Some(v) = t.get(k).and_then(|x| lemmas_as(net, x, Pos::Verb, &[Form::Ing]).first().copied()) {
                    found.push((Kind::UsedFor, v));
                }
            }
            ("used" | "use", Some("for" | "in")) => {
                // "used for pounding" (a verb), "used for cooking food", "used in surgery"
                let v = t.get(i + 2).and_then(|x| lemmas_as(net, x, Pos::Verb, &[Form::Ing]).first().copied());
                match v {
                    Some(v) => found.push((Kind::UsedFor, v)),
                    None => found.extend(heads(net, &t, i + 2).into_iter().take(1).map(|n| (Kind::UsedFor, n))),
                }
            }
            ("used", Some("to")) => {
                if let Some(v) = t.get(i + 2).and_then(|x| lemmas_as(net, x, Pos::Verb, &[Form::Base]).first().copied()) {
                    found.push((Kind::UsedFor, v));
                }
            }
            ("made" | "consisting", Some("of" | "from")) => found.extend(heads(net, &t, i + 2).into_iter().map(|n| (Kind::MadeOf, n))),
            ("having", _) => found.extend(heads(net, &t, i + 1).into_iter().take(1).map(|n| (Kind::HasPart, n))),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for &me in own {
        for &(k, n) in &found {
            if n != me && !out.contains(&(me, k, n)) {
                out.push((me, k, n));
            }
        }
    }
    out
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
    // a name: "Helsinki", "New York"
    if det.is_none() {
        for nl in 1..=3 {
            if i + nl > t.len() {
                break;
            }
            let name = t[i..i + nl].join(" ");
            if net.lookup(&name).iter().any(|&q| net.node(q).proper && net.node(q).pos == Pos::Noun) && !out.contains(&(i + nl, Num::Sg)) {
                out.push((i + nl, Num::Sg));
            }
        }
    }
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
                // a noun that may be uncountable: "bread", "the bread", never "a bread"
                // or "breads"; any other: "a dog", "the dog", "dogs", never "dog"
                let mass = net.node(n.lemma).mass;
                let ok = if mass {
                    num == Num::Sg && matches!(det, None | Some("the"))
                } else {
                    match det {
                        Some("a") | Some("an") => num == Num::Sg && lexicon::article(first, &cfg.article) == det.unwrap(),
                        Some(_) => true,
                        None => num == Num::Pl,
                    }
                };
                if ok && !out.contains(&(k + len, num)) {
                    out.push((k + len, num));
                    // "the capital of Finland": a noun of a name
                    if t.get(k + len).map(|w| w == "of").unwrap_or(false) {
                        for nl in 1..=3 {
                            if k + len + 1 + nl > t.len() {
                                break;
                            }
                            let name = t[k + len + 1..k + len + 1 + nl].join(" ");
                            if net.lookup(&name).iter().any(|&q| net.node(q).proper) && !out.contains(&(k + len + 1 + nl, num)) {
                                out.push((k + len + 1 + nl, num));
                            }
                        }
                    }
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
///   S  -> (NP | "it" | "they") VP "."
///   (VP also: Be "used for" X, Be "made of" NP, Verb Prep NP; NP also a name,
///   and "the" Noun "of" Name)
///   NP -> ("a" | "an" | "the")? Adj? Noun     (a/an: singular, its sound; none: plural,
///                                             or a noun that may be uncountable, singular)
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
    let mut subjects = noun_phrases(net, cfg, &t, 0);
    match t.first().map(|w| w.as_str()) {
        Some("it") => subjects.push((1, Num::Sg)),
        Some("they") => subjects.push((1, Num::Pl)),
        _ => {}
    }
    for (end, num) in subjects {
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
                // "is used for pounding", "is made of flour"
                if let Some(w2) = t.get(rest) {
                    let after = rest + 2;
                    if w2 == "used" && t.get(rest + 1).map(|w| w == "for").unwrap_or(false) && after < t.len() {
                        let ing = after + 1 == t.len() && net.lookup(&t[after]).iter().any(|&q| net.node(q).pos == Pos::Verb && net.node(q).form == Form::Ing);
                        if ing || noun_phrases(net, cfg, &t, after).iter().any(|&(e, _)| full(e)) {
                            return Ok("NP Be used for X".into());
                        }
                    }
                    if w2 == "made" && t.get(rest + 1).map(|w| w == "of" || w == "from").unwrap_or(false)
                        && noun_phrases(net, cfg, &t, after).iter().any(|&(e, _)| full(e))
                    {
                        return Ok("NP Be made of NP".into());
                    }
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
            // "fish live in water", "... on land and in water": a verb and where
            let place = |i: usize| -> Vec<usize> {
                if matches!(t.get(i).map(|w| w.as_str()), Some("in" | "on" | "at" | "near" | "under")) {
                    noun_phrases(net, cfg, &t, i + 1).into_iter().map(|(e, _)| e).collect()
                } else {
                    Vec::new()
                }
            };
            for e in place(rest) {
                if full(e) || (t.get(e).map(|w| w == "and").unwrap_or(false) && place(e + 1).into_iter().any(full)) {
                    return Ok("NP Verb Prep NP".into());
                }
            }
            // "dogs eat flesh and meat"
            if frames & TRANSITIVE != 0 {
                for (e, _) in noun_phrases(net, cfg, &t, rest) {
                    if t.get(e).map(|w| w == "and").unwrap_or(false) && noun_phrases(net, cfg, &t, e + 1).iter().any(|&(e2, _)| full(e2)) {
                        return Ok("NP Verb NP and NP".into());
                    }
                }
            }
        }
    }
    bail!("does not parse under the grammar")
}
