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
//!   he / she / it / they is / was ADJ, died  about what they talked of last
//!   nothing / not much                       nothing to tell
//!   you are ADJ / you are not NP / do you know that you ...   about Raven
//!   I made you / your creator                who made it
//!   I am feeling / I have been feeling ADJ   a feeling
//!   yes, I want to / no, not now             yes and no, in a few more words
//!   what do you mean / huh / that makes no sense    a repair: Raven said something wrong
//!
//! And in the light of what Raven asked last (Expect): after "what is your
//! name?" a bare "Joose" is a name; after "how are you?" a bare "good" or
//! "not bad" is a feeling; after "what is your cat's name?" a bare "Misu" is
//! the cat's name; after "what was your dog like?" "very playful and
//! friendly" describes the dog; after "how old is your cat?" "3 years" is its
//! age. After an open question ("what happened?") a story Raven cannot read
//! is heard, not refused.

use crate::lexicon;
use crate::network::{Form, Network, Pos};
use crate::question::{self, Question};

/// What Raven's last question waits for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expect {
    Name,
    Feeling,
    PetName(u32),
    Describe(u32),
    Age(u32),
    HaveIt(u32),
    WantTalk,
    /// an open question: whatever comes is a story to hear
    Story,
    /// "what would you like to talk about?"
    Topic,
    /// "how often do you code?": the verb
    Often(u32),
    /// a yes/no question the model asked: what a yes says
    Confirm(crate::mblm::Confirm),
}

