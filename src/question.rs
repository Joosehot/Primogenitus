//! The question parser: the user's sentence read whole, word for word. Every
//! word must have its place in one of the question forms below; a sentence
//! with a word left over is not understood (never a guess from key words).
//!
//!   hello / hi / hey                         greeting
//!   bye / goodbye                            goodbye
//!   thanks / thank you                       thanks
//!   help / what can you do                   help
//!   NP / tell me about NP / what about NP    about
//!   what is NP / what are NP                 what it is
//!   what is NP like / describe NP            what it is like
//!   what does NP do / what do NP do          what it does
//!   what does NP have / what do NP have      what it has
//!   what else (does NP do / ...)             the same, something not said yet
//!   is NP NP / are NP NP                     is it a kind of
//!   is NP ADJ / are NP ADJ                   is it like that
//!   does NP have NP / do NP have NP          does it have
//!   does NP VERB [NP] / do / can             does it do
//!
//! NP is a noun phrase (a/an/the/some, a noun of one or more words, singular
//! or plural) or a pronoun (it, they, them, one) for what was talked about.

use crate::lexicon;
use crate::network::{Form, Network, Pos};
use crate::parser;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Subject {
    Noun(u32),
    /// it, they, them: what was talked about last
    Pronoun,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Intent {
    About,
    WhatIs,
    WhatLike,
    WhatDoes,
    WhatHas,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Question {
    Greeting,
    Goodbye,
    Thanks,
    Help,
    /// an open question; `more`: something not said yet ("what else")
    Ask { topic: Subject, intent: Intent, more: bool },
    /// "what else?" alone: the last question again, something not said yet
    More,
    IsA { topic: Subject, kind: u32 },
    IsAdj { topic: Subject, adj: u32 },
    Has { topic: Subject, part: u32 },
    Does { topic: Subject, verb: u32, obj: Option<u32> },
    /// not understood, and why
    Unknown(String),
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || ",.?!;:".contains(c))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// Every noun phrase that starts at t[i], longest first: (subject, end).
/// Not only the longest: "what does a dog do" has the noun "dog do" in it,
/// and only "dog" lets the whole sentence be read.
fn nps(net: &Network, t: &[String], i: usize) -> Vec<(Subject, usize)> {
    let mut out = Vec::new();
    let Some(w) = t.get(i) else { return out };
    if matches!(w.as_str(), "it" | "they" | "them" | "one") {
        out.push((Subject::Pronoun, i + 1));
        return out;
    }
    let j = if matches!(w.as_str(), "a" | "an" | "the" | "some" | "all" | "every") { i + 1 } else { i };
    // the longest noun
    for len in (1..=4).rev() {
        if j + len > t.len() {
            continue;
        }
        let text = t[j..j + len].join(" ");
        if len == 1 && lexicon::closed(&text).is_some() {
            continue;
        }
        let n = net
            .lookup(&text)
            .iter()
            .map(|&q| net.node(q))
            .find(|n| n.pos == Pos::Noun && !n.proper && matches!(n.form, Form::Base | Form::Plural));
        if let Some(n) = n {
            out.push((Subject::Noun(n.lemma), j + len));
        }
    }
    out
}

fn adjective(net: &Network, w: &str) -> Option<u32> {
    net.lookup(w).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Adj && n.form == Form::Base).map(|n| n.lemma)
}

fn verb(net: &Network, w: &str) -> Option<u32> {
    if lexicon::closed(w).is_some() {
        return None;
    }
    net.lookup(w).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Verb && n.form == Form::Base).map(|n| n.lemma)
}

