//! What the user said in a turn, read whole. A question goes to Primo's
//! question parser; anything else is read here as one of the statement forms
//! below. A turn of several clauses ("I am sad because my dog died") is read
//! clause by clause.
//!
//!   my name is X / I am X / call me X        a name
//!   I am / I feel [so, very, ...] ADJ        a feeling
//!   I am a NP                                what they are (a teacher)
//!   I like / love / enjoy NP                 something they like
//!   I like / love / enjoy to VERB, VERBing   something they like doing
//!   I hate / dislike NP                      something they dislike
//!   I have [got] NP                          something they have
//!   my NP is ADJ / my NP is a NP             about something of theirs
//!   my NP VERBed / died / passed away        what happened to it
//!   his / her / its name is X                its name (a pet's)
//!   thanks / thank you [for ...]             thanks
//!   NP is / are ADJ                          what they think
//!   yes / no / maybe / I do not know         an answer
//!   haha / lol, ok / cool, sorry             the small words

use crate::lexicon;
use crate::network::{Form, Network, Pos};
use crate::question::{self, Question};

#[derive(Clone, Debug, PartialEq)]
pub enum Turn {
    Ask(Question),
    Name(String),
    Feel(u32),
    Role(u32),
    Like(u32),
    LikeDoing(u32),
    Dislike(u32),
    Have(u32),
    /// a pet's name, and the pronoun the user used for it (his, her, its)
    ItsName { name: String, pron: String },
    MyIs { noun: u32, adj: u32 },
    MyKind { noun: u32, kind: u32 },
    Happened { noun: u32, event: u32 },
    Opinion { noun: u32, adj: u32 },
    Yes,
    No,
    DontKnow,
    Laugh,
    Okay,
    Sorry,
}

/// The turn's words, lower case, contractions opened; and as written (for names).
fn words(text: &str) -> (Vec<String>, Vec<String>) {
    let mut low = Vec::new();
    let mut orig = Vec::new();
    for w in text.split(|c: char| c.is_whitespace() || ",.?!;:".contains(c)).filter(|s| !s.is_empty()) {
        let w = w.replace('\u{2019}', "'");
        let l = w.to_lowercase();
        let open: Vec<&str> = match l.as_str() {
            "i'm" | "im" => vec!["i", "am"],
            "i've" => vec!["i", "have"],
            "i'd" => vec!["i", "would"],
            "don't" | "dont" => vec!["do", "not"],
            "can't" | "cant" => vec!["can", "not"],
            "it's" => vec!["it", "is"],
            "that's" => vec!["that", "is"],
            _ => vec![],
        };
        if open.is_empty() {
            low.push(l);
            orig.push(w.to_string());
        } else {
            for o in open {
                low.push(o.to_string());
                orig.push(o.to_string());
            }
        }
    }
    (low, orig)
}

/// Words at the edges of a clause that change nothing Raven reads: "well,
/// I have a cat too", "I am happy now".
const LEAD: &[&str] = &["well", "oh", "um", "uh", "so", "actually", "honestly", "anyway", "hmm"];
const TAIL: &[&str] = &["too", "also", "now", "today", "again", "anymore", "though", "please", "tonight", "lately"];

const SOFT: &[&str] = &["so", "very", "really", "quite", "pretty", "a", "bit", "little", "kind", "of", "too", "a", "lot", "just", "also"];

fn noun_at(net: &Network, w: &[String], i: usize, end: usize) -> Option<u32> {
    if i >= end {
        return None;
    }
    let j = if matches!(w[i].as_str(), "a" | "an" | "the" | "some" | "my" | "two" | "three") { i + 1 } else { i };
    let text = w[j..end].join(" ");
    let mut readings: Vec<_> = net
        .lookup(&text)
        .iter()
        .map(|&q| net.node(q))
        .filter(|n| n.pos == Pos::Noun && !n.proper && matches!(n.form, Form::Base | Form::Plural))
        .collect();
    readings.sort_by_key(|n| (!(n.sure && net.node(n.lemma).sure), n.lemma));
    readings.first().map(|n| n.lemma)
}

fn adj_at(net: &Network, w: &[String], mut i: usize, end: usize) -> Option<u32> {
    while i + 1 < end && SOFT.contains(&w[i].as_str()) {
        i += 1;
    }
    (i + 1 == end).then(|| ()).and_then(|_| {
        net.lookup(&w[i]).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Adj && n.form == Form::Base).map(|n| n.lemma)
    })
}

fn verb_base(net: &Network, w: &str) -> Option<u32> {
    net.lookup(w).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Verb && matches!(n.form, Form::Base)).map(|n| n.lemma)
}

