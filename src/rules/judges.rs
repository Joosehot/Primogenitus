//! The judges. The grammar is built into them: a candidate against the
//! grammar is refused outright, so a winning sentence is always grammatical.
//! One judge (animacy) only scores: "the table barks" is grammatical, but a
//! person's verb wants a person.

use super::Verdict;
use crate::lexicon;
use crate::network::{Form, Pos, ATTRIBUTIVE_ONLY, POSTNOMINAL, PREDICATIVE_ONLY};
use crate::parser::{INTRANSITIVE, TRANSITIVE};
use crate::search::{noun_text, realize, verb_text, Cand, Ctx, Det, Np, Num, Pred, Tense};

pub type Judge = fn(&Ctx, &Cand) -> Verdict;

pub static ALL: [(&str, Judge); 9] = [
    ("frame", frame),
    ("determiner", determiner),
    ("agreement", agreement),
    ("forms", forms),
    ("adjective place", adjective_place),
    ("no repeat", no_repeat),
    ("article", article),
    // these two only score
    ("animacy", animacy),
    ("role", role),
];


/// Is the topic the one that does the verb? The parsed examples say who does
/// what ("the dog barked") and what is done to what ("toast the bread"),
/// for the topic and for its kinds (its most common sense, up to three steps).
fn role(cx: &Ctx, c: &Cand) -> Verdict {
    use crate::network::Kind;
    let verb = match &c.pred {
        Pred::Intr { verb, .. } | Pred::Tr { verb, .. } => *verb,
        _ => return Verdict::Ok(0.0),
    };
    let chain: Vec<u32> = cx.chain().into_iter().map(|(n, _)| n).collect();
    let does = chain.iter().any(|&n| cx.has_line(n, Kind::SubjectOf, verb));
    let undergoes = chain.iter().any(|&n| cx.has_line(n, Kind::ObjectOf, verb));
    Verdict::Ok(if does {
        cx.cfg.score.role_subject
    } else if undergoes {
        cx.cfg.score.role_object
    } else {
        0.0
    })
}

fn nps(c: &Cand) -> Vec<&Np> {
    let mut v: Vec<&Np> = c.subj.iter().collect();
    match &c.pred {
        Pred::Tr { obj: Some(o), .. } => v.push(o),
        Pred::CopNp { np: Some(o), .. } | Pred::Have { np: Some(o), .. } => v.push(o),
        _ => {}
    }
    v
}

/// The verb must allow the sentence's shape: "bark" alone, "chase" with an object.
fn frame(cx: &Ctx, c: &Cand) -> Verdict {
    match &c.pred {
        Pred::Intr { verb, .. } if cx.node(*verb).frames & INTRANSITIVE == 0 => Verdict::Refuse("the verb needs an object"),
        Pred::Tr { verb, .. } if cx.node(*verb).frames & TRANSITIVE == 0 => Verdict::Refuse("the verb takes no object"),
        _ => Verdict::Ok(0.0),
    }
}

/// "a" and "an" only with one; no determiner only with many (or with a noun
/// that may be uncountable: "bread").
fn determiner(cx: &Ctx, c: &Cand) -> Verdict {
    for np in nps(c) {
        if cx.node(np.noun).mass {
            if np.num == Num::Pl || np.det == Det::A {
                return Verdict::Refuse("a noun that may be uncountable, with \"a\" or in the plural");
            }
            continue;
        }
        match (np.det, np.num) {
            (Det::A, Num::Pl) => return Verdict::Refuse("a/an with a plural"),
            (Det::Bare, Num::Sg) => return Verdict::Refuse("a singular noun without a determiner"),
            _ => {}
        }
    }
    if let (Some(s), Pred::CopNp { np: Some(o), .. }) = (&c.subj, &c.pred) {
        if s.num != o.num {
            return Verdict::Refuse("\"is\" joins two different numbers");
        }
    }
    Verdict::Ok(0.0)
}

/// The verb agrees with its subject: "the dog barks", "dogs bark".
fn agreement(cx: &Ctx, c: &Cand) -> Verdict {
    let Some(s) = &c.subj else { return Verdict::Ok(0.0) };
    let (verb, tense) = match &c.pred {
        Pred::Intr { verb, tense } | Pred::Tr { verb, tense, .. } => (*verb, *tense),
        _ => return Verdict::Ok(0.0),
    };
    let want = match (tense, s.num) {
        (Tense::Past, _) => Form::Past,
        (Tense::Present, Num::Sg) => Form::ThirdSg,
        (Tense::Present, Num::Pl) => Form::Base,
    };
    match cx.net.form(verb, want) {
        Some(n) if n.sure && n.form == want => Verdict::Ok(0.0),
        _ => Verdict::Refuse("no sure form of the verb agrees with the subject"),
    }
}