pub fn read(net: &Network, text: &str) -> Question {
    let t = words(text);
    let s: Vec<&str> = t.iter().map(|w| w.as_str()).collect();
    use Question::*;
    match s.as_slice() {
        [] => return Unknown("say something".into()),
        ["hello" | "hi" | "hey" | "hiya" | "greetings"] | ["hello" | "hi" | "hey", "there" | "primo"]
        | ["good", "morning" | "afternoon" | "evening"] => return Greeting,
        ["bye" | "goodbye"] | ["see", "you"] | ["see", "you", "later"] | ["good", "night"] => return Goodbye,
        ["thanks" | "thx"] | ["thank", "you"] | ["thanks", "a", "lot"] => return Thanks,
        ["help"] | ["what", "can", "you", "do"] | ["what", "can", "i", "ask"] | ["what", "can", "i", "ask", "you"] => return Help,
        ["what", "else"] | ["anything", "else"] | ["more"] | ["tell", "me", "more"] => return More,
        _ => {}
    }
    let n = t.len();
    // the noun phrase from i that ends exactly at `end`
    let whole = |i: usize, end: usize| nps(net, &t, i).into_iter().find(|&(_, e)| e == end).map(|(s, _)| s);
    // "tell me about NP", "what about NP", "and NP", "NP"
    for (prefix, intent) in [(&["tell", "me", "about"][..], Intent::About), (&["what", "about"][..], Intent::About),
                             (&["and"][..], Intent::About), (&["describe"][..], Intent::WhatLike), (&[][..], Intent::About)] {
        if s.starts_with(prefix) {
            if let Some(topic) = whole(prefix.len(), n) {
                return Ask { topic, intent, more: false };
            }
        }
    }
    // "what [else] is/are NP [like]", "what [else] does/do NP do/have"
    if s.first() == Some(&"what") {
        let (more, k) = if s.get(1) == Some(&"else") { (true, 2) } else { (false, 1) };
        match s.get(k) {
            Some(&"is") | Some(&"are") => {
                if let Some(topic) = whole(k + 1, n) {
                    return Ask { topic, intent: Intent::WhatIs, more };
                }
                if s.last() == Some(&"like") {
                    if let Some(topic) = whole(k + 1, n - 1) {
                        return Ask { topic, intent: Intent::WhatLike, more };
                    }
                }
            }
            Some(&"does") | Some(&"do") | Some(&"can") => {
                let intent = match s.last() {
                    Some(&"do") => Some(Intent::WhatDoes),
                    Some(&"have") => Some(Intent::WhatHas),
                    _ => None,
                };
                if let Some(intent) = intent {
                    if let Some(topic) = whole(k + 1, n - 1) {
                        return Ask { topic, intent, more };
                    }
                }
            }
            _ => {}
        }
    }
    // "is/are NP NP", "is/are NP ADJ"
    if matches!(s.first(), Some(&"is") | Some(&"are")) {
        for (topic, e) in nps(net, &t, 1) {
            if let Some(Subject::Noun(kind)) = whole(e, n) {
                return IsA { topic, kind };
            }
            if e + 1 == n {
                if let Some(adj) = adjective(net, &t[e]) {
                    return IsAdj { topic, adj };
                }
            }
        }
    }
    // "does/do NP have NP", "does/do/can NP VERB [NP]"
    if matches!(s.first(), Some(&"does") | Some(&"do") | Some(&"can")) {
        for (topic, e) in nps(net, &t, 1) {
            if s.get(e) == Some(&"have") {
                if let Some(Subject::Noun(part)) = whole(e + 1, n) {
                    return Has { topic, part };
                }
            }
            if let Some(v) = t.get(e).and_then(|w| verb(net, w)) {
                if e + 1 == n {
                    return Does { topic, verb: v, obj: None };
                }
                if let Some(Subject::Noun(o)) = whole(e + 1, n) {
                    return Does { topic, verb: v, obj: Some(o) };
                }
            }
        }
    }
    Unknown(why_not(net, &t))
}

/// Why a sentence was not understood: an unknown word, or a shape not read.
fn why_not(net: &Network, t: &[String]) -> String {
    for w in t {
        if lexicon::closed(w).is_none() && net.lookup(w).is_empty() && !matches!(w.as_str(), "what" | "else" | "tell" | "describe") {
            return format!("I do not know the word \"{w}\" (did you mean \"{}\"?).", parser::closest(net, w));
        }
    }
    "I could not read the whole sentence. I read questions like: what is a dog, what does a dog do, what does a dog have, what is a dog like, is a dog an animal, does a dog have a tail, can dogs swim.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn reads_the_forms() {
        let frozen = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data").join("network.bin");
        if !frozen.exists() {
            return;
        }
        let net = Network::load(&frozen).unwrap();
        let dog = net.base("dog", Pos::Noun).unwrap();
        let animal = net.base("animal", Pos::Noun).unwrap();
        let tail = net.base("tail", Pos::Noun).unwrap();
        let bark = net.base("bark", Pos::Verb).unwrap();
        assert_eq!(read(&net, "Hello!"), Question::Greeting);
        assert_eq!(read(&net, "What is a dog?"), Question::Ask { topic: Subject::Noun(dog), intent: Intent::WhatIs, more: false });
        assert_eq!(read(&net, "what do dogs do"), Question::Ask { topic: Subject::Noun(dog), intent: Intent::WhatDoes, more: false });
        assert_eq!(read(&net, "What else does it have?"), Question::Ask { topic: Subject::Pronoun, intent: Intent::WhatHas, more: true });
        assert_eq!(read(&net, "Is a dog an animal?"), Question::IsA { topic: Subject::Noun(dog), kind: animal });
        assert_eq!(read(&net, "Does a dog have a tail?"), Question::Has { topic: Subject::Noun(dog), part: tail });
        assert_eq!(read(&net, "Can dogs bark?"), Question::Does { topic: Subject::Noun(dog), verb: bark, obj: None });
        assert_eq!(read(&net, "dogs"), Question::Ask { topic: Subject::Noun(dog), intent: Intent::About, more: false });
        assert!(matches!(read(&net, "What is a dog doing in the garden?"), Question::Unknown(_)));
        assert!(matches!(read(&net, "What is a blorft?"), Question::Unknown(_)));
    }
}
