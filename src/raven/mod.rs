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
pub mod memory;
pub mod turn;

use crate::chat::{kind_path, Chat, Reply};
use crate::config::Config;
use crate::lexicon;
use crate::network::{Form, Network, Pos};
use crate::question::Question;
use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet};
use memory::{Exchange, Fact, Transcript};
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
    /// the form of the last question asked ("do you feel")
    pub last_form: Option<String>,
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
    /// the model's probability for a question it generated (0: not generated)
    pub prob: f64,
    /// the words a generated question asks about (not asked again)
    pub about: Vec<u32>,
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
    /// they told of a bad feeling a short while ago, and nothing better since:
    /// the conversation stays gentle and on it
    pub low: bool,
    /// what they are talking about (the thing they brought up last), while it is fresh
    pub thread: Option<u32>,
    pub nouns: Vec<u32>,
}

const SAY_KEYS: &[&str] = &[
    "greeting", "goodbye", "thanks", "nice_to_meet", "empathy_bad", "empathy_good", "feel_neutral", "loss",
    "bad_news", "good_news", "neutral_news", "like", "like_doing", "dislike", "have", "role", "opinion", "kind_yes",
    "laugh", "okay", "sorry", "dont_know", "yes", "no", "not_followed", "not_known", "know_little", "q_name",
    "q_open", "q_how_are_you", "not_sure", "pet_name", "pet_name_lost", "q_ask_name", "age", "heard", "listening",
    "no_talk", "nice_trait", "heard_bad", "nothing", "creator", "compliment", "insult", "about_me", "empathy_mild",
    "often", "repair", "hard_news", "good_news_self", "yes_low", "heard_low", "who_am_i_more", "recall",
    "recall_none", "recall_none_about", "my_name", "my_name_none", "pet_name_is", "pet_name_was", "pet_name_none",
    "did", "my_role", "my_role_none", "why_line", "why_told", "why_none", "we_talked", "we_talked_none", "advise", "advise_about",
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
    /// the turn their bad feeling was told on
    low_since: Option<u64>,
    /// what Raven's last reply was about, for a repair
    last_topic: Option<u32>,
    /// the lines of the word network behind Raven's last reply (for "why?")
    last_why: Vec<String>,
    /// the whole conversation (compacted when it grows long)
    pub transcript: Transcript,
    /// the words the model's questions have asked about
    asked_about: HashSet<u32>,
    /// the thing the person brought up last, and the turn they did
    thread: Option<(u32, u64)>,
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
            low_since: None,
            last_topic: None,
            last_why: Vec::new(),
            transcript: Transcript::default(),
            asked_about: HashSet::new(),
            thread: None,
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
        let s = s.set("verb", self.net.node(v).text.clone()).set("noun", self.net.node(v).text.clone()).about(v);
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

    /// The model's own questions where a follow-up question is wanted: MBLM
    /// asks about the person (how they feel, what they have, like, do) or a
    /// thing of theirs, every question filled from the lines and weighed.
    fn generated(&self, key: &str, slots: &Slots) -> Option<Vec<Part>> {
        use crate::mblm::{Waits, Who};
        let base = |w: &str, pos| self.net.base(w, pos);
        let bad = matches!(key, "q_bad" | "q_tell" | "q_tell_more" | "q_hard_news" | "q_loss" | "q_pet_lost" | "q_like_doing_low");
        let good = matches!(key, "q_good" | "q_good_plain" | "q_mild");
        let mood = match self.person.mood {
            Some((a, v)) if (bad && v < 0.0) || (good && v > 0.0) => Some(a),
            _ if bad => base("sad", Pos::Adj),
            _ if good => base("happy", Pos::Adj),
            _ => None,
        };
        let noun = slots.1;
        let mut asks: Vec<Who> = Vec::new();
        match key {
            // a mild "fine" is not asked about as a feeling
            "q_mild" => {}
            "q_bad" | "q_tell" | "q_tell_more" | "q_hard_news" | "q_good" | "q_good_plain" => {
                // (q_tell is asked after "yes, I want to talk")
                asks.push(Who::You { feel: mood, thing: None, deed: None })
            }
            "q_loss" | "q_pet_lost" => {
                if let Some(n) = noun {
                    asks.push(Who::Your { noun: n, past: true });
                }
                asks.push(Who::You { feel: mood, thing: None, deed: None });
            }
            "q_have_animal" | "q_have" | "q_pet" | "q_memory" => {
                let n = noun?;
                asks.push(Who::Your { noun: n, past: self.person.lost.contains(&n) });
            }
            "q_like" | "q_topic" => asks.push(Who::You { feel: None, thing: noun, deed: None }),
            "q_role" => {
                // what they are, as what they do: a teacher teaches
                let deed = noun.and_then(|r| {
                    self.net.lines[r as usize].iter().find(|l| l.kind == crate::network::Kind::Derived && l.rank == 0 && self.net.node(l.to).pos == Pos::Verb).map(|l| l.to)
                });
                asks.push(Who::You { feel: None, thing: None, deed });
            }
            "q_like_doing" => asks.push(Who::You { feel: None, thing: None, deed: noun }),
            "q_like_doing_low" => {
                asks.push(Who::You { feel: mood, thing: None, deed: noun });
            }
            "q_dislike" => {}
            _ => return None,
        }
        let m = crate::mblm::Mblm::new(self.net, self.cfg);
        let mut out: Vec<Part> = Vec::new();
        for who in asks {
            let pet = match who {
                Who::Your { noun, .. } => self.person.pets.iter().rev().find(|p| p.0 == noun).map(|p| p.1.clone()),
                _ => None,
            };
            for a in m.questions(who, pet.as_deref(), &self.asked_about, self.cfg.mblm.ask_offer) {
                let expect = match a.waits {
                    Waits::Confirm(c) => Expect::Confirm(c),
                    Waits::Story => Expect::Story,
                    Waits::Describe(n) => Expect::Describe(n),
                    Waits::PetName(n) => Expect::PetName(n),
                };
                let mut why = a.why.clone();
                why.push(format!("mblm -asked-> {} (p {:.3})", a.text, a.p));
                let mut mentions = a.about.clone();
                if let Who::Your { noun, .. } = who {
                    mentions.push(noun);
                }
                out.push(Part {
                    text: a.text,
                    question: true,
                    expect: Some(expect),
                    prob: a.p,
                    about: a.about,
                    mentions,
                    why,
                    memory: matches!(who, Who::Your { .. }),
                    feel: if bad { -1.0 } else { 0.0 },
                    ..Default::default()
                });
            }
        }
        // the model's best question here weighs 1, the others by how much less likely they are
        let best = out.iter().map(|p| p.prob).fold(0.0, f64::max);
        if best > 0.0 {
            for p in out.iter_mut() {
                p.prob /= best;
            }
        }
        Some(out)
    }

    /// Parts from the templates under `key`, with what they carry (or, for a
    /// follow-up question, the model's own questions).
    fn parts(&self, key: &str, slots: &Slots, make: impl Fn(&mut Part)) -> Vec<Part> {
        if let Some(mut asked) = self.generated(key, slots) {
            for p in asked.iter_mut() {
                let keep = (p.expect, p.mentions.clone(), p.feel);
                make(p);
                p.question = true;
                p.expect = keep.0;
                p.mentions.extend(keep.1);
                p.mentions.dedup();
                if keep.2 < 0.0 {
                    p.feel = keep.2;
                }
            }
            return asked;
        }
        self.say(key)
            .iter()
            .filter_map(|t| {
                let text = slots.fill(t)?;
                let mut p = Part { text, ..Default::default() };
                p.name = t.contains("{name}");
                p.expect = expect_for(key, &p.text);
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
                if n.proper || !(matches!(n.pos, Pos::Adj | Pos::Noun) && n.form == Form::Base || n.pos == Pos::Verb) {
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
        // the system prompt's rules that come before any reply is made
        let low_text = text.to_lowercase();
        let sys = &self.cfg.raven.system;
        // 0. safety first: only how to reach help, nothing else
        if sys.crisis_words.iter().any(|w| low_text.contains(w.as_str())) {
            self.turn += 1;
            self.low_since = Some(self.turn);
            self.expect = None;
            let reply = Reply { text: sys.crisis_reply.clone(), why: vec!["raven -rule 0-> safety first".into()] };
            self.transcript.push(Exchange { turn: self.turn, user: text.to_string(), reply: reply.text.clone(), facts: vec![] },
                                 self.cfg.raven.memory.max_words, self.cfg.raven.memory.keep_words);
            return reply;
        }
        // 11. privacy: forgetting, all at once
        if crate::question::read(self.net, text) == Question::Forget {
            let fresh = Raven::new(self.net, self.cfg).expect("rules.toml was read once already");
            *self = fresh;
            return Reply { text: self.cfg.raven.system.forget_reply.clone(), why: vec!["raven -rule 11-> privacy".into()] };
        }
        // 12. no advice it cannot ground
        if turn::is_question(text) && sys.advice_words.iter().any(|w| low_text.split(|c: char| !c.is_alphanumeric()).any(|x| x == w) || (w.contains(' ') && low_text.contains(w.as_str()))) {
            self.turn += 1;
            return Reply { text: sys.advice_reply.clone(), why: vec!["raven -rule 12-> no advice it cannot ground".into()] };
        }
        self.turn += 1;
        let low_now = self.low_since.map(|t| self.turn - t <= self.cfg.raven.judges.low_turns).unwrap_or(false);
        let cx = turn::Context { expect: self.expect, referent: self.referent, low: low_now };
        let (turns, all) = turn::read(self.net, text, &cx);
        let asked_for = self.expect.take();
        let turns = match (asked_for, turns.as_slice()) {
            (Some(Expect::Confirm(c)), [Turn::Yes]) => {
                use crate::mblm::Confirm;
                vec![match c {
                    Confirm::Feel(a) => Turn::Feel(a),
                    Confirm::Has(x) => Turn::Have(x),
                    Confirm::Likes(x) => Turn::Like(x),
                    Confirm::Is(noun, adj) => Turn::MyIs { noun, adj },
                    Confirm::Kind(noun, kind) => Turn::MyKind { noun, kind },
                    Confirm::Does(..) => Turn::Yes,
                }]
            }
            _ => turns,
        };
        let mut ctx = TurnCtx::default();
        ctx.low = self.low_since.map(|t| self.turn - t <= self.cfg.raven.judges.low_turns).unwrap_or(false);
        // the thread: what they brought up, in this turn or a short while ago
        let brought: Option<u32> = turns.iter().rev().find_map(|t| match t {
            Turn::Like(x) | Turn::Have(x) | Turn::Role(x) => Some(*x),
            Turn::MyIs { noun, .. } | Turn::MyKind { noun, .. } | Turn::Happened { noun, .. } | Turn::Opinion { noun, .. } => Some(*noun),
            Turn::PetNamed { noun, .. } | Turn::Age { noun, .. } | Turn::HaveNamed { noun, .. } | Turn::About(noun) => Some(*noun),
            Turn::Ask(Question::Ask { topic: crate::question::Subject::Noun(n), .. }) => Some(*n),
            _ => None,
        });
        if let Some(n) = brought {
            self.thread = Some((n, self.turn));
            // "he", "it", "they" are what they brought up last; a role ("a programmer") is themselves
            if !turns.iter().any(|t| matches!(t, Turn::Role(_))) {
                self.referent = Some(n);
            }
        }
        ctx.thread = self.thread.filter(|&(_, t)| self.turn - t <= self.cfg.raven.judges.thread_turns).map(|(n, _)| n);
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
                Turn::Ask(Question::Recall(about)) => {
                    ctx.asked = true;
                    bodies.push(self.recall(about));
                }
                Turn::Ask(Question::MyName) => {
                    ctx.asked = true;
                    let key = if self.person.name.is_some() { "my_name" } else { "my_name_none" };
                    bodies.extend(self.parts(key, &b, |p| {
                        p.answer = true;
                        p.memory = true;
                        p.name = false;
                    }));
                }
                Turn::Ask(Question::PetNameOf(x)) => {
                    ctx.asked = true;
                    let pet = self.person.pets.iter().rev().find(|p| p.0 == x || kind_path(self.net, p.0, x).is_some()).cloned();
                    let s = self.noun_slots(pet.as_ref().map(|p| p.0).unwrap_or(x), b.clone());
                    let (key, s) = match pet {
                        Some((n, name, _)) if self.person.lost.contains(&n) => ("pet_name_was", s.set("pet", name)),
                        Some((_, name, _)) => ("pet_name_is", s.set("pet", name)),
                        None => ("pet_name_none", s),
                    };
                    bodies.extend(self.parts(key, &s, |p| {
                        p.answer = true;
                        p.memory = true;
                    }));
                }
                Turn::Ask(q0) => {
                    ctx.asked = true;
                    // "tell me more": more of the thread, if there is one
                    let q0 = match (q0, ctx.thread.or(self.referent)) {
                        (Question::More, Some(x)) => Question::Ask { topic: crate::question::Subject::Noun(x), intent: crate::question::Intent::About, more: true },
                        (q0, _) => q0,
                    };
                    // about itself: more than one way to say it, so asking again is not answered with a repeat
                    if q0 == Question::WhoAreYou {
                        bodies.extend(self.parts("who_am_i_more", &b, |p| p.answer = true));
                        if let Some(t) = self.cfg.raven.system.identity.get("who") {
                            bodies.push(Part { text: t.clone(), answer: true, ..Default::default() });
                        }
                    }
                    let what_is = matches!(q0, Question::Ask { intent: crate::question::Intent::WhatIs, .. });
                    // (a question about itself is about itself: no old topic comes back after it)
                    let about_me = matches!(q0, Question::Me(_) | Question::WhoAreYou | Question::HowAreYou | Question::Rules
                        | Question::YouCan(..) | Question::YouAre(_) | Question::YouAreAdj(_));
                    let r = self.chat.answer_question(q0);
                    let topic = if about_me { None } else { self.chat.topic() };
                    let mut text = self.warm(&r.text);
                    // what it is: the dictionary's own words too
                    if let (true, false, Some(tp)) = (what_is, r.why.is_empty(), topic) {
                        if let Some(d) = self.net.def(tp) {
                            text = format!("{text} My dictionary says: \"{d}\".");
                        }
                    }
                    let mut part = Part { text, answer: true, why: r.why.clone(), ..Default::default() };
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
                    } else if ctx.low {
                        let s = match self.person.mood {
                            Some((a, _)) => s.set("adj", self.net.node(a).text.clone()),
                            None => s,
                        };
                        openers.extend(self.parts("heard_low", &s, |p| p.feel = -1.0));
                        questions.extend(self.parts("q_tell_more", &s, q));
                    } else {
                        openers.extend(self.parts("heard", &s, |p| p.feel = if ctx.valence < 0.0 { -1.0 } else { tone }));
                    }
                }
                Turn::Often(often) => {
                    let s = match asked_for {
                        Some(Expect::Often(v)) => self.verb_slots(v, b.clone()),
                        _ => b.clone(),
                    }
                    .set("often", often);
                    openers.extend(self.parts("often", &s, |p| p.memory = true));
                }
                Turn::Repair => {
                    // what Raven said made no sense: say so, and go back to them
                    let s = match self.last_topic {
                        Some(tp) => self.noun_slots(tp, b.clone()),
                        None => b.clone(),
                    };
                    // "what do you mean?" is a question: the apology answers it
                    ctx.asked = true;
                    openers.extend(self.parts("repair", &s, |p| p.answer = true));
                    let s2 = match self.person.mood {
                        Some((a, v)) if v < 0.0 => b.clone().set("adj", self.net.node(a).text.clone()),
                        _ => b.clone(),
                    };
                    questions.extend(self.parts(if ctx.low { "q_tell" } else { "q_open" }, &s2, q));
                }
                Turn::IHappened(event) => {
                    let v = self.valence(event).unwrap_or(0.0);
                    ctx.feel(v);
                    if v < 0.0 {
                        self.low_since = Some(self.turn);
                        ctx.low = true;
                    }
                    let s = self.verb_slots(event, b.clone());
                    let (open, ask) = if v < 0.0 { ("hard_news", "q_hard_news") } else if v > 0.0 { ("good_news_self", "q_good") } else { ("heard", "q_tell_more") };
                    openers.extend(self.parts(open, &s, |p| p.feel = v));
                    questions.extend(self.parts(ask, &s, |p| {
                        p.question = true;
                        p.feel = v.min(0.0);
                    }));
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
                    // a bad feeling is the thread of the conversation for a while; a good one ends it
                    if v < 0.0 && !mild {
                        self.low_since = Some(self.turn);
                        ctx.low = true;
                    } else if v > 0.0 && !mild {
                        self.low_since = None;
                        ctx.low = false;
                    }
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
                    let mut s = self.verb_slots(v, b.clone());
                    openers.extend(self.parts("like_doing", &s, |p| p.mentions.push(v)));
                    // while they are low, what they like is asked about with how they feel
                    let low = match (ctx.low, self.person.mood) {
                        (true, Some((a, _))) => {
                            s = s.set("adj", self.net.node(a).text.clone());
                            true
                        }
                        _ => false,
                    };
                    questions.extend(self.parts(if low { "q_like_doing_low" } else { "q_like_doing" }, &s, |p| {
                        p.question = true;
                        p.feel = if low { -1.0 } else { 0.0 };
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
                Turn::YesWant if ctx.low || matches!(asked_for, Some(Expect::WantTalk)) => {
                    let s = match self.person.mood {
                        Some((a, v)) if v < 0.0 => b.clone().set("adj", self.net.node(a).text.clone()),
                        _ => b.clone(),
                    };
                    openers.extend(self.parts("listening", &s, |p| p.feel = -1.0));
                    questions.extend(self.parts("q_tell", &s, |p| {
                        p.question = true;
                        p.feel = -1.0;
                    }));
                    ctx.feel(-1.0);
                }
                Turn::YesWant => openers.extend(self.parts("yes", &b, |_| {})),
                Turn::Yes => match asked_for {
                    Some(Expect::HaveIt(x)) => {
                        self.person.has.push(x);
                        self.referent = Some(x);
                        ctx.nouns.push(x);
                        let s = self.noun_slots(x, b.clone());
                        openers.extend(self.parts("have", &s, |p| p.mentions.push(x)));
                        questions.extend(self.parts(if self.net.node(x).animate { "q_have_animal" } else { "q_have" }, &s, q));
                    }
                    // "yes, I want to": while they are low, to anything open, it means "listen"
                    Some(Expect::WantTalk) | Some(Expect::Story) | Some(Expect::Feeling)
                        if ctx.low || asked_for == Some(Expect::WantTalk) =>
                    {
                        let s = match self.person.mood {
                            Some((a, v)) if v < 0.0 => b.clone().set("adj", self.net.node(a).text.clone()),
                            _ => b.clone(),
                        };
                        openers.extend(self.parts("listening", &s, |p| p.feel = -1.0));
                        questions.extend(self.parts("q_tell", &s, q));
                        ctx.feel(-1.0);
                    }
                    _ if ctx.low => {
                        openers.extend(self.parts("yes_low", &b, |p| p.feel = -1.0));
                        let s = match self.person.mood {
                            Some((a, v)) if v < 0.0 => b.clone().set("adj", self.net.node(a).text.clone()),
                            _ => b.clone(),
                        };
                        questions.extend(self.parts("q_tell_more", &s, |p| {
                            p.question = true;
                            p.feel = -1.0;
                        }));
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
                Turn::HaveNamed { noun, name } => {
                    self.referent = Some(noun);
                    self.person.has.push(noun);
                    self.person.pets.push((noun, name.clone(), "it".to_string()));
                    ctx.nouns.push(noun);
                    let s = self.noun_slots(noun, b.clone()).set("pet", name);
                    openers.extend(self.parts("pet_name", &s, |p| {
                        p.memory = true;
                        p.mentions.push(noun);
                    }));
                    questions.extend(self.parts(if self.net.node(noun).animate { "q_have_animal" } else { "q_have" }, &s, |p| {
                        p.question = true;
                        p.mentions.push(noun);
                    }));
                }
                Turn::Did(v) => {
                    let s = self.verb_slots(v, b.clone());
                    openers.extend(self.parts("did", &s, |p| p.mentions.push(v)));
                    questions.extend(self.parts("q_like_doing", &s, |p| {
                        p.question = true;
                        p.mentions.push(v);
                    }));
                    ctx.nouns.push(v);
                }
                Turn::About(noun) => {
                    // said of what they talked of: heard, and the thread goes on with it
                    ctx.nouns.push(noun);
                    let s = self.noun_slots(noun, b.clone());
                    openers.extend(self.parts("heard", &s, |p| p.mentions.push(noun)));
                    bodies.extend(self.fact(noun));
                }
                Turn::Why => {
                    ctx.asked = true;
                    let line = self.last_why.iter().find(|w| w.contains(" -")).cloned();
                    let (key, s) = match (line, self.last_topic) {
                        (Some(l), _) => ("why_line", b.clone().set("line", l)),
                        (None, Some(tp)) => ("why_told", self.noun_slots(tp, b.clone())),
                        (None, None) => ("why_none", b.clone()),
                    };
                    bodies.extend(self.parts(key, &s, |p| p.answer = true));
                }
                Turn::MyRole => {
                    ctx.asked = true;
                    let role = self.transcript.facts().into_iter().rev().find_map(|(_, f)| if let Fact::Role(r) = f { Some(r) } else { None });
                    let (key, s) = match role {
                        Some(r) => ("my_role", self.noun_slots(r, b.clone())),
                        None => ("my_role_none", b.clone()),
                    };
                    bodies.extend(self.parts(key, &s, |p| {
                        p.answer = true;
                        p.memory = role.is_some();
                    }));
                }
                Turn::WhatAbout => {
                    ctx.asked = true;
                    let about = self.thread.map(|(n, _)| n).or(self.last_topic);
                    match about {
                        Some(x) => {
                            let s = self.noun_slots(x, b.clone());
                            let key = if self.person.has.contains(&x) { "we_talked" } else { "we_talked_none" };
                            bodies.extend(self.parts(key, &s, |p| {
                                p.answer = true;
                                p.mentions.push(x);
                            }));
                            if self.person.has.contains(&x) {
                                let mut r = self.recall(Some(x));
                                r.answer = false;
                                bodies.push(r);
                            }
                        }
                        None => bodies.push(self.recall(None)),
                    }
                }
                Turn::Advise(v) => {
                    ctx.asked = true;
                    // the choice is theirs; what they told of a trouble is kept in mind
                    let trouble = self.transcript.facts().into_iter().rev().find_map(|(_, f)| match f {
                        Fact::Happened(n, e) if self.valence(e).unwrap_or(0.0) < 0.0 => Some(n),
                        Fact::Is(n, a) if self.valence(a).unwrap_or(0.0) < 0.0 => Some(n),
                        _ => None,
                    });
                    let s = self.verb_slots(v, b.clone());
                    let (key, s) = match trouble {
                        Some(n) => ("advise_about", self.noun_slots(n, s)),
                        None => ("advise", s),
                    };
                    bodies.extend(self.parts(key, &s, |p| p.answer = true));
                }
                Turn::DontKnow => openers.extend(self.parts("dont_know", &b, |_| {})),
                Turn::Laugh => openers.extend(self.parts("laugh", &b, |_| {})),
                Turn::Okay => openers.extend(self.parts("okay", &b, |_| {})),
                Turn::Sorry => openers.extend(self.parts("sorry", &b, |_| {})),
            }
        }
        if (!all || turns.is_empty()) && !turns.contains(&Turn::Heard) {
            // a question not read: say what was not read, and what can be asked
            let asked_unread = turn::is_question(text);
            if asked_unread {
                ctx.asked = true;
            }
            let reason = if turns.is_empty() {
                match crate::question::read(self.net, text) {
                    Question::Unknown(r) if r.starts_with("I do not know the word") || asked_unread => Some(r),
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
        // the thread: questions about what they are talking about are always on offer
        if let Some(th) = ctx.thread {
            if !ctx.closing && !questions.iter().any(|p| p.mentions.contains(&th)) {
                let key = if self.person.has.contains(&th) {
                    if self.net.node(th).animate { "q_have_animal" } else { "q_have" }
                } else {
                    "q_topic"
                };
                let s = self.noun_slots(th, self.base());
                questions.extend(self.parts(key, &s, |p| {
                    p.question = true;
                    p.mentions.push(th);
                }));
            }
        }
        // the thread stays while what they say stays low
        if ctx.low && ctx.valence < 0.0 {
            self.low_since = Some(self.turn);
        }
        // something to go on with when the turn gives nothing to ask about
        // a question that would fit anyone ("what have you been up to?") is generic
        let anyone: Vec<String> = ["q_open"].iter().flat_map(|k| self.say(k).iter().cloned()).collect();
        for p in questions.iter_mut() {
            if anyone.contains(&p.text) {
                p.generic = true;
            }
        }
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
        let reply = self.search(openers, bodies, questions, &ctx);
        self.last_why = reply.why.iter().filter(|w| !w.starts_with("raven ") && !w.starts_with("mblm ")).cloned().collect();
        // the whole conversation is remembered: the exchange, and what was read from it
        let facts = self.facts_of(&turns);
        let m = &self.cfg.raven.memory;
        self.transcript.push(Exchange { turn: self.turn, user: text.to_string(), reply: reply.text.clone(), facts }, m.max_words, m.keep_words);
        let mut reply = reply;
        reply.why.push(format!(
            "raven -remembers-> {} turns ({} compacted, {} facts kept from them)",
            self.transcript.turns(),
            self.transcript.compacted.turns,
            self.transcript.compacted.facts.len()
        ));
        reply
    }

    /// What Raven read from a turn, to remember.
    fn facts_of(&self, turns: &[Turn]) -> Vec<Fact> {
        let mut out = Vec::new();
        for t in turns {
            match t {
                Turn::Name(n) => out.push(Fact::Name(n.clone())),
                Turn::Feel(a) => out.push(Fact::Feel(*a, self.valence(*a).unwrap_or(0.0))),
                Turn::FeelNot(a) => out.push(Fact::Feel(*a, -self.valence(*a).unwrap_or(0.0))),
                Turn::Have(x) => out.push(Fact::Has(*x)),
                Turn::HaveNamed { noun, name } => {
                    out.push(Fact::Has(*noun));
                    out.push(Fact::PetName(*noun, name.clone()));
                }
                Turn::Did(v) => out.push(Fact::TheyHappened(*v)),
                Turn::Like(x) | Turn::LikeDoing(x) => out.push(Fact::Likes(*x)),
                Turn::Dislike(x) => out.push(Fact::Dislikes(*x)),
                Turn::MyIs { noun, adj } => out.push(Fact::Is(*noun, *adj)),
                Turn::Happened { noun, event } => out.push(Fact::Happened(*noun, *event)),
                Turn::IHappened(e) => out.push(Fact::TheyHappened(*e)),
                Turn::PetNamed { noun, name } => out.push(Fact::PetName(*noun, name.clone())),
                Turn::ItsName { name, .. } => {
                    if let Some(p) = self.person.pets.iter().rev().find(|p| &p.1 == name) {
                        out.push(Fact::PetName(p.0, name.clone()));
                    }
                }
                Turn::Age { noun, age } => out.push(Fact::Age(*noun, age.clone())),
                Turn::Role(r) => out.push(Fact::Role(*r)),
                Turn::Ask(_) => {
                    if let Some(tp) = self.chat.topic() {
                        out.push(Fact::Asked(tp));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// A verb in the past, a phrase one too ("break up" -> "broke up").
    fn past(&self, v: u32) -> String {
        let text = &self.net.node(v).text;
        if let Some(p) = self.net.form(v, Form::Past) {
            return p.text.clone();
        }
        match text.split_once(' ') {
            Some((head, rest)) => match self.net.base(head, Pos::Verb).and_then(|h| self.net.form(h, Form::Past)) {
                Some(p) => format!("{} {rest}", p.text),
                None => text.clone(),
            },
            None => text.clone(),
        }
    }

    /// What Raven remembers of the whole conversation (about one thing, or all).
    fn recall(&self, about: Option<u32>) -> Part {
        let mut said: Vec<String> = Vec::new();
        let mut asked = 0;
        for (_, f) in self.transcript.facts().into_iter().rev() {
            if let Some(x) = about {
                let touches = f.about().iter().any(|&n| n == x || kind_path(self.net, n, x).is_some() || kind_path(self.net, x, n).is_some());
                if !touches {
                    continue;
                }
            }
            let gone = |n: u32| self.person.lost.contains(&n);
            let s = |n: u32| self.noun_slots(n, Slots::default());
            let phrase = match &f {
                Fact::Name(name) => Slots::default().set("name", name.clone()).fill("your name is {name}"),
                Fact::Feel(a, _) => Some(format!("you felt {}", self.net.node(*a).text)),
                Fact::Has(n) if !gone(*n) => s(*n).fill("you have {a_noun}").or_else(|| s(*n).fill("you have {noun}")),
                Fact::Likes(n) if self.net.node(*n).pos == Pos::Verb => self.verb_slots(*n, Slots::default()).fill("you like {verbing}"),
                Fact::Likes(n) => s(*n).fill("you like {nouns}").or_else(|| Some(format!("you like {}", self.net.node(*n).text))),
                Fact::Dislikes(n) => s(*n).fill("you do not like {nouns}"),
                Fact::Is(n, a) => Some(format!("your {} {} {}", self.net.node(*n).text, if gone(*n) { "was" } else { "is" }, self.net.node(*a).text)),
                Fact::Happened(n, e) => Some(format!("your {} {}", self.net.node(*n).text, self.past(*e))),
                Fact::TheyHappened(e) => Some(format!("you {}", self.past(*e))),
                Fact::PetName(n, name) => Some(format!("your {} {} called {name}", self.net.node(*n).text, if gone(*n) { "was" } else { "is" })),
                Fact::Age(n, age) => Some(format!("your {} is {age} old", self.net.node(*n).text)),
                Fact::Role(n) => s(*n).fill("you are {a_noun}"),
                Fact::Asked(n) if asked < 2 && about.is_none() => {
                    asked += 1;
                    s(*n).fill("you asked me about {nouns}")
                }
                _ => None,
            };
            if let Some(p) = phrase {
                let p = p[..1].to_lowercase() + &p[1..];
                if !said.contains(&p) {
                    said.push(p);
                }
            }
        }
        said.reverse();
        let text = match (said.len(), about) {
            (0, Some(x)) => self.noun_slots(x, self.base()).fill(&self.say("recall_none_about")[0]).unwrap_or_default(),
            (0, None) => self.say("recall_none")[0].clone(),
            _ => {
                let list = match said.len() {
                    1 => said[0].clone(),
                    k => format!("{} and {}", said[..k - 1].join(", "), said[k - 1]),
                };
                Slots::default().set("list", list).fill(&self.say("recall")[0]).unwrap_or_default()
            }
        };
        Part { text, answer: true, memory: !said.is_empty(), mentions: about.into_iter().collect(), ..Default::default() }
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
        let mut refused = 0usize;
        // (only if no reply keeps every rule: the best of the rest, said openly in the why)
        let mut fallback: Option<(f64, Vec<&Part>, Vec<&'static str>)> = None;
        let can_answer = bodies.iter().chain(openers.iter()).any(|p| p.answer);
        let can_sympathize = openers.iter().any(|p| p.feel < 0.0 && !p.question);
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
                    // the system prompt's rules: a reply that breaks one is never said
                    if judges::breaks(&parts, ctx, &self.hist, &self.cfg.raven.system, can_answer, can_sympathize).is_some() {
                        refused += 1;
                        if fallback.as_ref().map(|b| s > b.0).unwrap_or(true) {
                            fallback = Some((s, parts, why));
                        }
                        continue;
                    }
                    if best.as_ref().map(|b| s > b.0).unwrap_or(true) {
                        best = Some((s, parts, why));
                    }
                }
            }
        }
        let kept_rules = best.is_some();
        let Some((_, parts, mut reasons)) = best.or(fallback) else {
            return Reply { text: self.say("not_followed")[0].clone(), why: Vec::new() };
        };
        let text = parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ");
        let mut why: Vec<String> = parts.iter().flat_map(|p| p.why.iter().cloned()).collect();
        if !kept_rules {
            reasons.push("no reply kept every rule");
        }
        why.push(format!(
            "raven -weighed-> {weighed} replies ({refused} broke its rules) -chose for-> {}",
            if reasons.is_empty() { "fit".to_string() } else { reasons.join(", ") }
        ));
        for p in &parts {
            self.hist.said.push(p.text.clone());
        }
        self.expect = parts.iter().find_map(|p| p.expect);
        for p in &parts {
            self.asked_about.extend(p.about.iter().copied());
        }
        self.last_topic = parts.iter().flat_map(|p| p.mentions.iter()).next().copied().or(self.chat.topic());
        self.hist.first.push(parts[0].text.clone());
        self.hist.asked.push(parts.iter().any(|p| p.question || p.text.contains('?')));
        self.hist.name_used.push(parts.iter().any(|p| p.name));
        if let Some(q) = parts.iter().find(|p| p.question) {
            self.hist.last_form = Some(judges::form_of(&q.text));
        }
        Reply { text, why }
    }
}

/// What a question waits for, by the kind of question it is.
/// (The model's own questions carry what they wait for themselves.)
fn expect_for(key: &str, text: &str) -> Option<Expect> {
    Some(match key {
        "q_ask_name" => Expect::Name,
        _ if text.contains(" feel?") || text.starts_with("How are you feeling") => Expect::Feeling,
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