/// What the turn is read in the light of.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    pub expect: Option<Expect>,
    /// what "he", "she", "it", "they" are: the user's thing talked of last
    pub referent: Option<u32>,
    /// they are low: nothing they say is refused, it is heard
    pub low: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Turn {
    Ask(Question),
    /// "not bad": a feeling denied
    FeelNot(u32),
    /// the age of their thing, as said ("3 years")
    Age { noun: u32, age: String },
    /// a pet's name given right after Raven asked for it
    PetNamed { noun: u32, name: String },
    /// a story Raven asked for but cannot read: heard
    Heard,
    /// an answer to "how often do you ...?": as said ("every day")
    Often(String),
    /// "what do you mean?": what Raven said did not make sense
    Repair,
    /// "I broke up", "I lost my job", "we broke up": what happened to them
    IHappened(u32),
    /// "nothing really", "not much"
    Nothing,
    /// "you are smart", "you are not an AI": something said about Raven (the
    /// adjective when one was read)
    AboutMe(Option<u32>),
    /// "I made you", "your creator"
    Creator,
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
    /// "yes, I want to": wanting it (to talk, most often)
    YesWant,
    No,
    DontKnow,
    Laugh,
    Okay,
    Sorry,
    /// "I have a dog called Musti": the thing and its name at once
    HaveNamed { noun: u32, name: String },
    /// "we ran ten kilometers": something they did
    Did(u32),
    /// "they need a lot of exercise": something said of what they talked of last
    About(u32),
    /// "why?": why Raven said what it said
    Why,
    /// "what do I do for work?": what they told they are
    MyRole,
    /// "what were we talking about?"
    WhatAbout,
    /// "should I rest?": a choice that is theirs
    Advise(u32),
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
const TAIL: &[&str] = &["too", "also", "now", "today", "again", "anymore", "though", "please", "tonight", "lately", "really"];

/// "I am fine, I guess": hedges at the end change nothing Raven reads.
const HEDGE: &[&[&str]] = &[&["i", "guess"], &["i", "think"], &["i", "suppose"], &["kind", "of"], &["sort", "of"], &["or", "so"]];

const SOFT: &[&str] = &["so", "very", "really", "quite", "pretty", "a", "bit", "little", "kind", "of", "too", "a", "lot", "just", "also"];

fn noun_at(net: &Network, w: &[String], i: usize, end: usize) -> Option<u32> {
    if i >= end {
        return None;
    }
    let mut j = if matches!(w[i].as_str(), "a" | "an" | "the" | "some" | "my" | "two" | "three" | "many" | "several" | "few" | "four" | "five") {
        i + 1
    } else {
        i
    };
    // "plenty of motors", "a lot of cats", "lots of books", "a few dogs"
    for q in [&["plenty", "of"][..], &["a", "lot", "of"][..], &["lots", "of"][..], &["a", "few"][..], &["a", "couple", "of"][..], &["loads", "of"][..]] {
        if w[i..end].iter().map(|x| x.as_str()).take(q.len()).eq(q.iter().copied()) && i + q.len() < end {
            j = i + q.len();
        }
    }
    // "my old dog": adjectives before the noun (the whole first, "hot dog" is a noun)
    if net.lookup(&w[j..end].join(" ")).iter().all(|&q| net.node(q).pos != Pos::Noun) {
        while j + 1 < end
            && net.lookup(&w[j]).iter().any(|&q| net.node(q).pos == Pos::Adj)
            && lexicon::closed(&w[j]).is_none()
        {
            j += 1;
        }
    }
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
fn clause(net: &Network, w: &[String], o: &[String], cx: &Context) -> Option<Turn> {
    use Turn::*;
    let (mut a, mut b) = (0, w.len());
    while a < b && LEAD.contains(&w[a].as_str()) {
        a += 1;
    }
    loop {
        if b > a + 1 && (TAIL.contains(&w[b - 1].as_str()) || matches!(w[b - 1].as_str(), "yesterday" | "recently")) {
            b -= 1;
        } else if b > a + 2 && w[b - 1] == "well" && w[b - 2] == "as" {
            b -= 2;
        } else if b > a + 2 && matches!(w[b - 2].as_str(), "last" | "this") && matches!(w[b - 1].as_str(), "week" | "month" | "year" | "night" | "morning" | "summer" | "winter" | "weekend") {
            b -= 2;
        } else if let Some(h) = HEDGE.iter().find(|h| b >= a + 1 + h.len() && w[b - h.len()..b].iter().map(|x| x.as_str()).eq(h.iter().copied())) {
            b -= h.len();
        } else if b > a + 2 && w[b - 1] == "ago" {
            // "a while ago", "two years ago"
            b -= if b > a + 3 && matches!(w[b - 3].as_str(), "a" | "an" | "two" | "three" | "few" | "many" | "some") { 3 } else { 2 };
        } else {
            break;
        }
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
    // "why?", "why is that?", "how come?": why Raven said it
    if matches!(s.as_slice(), ["why"] | ["why", "is", "that"] | ["why", "so"] | ["why", "not"] | ["how", "come"] | ["why", "do", "you", "say", "that"]
        | ["why", "do", "you", "think", "so"] | ["why", "would", "you", "say", "that"]) {
        return Some(Why);
    }
    // "what do I do (for work)?", "what is my job?", "what am I?"
    if matches!(s.as_slice(), ["what", "do", "i", "do", ..] | ["what", "is", "my", "job" | "work" | "profession" | "occupation"] | ["what", "am", "i"]
        | ["what", "do", "i", "work", "as"] | ["what", "is", "my", "line", "of", "work"]) {
        return Some(MyRole);
    }
    // "what were we talking about?"
    if matches!(s.as_slice(), ["what", "were" | "are", "we", "talking", "about"] | ["what", "did", "we", "talk", "about"]
        | ["what", "was", "i", "saying"] | ["where", "were", "we"]) {
        return Some(WhatAbout);
    }
    // "what do you think about huskies?": what Raven knows of it
    if s.starts_with(&["what", "do", "you", "think"]) && matches!(s.get(4), Some(&"about") | Some(&"of")) {
        if let Some(x) = noun_at(net, w, 5, n) {
            return Some(Ask(Question::Ask { topic: question::Subject::Noun(x), intent: question::Intent::About, more: false }));
        }
    }
    // "should I rest?", "do you think I should rest?": a choice that is theirs
    let sh = if s.starts_with(&["should", "i"]) { 2 } else if s.starts_with(&["do", "you", "think", "i", "should"]) { 5 } else { 0 };
    if sh > 0 && sh < n {
        if let Some(v) = verb_base(net, s[sh]) {
            return Some(Advise(v));
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
        ["nothing"] | ["nothing", "much"] | ["not", "much"] | ["not", "a", "lot"] | ["nothing", "special"] | ["the", "usual"] => {
            return Some(Nothing)
        }
        ["what", "do", "you", "mean", ..] | ["what", "does", "that", "mean"] | ["huh"] | ["what"] | ["that", "makes", "no", "sense"]
        | ["what", "are", "you", "talking", "about"] | ["i", "do", "not", "understand"] | ["that", "is", "weird" | "strange" | "odd"]
        | ["what", "is", "that", "supposed", "to", "mean"] | ["excuse", "me"] => return Some(Repair),
        _ => {}
    }
    // "yes, I want to", "yeah I do", "no, not now", "I would rather not"
    const YES_TAIL: &[&[&str]] = &[&["i", "want", "to"], &["i", "do"], &["i", "would"], &["i", "would", "like", "to"], &["please"],
                                   &["of", "course"], &["let", "us"], &["lets"], &["i", "guess"], &["sure"], &["it", "is"],
                                   &["it", "was"], &["it", "does"], &["it", "really", "is"], &["i", "am"], &["i", "know"],
                                   &["that", "is", "right"], &["that", "is", "true"], &["exactly"], &["true"], &["right"], &["it", "did"]];
    const NO_TAIL: &[&[&str]] = &[&["i", "do", "not"], &["not", "now"], &["not", "really"], &["thanks"], &["thank", "you"],
                                  &["i", "would", "rather", "not"], &["maybe", "later"], &["it", "is", "not"], &["it", "was", "not"],
                                  &["i", "am", "not"], &["not", "at", "all"]];
    let tail_in = |k: usize, list: &[&[&str]]| list.iter().any(|t| s[k..].iter().copied().eq(t.iter().copied()));
    const WANT: &[&[&str]] = &[&["i", "want", "to"], &["i", "would", "like", "to"], &["please"], &["i", "would"], &["let", "us"], &["lets"]];
    match s[0] {
        "yes" | "yeah" | "yep" | "sure" | "ok" | "okay" if n > 1 && tail_in(1, WANT) => return Some(YesWant),
        _ if tail_in(0, &[&["i", "want", "to"], &["i", "would", "like", "to"]]) => return Some(YesWant),
        "yes" | "yeah" | "yep" | "sure" | "ok" | "okay" if n > 1 && tail_in(1, YES_TAIL) => return Some(Yes),
        "no" | "nope" | "nah" if n > 1 && tail_in(1, NO_TAIL) => return Some(No),
        _ if tail_in(0, &[&["i", "want", "to"], &["i", "would", "like", "to"], &["i", "do"], &["it", "is"], &["it", "really", "is"],
                          &["it", "was"], &["i", "know"], &["exactly"], &["true"], &["right"], &["that", "is", "true"],
                          &["that", "is", "right"], &["it", "does"], &["it", "did"]]) => return Some(Yes),
        _ if tail_in(0, &[&["i", "would", "rather", "not"], &["not", "now"], &["maybe", "later"]]) => return Some(No),
        _ => {}
    }
    // a feeling: "i am feeling a little sad", "i have been feeling down"
    for p in [&["i", "am", "feeling"][..], &["i", "have", "been", "feeling"][..], &["i", "have", "been"][..], &["feeling"][..]] {
        if s.starts_with(p) {
            if let Some(a) = adj_at(net, w, p.len(), n) {
                return Some(Feel(a));
            }
        }
    }
    // what happened to them: "I broke up", "we broke up", "I lost my job", "I got fired"
    if matches!(s[0], "i" | "we") && n >= 2 {
        let k = if matches!(s[1], "got" | "have" | "just" | "was") && n >= 3 { 2 } else { 1 };
        let k = if s.get(k) == Some(&"just") { k + 1 } else { k };
        // a verb of two words first ("broke up" is "break up"), then one
        for len in [2usize, 1] {
            if k + len > n {
                continue;
            }
            let head = s[k];
            let rest = s[k + 1..k + len].join(" ");
            for v in net.lookup(head).iter().map(|&q| net.node(q)).filter(|x| x.pos == Pos::Verb && matches!(x.form, Form::Past | Form::PastPart)) {
                let base = &net.node(v.lemma).text;
                // "I am fine" is no event ("am" is a form of "be", "fine" also a noun)
                if matches!(base.as_str(), "be" | "have" | "do") || !v.sure {
                    continue;
                }
                let phrase = if len == 2 { format!("{base} {rest}") } else { base.clone() };
                if let Some(event) = net.base(&phrase, Pos::Verb) {
                    // what follows may only be what it happened to ("my job", "with her")
                    let after = &s[k + len..];
                    let ok = after.is_empty()
                        || noun_at(net, w, k + len, n).is_some()
                        || matches!(after, ["with", ..] | ["again"] | ["today"]);
                    if ok {
                        // "we ran ten kilometers": done, not lost ("ran away" is the loss)
                        if len == 1 && matches!(base.as_str(), "run" | "walk" | "swim" | "ride" | "climb" | "hike" | "cycle" | "train" | "play") {
                            return Some(Did(event));
                        }
                        return Some(IHappened(event));
                    }
                }
            }
        }
    }
    // who made Raven: "I made you", "I am your creator", "your original creator"
    let creator_word = |x: &str| matches!(x, "creator" | "maker" | "author" | "father" | "mother" | "parent" | "developer" | "inventor" | "programmer");
    if (s.first() == Some(&"your") || s.starts_with(&["i", "am", "your"]) || s.starts_with(&["i", "am", "the"]))
        && s.last().map(|x| creator_word(x)).unwrap_or(false)
    {
        return Some(Creator);
    }
    if s.first() == Some(&"i") && n >= 3 && matches!(s[1], "made" | "created" | "built" | "wrote" | "programmed" | "invented" | "coded") && s[2] == "you" {
        return Some(Creator);
    }
    // about Raven: "you are smart", "you are not an AI", "do you know that you are ..."
    let k = if s.starts_with(&["do", "you", "know", "that"]) { 4 } else { 0 };
    if s.get(k) == Some(&"you") && n > k + 1 {
        let j = k + 1 + usize::from(matches!(s.get(k + 1), Some(&"are") | Some(&"were")));
        let adj = adj_at(net, w, j, n);
        return Some(AboutMe(adj));
    }
    if k == 4 {
        // "do you know that dogs bark": told, heard
        return Some(Heard);
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
    // "... with him", "... with my dog": who it is done with changes nothing read
    let n = match s.as_slice() {
        [.., "with", "him" | "her" | "them" | "it" | "you" | "friends"] if n > 3 => n - 2,
        _ => n,
    };
    let s = &s[..n];
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
            // "i like to go running"
            if s.get(k + 1) == Some(&"to") && s.get(k + 2) == Some(&"go") && k + 4 == n {
                if let Some(v) = verb_any(net, s[k + 3], &[Form::Ing]) {
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
            // "i have a dog called Musti", "... named Musti"
            if let Some(c) = (j + 1..n.saturating_sub(1)).find(|&c| matches!(s[c], "called" | "named")) {
                if let Some(x) = noun_at(net, w, j, c) {
                    return Some(HaveNamed { noun: x, name: o[c + 1..n].join(" ") });
                }
            }
            if let Some(x) = noun_at(net, w, j, n) {
                return Some(Have(x));
            }
        }
    }
    // "my dog is old", "my dog is a poodle", "my dog died", "my dog passed away"
    if s.first() == Some(&"my") {
        for e in (2..n).rev() {
            let Some(noun) = noun_at(net, w, 1, e) else { continue };
            let mut rest = &s[e..];
            while rest.len() > 1 && !matches!(rest[0], "is" | "was") && SOFT.contains(&rest[rest.len() - 1]) {
                rest = &rest[..rest.len() - 1];
            }
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
    // "she was very playful", "it is sick", "he died": what they talked of last
    if let (Some(noun), true) = (cx.referent, matches!(s[0], "he" | "she" | "it" | "they")) {
        if n >= 3 && matches!(s[1], "is" | "was" | "are" | "were") {
            if let Some(adj) = adj_at(net, w, 2, n) {
                return Some(MyIs { noun, adj });
            }
            if let Some(kind) = noun_at(net, w, 2, n) {
                return Some(MyKind { noun, kind });
            }
        }
        if n == 2 {
            if let Some(event) = verb_any(net, s[1], &[Form::Past, Form::ThirdSg]) {
                return Some(Happened { noun, event });
            }
        }
        if s[1..] == ["passed", "away"] {
            return verb_base(net, "die").map(|event| Happened { noun, event });
        }
        // "they need a lot of exercise", "he loves the snow": said of it, heard, the thread kept
        if n >= 2 && verb_any(net, s[1], &[Form::Base, Form::ThirdSg, Form::Past]).is_some() {
            return Some(About(noun));
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

/// A bare answer to what Raven asked: a name, a feeling, a description, an age.
fn answer(net: &Network, text: &str, cx: &Context) -> Option<Vec<Turn>> {
    let (w, o) = words(text);
    let mut a = 0;
    while a < w.len() && LEAD.contains(&w[a].as_str()) {
        a += 1;
    }
    // "fine, thanks", "good and you?", "fine, I guess"
    let mut b = w.len();
    loop {
        if b > a && matches!(w[b - 1].as_str(), "thanks" | "thank" | "you" | "and" | "too" | "now" | "today" | "though") {
            b -= 1;
        } else if let Some(h) = HEDGE.iter().find(|h| b >= a + 1 + h.len() && w[b - h.len()..b].iter().map(|x| x.as_str()).eq(h.iter().copied())) {
            b -= h.len();
        } else {
            break;
        }
    }
    let (w, o) = (&w[a..b], &o[a..b]);
    let n = w.len();
    if n == 0 {
        return None;
    }
    let plain = |i: usize| w[i].chars().all(|c| c.is_alphabetic() || c == '-') && lexicon::closed(&w[i]).is_none()
        && !matches!(w[i].as_str(), "yes" | "no" | "yeah" | "nope" | "ok" | "okay" | "hello" | "hi" | "hey" | "what" | "why" | "how");
    // talking of an animal, "3 years" is its age even unasked
    let expect = match (cx.expect, cx.referent) {
        (None | Some(Expect::Story) | Some(Expect::Topic), Some(r)) if net.node(r).animate => Expect::Age(r),
        (Some(e), _) => e,
        (None, _) => return None,
    };
    match expect {
        Expect::Name | Expect::PetName(_) => {
            // "Joose", "Misu", "it is Misu"
            let k = if w.starts_with(&["it".to_string(), "is".to_string()]) { 2 } else { 0 };
            if (1..=2).contains(&(n - k)) && (k..n).all(plain) {
                let name = o[k..].iter().map(|x| {
                    let mut c = x.chars();
                    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                }).collect::<Vec<_>>().join(" ");
                return Some(vec![match expect {
                    Expect::PetName(noun) => Turn::PetNamed { noun, name },
                    _ => Turn::Name(name),
                }]);
            }
            None
        }
        Expect::Age(noun) | Expect::Describe(noun)
            if n == 1 && net.node(noun).animate && o[0].chars().next().map(|c| c.is_uppercase()).unwrap_or(false) && net.lookup(&w[0]).is_empty() =>
        {
            Some(vec![Turn::PetNamed { noun, name: o[0].clone() }])
        }
        // one feeling word to anything open: "broken"
        Expect::WantTalk | Expect::Story | Expect::Topic if n <= 3 && adj_at(net, w, 0, n).is_some() => {
            adj_at(net, w, 0, n).map(|adj| vec![Turn::Feel(adj)])
        }
        Expect::Feeling => {
            // "good", "pretty good", "not bad", "a bit tired"
            let not = w[0] == "not";
            let k = usize::from(not);
            adj_at(net, w, k, n).map(|adj| vec![if not { Turn::FeelNot(adj) } else { Turn::Feel(adj) }])
        }
        Expect::Describe(noun) => {
            // "very playful and friendly", "she was old"
            let k = if n >= 3 && matches!(w[0].as_str(), "he" | "she" | "it" | "they") && matches!(w[1].as_str(), "is" | "was" | "are" | "were") { 2 } else { 0 };
            let mut out = Vec::new();
            let mut start = k;
            for i in k..=n {
                if i == n || w[i] == "and" {
                    out.push(Turn::MyIs { noun, adj: adj_at(net, w, start, i)? });
                    start = i + 1;
                }
            }
            Some(out)
        }
        Expect::Often(_) => {
            // "every day", "daily", "twice a week", "sometimes"
            const OFTEN: &[&str] = &["every", "each", "day", "daily", "week", "weekly", "month", "monthly", "year", "night", "morning",
                                     "evening", "weekend", "weekends", "sometimes", "often", "always", "never", "rarely", "usually",
                                     "once", "twice", "a", "an", "times", "few", "lot", "all", "the", "time", "most", "days", "nights",
                                     "now", "and", "then", "almost", "pretty", "much", "hour", "hours", "two", "three"];
            w.iter()
                .all(|x| OFTEN.contains(&x.as_str()) || x.chars().all(|c| c.is_ascii_digit()))
                .then(|| vec![Turn::Often(o.join(" ").to_lowercase())])
        }
        Expect::Age(noun) => {
            // "3", "three years", "2 years old", "six months"
            const NUMS: &[&str] = &["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
                                    "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty"];
            let num = w[0].chars().all(|c| c.is_ascii_digit()) || NUMS.contains(&w[0].as_str());
            let rest_ok = w[1..].iter().all(|x| matches!(x.as_str(), "year" | "years" | "month" | "months" | "week" | "weeks" | "old"));
            (num && rest_ok).then(|| {
                let unit = w.get(1).filter(|x| x.as_str() != "old").cloned().unwrap_or_else(|| "years".to_string());
                vec![Turn::Age { noun, age: format!("{} {unit}", w[0]) }]
            })
        }
        _ => None,
    }
}

/// Whether a turn asks something: a question mark, or a question word first.
pub fn is_question(text: &str) -> bool {
    let first = words(text).0.first().cloned().unwrap_or_default();
    text.trim_end().ends_with('?')
        || matches!(first.as_str(), "what" | "who" | "whom" | "where" | "when" | "why" | "how" | "which" | "whose" | "is" | "are" | "do" | "does"
            | "did" | "can" | "could" | "will" | "would" | "should" | "was" | "were")
}

/// The turn: its clauses read one by one, in the light of what Raven asked
/// last, and whether all of them were read.
pub fn read(net: &Network, text: &str, cx: &Context) -> (Vec<Turn>, bool) {
    if let Some(t) = answer(net, text, cx) {
        return (t, true);
    }
    // a statement or a small word on its own ("yes" is also a noun to the question parser)
    if !text.contains([',', ';']) {
        let (w, o) = words(text);
        if !w.iter().any(|x| matches!(x.as_str(), "and" | "but" | "because" | "since" | "though")) {
            if let Some(t) = clause(net, &w, &o, cx) {
                return (vec![t], true);
            }
        }
    }
    // (a turn of several sentences is read sentence by sentence below)
    let several = text.trim_end_matches(['?', '!', '.']).contains(['?', '!', '.']);
    let q = if several { Question::Unknown(String::new()) } else { question::read(net, text) };
    // a bare noun phrase after Raven asked something ("every day") is an
    // answer to it, not a new thing to talk about
    let bare = matches!(q, Question::Ask { intent: question::Intent::About, .. })
        && !text.trim_end().ends_with('?')
        && !words(text).0.first().map(|w| matches!(w.as_str(), "tell" | "what" | "and")).unwrap_or(false);
    if bare && !matches!(cx.expect, None | Some(Expect::Topic)) {
        return (vec![Turn::Heard], true);
    }
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
                // a clause of hedges alone ("fine, I guess") adds nothing
                if HEDGE.iter().any(|h| w.iter().map(|x| x.as_str()).eq(h.iter().copied())) {
                    start = i + 1;
                    continue;
                }
                // a clause on its own may still be a question ("and what do dogs eat")
                // "I like motors, not robots": "not X" turns the clause before round
                if w.first().map(|x| x == "not").unwrap_or(false) && w.len() >= 2 {
                    let x = noun_at(net, w, 1, w.len());
                    let turned = match (out.last(), x) {
                        (Some(Turn::Like(_)), Some(x)) => Some(Turn::Dislike(x)),
                        (Some(Turn::Dislike(_)), Some(x)) => Some(Turn::Like(x)),
                        (Some(Turn::Have(_)), Some(_)) => Some(Turn::Okay),
                        _ => None,
                    };
                    if let Some(t) = turned {
                        out.push(t);
                        start = i + 1;
                        continue;
                    }
                }
                // "what are you? a robot, an AI or a frozen neural net?": the options are part of the question
                if out.iter().any(|t| matches!(t, Turn::Ask(_))) {
                    if let Question::Ask { intent: question::Intent::About, .. } | Question::Unknown(_) = question::read(net, &o.join(" ")) {
                        if !w.is_empty() && !matches!(w[0].as_str(), "i" | "my" | "we") {
                            start = i + 1;
                            continue;
                        }
                    }
                }
                // "she is playful and friendly": a bare adjective goes on with the clause before
                let more = match (out.last(), adj_at(net, w, 0, w.len())) {
                    (Some(Turn::MyIs { noun, .. }), Some(adj)) => Some(Turn::MyIs { noun: *noun, adj }),
                    (Some(Turn::Opinion { noun, .. }), Some(adj)) => Some(Turn::Opinion { noun: *noun, adj }),
                    (Some(Turn::Feel(_)), Some(adj)) => Some(Turn::Feel(adj)),
                    _ => None,
                };
                if let Some(t) = more {
                    out.push(t);
                    start = i + 1;
                    continue;
                }
                match clause(net, w, o, cx) {
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
    // a story Raven asked for is heard, even when it cannot be read
    let personal = words(text).0.first().map(|w| matches!(w.as_str(), "i" | "we" | "my" | "our" | "he" | "she" | "it" | "they" | "his" | "her" | "their")).unwrap_or(false);
    // a question not read is never taken for a story: it is said plainly that it was not read
    if !all && is_question(text) {
        return (out, false);
    }
    if !all && (cx.expect.is_some() || personal || cx.low) {
        out.push(Turn::Heard);
        return (out, true);
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
        let no = Context::default();
        let read = |net: &Network, t: &str| super::read(net, t, &no);
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
