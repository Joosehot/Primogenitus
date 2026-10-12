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
//!   what does NP VERB / what can NP VERB     what it does that to   (what do dogs eat)
//!   what VERBs [NP] / who VERBs [NP]         who does it            (what eats fish)
//!   which NP VERB [NP] / what NP VERB [NP]   which kinds do it      (which animals swim)
//!   which NP have NP / what NP have NP       which kinds have it    (what animals have wings)
//!   what kind/type/sort of NP is NP          the kind on the way    (what kind of animal is a dog)
//!   how ADJ is NP                            how it is, by the same measure (how big is a whale)
//!   who is NP                                as "what is"
//!   who are you / what is your name / how are you    about the bot
//!   can/do/will you VERB [NP] [PREP NP]...   what the bot can do  (can you have a conversation with me)
//!   are you NP / are you ADJ                 what the bot is      (are you a robot)
//!   do you know [about] NP                   as "tell me about"
//!   do you know / can you tell me what ...   the question inside, read whole
//!                                            (what's AI; what AI is)
//!   tell me about yourself / what about you / i meant you      about the bot
//!   and you / how about you                  how the bot is
//!   what do you remember [about me / my NP]  what the bot remembers of the whole conversation
//!   what is my name / who am I               the user's name
//!   what is my NP's name / what was my NP called    their pet's name
//!   where do NP live / where are NP found    where it lives (read from the definitions)
//!   what is NP used for / made of            what it is for, what it is made of
//!   what is the NP of NAME                   the capital of Finland
//!   define NP / what does W mean             the dictionary's definition
//!   where / when / why / how ...             said plainly: no lines for places, times, reasons, ways
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
    /// what does NP VERB: what it does that to
    ObjectOf { topic: Subject, verb: u32 },
    /// what VERBs [NP]: who or what does it
    SubjectOf { verb: u32, obj: Option<u32> },
    /// which NP VERB [NP]
    WhichDo { kind: u32, verb: u32, obj: Option<u32> },
    /// which NP have NP
    WhichHave { kind: u32, part: u32 },
    /// what kind of NP is NP
    KindOf { kind: u32, topic: Subject },
    /// how ADJ is NP
    HowAdj { adj: u32, topic: Subject },
    /// who are you, what is your name
    WhoAreYou,
    /// what do you remember [about my NP]
    Recall(Option<u32>),
    /// what is my name
    MyName,
    /// what is my NP's name
    PetNameOf(u32),
    /// where do NP live
    WhereLive(Subject),
    /// what is NP used for
    UsedFor(Subject),
    /// what is NP made of
    MadeOf(Subject),
    /// what is the NP of NAME ("the capital of Finland")
    PartNamed { kind: u32, of: u32 },
    /// define NP, what does W mean
    Define(Subject),
    /// what are your rules
    Rules,
    /// about itself: the key of its answer in [raven.system.identity]
    Me(&'static str),
    /// forget what I told you
    Forget,
    /// how are you
    HowAreYou,
    /// can you VERB [NP]: the verb and its object
    YouCan(u32, Option<u32>),
    /// are you NP
    YouAre(u32),
    /// are you ADJ
    YouAreAdj(u32),
    /// where, when, why, how ...: the word asked
    Cannot(String),
    IsA { topic: Subject, kind: u32 },
    IsAdj { topic: Subject, adj: u32 },
    Has { topic: Subject, part: u32 },
    Does { topic: Subject, verb: u32, obj: Option<u32> },
    /// not understood, and why
    Unknown(String),
}

fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in text.split(|c: char| c.is_whitespace() || ",.?!;:".contains(c)).filter(|s| !s.is_empty()) {
        let w = w.to_lowercase().replace('\u{2019}', "'");
        // "what's" is "what is"
        match w.strip_suffix("'s") {
            Some(head) if matches!(head, "what" | "who" | "where" | "how" | "it" | "that") => {
                out.push(head.to_string());
                out.push("is".to_string());
            }
            _ => out.push(w),
        }
    }
    out
}

