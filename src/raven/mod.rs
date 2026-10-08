//! Raven: Primo's conversation, made as humane as it can be. Primo knows what
//! is in the world; Raven decides how to say it to a person.
//!
//! Every user turn is read whole (turn.rs). Raven remembers the person (their
//! name, how they feel, what they like and have, what happened to them). It
//! then makes every reply it can for the turn, each a few parts: an opener
//! (sympathy, joy, a greeting, taking up what was said), a body (Primo's
//! answer, a fact from the network) and a question back, and searches them:
//! the humane judges (judges.rs) score each whole reply, and the best one is
//! said. This is the old Raven's search (moves scored, MAX picks), with the
//! judges as its evaluator instead of a trained model: nothing is trained.
//!
//! The words Raven says around Primo's facts are in rules.toml ([raven.say]),
//! the judges' weights in [raven.judges], the feeling anchors in [raven.feeling].

pub mod feeling;
pub mod judges;
pub mod turn;

use crate::chat::{kind_path, Chat, Reply};
use crate::config::Config;
use crate::lexicon;
use crate::network::{Form, Network, Pos};
use crate::question::Question;
use anyhow::{bail, Result};
use std::collections::HashMap;
use turn::{Expect, Turn};

/// What Raven knows about the person it talks with.
#[derive(Default, Debug)]
pub struct Person {
    pub name: Option<String>,
    pub likes: Vec<u32>,
    pub dislikes: Vec<u32>,
    pub has: Vec<u32>,
    /// the last feeling they told, and how it feels
    pub mood: Option<(u32, f32)>,
    /// what they lost (a pet that died): never asked about as if it were there
    pub lost: Vec<u32>,
    /// pets' names: (the pet, its name, the pronoun for it: him, her, it)
    pub pets: Vec<(u32, String, String)>,
    /// the things whose age they told
    pub ages: Vec<u32>,
    /// the things they described
    pub described: Vec<u32>,
    /// they said they made Raven
    pub creator: bool,
}

impl TurnCtx {
    /// A feeling in the turn: pain, once there, is not outweighed by joy.
    fn feel(&mut self, v: f32) {
        self.valence = if v < 0.0 || self.valence < 0.0 { self.valence.min(v) } else { self.valence.max(v) };
    }
}

/// What Raven has said.
#[derive(Default)]
pub struct History {
    pub said: Vec<String>,
    pub first: Vec<String>,
    pub asked: Vec<bool>,
    pub name_used: Vec<bool>,
}

/// One part of a reply, with what the judges look at.
#[derive(Clone, Debug, Default)]
pub struct Part {
    pub text: String,
    /// sympathy (<0) or shared joy (>0)
    pub feel: f32,
    pub answer: bool,
    pub fact: bool,
    pub question: bool,
    pub memory: bool,
    pub name: bool,
    /// a reply that would fit anything
    pub generic: bool,
    /// greets a name just told ("Nice to meet you, Joose")
    pub meets: bool,
    /// what a question part waits for
    pub expect: Option<Expect>,
    pub mentions: Vec<u32>,
    /// the lines it stands on
    pub why: Vec<String>,
}

/// What the judges know about the turn.
#[derive(Default)]
pub struct TurnCtx {
    /// the strongest feeling in the turn: <0 pain, >0 joy
    pub valence: f32,
    pub asked: bool,
    pub closing: bool,
    /// the user told their name in this turn
    pub new_name: bool,
    pub nouns: Vec<u32>,
}

const SAY_KEYS: &[&str] = &[
    "greeting", "goodbye", "thanks", "nice_to_meet", "empathy_bad", "empathy_good", "feel_neutral", "loss", "bad_news",
    "good_news", "neutral_news", "like", "like_doing", "dislike", "have", "role", "opinion", "kind_yes", "laugh", "okay",
    "sorry", "dont_know", "yes", "no", "not_followed", "not_known", "know_little", "q_bad", "q_good", "q_loss",
    "q_have_animal", "q_have", "q_like", "q_like_doing", "q_dislike", "q_role", "q_name", "q_topic", "q_open",
    "q_memory", "q_how_are_you", "not_sure", "pet_name", "pet_name_lost", "q_pet", "q_pet_lost", "q_ask_name", "age",
    "heard", "listening", "no_talk", "q_tell", "q_good_plain", "nice_trait", "heard_bad", "nothing", "creator",
    "compliment", "insult", "about_me", "empathy_mild", "q_mild",
];