fn verb_any(net: &Network, w: &str, forms: &[Form]) -> Option<u32> {
    net.lookup(w).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Verb && forms.contains(&n.form)).map(|n| n.lemma)
}

fn is_name(orig: &str, low: &str, net: &Network) -> bool {
    let cap = orig.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
    cap || net.lookup(low).is_empty()
}

/// One clause, read whole.
fn clause(net: &Network, w: &[String], o: &[String]) -> Option<Turn> {
    use Turn::*;
    let (mut a, mut b) = (0, w.len());
    while a < b && LEAD.contains(&w[a].as_str()) {
        a += 1;
    }
    while b > a + 1 && (TAIL.contains(&w[b - 1].as_str()) || (w[b - 1] == "well" && b >= 2 && w[b - 2] == "as")) {
        b -= if w[b - 1] == "well" { 2 } else { 1 };
    }
    let (w, o) = (&w[a..b], &o[a..b]);
    let s: Vec<&str> = w.iter().map(|x| x.as_str()).collect();
    let n = s.len();
    if n == 0 {
        return None;
    }
    // thanks, whatever it is for
    if s[0] == "thanks" || s[0] == "thx" || s.starts_with(&["thank", "you"]) {
        let k = if s[0] == "thank" { 2 } else { 1 };
        if n == k || s.get(k) == Some(&"for") || s[k..] == ["so", "much"] || s[k..] == ["a", "lot"] {
            return Some(Ask(Question::Thanks));
        }
    }
    // a pet's name: "her name was Musti", "he is called Rex"
    if matches!(s[0], "his" | "her" | "its" | "their") && s.get(1) == Some(&"name") && matches!(s.get(2), Some(&"is") | Some(&"was")) && n == 4 {
        return Some(ItsName { name: o[3].clone(), pron: s[0].to_string() });
    }
    if matches!(s[0], "he" | "she" | "it") && matches!(s.get(1), Some(&"is") | Some(&"was")) && s.get(2) == Some(&"called") && n == 4 {
        let pron = match s[0] { "he" => "his", "she" => "her", _ => "its" };
        return Some(ItsName { name: o[3].clone(), pron: pron.to_string() });
    }
    match s.as_slice() {
        ["yes" | "yeah" | "yep" | "sure" | "of course" | "definitely" | "indeed"] | ["yes", "please"] => return Some(Yes),
        ["no" | "nope" | "nah"] | ["not", "really"] | ["no", "thanks"] => return Some(No),
        ["maybe" | "perhaps"] | ["i", "do", "not", "know"] | ["idk"] | ["not", "sure"] | ["i", "am", "not", "sure"] => return Some(DontKnow),
        ["haha" | "hahaha" | "lol" | "hehe" | "ha"] => return Some(Laugh),
        ["ok" | "okay" | "cool" | "nice" | "great" | "alright" | "fine" | "good" | "awesome"] | ["i", "see"] | ["all", "right"] => {
            return Some(Okay)
        }
        ["sorry"] | ["i", "am", "sorry"] | ["my", "bad"] => return Some(Sorry),
        _ => {}
    }
    // a name
    if s.starts_with(&["my", "name", "is"]) && (n == 4 || n == 5) {
        return Some(Name(o[3..].join(" ")));
    }
    if s.starts_with(&["call", "me"]) && n == 3 {
        return Some(Name(o[2].clone()));
    }
    if s.starts_with(&["i", "am"]) && n == 3 && is_name(&o[2], &s[2], net) && adj_at(net, w, 2, n).is_none() {
        return Some(Name(o[2].clone()));
    }
    // a feeling: "i am (so) sad", "i feel tired"
    if s.starts_with(&["i", "am"]) || s.starts_with(&["i", "feel"]) {
        if let Some(a) = adj_at(net, w, 2, n) {
            return Some(Feel(a));
        }
        if s.starts_with(&["i", "am"]) && n >= 4 && matches!(s[2], "a" | "an") {
            if let Some(r) = noun_at(net, w, 2, n) {
                return Some(Role(r));
            }
        }
    }
    // likes and dislikes: "i (really) like dogs", "i love to swim", "i enjoy swimming"
    let mut k = 1;
    if s.first() == Some(&"i") {
        while k < n && SOFT.contains(&s[k]) {
            k += 1;
        }
        let liking = match s.get(k) {
            Some(&"like" | &"love" | &"enjoy" | &"adore") => Some(true),
            Some(&"hate" | &"dislike" | &"detest") => Some(false),
            _ => None,
        };
        if let Some(good) = liking {
            if s.get(k + 1) == Some(&"to") && k + 3 == n {
                if let Some(v) = verb_base(net, s[k + 2]) {
                    return Some(if good { LikeDoing(v) } else { Dislike(v) });
                }
            }
            if k + 2 == n {
                if let Some(v) = verb_any(net, s[k + 1], &[Form::Ing]) {
                    return Some(if good { LikeDoing(v) } else { Dislike(v) });
                }
            }
            if let Some(x) = noun_at(net, w, k + 1, n) {
                return Some(if good { Like(x) } else { Dislike(x) });
            }
        }
        // "i have (got) a dog"
        if s.get(1) == Some(&"have") {
            let j = if s.get(2) == Some(&"got") { 3 } else { 2 };
            if let Some(x) = noun_at(net, w, j, n) {
                return Some(Have(x));
            }
        }
    }
    // "my dog is old", "my dog is a poodle", "my dog died", "my dog passed away"
    if s.first() == Some(&"my") {
        for e in (2..n).rev() {
            let Some(noun) = noun_at(net, w, 1, e) else { continue };
            let rest = &s[e..];
            match rest {
                ["is" | "was", ..] if rest.len() >= 2 => {
                    if let Some(adj) = adj_at(net, w, e + 1, n) {
                        return Some(MyIs { noun, adj });
                    }
                    if let Some(kind) = noun_at(net, w, e + 1, n) {
                        return Some(MyKind { noun, kind });
                    }
                }
                ["passed", "away"] | ["passed"] => return verb_base(net, "die").map(|event| Happened { noun, event }),
                [v] => {
                    if let Some(event) = verb_any(net, v, &[Form::Past, Form::ThirdSg]) {
                        return Some(Happened { noun, event });
                    }
                }
                [v, "away"] => {
                    if let Some(event) = verb_any(net, v, &[Form::Past]) {
                        return Some(Happened { noun, event });
                    }
                }
                _ => {}
            }
        }
    }
    // "dogs are cute"
    for e in (1..n).rev() {
        if matches!(s[e], "is" | "are") && lexicon::closed(&s[0]).is_none() {
            if let (Some(noun), Some(adj)) = (noun_at(net, w, 0, e), adj_at(net, w, e + 1, n)) {
                return Some(Opinion { noun, adj });
            }
        }
    }
    None
}