/// A name ("Finland", "New York") from t[i], longest first: (its node, end).
fn names(net: &Network, t: &[String], i: usize) -> Vec<(u32, usize)> {
    let mut out = Vec::new();
    for len in (1..=3).rev() {
        if i + len > t.len() {
            continue;
        }
        let text = t[i..i + len].join(" ");
        if let Some(n) = net.lookup(&text).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Noun && n.form == Form::Base) {
            out.push((n.lemma, i + len));
        }
    }
    out
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
        // a word may read two ways ("wings": the plural of "wing", or the pilot's
        // badge); the reading whose number is sure comes first
        let mut readings: Vec<&crate::network::Node> = net
            .lookup(&text)
            .iter()
            .map(|&q| net.node(q))
            .filter(|n| n.pos == Pos::Noun && !n.proper && matches!(n.form, Form::Base | Form::Plural))
            .collect();
        // and the number the determiner asks for: none, a plural ("people" as
        // persons); "a"/"an", one
        let want = if j == i {
            Some(Form::Plural)
        } else if matches!(w.as_str(), "a" | "an") {
            Some(Form::Base)
        } else {
            None
        };
        readings.sort_by_key(|n| (!(n.sure && net.node(n.lemma).sure), want.map(|f| n.form != f).unwrap_or(false), n.lemma));
        if let Some(n) = readings.first() {
            out.push((Subject::Noun(n.lemma), j + len));
        }
    }
    out
}

/// From i to the end only phrases like "with me", "in english", "about dogs".
fn phrases_to_end(net: &Network, t: &[String], mut i: usize) -> bool {
    while i < t.len() {
        if lexicon::closed(&t[i]) != Some(lexicon::Closed::Prep) {
            return false;
        }
        i += 1;
        match t.get(i).map(|w| w.as_str()) {
            Some("me" | "you" | "us" | "it" | "them" | "english") => i += 1,
            Some(_) => match nps(net, t, i).first() {
                Some(&(_, e)) => i = e,
                None => return false,
            },
            None => return false,
        }
    }
    true
}

fn adjective(net: &Network, w: &str) -> Option<u32> {
    net.lookup(w).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Adj && n.form == Form::Base).map(|n| n.lemma)
}

fn verb(net: &Network, w: &str) -> Option<u32> {
    verb_as(net, w, &[Form::Base])
}

fn verb_as(net: &Network, w: &str, forms: &[Form]) -> Option<u32> {
    if lexicon::closed(w).is_some() {
        return None;
    }
    net.lookup(w).iter().map(|&q| net.node(q)).find(|n| n.pos == Pos::Verb && forms.contains(&n.form)).map(|n| n.lemma)
}