pub struct Raven<'a> {
    net: &'a Network,
    cfg: &'a Config,
    chat: Chat<'a>,
    pub person: Person,
    hist: History,
    turn: u64,
    /// what Raven's last question waits for
    expect: Option<Expect>,
    /// the user's thing talked of last ("she", "it")
    referent: Option<u32>,
}

/// The words for a template's slots: {name}, {noun}, {a_noun}, {nouns},
/// {adj}, {verb}, {verbing}, {that}, {what}. A template whose slot has no
/// word (a mass noun has no "a") is not used.
#[derive(Default, Clone)]
struct Slots(HashMap<&'static str, String>, Option<u32>);

impl Slots {
    fn set(mut self, k: &'static str, v: impl Into<String>) -> Slots {
        self.0.insert(k, v.into());
        self
    }

    /// The thing the slots are about (what a question's answer will be about).
    fn about(mut self, n: u32) -> Slots {
        self.1 = Some(n);
        self
    }

    fn fill(&self, t: &str) -> Option<String> {
        let mut out = String::new();
        let mut rest = t;
        while let Some(a) = rest.find('{') {
            out.push_str(&rest[..a]);
            let b = rest[a..].find('}')? + a;
            out.push_str(self.0.get(&rest[a + 1..b])?);
            rest = &rest[b + 1..];
        }
        out.push_str(rest);
        let mut c = out.chars();
        Some(match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => out,
        })
    }
}