/// Every word in the form the sentence needs, and known to be right.
fn forms(cx: &Ctx, c: &Cand) -> Verdict {
    for np in nps(c) {
        if noun_text(cx, np).is_none() {
            return Verdict::Refuse("a noun's form is unknown or unsure");
        }
    }
    if let (Some(s), Pred::Intr { verb, tense } | Pred::Tr { verb, tense, .. }) = (&c.subj, &c.pred) {
        if verb_text(cx, *verb, *tense, s.num).is_none() {
            return Verdict::Refuse("the verb's form is unknown or unsure");
        }
    }
    Verdict::Ok(0.0)
}

/// "afraid" only after "is", "former" only before a noun.
fn adjective_place(cx: &Ctx, c: &Cand) -> Verdict {
    if let Pred::CopAdj { adj, .. } = &c.pred {
        let m = cx.node(*adj).marker;
        if m == ATTRIBUTIVE_ONLY || m == POSTNOMINAL {
            return Verdict::Refuse("this adjective cannot stand after \"is\"");
        }
    }
    for np in nps(c) {
        if let Some(a) = np.adj {
            let m = cx.node(a).marker;
            if m == PREDICATIVE_ONLY || m == POSTNOMINAL {
                return Verdict::Refuse("this adjective cannot stand before a noun");
            }
            if cx.node(a).pos != Pos::Adj {
                return Verdict::Refuse("not an adjective");
            }
        }
    }
    Verdict::Ok(0.0)
}

/// No word twice ("dogs dog", "a dog is a dog").
fn no_repeat(cx: &Ctx, c: &Cand) -> Verdict {
    let mut words: Vec<String> = vec![cx.node(cx.topic).text.clone()];
    let mut add = |i: u32| words.push(cx.node(i).text.clone());
    match &c.pred {
        Pred::Intr { verb, .. } => add(*verb),
        Pred::Tr { verb, obj, .. } => {
            add(*verb);
            if let Some(o) = obj {
                add(o.noun);
            }
        }
        Pred::CopAdj { adj, .. } => add(*adj),
        Pred::CopNp { noun, .. } => add(*noun),
        Pred::Have { part, .. } => add(*part),
    }
    for np in nps(c) {
        if let Some(a) = np.adj {
            words.push(cx.node(a).text.clone());
        }
    }
    // nor the topic again under another name ("cars cut an automobile")
    let others: Vec<u32> = match &c.pred {
        Pred::Tr { obj: Some(o), .. } => vec![o.noun],
        Pred::Have { part, .. } => vec![*part],
        _ => vec![],
    };
    if others.iter().any(|&o| cx.has_line(cx.topic, crate::network::Kind::Synonym, o)) {
        return Verdict::Refuse("the topic again under another name");
    }
    // nor inside another word ("the apple is an apple tree")
    let topic = cx.node(cx.topic).text.clone();
    let inside = |i: u32| cx.node(i).text.split(' ').any(|w| w == topic) && i != cx.topic;
    let named: Vec<u32> = match &c.pred {
        Pred::Tr { obj: Some(o), .. } => vec![o.noun],
        Pred::CopNp { noun, .. } => vec![*noun],
        Pred::Have { part, .. } => vec![*part],
        _ => vec![],
    };
    if named.into_iter().any(inside) {
        return Verdict::Refuse("the topic again inside another word");
    }
    let n = words.len();
    words.sort();
    words.dedup();
    if words.len() < n {
        Verdict::Refuse("a word twice")
    } else {
        Verdict::Ok(0.0)
    }
}

/// "a" before a consonant sound, "an" before a vowel sound, in the finished words.
fn article(cx: &Ctx, c: &Cand) -> Verdict {
    if !c.complete() {
        return Verdict::Ok(0.0);
    }
    let Some(text) = realize(cx, c) else { return Verdict::Refuse("cannot be written") };
    let t: Vec<String> = text.trim_end_matches('.').split(' ').map(|w| w.to_lowercase()).collect();
    for k in 0..t.len().saturating_sub(1) {
        if (t[k] == "a" || t[k] == "an") && lexicon::article(&t[k + 1], &cx.cfg.article) != t[k] {
            return Verdict::Refuse("the wrong article");
        }
    }
    Verdict::Ok(0.0)
}

/// A person's verb wants a person (or an animal), a thing's verb a thing:
/// WordNet's frames say "Somebody ----s" or "Something ----s".
fn animacy(cx: &Ctx, c: &Cand) -> Verdict {
    let Some(s) = &c.subj else { return Verdict::Ok(0.0) };
    let sa = cx.node(s.noun).animate;
    let ok = match &c.pred {
        Pred::Intr { verb, .. } => cx.node(*verb).frames & (1 << if sa { 2 } else { 1 }) != 0,
        Pred::Tr { verb, obj: Some(o), .. } => {
            let oa = cx.node(o.noun).animate;
            let f = match (sa, oa) {
                (true, true) => 9,
                (true, false) => 8,
                (false, true) => 10,
                (false, false) => 11,
            };
            cx.node(*verb).frames & (1 << f) != 0
        }
        _ => true,
    };
    Verdict::Ok(if ok { 0.0 } else { cx.cfg.score.animacy })
}