pub fn read(net: &Network, text: &str) -> Question {
    let mut t = words(text);
    // "but let's talk about women", "well, can we talk about it": the lead word changes nothing asked
    while t.len() > 1 && matches!(t[0].as_str(), "but" | "well" | "so" | "ok" | "okay" | "anyway" | "oh" | "now" | "then") {
        t.remove(0);
    }
    let s: Vec<&str> = t.iter().map(|w| w.as_str()).collect();
    use Question::*;
    match s.as_slice() {
        [] => return Unknown("say something".into()),
        ["hello" | "hi" | "hey" | "hiya" | "greetings"] | ["hello" | "hi" | "hey", "there" | "primo"]
        | ["good", "morning" | "afternoon" | "evening"] => return Greeting,
        ["bye" | "goodbye"] | ["see", "you"] | ["see", "you", "later"] | ["good", "night"] => return Goodbye,
        ["thanks" | "thx"] | ["thank", "you"] | ["thanks", "a", "lot"] => return Thanks,
        ["what", "can", "you", "do"] => return Me("can"),
        ["help"] | ["what", "can", "i", "ask"] | ["what", "can", "i", "ask", "you"] => return Help,
        ["what", "else"] | ["anything", "else"] | ["more"] | ["tell", "me", "more"] => return More,
        ["who", "are", "you"] => return WhoAreYou,
        // about itself (its identity): "who made you", "how do you work", "what is raven"
        ["who", "made" | "created" | "built" | "wrote" | "programmed" | "invented" | "designed", "you"]
        | ["who", "is", "your", "creator" | "maker" | "author" | "father" | "mother" | "developer" | "programmer"]
        | ["who", "are", "your", "creators" | "makers" | "parents"] => return Me("made_by"),
        ["how", "old", "are", "you"] | ["when", "were", "you", "made" | "born" | "created" | "built"] | ["when", "is", "your", "birthday"]
        | ["what", "is", "your", "age"] => return Me("born"),
        ["how", "do", "you", "work"] | ["how", "do", "you", "think"] | ["how", "were", "you", "made" | "built"] | ["how", "do", "you", "answer"]
        | ["how", "does", "it", "work"] | ["how", "do", "you", "talk"] => return Me("how"),
        ["what", "kind", "of", "ai" | "thing" | "program" | "bot" | "chatbot", "are", "you"] | ["what", "exactly", "are", "you"] => return Me("what"),
        ["are", "you", "an", "ai"] | ["are", "you", "ai"] | ["are", "you", "artificial", "intelligence"] | ["are", "you", "a", "neural", "network"]
        | ["are", "you", "an", "llm"] | ["are", "you", "a", "language", "model"] | ["are", "you", "trained"] => return Me("ai"),
        ["are", "you", "a", "chatbot" | "bot"] | ["what", "are", "you"] => return Me("what"),
        ["what", "is", "your", "purpose" | "goal" | "job"] | ["why", "do", "you", "exist"] | ["why", "were", "you", "made" | "created" | "built"]
        | ["what", "are", "you", "for"] => return Me("purpose"),
        ["what" | "who", "is", "raven"] | ["what", "does", "raven", "do"] => return Me("raven"),
        ["what", "is", "mblm"] | ["what", "is", "a", "mblm"] | ["what", "is", "your", "language", "model"] => return Me("mblm"),
        ["what", "is", "the", "motor"] | ["what", "is", "your", "motor"] => return Me("motor"),
        ["what" | "who", "is", "primo"] | ["what" | "who", "is", "primogenitus"] | ["what", "is", "your", "name"] | ["what", "is", "your", "full", "name"] => {
            return Me("name")
        }
        ["what", "are", "your", "parts"] | ["what", "parts", "do", "you", "have"] | ["what", "are", "you", "built", "from"] => return Me("parts"),
        ["what", "are", "you", "made", "of" | "from"] | ["what", "is", "inside", "you"] => return Me("made_of"),
        ["where", "do", "you", "live"] | ["where", "are", "you"] | ["where", "are", "you", "from"] | ["where", "do", "you", "come", "from"]
        | ["where", "is", "your", "code"] => return Me("home"),
        ["what", "can", "you", "not", "do"] | ["what", "can", "not", "you", "do"] | ["what", "are", "your", "limits"] | ["what", "can't", "you", "do"] => {
            return Me("cannot")
        }
        ["do", "you", "have", "feelings" | "emotions" | "a", "heart"] | ["can", "you", "feel"] | ["do", "you", "feel", "anything"] => return Me("feelings"),
        ["what", "do", "you", "like"] | ["what", "is", "your", "favorite", ..] | ["what", "is", "your", "favourite", ..] => return Me("likes"),
        ["do", "you", "remember", "things"] | ["how", "much", "do", "you", "remember"] | ["what", "do", "you", "remember", "about", "yourself"]
        | ["how", "does", "your", "memory", "work"] => return Me("memory"),
        ["what", "version", "are", "you"] | ["which", "version", "are", "you"] => return Me("version"),
        ["what", "are", "your", "rules"] | ["what", "is", "your", "system", "prompt"] | ["show", "me", "your", "rules"] | ["your", "rules"]
        | ["what", "rules", "do", "you", "follow"] => return Rules,
        ["forget", "me"] | ["forget", "everything"] | ["forget", "what", "i", "told", "you"] | ["please", "forget", "everything"]
        | ["please", "forget", "what", "i", "told", "you"] | ["forget", "all", "of", "it"] | ["delete", "my", "data"] => return Forget,
        ["how", "are", "you"] | ["how", "are", "you", "doing"] | ["how", "is", "it", "going"] | ["and", "you"]
        | ["how", "about", "you"] | ["what", "about", "yourself"] => return HowAreYou,
        ["what", "about", "you"] | ["about", "you"] | ["you"] | ["i", "meant" | "mean", "you"] | ["i", "was", "asking", "about", "you"]
        | ["describe", "yourself"] | ["introduce", "yourself"] => return WhoAreYou,
        _ => {}
    }
    // what Raven remembers: "what do you remember (about me)", "what is my name", "what is my dog's name"
    match s.as_slice() {
        ["what", "do", "you", "remember"] | ["what", "do", "you", "remember", "about", "me"] | ["what", "do", "you", "know", "about", "me"]
        | ["what", "did", "i", "tell", "you"] | ["do", "you", "remember", "me"] | ["what", "have", "i", "told", "you"]
        | ["do", "you", "remember", "what", "i", "told", "you"] | ["what", "do", "you", "remember", "of", "me"] => return Recall(None),
        ["what", "is", "my", "name"] | ["who", "am", "i"] | ["do", "you", "remember", "my", "name"] | ["do", "you", "know", "my", "name"] => {
            return MyName
        }
        _ => {}
    }
    {
        // "what is my dog's name", "what is the name of my dog", "do you remember my dog's name"
        let n = t.len();
        let poss = |i: usize| -> Option<u32> {
            let w = t.get(i)?.strip_suffix("'s")?;
            nps(net, &[w.to_string()], 0).into_iter().find_map(|(s, _)| match s {
                Subject::Noun(x) => Some(x),
                _ => None,
            })
        };
        // "what is my dog called", "what was my dog called"
        if (s.starts_with(&["what", "is", "my"]) || s.starts_with(&["what", "was", "my"])) && n >= 5 && s[n - 1] == "called" {
            if let Some(Subject::Noun(x)) = nps(net, &t, 3).into_iter().find(|&(_, e)| e == n - 1).map(|(s, _)| s) {
                return PetNameOf(x);
            }
        }
        if (s.starts_with(&["what", "is", "my"]) || s.starts_with(&["what", "was", "my"]) || s.starts_with(&["do", "you", "remember", "my"]) || s.starts_with(&["do", "you", "know", "my"]))
            && n >= 5
            && s[n - 1] == "name"
        {
            if let Some(x) = poss(n - 2) {
                return PetNameOf(x);
            }
        }
        if s.starts_with(&["what", "is", "the", "name", "of", "my"]) {
            if let Some(Subject::Noun(x)) = nps(net, &t, 6).into_iter().find(|&(_, e)| e == n).map(|(s, _)| s) {
                return PetNameOf(x);
            }
        }
        // "what do you remember about my dog", "what did i tell you about my cat", "do you remember my dog"
        for p in [&["what", "do", "you", "remember", "about", "my"][..], &["what", "did", "i", "tell", "you", "about", "my"][..],
                  &["what", "do", "you", "know", "about", "my"][..], &["do", "you", "remember", "my"][..]] {
            if s.starts_with(p) {
                if let Some(Subject::Noun(x)) = nps(net, &t, p.len()).into_iter().find(|&(_, e)| e == n).map(|(s, _)| s) {
                    return Recall(Some(x));
                }
            }
        }
    }
    // "can you tell me something about yourself": only polite words around "yourself"
    if s.last() == Some(&"yourself")
        && s.iter().all(|w| matches!(*w, "can" | "could" | "would" | "will" | "you" | "tell" | "me" | "something" | "more" | "a"
            | "little" | "bit" | "about" | "yourself" | "please" | "describe" | "introduce" | "talk"))
    {
        return WhoAreYou;
    }
    // "do you know what's AI", "can you tell me what a dog is": the question inside
    let n = t.len();
    let inner = [&["do", "you", "know"][..], &["can", "you", "tell", "me"][..], &["could", "you", "tell", "me"][..],
                 &["tell", "me"][..], &["i", "wonder"][..], &["i", "want", "to", "know"][..]];
    for p in inner {
        if s.starts_with(p) && s.len() > p.len() + 1 && matches!(s[p.len()], "what" | "who" | "which" | "how" | "where" | "why" | "when") {
            let rest = t[p.len()..].join(" ");
            let q = read(net, &rest);
            if !matches!(q, Unknown(_)) {
                return q;
            }
            // the inside order: "what a dog is", "what dogs are like", "what dogs do", "what a dog has"
            let k = p.len() + 1;
            if s[p.len()] == "what" {
                match s.last() {
                    Some(&"is") | Some(&"are") => {
                        if let Some(topic) = nps(net, &t, k).into_iter().find(|&(_, e)| e == n - 1).map(|(s, _)| s) {
                            return Ask { topic, intent: Intent::WhatIs, more: false };
                        }
                    }
                    Some(&"like") if n >= k + 3 && matches!(s[n - 2], "is" | "are") => {
                        if let Some(topic) = nps(net, &t, k).into_iter().find(|&(_, e)| e == n - 2).map(|(s, _)| s) {
                            return Ask { topic, intent: Intent::WhatLike, more: false };
                        }
                    }
                    Some(&"do") | Some(&"does") => {
                        if let Some(topic) = nps(net, &t, k).into_iter().find(|&(_, e)| e == n - 1).map(|(s, _)| s) {
                            return Ask { topic, intent: Intent::WhatDoes, more: false };
                        }
                    }
                    Some(&"have") | Some(&"has") => {
                        if let Some(topic) = nps(net, &t, k).into_iter().find(|&(_, e)| e == n - 1).map(|(s, _)| s) {
                            return Ask { topic, intent: Intent::WhatHas, more: false };
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let n = t.len();
    // the noun phrase from i that ends exactly at `end`
    let whole = |i: usize, end: usize| nps(net, &t, i).into_iter().find(|&(_, e)| e == end).map(|(s, _)| s);
    // "let's talk", "let's chat", "can we talk": gladly, about what?
    if matches!(s.as_slice(), ["let's" | "lets", "talk" | "chat"] | ["let", "us", "talk" | "chat"] | ["can" | "could" | "shall", "we", "talk" | "chat"]
        | ["let's" | "lets", "talk" | "chat", "a", "bit"] | ["i", "want", "to", "talk" | "chat"] | ["i", "just", "want", "to", "talk"]
        | ["let's" | "lets", "have", "a", "chat" | "talk" | "conversation"] | ["let's" | "lets", "talk", "then"])
    {
        return Me("talk");
    }
    // a topic offered: "let's talk about you", "can we talk about dogs", "I want to know more about you"
    for p in [&["let's", "talk", "about"][..], &["lets", "talk", "about"][..], &["let", "us", "talk", "about"][..],
              &["can", "we", "talk", "about"][..], &["could", "we", "talk", "about"][..], &["shall", "we", "talk", "about"][..],
              &["i", "want", "to", "talk", "about"][..], &["i'd", "like", "to", "talk", "about"][..], &["talk", "about"][..],
              &["i", "would", "like", "to", "talk", "about"][..], &["i", "would", "love", "to", "talk", "about"][..],
              &["i", "wanna", "talk", "about"][..], &["we", "could", "talk", "about"][..], &["how", "about", "we", "talk", "about"][..],
              &["i", "would", "rather", "talk", "about"][..], &["i", "want", "to", "talk", "more", "about"][..],
              &["i", "want", "to", "know", "about"][..], &["i", "want", "to", "know", "more", "about"][..],
              &["tell", "me", "more", "about"][..], &["i", "would", "like", "to", "know", "more", "about"][..]] {
        if s.starts_with(p) {
            let rest = &s[p.len()..];
            if matches!(rest, ["you"] | ["yourself"] | ["you", "primo"] | ["you", "then"] | ["you", "instead"] | ["primo"] | ["primogenitus"]) {
                return Me("about");
            }
            // "let's talk about something (else)": gladly, about what?
            if matches!(rest, ["something"] | ["something", "else"] | ["anything"] | ["anything", "else"] | ["it"] | ["that"] | ["this"]) {
                return Me("talk");
            }
            // a topic offered starts with what it is
            if let Some(topic) = whole(p.len(), n) {
                return Ask { topic, intent: Intent::WhatIs, more: false };
            }
        }
    }
    // "tell me about NP", "what about NP", "and NP", "NP"
    for (prefix, intent) in [(&["tell", "me", "about"][..], Intent::About), (&["what", "about"][..], Intent::About),
                             (&["and"][..], Intent::About), (&["describe"][..], Intent::WhatLike), (&[][..], Intent::About)] {
        if s.starts_with(prefix) {
            if let Some(topic) = whole(prefix.len(), n) {
                return Ask { topic, intent, more: false };
            }
        }
    }
    // about the bot: "can you talk", "can you have a conversation with me", "do you learn"
    if matches!(s.first(), Some(&"can") | Some(&"could") | Some(&"do") | Some(&"will") | Some(&"would"))
        && s.get(1) == Some(&"you")
        && n >= 3
    {
        if s[2] == "know" {
            let k = if s.get(3) == Some(&"about") { 4 } else { 3 };
            if let Some(topic) = whole(k, n) {
                return Ask { topic, intent: Intent::About, more: false };
            }
        }
        // "can you tell me about cats": a request to be told
        if matches!(s[2], "tell" | "describe" | "explain") {
            if let Some(k) = s.iter().position(|w| *w == "about") {
                if let Some(topic) = whole(k + 1, n) {
                    return Ask { topic, intent: Intent::About, more: false };
                }
            }
        }
        // right after "can you" a verb comes, even one the closed words also
        // have ("have", "like", "do")
        let v = net.base(&t[2], Pos::Verb).filter(|_| lexicon::closed(&t[2]) != Some(lexicon::Closed::Pron));
        if let Some(v) = v {
            // the object, if any ("me" and "something" count as none), then
            // phrases of a preposition and what it governs, to the end
            let mut objects: Vec<(Option<u32>, usize)> = vec![(None, 3)];
            if matches!(s.get(3), Some(&"me") | Some(&"something") | Some(&"anything") | Some(&"things") | Some(&"well") | Some(&"english")) {
                objects.push((None, 4));
            }
            for (o, e) in nps(net, &t, 3) {
                if let Subject::Noun(o) = o {
                    objects.push((Some(o), e));
                }
            }
            for (obj, e) in objects {
                if phrases_to_end(net, &t, e) {
                    return YouCan(v, obj);
                }
            }
        }
    }
    // "are you a robot", "are you alive"
    if s.first() == Some(&"are") && s.get(1) == Some(&"you") && n >= 3 {
        if n == 3 {
            if let Some(a) = adjective(net, &t[2]) {
                return YouAreAdj(a);
            }
        }
        if let Some(Subject::Noun(k)) = whole(2, n) {
            return YouAre(k);
        }
    }
    // "who is NP": what it is
    if s.first() == Some(&"who") && matches!(s.get(1), Some(&"is") | Some(&"are")) {
        if let Some(topic) = whole(2, n) {
            return Ask { topic, intent: Intent::WhatIs, more: false };
        }
    }
    // "how ADJ is NP"
    if s.first() == Some(&"how") && n >= 4 && matches!(s[2], "is" | "are") {
        if let (Some(adj), Some(topic)) = (adjective(net, &t[1]), whole(3, n)) {
            return HowAdj { adj, topic };
        }
    }
    // "what kind/type/sort of NP is NP"
    if s.first() == Some(&"what") && matches!(s.get(1), Some(&"kind") | Some(&"type") | Some(&"sort")) && s.get(2) == Some(&"of") {
        for (kind, e) in nps(net, &t, 3) {
            if let (Subject::Noun(kind), Some(&"is") | Some(&"are")) = (kind, s.get(e)) {
                if let Some(topic) = whole(e + 1, n) {
                    return KindOf { kind, topic };
                }
            }
        }
    }
    // "what/who VERBs [NP]": who or what does it
    if matches!(s.first(), Some(&"what") | Some(&"who")) && n >= 2 {
        if let Some(v) = verb_as(net, &t[1], &[Form::ThirdSg, Form::Past]) {
            if n == 2 {
                return SubjectOf { verb: v, obj: None };
            }
            if let Some(Subject::Noun(o)) = whole(2, n) {
                return SubjectOf { verb: v, obj: Some(o) };
            }
        }
    }
    // "which/what NP VERB [NP]", "which/what NP have NP"
    if matches!(s.first(), Some(&"which") | Some(&"what")) {
        for (kind, e) in nps(net, &t, 1) {
            let Subject::Noun(kind) = kind else { continue };
            if matches!(s.get(e), Some(&"have") | Some(&"has")) {
                if let Some(Subject::Noun(part)) = whole(e + 1, n) {
                    return WhichHave { kind, part };
                }
            }
            if let Some(v) = t.get(e).and_then(|w| verb_as(net, w, &[Form::Base, Form::ThirdSg, Form::Past])) {
                if e + 1 == n {
                    return WhichDo { kind, verb: v, obj: None };
                }
                if let Some(Subject::Noun(o)) = whole(e + 1, n) {
                    return WhichDo { kind, verb: v, obj: Some(o) };
                }
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
                // "what do dogs eat": what it does that to ("what does love mean": its definition)
                if let Some(v) = s.last().and_then(|w| verb(net, w)) {
                    if let Some(topic) = whole(k + 1, n - 1) {
                        if s.last() == Some(&"mean") {
                            return Define(topic);
                        }
                        return ObjectOf { topic, verb: v };
                    }
                }
            }
            _ => {}
        }
    }
    // "is/are NP NP", "is/are NP ADJ"
    if matches!(s.first(), Some(&"is") | Some(&"are")) {
        for (topic, e) in nps(net, &t, 1) {
            if e + 1 == n {
                if let Some(adj) = adjective(net, &t[e]) {
                    return IsAdj { topic, adj };
                }
            }
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
    // where it lives: "where do fish live", "where are pandas found", "where can owls be found"
    if s.first() == Some(&"where") && n >= 3 {
        for (topic, e) in nps(net, &t, 2) {
            let rest = &s[e..];
            let aux_ok = matches!(s[1], "do" | "does") && matches!(rest, ["live"] | ["come", "from"] | ["grow"] | ["dwell"])
                || matches!(s[1], "are" | "is") && matches!(rest, ["found"] | ["from"])
                || s[1] == "can" && matches!(rest, ["be", "found"]);
            if aux_ok {
                return WhereLive(topic);
            }
        }
    }
    // what it is used for / made of: "what is a hammer used for", "what is bread made of"
    if s.first() == Some(&"what") && matches!(s.get(1), Some(&"is") | Some(&"are")) && n >= 5 {
        if s[n - 2..] == ["used", "for"] {
            if let Some(topic) = whole(2, n - 2) {
                return UsedFor(topic);
            }
        }
        if s[n - 2] == "made" && matches!(s[n - 1], "of" | "from") {
            if let Some(topic) = whole(2, n - 2) {
                return MadeOf(topic);
            }
        }
    }
    if s.starts_with(&["what", "do", "people", "use"]) || s.starts_with(&["what", "do", "you", "use"]) {
        if s.last() == Some(&"for") {
            if let Some(topic) = whole(4, n - 1) {
                return UsedFor(topic);
            }
        }
    }
    // "what is the capital of finland": a part of a named place (or thing) that is that kind
    if matches!(s.first(), Some(&"what") | Some(&"which")) && s.get(1) == Some(&"is") && s.get(2) == Some(&"the") {
        if let Some(k) = s.iter().position(|w| *w == "of") {
            let kind = nps(net, &t, 3).into_iter().find(|&(_, e)| e == k).and_then(|(s, _)| match s {
                Subject::Noun(x) => Some(x),
                _ => None,
            });
            let of = names(net, &t, k + 1).into_iter().find(|&(_, e)| e == n).map(|(x, _)| x);
            if let (Some(kind), Some(of)) = (kind, of) {
                if !matches!(s[3], "meaning" | "definition") {
                    return PartNamed { kind, of };
                }
            }
        }
    }
    // the dictionary's definition: "define love", "what does love mean", "what is the meaning of life"
    match s.as_slice() {
        ["define", ..] => {
            if let Some(topic) = whole(1, n) {
                return Define(topic);
            }
        }
        ["what", "does" | "do", .., "mean"] => {
            if let Some(topic) = whole(2, n - 1) {
                return Define(topic);
            }
        }
        ["what", "is", "the", "meaning" | "definition", "of", ..] | ["what", "is", "a", "definition", "of", ..] => {
            if let Some(topic) = whole(5, n) {
                return Define(topic);
            }
        }
        ["meaning", "of", ..] => {
            if let Some(topic) = whole(2, n) {
                return Define(topic);
            }
        }
        _ => {}
    }
    // where, when, why, how: no lines for these yet
    if let Some(&w) = s.first() {
        if matches!(w, "where" | "when" | "why" | "how") {
            return Cannot(w.to_string());
        }
    }
    Unknown(why_not(net, &t))
}

/// Why a sentence was not understood: an unknown word, or a shape not read.
fn why_not(net: &Network, t: &[String]) -> String {
    for w in t {
        if lexicon::closed(w).is_none()
            && net.lookup(w).is_empty()
            && !matches!(w.as_str(), "what" | "else" | "tell" | "describe" | "which" | "how" | "why" | "your" | "you")
        {
            return format!("I do not know the word \"{w}\" (did you mean \"{}\"?).", parser::closest(net, w));
        }
    }
    "I could not read the whole sentence. I read questions like: what is a dog, what do dogs eat, where do frogs live, what is a knife used for, what is bread made of, what is the capital of France, define love, which animals swim, is a whale a mammal, does a dog have a tail.".into()
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