impl<'a> Raven<'a> {
    pub fn new(net: &'a Network, cfg: &'a Config) -> Result<Raven<'a>> {
        for k in SAY_KEYS {
            if !cfg.raven.say.contains_key(*k) {
                bail!("rules.toml: [raven.say] has no \"{k}\"");
            }
        }
        if let Some(k) = cfg.raven.say.keys().find(|k| !SAY_KEYS.contains(&k.as_str())) {
            bail!("rules.toml: [raven.say] has an unknown key \"{k}\"");
        }
        Ok(Raven {
            net,
            cfg,
            chat: Chat::new(net, cfg),
            person: Person::default(),
            hist: History::default(),
            turn: 0,
            expect: None,
            referent: None,
        })
    }

    /// The first words: a greeting and the question for a name (whose answer
    /// is then read as a name).
    pub fn greeting(&mut self) -> String {
        self.expect = Some(Expect::Name);
        format!("{} {}", self.cfg.raven.say["greeting"][0], self.cfg.raven.say["q_ask_name"][0])
    }

    fn say(&self, key: &str) -> &[String] {
        &self.cfg.raven.say[key]
    }

    /// A noun's slots: {noun}, {a_noun} (none for a mass noun), {nouns}.
    fn noun_slots(&self, n: u32, s: Slots) -> Slots {
        let node = self.net.node(n);
        let s = s.set("noun", node.text.clone()).about(n);
        if node.mass {
            return s.set("nouns", node.text.clone());
        }
        let s = s.set("a_noun", format!("{} {}", lexicon::article(&node.text, &self.cfg.article), node.text));
        match self.net.form(n, Form::Plural) {
            Some(p) if p.sure => s.set("nouns", p.text.clone()),
            _ => s,
        }
    }

    fn plural(&self, n: u32) -> String {
        match self.net.form(n, Form::Plural) {
            Some(p) if p.sure && !self.net.node(n).mass => p.text.clone(),
            _ => self.net.node(n).text.clone(),
        }
    }

    fn verb_slots(&self, v: u32, s: Slots) -> Slots {
        let s = s.set("verb", self.net.node(v).text.clone()).set("noun", self.net.node(v).text.clone());
        match self.net.form(v, Form::Ing) {
            Some(g) => s.set("verbing", g.text.clone()).set("nouns", g.text.clone()),
            None => s,
        }
    }

    fn base(&self) -> Slots {
        match &self.person.name {
            Some(n) => Slots::default().set("name", n.clone()),
            None => Slots::default(),
        }
    }

    /// Parts from the templates under `key`, with what they carry.
    fn parts(&self, key: &str, slots: &Slots, make: impl Fn(&mut Part)) -> Vec<Part> {
        self.say(key)
            .iter()
            .filter_map(|t| {
                let text = slots.fill(t)?;
                let mut p = Part { text, ..Default::default() };
                p.name = t.contains("{name}");
                p.expect = expect_for(key, &p.text, slots.1);
                // a pet called by its name is remembered
                p.memory = t.contains("{pet}");
                make(&mut p);
                // it takes up only what its words name ("I am sorry to hear that" names no cat)
                let low = p.text.to_lowercase();
                p.mentions.retain(|&m| {
                    let w = &self.net.node(m).text;
                    low.contains(w.as_str()) || self.net.form(m, Form::Plural).map(|f| low.contains(f.text.as_str())).unwrap_or(false)
                });
                // never ask what is known already (its name, its age)
                let known = match p.expect {
                    Some(Expect::PetName(n)) => self.person.pets.iter().any(|x| x.0 == n),
                    Some(Expect::Age(n)) => self.person.ages.contains(&n),
                    Some(Expect::Describe(n)) => self.person.described.contains(&n),
                    Some(Expect::Name) => self.person.name.is_some(),
                    Some(Expect::HaveIt(n)) => self.person.has.contains(&n),
                    _ => false,
                };
                (!known).then_some(p)
            })
            .collect()
    }

    fn valence(&self, w: u32) -> Option<f32> {
        feeling::valence(self.net, &self.cfg.raven.feeling, w)
    }

    /// The user's thing this noun touches: the same, or a kind of it either
    /// way ("what do dogs eat" after "I have a puppy").
    fn theirs(&self, n: u32) -> Option<u32> {
        self.person
            .has
            .iter()
            .chain(self.person.likes.iter())
            .copied()
            .filter(|h| !self.person.lost.contains(h))
            .find(|&h| h == n || kind_path(self.net, h, n).is_some() || kind_path(self.net, n, h).is_some())
    }

    /// How a turn's words feel, when the turn itself cannot be read: the
    /// strongest bad word wins over good ones ("the helpless search").
    fn tone(&self, text: &str) -> f32 {
        let mut tone = 0.0f32;
        for w in text.split(|c: char| !c.is_alphabetic()).filter(|w| w.len() > 2) {
            let w = w.to_lowercase();
            for &q in self.net.lookup(&w) {
                let n = self.net.node(q);
                if n.form != Form::Base || !matches!(n.pos, Pos::Adj | Pos::Noun) || n.proper {
                    continue;
                }
                if let Some(v) = self.valence(n.lemma) {
                    tone = if v < 0.0 || tone < 0.0 { tone.min(v) } else { tone.max(v) };
                }
                break;
            }
        }
        tone
    }

    /// Primo's words for an answer, said a little more warmly.
    fn warm(&self, text: &str) -> String {
        if let Some(that) = text.strip_prefix("Not that I know: no line in my network says that ") {
            if let Some(t) = Slots::default().set("that", that.trim_end_matches('.')).fill(&self.say("not_known")[0]) {
                return t;
            }
        }
        if let Some(rest) = text.strip_prefix("I know of nothing that ") {
            if let Some((that, _)) = rest.split_once(':') {
                if let Some(t) = Slots::default().set("that", that).fill(&self.say("not_sure")[0]) {
                    return t;
                }
            }
        }
        if let Some(rest) = text.strip_prefix("I know of nothing about ") {
            if let Some((what, _)) = rest.split_once(':') {
                if let Some(t) = Slots::default().set("what", what).fill(&self.say("know_little")[0]) {
                    return t;
                }
            }
        }
        text.to_string()
    }

    /// A fact about a noun from Primo, as a body part.
    fn fact(&mut self, n: u32) -> Vec<Part> {
        let r = self.chat.answer_question(Question::Ask {
            topic: crate::question::Subject::Noun(n),
            intent: crate::question::Intent::About,
            more: false,
        });
        if r.why.is_empty() {
            return Vec::new();
        }
        vec![Part { text: r.text, fact: true, mentions: vec![n], why: r.why, ..Default::default() }]
    }

    pub fn reply(&mut self, text: &str) -> Reply {
        self.turn += 1;
        let cx = turn::Context { expect: self.expect, referent: self.referent };
        let (turns, all) = turn::read(self.net, text, &cx);
        let asked_for = self.expect.take();
        let mut ctx = TurnCtx::default();
        let (mut openers, mut bodies, mut questions): (Vec<Part>, Vec<Part>, Vec<Part>) = (Vec::new(), Vec::new(), Vec::new());
        let q = |p: &mut Part| p.question = true;

        for t in &turns {
            let b = self.base();
            match t.clone() {
                Turn::Ask(Question::Greeting) => {
                    openers.extend(self.parts("greeting", &b, |_| {}));
                    if self.person.name.is_none() {
                        questions.extend(self.parts("q_ask_name", &b, q));
                    } else {
                        questions.extend(self.parts("q_how_are_you", &b, q));
                    }
                }
                Turn::Ask(Question::Goodbye) => {
                    ctx.closing = true;
                    openers.extend(self.parts("goodbye", &b, |_| {}));
                }
                Turn::Ask(Question::Thanks) => {
                    ctx.closing = true;
                    openers.extend(self.parts("thanks", &b, |_| {}));
                }
                Turn::Ask(q0) => {
                    ctx.asked = true;
                    let r = self.chat.answer_question(q0);
                    let topic = self.chat.topic();
                    let mut part = Part { text: self.warm(&r.text), answer: true, why: r.why.clone(), ..Default::default() };
                    if let Some(tp) = topic {
                        part.mentions.push(tp);
                        ctx.nouns.push(tp);
                    }
                    bodies.push(part);
                    if let (Some(tp), true) = (topic, !r.why.is_empty()) {
                        let s = self.noun_slots(tp, b.clone());
                        match self.theirs(tp) {
                            Some(h) => {
                                let s2 = self.noun_slots(h, b.clone());
                                questions.extend(self.parts("q_memory", &s2, |p| {
                                    p.question = true;
                                    p.memory = true;
                                    p.mentions.push(tp);
                                }));
                            }
                            None => questions.extend(self.parts("q_topic", &s, |p| {
                                p.question = true;
                                p.mentions.push(tp);
                            })),
                        }
                    }
                }
                Turn::Name(name) => {
                    self.person.name = Some(name.clone());
                    ctx.new_name = true;
                    let s = Slots::default().set("name", name);
                    openers.extend(self.parts("nice_to_meet", &s, |p| {
                        p.name = false;
                        p.meets = true;
                    }));
                    questions.extend(self.parts("q_name", &s, q));
                }
                Turn::FeelNot(a) => {
                    // "not bad": the feeling turned round, said without the word
                    let v = -self.valence(a).unwrap_or(0.0);
                    ctx.feel(v);
                    let (open, ask) = if v < 0.0 { ("empathy_bad", "q_bad") } else { ("empathy_good", "q_good_plain") };
                    openers.extend(self.parts(open, &b, |p| p.feel = v));
                    questions.extend(self.parts(ask, &b, q));
                }
                Turn::Age { noun, age } => {
                    self.referent = Some(noun);
                    self.person.ages.push(noun);
                    ctx.nouns.push(noun);
                    let pet = self.person.pets.iter().rev().find(|p| p.0 == noun).map(|p| p.1.clone());
                    let mut s = self.noun_slots(noun, b.clone()).set("age", age);
                    if let Some(pet) = pet {
                        s = s.set("pet", pet);
                    }
                    openers.extend(self.parts("age", &s, |p| {
                        p.memory = true;
                        p.mentions.push(noun);
                    }));
                    questions.extend(self.parts(if self.net.node(noun).animate { "q_pet" } else { "q_have" }, &s, q));
                }
                Turn::PetNamed { noun, name } => {
                    self.referent = Some(noun);
                    self.person.pets.push((noun, name.clone(), String::new()));
                    ctx.nouns.push(noun);
                    let s = self.noun_slots(noun, b.clone()).set("pet", name);
                    let gone = self.person.lost.contains(&noun);
                    if gone {
                        ctx.feel(-1.0);
                    }
                    let feel = if gone { -1.0 } else { 0.0 };
                    openers.extend(self.parts(if gone { "pet_name_lost" } else { "pet_name" }, &s, |p| {
                        p.feel = feel;
                        p.memory = true;
                        p.mentions.push(noun);
                    }));
                    questions.extend(self.parts(if gone { "q_pet_lost" } else { "q_pet" }, &s, q));
                }
                Turn::Heard => {
                    let s = match self.referent {
                        Some(r) => self.noun_slots(r, b.clone()),
                        None => b.clone(),
                    };
                    // what it is about cannot be read, but how it feels can be: its words
                    let tone = self.tone(text);
                    ctx.feel(tone);
                    if tone < 0.0 {
                        openers.extend(self.parts("heard_bad", &s, |p| p.feel = -1.0));
                        questions.extend(self.parts("q_bad", &s, q));
                    } else {
                        openers.extend(self.parts("heard", &s, |p| p.feel = if ctx.valence < 0.0 { -1.0 } else { tone }));
                    }
                }
                Turn::Nothing => {
                    openers.extend(self.parts("nothing", &b, |_| {}));
                    questions.extend(self.parts("q_open", &b, q));
                }
                Turn::Creator => {
                    self.person.creator = true;
                    openers.extend(self.parts("creator", &b, |p| p.meets = true));
                    ctx.new_name = true;
                }
                Turn::AboutMe(adj) => {
                    let v = adj.and_then(|a| self.valence(a)).unwrap_or(0.0);
                    let key = if v > 0.0 { "compliment" } else if v < 0.0 { "insult" } else { "about_me" };
                    let s = match adj {
                        Some(a) => b.clone().set("adj", self.net.node(a).text.clone()),
                        None => b.clone(),
                    };
                    bodies.extend(self.parts(key, &s, |p| p.answer = true));
                    ctx.asked = true;
                }
                Turn::Feel(a) => {
                    let v = self.valence(a).unwrap_or(0.0);
                    self.person.mood = Some((a, v));
                    ctx.feel(v);
                    let s = b.clone().set("adj", self.net.node(a).text.clone());
                    let mild = self.cfg.raven.feeling.mild_adjectives.iter().any(|m| self.net.base(m, Pos::Adj) == Some(a));
                    let (open, ask) = if mild {
                        ("empathy_mild", "q_mild")
                    } else if v < 0.0 {
                        ("empathy_bad", "q_bad")
                    } else if v > 0.0 {
                        ("empathy_good", "q_good")
                    } else {
                        ("feel_neutral", "q_good")
                    };
                    openers.extend(self.parts(open, &s, |p| p.feel = v));
                    questions.extend(self.parts(ask, &s, q));
                }
                Turn::Role(r) => {
                    let s = self.noun_slots(r, b.clone());
                    openers.extend(self.parts("role", &s, |p| p.mentions.push(r)));
                    questions.extend(self.parts("q_role", &s, |p| {
                        p.question = true;
                        p.mentions.push(r);
                    }));
                    ctx.nouns.push(r);
                }
                Turn::Like(x) | Turn::Have(x) => {
                    let have = matches!(t, Turn::Have(_));
                    if have {
                        self.referent = Some(x);
                    }
                    if have {
                        self.person.has.push(x);
                    } else {
                        self.person.likes.push(x);
                    }
                    ctx.nouns.push(x);
                    let s = self.noun_slots(x, b.clone());
                    openers.extend(self.parts(if have { "have" } else { "like" }, &s, |p| p.mentions.push(x)));
                    bodies.extend(self.fact(x));
                    let ask = if !have {
                        "q_like"
                    } else if self.net.node(x).animate {
                        "q_have_animal"
                    } else {
                        "q_have"
                    };
                    questions.extend(self.parts(ask, &s, |p| {
                        p.question = true;
                        p.mentions.push(x);
                    }));
                }
                Turn::LikeDoing(v) => {
                    let s = self.verb_slots(v, b.clone());
                    openers.extend(self.parts("like_doing", &s, |p| p.mentions.push(v)));
                    questions.extend(self.parts("q_like_doing", &s, |p| {
                        p.question = true;
                        p.mentions.push(v);
                    }));
                    ctx.nouns.push(v);
                }
                Turn::Dislike(x) => {
                    self.person.dislikes.push(x);
                    let s = if self.net.node(x).pos == Pos::Verb { self.verb_slots(x, b.clone()) } else { self.noun_slots(x, b.clone()) };
                    openers.extend(self.parts("dislike", &s, |p| p.mentions.push(x)));
                    questions.extend(self.parts("q_dislike", &s, |p| {
                        p.question = true;
                        p.mentions.push(x);
                    }));
                    ctx.nouns.push(x);
                }
                Turn::MyIs { noun, adj } => {
                    self.referent = Some(noun);
                    self.person.described.push(noun);
                    if !self.person.has.contains(&noun) {
                        self.person.has.push(noun);
                    }
                    let v = self.valence(adj).unwrap_or(0.0);
                    ctx.feel(v);
                    ctx.nouns.push(noun);
                    let s = self.noun_slots(noun, b.clone()).set("adj", self.net.node(adj).text.clone());
                    let about_it = if self.net.node(noun).animate { "q_have_animal" } else { "q_have" };
                    let (open, ask) = if v < 0.0 {
                        ("bad_news", "q_bad")
                    } else if v > 0.0 {
                        ("nice_trait", about_it)
                    } else {
                        ("neutral_news", about_it)
                    };
                    let s = match self.person.pets.iter().rev().find(|p| p.0 == noun) {
                        Some(p) => s.set("pet", p.1.clone()),
                        None => s,
                    };
                    openers.extend(self.parts(open, &s, |p| {
                        p.feel = v;
                        p.mentions.push(noun);
                    }));
                    questions.extend(self.parts(ask, &s, q));
                }
                Turn::MyKind { noun, kind } => {
                    self.referent = Some(noun);
                    if !self.person.has.contains(&noun) {
                        self.person.has.push(noun);
                    }
                    ctx.nouns.push(noun);
                    let s = self.noun_slots(noun, b.clone());
                    // "my dog is a poodle": poodles are dogs, yes (when a line says so)
                    if let Some(path) = kind_path(self.net, kind, noun) {
                        let why = vec![path.iter().map(|&x| self.net.node(x).text.clone()).collect::<Vec<_>>().join(" -is a-> ")];
                        let ks = Slots::default().set("nouns", self.plural(noun)).set("kinds", self.plural(kind));
                        for t in self.say("kind_yes") {
                            if let Some(text) = ks.fill(t) {
                                bodies.push(Part { text, fact: true, mentions: vec![noun], why: why.clone(), ..Default::default() });
                            }
                        }
                    }
                    let kind_text = &self.net.node(kind).text;
                    let a_kind = format!("{} {kind_text}", lexicon::article(kind_text, &self.cfg.article));
                    openers.extend(self.parts("neutral_news", &s.clone().set("adj", a_kind), |p| p.mentions.push(noun)));
                    questions.extend(self.parts(if self.net.node(noun).animate { "q_have_animal" } else { "q_have" }, &s, q));
                }
                Turn::Happened { noun, event } => {
                    self.referent = Some(noun);
                    if !self.person.has.contains(&noun) {
                        self.person.has.push(noun);
                    }
                    let v = self.valence(event).unwrap_or(0.0);
                    ctx.feel(v);
                    ctx.nouns.push(noun);
                    let s = self.noun_slots(noun, b.clone()).set("verb", self.net.node(event).text.clone());
                    let lost = matches!(self.net.node(event).text.as_str(), "die" | "lose" | "leave" | "disappear" | "vanish" | "run");
                    let (open, ask) = if v < 0.0 && lost {
                        ("loss", "q_loss")
                    } else if v < 0.0 {
                        ("bad_news", "q_bad")
                    } else if v > 0.0 {
                        ("good_news", "q_good")
                    } else {
                        ("neutral_news", "q_have")
                    };
                    if lost && v < 0.0 && !self.person.lost.contains(&noun) {
                        self.person.lost.push(noun);
                    }
                    let s = s.set("adj", self.net.form(event, Form::PastPart).map(|n| n.text.clone()).unwrap_or_default());
                    openers.extend(self.parts(open, &s, |p| {
                        p.feel = v;
                        p.mentions.push(noun);
                    }));
                    questions.extend(self.parts(ask, &s, q));
                }
                Turn::Opinion { noun, adj } => {
                    ctx.nouns.push(noun);
                    let s = self.noun_slots(noun, b.clone()).set("adj", self.net.node(adj).text.clone());
                    openers.extend(self.parts("opinion", &s, |p| p.mentions.push(noun)));
                    bodies.extend(self.fact(noun));
                    questions.extend(self.parts("q_like", &s, q));
                }
                Turn::ItsName { name, pron } => {
                    // whose name: the last animal they told of
                    let Some(&pet) = self.person.has.iter().rev().find(|&&h| self.net.node(h).animate) else {
                        openers.extend(self.parts("pet_name", &b.clone().set("pet", name.clone()).set("pron", "it"), |_| {}));
                        continue;
                    };
                    let obj = match pron.as_str() { "his" => "him", "her" => "her", "their" => "them", _ => "it" };
                    self.person.pets.push((pet, name.clone(), obj.to_string()));
                    ctx.nouns.push(pet);
                    let s = self.noun_slots(pet, b.clone()).set("pet", name).set("pron", obj);
                    let gone = self.person.lost.contains(&pet);
                    if gone {
                        ctx.feel(-1.0);
                    }
                    let feel = if gone { -1.0 } else { 0.0 };
                    openers.extend(self.parts(if gone { "pet_name_lost" } else { "pet_name" }, &s, |p| {
                        p.feel = feel;
                        p.memory = true;
                        p.mentions.push(pet);
                    }));
                    questions.extend(self.parts(if gone { "q_pet_lost" } else { "q_pet" }, &s, q));
                }
                Turn::Yes => match asked_for {
                    Some(Expect::HaveIt(x)) => {
                        self.person.has.push(x);
                        self.referent = Some(x);
                        ctx.nouns.push(x);
                        let s = self.noun_slots(x, b.clone());
                        openers.extend(self.parts("have", &s, |p| p.mentions.push(x)));
                        questions.extend(self.parts(if self.net.node(x).animate { "q_have_animal" } else { "q_have" }, &s, q));
                    }
                    Some(Expect::WantTalk) => {
                        openers.extend(self.parts("listening", &b, |p| p.feel = -1.0));
                        questions.extend(self.parts("q_tell", &b, q));
                        ctx.feel(-1.0);
                    }
                    _ => openers.extend(self.parts("yes", &b, |_| {})),
                },
                Turn::No => match asked_for {
                    Some(Expect::WantTalk) => {
                        openers.extend(self.parts("no_talk", &b, |p| p.feel = -1.0));
                        ctx.feel(-1.0);
                        ctx.closing = true;
                    }
                    _ => openers.extend(self.parts("no", &b, |_| {})),
                },
                Turn::DontKnow => openers.extend(self.parts("dont_know", &b, |_| {})),
                Turn::Laugh => openers.extend(self.parts("laugh", &b, |_| {})),
                Turn::Okay => openers.extend(self.parts("okay", &b, |_| {})),
                Turn::Sorry => openers.extend(self.parts("sorry", &b, |_| {})),
            }
        }
        if (!all || turns.is_empty()) && !turns.contains(&Turn::Heard) {
            let reason = if turns.is_empty() {
                match crate::question::read(self.net, text) {
                    Question::Unknown(r) if r.starts_with("I do not know the word") => Some(r),
                    _ => None,
                }
            } else {
                None
            };
            let b = self.base();
            openers.extend(self.parts("not_followed", &b, |_| {}));
            if let Some(r) = reason {
                bodies.push(Part { text: r, answer: true, ..Default::default() });
            }
        }
        // something to go on with when the turn gives nothing to ask about
        if questions.is_empty() && !ctx.closing {
            let b = self.base();
            questions.extend(self.parts("q_open", &b, |p| {
                p.question = true;
                p.generic = true;
            }));
        }
        // the name now and then: "I am sorry to hear that, Joose."
        if let Some(name) = self.person.name.clone() {
            let named: Vec<Part> = openers
                .iter()
                .filter(|p| !p.text.contains(name.as_str()) && p.text.ends_with('.') && p.text.split_whitespace().count() <= 8)
                .map(|p| Part { text: format!("{}, {name}.", p.text.trim_end_matches('.')), name: true, ..p.clone() })
                .collect();
            openers.extend(named);
        }
        self.search(openers, bodies, questions, &ctx)
    }

    /// Every reply of at most one opener, one body and one question, scored by
    /// the humane judges; the best is said (ties broken by the turn's seed).
    fn search(&mut self, openers: Vec<Part>, bodies: Vec<Part>, questions: Vec<Part>, ctx: &TurnCtx) -> Reply {
        fn none(v: &[Part]) -> Vec<Option<&Part>> {
            std::iter::once(None).chain(v.iter().map(Some)).collect()
        }
        let (os, bs, qs) = (none(&openers), none(&bodies), none(&questions));
        let mut best: Option<(f64, Vec<&Part>, Vec<&'static str>)> = None;
        let mut weighed = 0usize;
        for o in &os {
            for b in &bs {
                for q in &qs {
                    let parts: Vec<&Part> = [*o, *b, *q].into_iter().flatten().collect();
                    if parts.is_empty() {
                        continue;
                    }
                    weighed += 1;
                    let (mut s, why) = judges::score(&parts, ctx, &self.hist, &self.cfg.raven.judges);
                    let text: String = parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ");
                    s += self.cfg.raven.judges.jitter * seed01(self.turn, &text);
                    if best.as_ref().map(|b| s > b.0).unwrap_or(true) {
                        best = Some((s, parts, why));
                    }
                }
            }
        }
        let Some((_, parts, reasons)) = best else {
            return Reply { text: self.say("not_followed")[0].clone(), why: Vec::new() };
        };
        let text = parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ");
        let mut why: Vec<String> = parts.iter().flat_map(|p| p.why.iter().cloned()).collect();
        why.push(format!("raven -weighed-> {weighed} replies -chose for-> {}", if reasons.is_empty() { "fit".to_string() } else { reasons.join(", ") }));
        for p in &parts {
            self.hist.said.push(p.text.clone());
        }
        self.expect = parts.iter().find_map(|p| p.expect);
        self.hist.first.push(parts[0].text.clone());
        self.hist.asked.push(parts.iter().any(|p| p.question));
        self.hist.name_used.push(parts.iter().any(|p| p.name));
        Reply { text, why }
    }
}