/// The turn: its clauses read one by one, and whether all of them were read.
pub fn read(net: &Network, text: &str) -> (Vec<Turn>, bool) {
    let q = question::read(net, text);
    if !matches!(q, Question::Unknown(_)) {
        return (vec![Turn::Ask(q)], true);
    }
    let mut out = Vec::new();
    let mut all = true;
    let cut = |w: &str| matches!(w, "and" | "but" | "because" | "since" | "though");
    // clauses end at a comma or a full stop, and before "and", "but", "because"
    for piece in text.split(|c: char| ",.;!?".contains(c)) {
        let (low, orig) = words(piece);
        let mut start = 0;
        for i in 0..=low.len() {
            if i == low.len() || (cut(&low[i]) && i > start) {
                let (w, o) = (&low[start..i], &orig[start..i]);
                // a clause on its own may still be a question ("and what do dogs eat")
                match clause(net, w, o) {
                    Some(t) => out.push(t),
                    None if !w.is_empty() => match question::read(net, &o.join(" ")) {
                        Question::Unknown(_) => all = false,
                        q => out.push(Turn::Ask(q)),
                    },
                    None => {}
                }
                start = i + 1;
            }
        }
    }
    (out, all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn reads_statements() {
        let frozen = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data").join("network.bin");
        if !frozen.exists() {
            return;
        }
        let net = Network::load(&frozen).unwrap();
        let dog = net.base("dog", Pos::Noun).unwrap();
        let sad = net.base("sad", Pos::Adj).unwrap();
        assert_eq!(read(&net, "My name is Joose").0, vec![Turn::Name("Joose".into())]);
        assert_eq!(read(&net, "I'm Joose").0, vec![Turn::Name("Joose".into())]);
        assert_eq!(read(&net, "I am so sad").0, vec![Turn::Feel(sad)]);
        assert_eq!(read(&net, "I have a dog").0, vec![Turn::Have(dog)]);
        assert_eq!(read(&net, "I really love dogs").0, vec![Turn::Like(dog)]);
        assert!(matches!(read(&net, "My dog died").0[0], Turn::Happened { noun, .. } if noun == dog));
        let (t, all) = read(&net, "I am sad because my dog died");
        assert!(all && t.len() == 2, "{t:?}");
    }
}