/// What a question waits for, by the kind of question it is.
fn expect_for(key: &str, text: &str, noun: Option<u32>) -> Option<Expect> {
    let with = |f: fn(u32) -> Expect| noun.map(f);
    Some(match key {
        "q_ask_name" => Expect::Name,
        "q_bad" if text.contains("talk about it") => Expect::WantTalk,
        "q_loss" if text.contains(" like?") => return with(Expect::Describe),
        "q_loss" => Expect::WantTalk,
        "q_have_animal" | "q_pet" if text.contains("name?") => return with(Expect::PetName),
        "q_have_animal" | "q_pet" if text.contains("How old") => return with(Expect::Age),
        "q_have_animal" | "q_pet" | "q_have" if text.contains(" like?") => return with(Expect::Describe),
        "q_like" | "q_topic" if text.starts_with("Do you have") => return with(Expect::HaveIt),
        "q_name" | "q_open" | "q_how_are_you" if text.contains("How are you") || text.contains("How is your day") => Expect::Feeling,
        "q_name" | "q_open" if text.contains("talk about") => Expect::Topic,
        k if k.starts_with("q_") => Expect::Story,
        _ => return None,
    })
}

/// A number in [0, 1) from the turn and the reply: a deterministic tie-break.
fn seed01(turn: u64, text: &str) -> f64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ turn.wrapping_mul(0x9E3779B97F4A7C15);
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (h >> 11) as f64 / (1u64 << 53) as f64
}
