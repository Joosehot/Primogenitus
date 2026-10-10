//! The chatbot: a question read whole (question.rs), answered from the frozen
//! network by the same pipeline, with a memory of the conversation: what was
//! talked about last ("it", "they") and what has been said already ("what else").
//!
//! Every sentence the bot says about the world passes the judges' grammar and
//! the parser's check. A question the network has no line for gets that answer
//! ("I know of nothing that ..."), never a guess.

use crate::config::Config;
use crate::network::{Form, Kind, Network, Pos};

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
use crate::question::{self, Intent, Question, Subject};
use crate::rules::{judges, Verdict};
use crate::search::{self, Cand, Ctx, Det, Np, Num, Pred, Tense};
use std::collections::{HashSet, VecDeque};

pub struct Reply {
    pub text: String,
    /// the lines the answer stands on
    pub why: Vec<String>,
}

impl Reply {
    fn plain(text: impl Into<String>) -> Reply {
        Reply { text: text.into(), why: Vec::new() }
    }
}

pub struct Chat<'a> {
    net: &'a Network,
    cfg: &'a Config,
    /// what was talked about last
    topic: Option<u32>,
    /// the last question, its topic resolved, for "what else"
    last: Option<Question>,
    said: HashSet<Vec<u32>>,
    /// the (topic, question) pairs answered at least once
    answered: HashSet<(u32, Intent)>,
}

fn patterns(intent: Intent) -> &'static [&'static str] {
    match intent {
        Intent::About => &["is_a", "has_part", "does", "does_to", "is_adj"],
        Intent::WhatIs => &["is_a"],
        Intent::WhatLike => &["is_adj"],
        Intent::WhatDoes => &["does", "does_to"],
        Intent::WhatHas => &["has_part"],
    }
}

/// The grammar judges only (not the ones that score, nor "no repeat":
/// "a car is an automobile" is a fair answer to a question).
fn grammatical(cx: &Ctx, c: &Cand) -> bool {
    judges::ALL
        .iter()
        .filter(|(name, _)| !matches!(*name, "no repeat" | "animacy" | "role"))
        .all(|(_, j)| !matches!(j(cx, c), Verdict::Refuse(_)))
}

impl<'a> Chat<'a> {
    pub fn new(net: &'a Network, cfg: &'a Config) -> Chat<'a> {
        Chat { net, cfg, topic: None, last: None, said: HashSet::new(), answered: HashSet::new() }
    }

    pub fn reply(&mut self, input: &str) -> Reply {
        let q = question::read(self.net, input);
        if q == Question::More {
            return match self.last.clone() {
                Some(last) => self.answer(last),
                None => Reply::plain(self.cfg.chat.nothing_yet.clone()),
            };
        }
        self.answer(q)
    }

    /// Answer a question already read (Raven reads the turn itself).
    pub fn answer_question(&mut self, q: Question) -> Reply {
        self.answer(q)
    }

    /// What was talked about last.
    pub fn topic(&self) -> Option<u32> {
        self.topic
    }

    fn answer(&mut self, q: Question) -> Reply {
        let c = &self.cfg.chat;
        // what "what else" asks again: the question, with "it" made what it meant
        let fix = |s: Subject, topic: Option<u32>| match (s, topic) {
            (Subject::Pronoun, Some(t)) => Subject::Noun(t),
            (s, _) => s,
        };
        let again = match &q {
            Question::Ask { topic, intent, more } => Some(Question::Ask { topic: fix(*topic, self.topic), intent: *intent, more: *more }),
            Question::ObjectOf { topic, verb } => Some(Question::ObjectOf { topic: fix(*topic, self.topic), verb: *verb }),
            Question::SubjectOf { .. } | Question::WhichDo { .. } | Question::WhichHave { .. } => Some(q.clone()),
            _ => None,
        };
        if again.is_some() {
            self.last = again;
        }
        match q {
            Question::Greeting => Reply::plain(c.greeting.clone()),
            Question::Goodbye => Reply::plain(c.goodbye.clone()),
            Question::Thanks => Reply::plain(c.thanks.clone()),
            Question::Help => Reply::plain(c.help.clone()),
            Question::Unknown(why) => Reply::plain(why),
            Question::WhoAreYou => Reply::plain(c.who_am_i.clone()),
            Question::WhereLive(topic) => match self.resolve(topic) {
                Ok(t) => self.where_live(t),
                Err(r) => r,
            },
            Question::UsedFor(topic) => match self.resolve(topic) {
                Ok(t) => self.used_for(t),
                Err(r) => r,
            },
            Question::MadeOf(topic) => match self.resolve(topic) {
                Ok(t) => self.made_of(t),
                Err(r) => r,
            },
            Question::PartNamed { kind, of } => self.part_named(kind, of),
            Question::Rules => Reply::plain(self.cfg.raven.system.prompt.trim().to_string()),
            Question::Me(key) => Reply {
                text: self.cfg.raven.system.identity.get(key).cloned().unwrap_or_else(|| c.who_am_i.clone()),
                why: vec![format!("raven -knows who it is-> {key}")],
            },
            Question::Forget => {
                *self = Chat::new(self.net, self.cfg);
                Reply::plain(self.cfg.raven.system.forget_reply.clone())
            }
            Question::Define(topic) => match topic {
                Subject::Noun(t) => self.define(t),
                Subject::Pronoun => match self.topic {
                    Some(t) => self.define(t),
                    None => Reply::plain(c.no_pronoun.clone()),
                },
            },
            // what is remembered of a conversation is Raven's (primogenitus chat, the page)
            Question::Recall(_) | Question::MyName | Question::PetNameOf(_) => {
                Reply::plain("I remember a conversation only while we have one: talk with me in the chat.")
            }
            Question::HowAreYou => Reply::plain(c.how_am_i.clone()),
            Question::YouCan(v, obj) => Reply::plain(self.me_can(v, obj).unwrap_or_else(|| self.cfg.me.cannot.clone())),
            Question::YouAreAdj(a) => Reply::plain(self.me_word(a, &self.cfg.me.is, Pos::Adj).unwrap_or_else(|| self.cfg.me.unknown_adj.clone())),
            Question::YouAre(k) => self.you_are(k),
            Question::Cannot(w) => Reply::plain(c.cannot.replace("{word}", &w).replace(
                "{what}",
                match w.as_str() {
                    "where" => "places",
                    "when" => "times",
                    "why" => "reasons",
                    _ => "ways and amounts",
                },
            )),
            Question::ObjectOf { topic, verb } => match self.resolve(topic) {
                Ok(t) => self.object_of(t, verb),
                Err(r) => r,
            },
            Question::SubjectOf { verb, obj } => self.subject_of(verb, obj, None),
            Question::WhichDo { kind, verb, obj } => self.subject_of(verb, obj, Some(kind)),
            Question::WhichHave { kind, part } => self.which_have(kind, part),
            Question::KindOf { kind, topic } => match self.resolve(topic) {
                Ok(t) => self.kind_of(t, kind),
                Err(r) => r,
            },
            Question::HowAdj { adj, topic } => match self.resolve(topic) {
                Ok(t) => self.how_adj(t, adj),
                Err(r) => r,
            },
            Question::More => Reply::plain(c.nothing_yet.clone()),
            Question::Ask { topic, intent, .. } => match self.resolve(topic) {
                Ok(t) => self.ask(t, intent),
                Err(r) => r,
            },
            Question::IsA { topic, kind } => match self.resolve(topic) {
                Ok(t) => self.is_a(t, kind),
                Err(r) => r,
            },
            Question::IsAdj { topic, adj } => match self.resolve(topic) {
                Ok(t) => self.is_adj(t, adj),
                Err(r) => r,
            },
            Question::Has { topic, part } => match self.resolve(topic) {
                Ok(t) => self.has(t, part),
                Err(r) => r,
            },
            Question::Does { topic, verb, obj } => match self.resolve(topic) {
                Ok(t) => self.does(t, verb, obj),
                Err(r) => r,
            },
        }
    }

    fn resolve(&mut self, s: Subject) -> Result<u32, Reply> {
        let t = match s {
            Subject::Noun(n) => n,
            Subject::Pronoun => match self.topic {
                Some(t) => t,
                None => return Err(Reply::plain(self.cfg.chat.no_pronoun.clone())),
            },
        };
        if !self.net.node(t).sure {
            return Err(Reply::plain(format!(
                "I know the word \"{}\", but not whether it is one thing or many, so I cannot say anything about it that I am sure is right.",
                self.net.node(t).text
            )));
        }
        self.topic = Some(t);
        Ok(t)
    }

    /// How the topic is named inside a framing sentence: "dogs", "the bread".
    fn named(&self, t: u32) -> (String, Num) {
        let n = self.net.node(t);
        if n.mass {
            return (n.text.clone(), Num::Sg);
        }
        match self.net.form(t, Form::Plural) {
            Some(p) if p.sure => (p.text.clone(), Num::Pl),
            _ => (format!("the {}", n.text), Num::Sg),
        }
    }

    fn ask(&mut self, topic: u32, intent: Intent) -> Reply {
        let (seeds, _) = crate::generate_with(self.net, self.cfg, topic, self.cfg.search.seeds, patterns(intent));
        let fresh = seeds.into_iter().find(|s| s.check.is_ok() && !self.said.contains(&s.core));
        let (name, num) = self.named(topic);
        let what = match (intent, num) {
            (Intent::About, _) => name.clone(),
            (Intent::WhatIs, Num::Pl) => format!("what {name} are"),
            (Intent::WhatIs, Num::Sg) => format!("what {name} is"),
            (Intent::WhatLike, Num::Pl) => format!("what {name} are like"),
            (Intent::WhatLike, Num::Sg) => format!("what {name} is like"),
            (Intent::WhatDoes, Num::Pl) => format!("what {name} do"),
            (Intent::WhatDoes, Num::Sg) => format!("what {name} does"),
            (Intent::WhatHas, Num::Pl) => format!("what {name} have"),
            (Intent::WhatHas, Num::Sg) => format!("what {name} has"),
        };
        match fresh {
            Some(s) => {
                self.said.insert(s.core.clone());
                self.answered.insert((topic, intent));
                Reply { text: s.text, why: s.cand.why }
            }
            None if self.answered.contains(&(topic, intent)) => Reply::plain(format!("That is all I know about {what}.")),
            None => Reply::plain(format!("I know of nothing about {what}: no line in my network says it.")),
        }
    }

    /// A checked sentence about the topic: subject forms tried in order until
    /// the grammar judges and the parser's check agree.
    fn say(&self, cx: &Ctx, make: impl Fn(Np) -> Pred) -> Option<String> {
        let mass = self.net.node(cx.topic).mass;
        let forms: &[(Det, Num)] = if mass { &[(Det::Bare, Num::Sg), (Det::The, Num::Sg)] } else { &[(Det::Bare, Num::Pl), (Det::A, Num::Sg), (Det::The, Num::Sg)] };
        for &(det, num) in forms {
            let subj = Np { det, num, adj: None, noun: cx.topic };
            let c = Cand { pattern: "answer", subj: Some(subj.clone()), pred: make(subj), parts: vec![], why: vec![] };
            if !grammatical(cx, &c) {
                continue;
            }
            if let Some(text) = search::realize(cx, &c) {
                if crate::parser::check(self.net, self.cfg, &text).is_ok() {
                    return Some(text);
                }
            }
        }
        None
    }

    fn complement(&self, noun: u32, subj: &Np) -> Np {
        if self.net.node(noun).mass {
            Np { det: Det::Bare, num: Num::Sg, adj: None, noun }
        } else if subj.num == Num::Sg {
            Np { det: Det::A, num: Num::Sg, adj: None, noun }
        } else {
            Np { det: Det::Bare, num: Num::Pl, adj: None, noun }
        }
    }

    /// "Yes. ..." with the sentence, or "Not that I know: no line says ...".
    fn verdict(&self, yes: bool, sentence: Option<String>, why: Vec<String>) -> Reply {
        match (yes, sentence) {
            (true, Some(s)) => Reply { text: format!("Yes. {s}"), why },
            (true, None) => Reply { text: "Yes.".into(), why },
            (false, Some(s)) => {
                let mut low = s.trim_end_matches('.').to_string();
                low = low[..1].to_lowercase() + &low[1..];
                Reply::plain(format!("Not that I know: no line in my network says that {low}."))
            }
            (false, None) => Reply::plain("Not that I know: no line in my network says so."),
        }
    }

    /// "Can you have a conversation": after a light verb (have, make, take,
    /// give, do) the object carries the meaning, read as its verb through the
    /// dictionary's derivation line (conversation -> converse, a kind of talk).
    fn me_can(&self, verb: u32, obj: Option<u32>) -> Option<String> {
        let light = matches!(self.net.node(verb).text.as_str(), "have" | "make" | "take" | "give" | "do" | "hold" | "get");
        // "do you have a name": what Primo has
        if let (true, Some(o)) = (self.net.node(verb).text == "have", obj) {
            if let Some(r) = self.me_word(o, &self.cfg.me.has, Pos::Noun) {
                return Some(r);
            }
        }
        if let (true, Some(o)) = (light, obj) {
            for l in &self.net.lines[o as usize] {
                if l.kind == Kind::Derived && self.net.node(l.to).pos == Pos::Verb && l.rank == 0 {
                    if let Some(r) = self.me_word(l.to, &self.cfg.me.can, Pos::Verb) {
                        return Some(r);
                    }
                }
            }
        }
        self.me_word(verb, &self.cfg.me.can, Pos::Verb)
    }

    /// What Primo says about itself for this verb or adjective: the word
    /// itself, or a word of the same meaning (speak for talk), in rules.toml.
    fn me_word(&self, w: u32, table: &std::collections::BTreeMap<String, String>, pos: Pos) -> Option<String> {
        let keys: Vec<(u32, &String)> = table.iter().filter_map(|(k, r)| self.net.base(k, pos).map(|id| (id, r))).collect();
        // a key itself, or a synonym of one in a sense that is the most common
        // for one of the two and common for both ("speak" for "talk"; not
        // "see" for "learn", which share only a rare sense)
        let common = |l: &crate::network::Line| l.rank.min(l.to_rank) == 0 && l.rank.max(l.to_rank) <= 2;
        let matches = |c: u32| {
            keys.iter()
                .find(|(k, _)| *k == c || self.net.lines[c as usize].iter().any(|l| l.kind == Kind::Synonym && l.to == *k && common(l)))
                .map(|(_, r)| (*r).clone())
        };
        if let Some(r) = matches(w) {
            return Some(r);
        }
        // or a kind of one, up all the is-a lines of its most common sense, three
        // steps at most ("converse" is a kind of "talk")
        let mut frontier = vec![(w, 0u8)];
        for _ in 0..3 {
            let mut next = Vec::new();
            for &(node, sense) in &frontier {
                for l in self.net.lines[node as usize].iter().filter(|l| l.kind == Kind::Hypernym && l.rank == sense) {
                    if let Some(r) = matches(l.to) {
                        return Some(r);
                    }
                    next.push((l.to, l.to_rank));
                }
            }
            frontier = next;
        }
        None
    }

    /// "Are you a program?": yes when the noun is one of Primo's kinds or a
    /// kind of them up the is-a lines (a program is software).
    fn you_are(&self, noun: u32) -> Reply {
        for key in &self.cfg.me.kinds {
            let Some(k) = self.net.base(key, Pos::Noun) else { continue };
            if let Some(path) = if k == noun { Some(vec![k]) } else { kind_path(self.net, k, noun) } {
                let word = &self.net.node(noun).text;
                let why = vec![path.iter().map(|&x| self.net.node(x).text.clone()).collect::<Vec<_>>().join(" -is a-> ")];
                let named = if self.net.node(noun).mass {
                    word.clone()
                } else {
                    format!("{} {word}", crate::lexicon::article(word, &self.cfg.article))
                };
                return Reply { text: format!("Yes. I am {named}."), why };
            }
        }
        Reply::plain(self.cfg.me.not_kind.clone())
    }

    /// The first of the topic's kinds (itself first) with lines of this kind:
    /// (the kind, its lines' targets best first, the path to it).
    fn inherited(&self, topic: u32, kind: Kind) -> Option<(u32, Vec<u32>, Vec<String>)> {
        for (k, _) in kinds(self.net, topic, 16) {
            // a kind so general that what is said of it says nothing of the topic
            if self.cfg.chat.inherit_stop.iter().any(|w| w == &self.net.node(k).text) {
                break;
            }
            let mut to: Vec<(u32, u8, u16)> = self.net.lines[k as usize]
                .iter()
                .filter(|l| l.kind == kind && self.net.node(l.to).pos != Pos::Adv)
                .map(|l| (l.to, l.rank, self.net.node(l.to).fam))
                .collect();
            if to.is_empty() {
                continue;
            }
            to.sort_by_key(|&(n, r, f)| (r, std::cmp::Reverse(f), n));
            let path = kind_path(self.net, topic, k)
                .map(|p| vec![p.iter().map(|&x| self.net.node(x).text.clone()).collect::<Vec<_>>().join(" -is a-> ")])
                .unwrap_or_default();
            return Some((k, to.into_iter().map(|x| x.0).collect(), path));
        }
        None
    }

    /// How a noun stands after a preposition or as an object: "water", "trees", "the sea", "Asia".
    fn np_for(&self, n: u32) -> String {
        let node = self.net.node(n);
        if node.mass || node.proper {
            return node.text.clone();
        }
        if matches!(node.text.as_str(), "sea" | "ocean" | "desert" | "arctic" | "tropics" | "ground" | "air" | "sky" | "forest" | "jungle" | "wild") {
            return format!("the {}", node.text);
        }
        match self.net.form(n, Form::Plural) {
            Some(p) if p.sure => p.text.clone(),
            _ => format!("the {}", node.text),
        }
    }

    /// The topic as a sentence's subject, with the verb agreeing: ("Fish", plural).
    fn subject(&self, t: u32) -> (String, Num) {
        let node = self.net.node(t);
        if node.mass {
            return (cap(&node.text), Num::Sg);
        }
        match self.net.form(t, Form::Plural) {
            Some(p) if p.sure => (cap(&p.text), Num::Pl),
            _ => (cap(&format!("{} {}", crate::lexicon::article(&node.text, &self.cfg.article), node.text)), Num::Sg),
        }
    }

    /// A sentence written here, said only if the grammar check reads it.
    fn checked(&self, text: String, why: Vec<String>) -> Option<Reply> {
        crate::parser::check(self.net, self.cfg, &text).is_ok().then_some(Reply { text, why })
    }

    /// "Where do fish live?": the lives-in lines read from the definitions, inherited.
    fn where_live(&mut self, t: u32) -> Reply {
        let (subj, num) = self.subject(t);
        if let Some((k, places, mut why)) = self.inherited(t, Kind::LivesIn) {
            why.insert(0, format!("{} -lives in-> {}", self.net.node(k).text, self.net.node(places[0]).text));
            let verb = if num == Num::Pl { "live" } else { "lives" };
            // "in water", "on land"
            let place = |p: u32| match self.net.node(p).text.as_str() {
                "land" | "ground" => format!("on {}", self.net.node(p).text),
                _ => format!("in {}", self.np_for(p)),
            };
            let mut ps: Vec<String> = places.iter().take(2).map(|&p| place(p)).collect();
            ps.dedup();
            for text in [format!("{subj} {verb} {}.", ps.join(" and ")), format!("{subj} {verb} {}.", ps[0])] {
                if let Some(r) = self.checked(text, why.clone()) {
                    return r;
                }
            }
        }
        Reply::plain(format!("I do not know where {} live: no definition in my dictionary says.", self.subject(t).0.to_lowercase()))
    }

    /// "What is a hammer used for?"
    fn used_for(&mut self, t: u32) -> Reply {
        let (subj, num) = self.subject(t);
        let be = if num == Num::Pl { "are" } else { "is" };
        if let Some((k, uses, mut why)) = self.inherited(t, Kind::UsedFor) {
            for &u in uses.iter().take(3) {
                let what = match self.net.node(u).pos {
                    Pos::Verb => match self.net.form(u, Form::Ing) {
                        Some(g) => g.text.clone(),
                        None => continue,
                    },
                    _ => self.np_for(u),
                };
                let mut w = why.clone();
                w.insert(0, format!("{} -used for-> {}", self.net.node(k).text, self.net.node(u).text));
                if let Some(r) = self.checked(format!("{subj} {be} used for {what}."), w) {
                    return r;
                }
            }
            why.clear();
        }
        Reply::plain(format!("I do not know what {} {be} used for: no definition in my dictionary says.", subj.to_lowercase()))
    }

    /// "What is bread made of?"
    fn made_of(&mut self, t: u32) -> Reply {
        let (subj, num) = self.subject(t);
        let be = if num == Num::Pl { "are" } else { "is" };
        if let Some((k, stuff, why)) = self.inherited(t, Kind::MadeOf) {
            for &m in stuff.iter().take(3) {
                let mut w = why.clone();
                w.insert(0, format!("{} -made of-> {}", self.net.node(k).text, self.net.node(m).text));
                if let Some(r) = self.checked(format!("{subj} {be} made of {}.", self.np_for(m)), w) {
                    return r;
                }
            }
        }
        Reply::plain(format!("I do not know what {} {be} made of: no line in my network says.", subj.to_lowercase()))
    }

    /// "What is the capital of Finland?": a part of the named thing that is of that kind.
    fn part_named(&mut self, kind: u32, of: u32) -> Reply {
        let name = self.net.node(of).text.clone();
        let kind_text = self.net.node(kind).text.clone();
        // the parts of that kind, each by its sense's own main name ("Helsinki", not
        // "Helsingfors", "Finnish capital" or "capital of Finland")
        let mut found: Vec<(u32, Vec<u32>)> = self.net.lines[of as usize]
            .iter()
            .filter(|l| l.kind == Kind::HasPart)
            .filter(|l| {
                let t = &self.net.node(l.to).text;
                !t.contains(" of ") && !t.to_lowercase().contains(&kind_text)
            })
            .filter_map(|l| kind_path(self.net, l.to, kind).map(|p| (l.to, p)))
            .collect();
        found.sort_by_key(|(x, _)| (!self.net.node(*x).head, self.net.node(*x).text.split(' ').count(), *x));
        for (own, path) in found {
            let part = self.net.node(own).text.clone();
            let why = vec![
                format!("{name} -has part-> {part}"),
                path.iter().map(|&x| self.net.node(x).text.clone()).collect::<Vec<_>>().join(" -is a-> "),
            ];
            if let Some(r) = self.checked(format!("The {kind_text} of {name} is {part}."), why) {
                return r;
            }
        }
        Reply::plain(format!("I do not know the {kind_text} of {name}: no line in my network says."))
    }

    /// The dictionary's own definition, quoted as it is.
    fn define(&mut self, t: u32) -> Reply {
        let word = self.net.node(t).text.clone();
        match self.net.def(t) {
            Some(d) => Reply { text: format!("My dictionary says {word} is \"{d}\"."), why: vec![format!("{word} -defined as-> {d}")] },
            None => Reply::plain(format!("My dictionary has no definition of {word}.")),
        }
    }

    /// "What do dogs eat?": the topic (or its kind) does the verb in the
    /// examples, and in that sense the verb is done to these.
    fn object_of(&mut self, topic: u32, verb: u32) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
        // what it eats: read from the definitions ("flesh-eating", "feeds on fish"), inherited
        if matches!(self.net.node(verb).text.as_str(), "eat" | "feed" | "consume" | "devour") {
            if let Some((k, food, mut why)) = self.inherited(topic, Kind::Eats) {
                let (subj, num) = self.subject(topic);
                let v = if num == Num::Pl { "eat" } else { "eats" };
                let mut fs: Vec<String> = food.iter().take(2).map(|&f| self.np_for(f)).collect();
                fs.dedup();
                why.insert(0, format!("{} -eats-> {}", self.net.node(k).text, self.net.node(food[0]).text));
                for text in [format!("{subj} {v} {}.", fs.join(" and ")), format!("{subj} {v} {}.", fs[0])] {
                    if let Some(r) = self.checked(text, why.clone()) {
                        return r;
                    }
                }
            }
        }
        let (name, num) = self.named(topic);
        let vtext = &self.net.node(verb).text;
        let doing = if num == Num::Pl { format!("{name} {vtext}") } else { format!("{name} {}", self.net.form(verb, Form::ThirdSg).map(|n| n.text.as_str()).unwrap_or(vtext)) };
        let mut found = None;
        for (kind, _) in kinds(self.net, topic, 16) {
            if let Some(l) = self.net.lines[kind as usize].iter().find(|l| l.kind == Kind::SubjectOf && l.to == verb) {
                found = Some((kind, l.rank));
                break;
            }
        }
        let Some((by, sense)) = found else {
            return Reply::plain(format!("I know of nothing that {doing}: no line in my network says {name} {vtext} anything."));
        };
        let mut objects: Vec<(u32, f64)> = self.net.lines[verb as usize]
            .iter()
            .filter(|l| l.kind == Kind::HasObject && l.rank == sense && cx.usable(l.to, Pos::Noun))
            .map(|l| (l.to, cx.familiarity(l.to) + 0.5 * cx.meaning(l.to).max(-6.0)))
            .collect();
        objects.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        for (o, _) in objects {
            let key = vec![topic, verb, o];
            if self.said.contains(&key) {
                continue;
            }
            let sentence = self.say(&cx, |_| Pred::Tr { verb, tense: Tense::Present, obj: Some(self.object_np(o)), sense });
            if let Some(s) = sentence {
                self.said.insert(key);
                let why = vec![
                    format!("{} -does-> {}", self.net.node(by).text, vtext),
                    format!("{} -done to-> {}", vtext, self.net.node(o).text),
                ];
                return Reply { text: s, why };
            }
        }
        if self.said.iter().any(|k| k.len() == 3 && k[0] == topic && k[1] == verb) {
            return Reply::plain(format!("That is all I know about what {doing}."));
        }
        Reply::plain(format!("I know that {doing}, but not what: no example in my network says it."))
    }

    fn object_np(&self, o: u32) -> Np {
        if self.net.node(o).mass {
            Np { det: Det::Bare, num: Num::Sg, adj: None, noun: o }
        } else {
            Np { det: Det::Bare, num: Num::Pl, adj: None, noun: o }
        }
    }

    /// "What eats fish?", "Which animals swim?": the words the examples show
    /// doing the verb (to that object, in the same sense), of that kind if asked.
    fn subject_of(&mut self, verb: u32, obj: Option<u32>, kind: Option<u32>) -> Reply {
        let vtext = self.net.node(verb).text.clone();
        let senses: Vec<u8> = match obj {
            Some(o) => self.net.lines[verb as usize].iter().filter(|l| l.kind == Kind::HasObject && l.to == o).map(|l| l.rank).collect(),
            None => Vec::new(),
        };
        let mut doers: Vec<(u32, f64, Vec<String>)> = Vec::new();
        for l in self.net.lines[verb as usize].iter().filter(|l| l.kind == Kind::HasSubject) {
            let s = l.to;
            let n = self.net.node(s);
            if n.pos != Pos::Noun || !n.sure || n.proper || n.text.contains(' ') {
                continue;
            }
            if obj.is_some() && !senses.contains(&l.rank) {
                continue;
            }
            let mut why = vec![format!("{} -does-> {vtext}", n.text)];
            if let Some(k) = kind {
                match kind_path(self.net, s, k) {
                    Some(p) if s != k => why.push(p.iter().map(|&x| self.net.node(x).text.clone()).collect::<Vec<_>>().join(" -is a-> ")),
                    _ => continue,
                }
            }
            if doers.iter().any(|d| d.0 == s) {
                continue;
            }
            doers.push((s, (1.0 + n.fam as f64).ln(), why));
        }
        doers.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        for (s, _, mut why) in doers {
            let key = vec![s, verb, obj.unwrap_or(u32::MAX)];
            if self.said.contains(&key) {
                continue;
            }
            let cx = Ctx::new(self.net, self.cfg, s);
            let sentence = match obj {
                None => self.say(&cx, |_| Pred::Intr { verb, tense: Tense::Present }),
                Some(o) => self.say(&cx, |_| Pred::Tr { verb, tense: Tense::Present, obj: Some(self.object_np(o)), sense: 0 }),
            };
            if let Some(text) = sentence {
                self.said.insert(key);
                self.topic = Some(s);
                if let Some(o) = obj {
                    why.push(format!("{vtext} -done to-> {}", self.net.node(o).text));
                }
                return Reply { text, why };
            }
        }
        if self.said.iter().any(|k| k.len() == 3 && k[1] == verb && k[2] == obj.unwrap_or(u32::MAX)) {
            return Reply::plain("That is all I know.".to_string());
        }
        let what = match (kind, obj) {
            (Some(k), _) => format!("no {} that {vtext}{}", self.plural(k), obj.map(|o| format!(" {}", self.plural(o))).unwrap_or_default()),
            (None, Some(o)) => format!("nothing that {} {}", self.net.form(verb, Form::ThirdSg).map(|n| n.text.clone()).unwrap_or(vtext.clone()), self.plural(o)),
            (None, None) => format!("nothing that {}", self.net.form(verb, Form::ThirdSg).map(|n| n.text.clone()).unwrap_or(vtext.clone())),
        };
        Reply::plain(format!("I know of {what}."))
    }

    fn plural(&self, n: u32) -> String {
        match self.net.form(n, Form::Plural) {
            Some(p) if p.sure && !self.net.node(n).mass => p.text.clone(),
            _ => self.net.node(n).text.clone(),
        }
    }

    /// "What animals have wings?": the wholes the part belongs to, of that kind.
    fn which_have(&mut self, kind: u32, part: u32) -> Reply {
        let mut wholes: Vec<(u32, f64, Vec<String>)> = Vec::new();
        for l in self.net.lines[part as usize].iter().filter(|l| l.kind == Kind::PartOf) {
            let w = l.to;
            let n = self.net.node(w);
            if n.pos != Pos::Noun || !n.sure || n.proper {
                continue;
            }
            let Some(p) = kind_path(self.net, w, kind).or_else(|| (w == kind).then(|| vec![w])) else { continue };
            let why = vec![
                format!("{} -part of-> {}", self.net.node(part).text, n.text),
                p.iter().map(|&x| self.net.node(x).text.clone()).collect::<Vec<_>>().join(" -is a-> "),
            ];
            wholes.push((w, (1.0 + n.fam as f64).ln(), why));
        }
        wholes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        for (w, _, why) in wholes {
            let key = vec![w, part, kind];
            if self.said.contains(&key) {
                continue;
            }
            let cx = Ctx::new(self.net, self.cfg, w);
            if let Some(text) = self.say(&cx, |s| Pred::Have { part, np: Some(self.complement(part, &s)) }) {
                self.said.insert(key);
                self.topic = Some(w);
                return Reply { text, why };
            }
        }
        if self.said.iter().any(|k| k.len() == 3 && k[1] == part && k[2] == kind) {
            return Reply::plain(format!("That is all I know about {} that have {}.", self.plural(kind), self.plural(part)));
        }
        Reply::plain(format!("I know of no {} that have {}.", self.plural(kind), self.plural(part)))
    }

    /// "What kind of animal is a dog?": the step just before the kind on the is-a path.
    fn kind_of(&mut self, topic: u32, kind: u32) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
        let Some(path) = kind_path(self.net, topic, kind) else {
            let (name, num) = self.named(topic);
            let be = if num == Num::Pl { "are" } else { "is" };
            return Reply::plain(format!("No line in my network says that {name} {be} {}.", self.plural(kind)));
        };
        let why = vec![path.iter().map(|&x| self.net.node(x).text.clone()).collect::<Vec<_>>().join(" -is a-> ")];
        // the most telling kind: the one right before the asked kind, else the asked kind itself
        let step = if path.len() >= 3 { path[path.len() - 2] } else { kind };
        match self.say(&cx, |s| Pred::CopNp { noun: step, np: Some(self.complement(step, &s)) }) {
            Some(text) => Reply { text, why },
            None => Reply { text: "Yes.".into(), why },
        }
    }

    /// "How big is a whale?": the adjectives the examples describe the topic
    /// (or its kind) with, by the same measure (WordNet's attribute: size).
    fn how_adj(&mut self, topic: u32, adj: u32) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
        let measures: Vec<u32> = self.net.lines[adj as usize].iter().filter(|l| l.kind == Kind::Attribute).map(|l| l.to).collect();
        let same_measure =
            |a: u32| a == adj || self.net.lines[a as usize].iter().any(|l| l.kind == Kind::Attribute && measures.contains(&l.to));
        let (name, num) = self.named(topic);
        let be = if num == Num::Pl { "are" } else { "is" };
        if measures.is_empty() {
            return Reply::plain(format!("I do not know what \"{}\" measures, so I cannot say how {} {name} {be}.", self.net.node(adj).text, self.net.node(adj).text));
        }
        for (a, _, why) in cx.lined(Kind::ModifiedBy, Pos::Adj, crate::search::predicative) {
            if !same_measure(a) {
                continue;
            }
            if let Some(text) = self.say(&cx, |_| Pred::CopAdj { adj: a, tense: Tense::Present }) {
                let m = self.net.node(measures[0]).text.clone();
                return Reply { text, why: vec![why, format!("{} -attribute-> {m}", self.net.node(adj).text)] };
            }
        }
        Reply::plain(format!("I do not know how {} {name} {be}: no example in my network describes {name} by {}.",
                             self.net.node(adj).text, self.net.node(measures[0]).text))
    }

    fn is_a(&mut self, topic: u32, kind: u32) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
        let path = kind_path(self.net, topic, kind);
        let sentence = self.say(&cx, |s| Pred::CopNp { noun: kind, np: Some(self.complement(kind, &s)) });
        let why = path.iter().map(|p| p.iter().map(|&n| self.net.node(n).text.clone()).collect::<Vec<_>>().join(" -is a-> ")).collect();
        self.verdict(path.is_some(), sentence, why)
    }

    fn is_adj(&mut self, topic: u32, adj: u32) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
        let found = cx.lined(Kind::ModifiedBy, Pos::Adj, |_| true).into_iter().find(|x| x.0 == adj);
        let sentence = self.say(&cx, |_| Pred::CopAdj { adj, tense: Tense::Present });
        let why = found.iter().map(|x| x.2.clone()).collect();
        self.verdict(found.is_some(), sentence, why)
    }

    fn has(&mut self, topic: u32, part: u32) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
        let mut why = Vec::new();
        for (kind, _) in kinds(self.net, topic, 16) {
            if self.net.lines[kind as usize].iter().any(|l| l.kind == Kind::HasPart && l.to == part) {
                why.push(format!("{} -has part-> {}", self.net.node(kind).text, self.net.node(part).text));
                break;
            }
        }
        let sentence = self.say(&cx, |s| Pred::Have { part, np: Some(self.complement(part, &s)) });
        self.verdict(!why.is_empty(), sentence, why)
    }

    fn does(&mut self, topic: u32, verb: u32, obj: Option<u32>) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
        let mut why = Vec::new();
        for (kind, _) in kinds(self.net, topic, 16) {
            if self.net.lines[kind as usize].iter().any(|l| l.kind == Kind::SubjectOf && l.to == verb) {
                why.push(format!("{} -does-> {}", self.net.node(kind).text, self.net.node(verb).text));
                break;
            }
        }
        if let (Some(o), false) = (obj, why.is_empty()) {
            if self.net.lines[verb as usize].iter().any(|l| l.kind == Kind::HasObject && l.to == o) {
                why.push(format!("{} -done to-> {}", self.net.node(verb).text, self.net.node(o).text));
            } else {
                why.clear();
            }
        }
        let sentence = match obj {
            None => self.say(&cx, |_| Pred::Intr { verb, tense: Tense::Present }),
            Some(o) => self.say(&cx, |_| Pred::Tr { verb, tense: Tense::Present, obj: Some(Np { det: Det::Bare, num: Num::Pl, adj: None, noun: o }), sense: 0 }),
        };
        self.verdict(!why.is_empty(), sentence, why)
    }
}

/// The topic and its kinds up the is-a lines, each step on from the sense the
/// last one landed in (most common sense first): (word, sense).
pub fn kinds(net: &Network, topic: u32, depth: usize) -> Vec<(u32, u8)> {
    let mut out = vec![(topic, 0u8)];
    let (mut cur, mut sense) = (topic, 0u8);
    for _ in 0..depth {
        match net.lines[cur as usize].iter().find(|l| l.kind == Kind::Hypernym && l.rank == sense) {
            Some(l) => {
                out.push((l.to, l.to_rank));
                (cur, sense) = (l.to, l.to_rank);
            }
            None => break,
        }
    }
    out
}

/// A path of is-a lines from the topic (any of its three most common senses)
/// to the kind, never jumping senses: dog -> canine -> carnivore -> ... -> animal.
pub fn kind_path(net: &Network, topic: u32, kind: u32) -> Option<Vec<u32>> {
    if net.lines[topic as usize].iter().any(|l| l.kind == Kind::Synonym && l.to == kind && l.rank == 0) {
        return Some(vec![topic, kind]);
    }
    let mut queue: VecDeque<(u32, u8, Vec<u32>)> = (0..3u8).map(|r| (topic, r, vec![topic])).collect();
    let mut seen: HashSet<(u32, u8)> = HashSet::new();
    while let Some((node, sense, path)) = queue.pop_front() {
        if path.len() > 20 || !seen.insert((node, sense)) {
            continue;
        }
        for l in net.lines[node as usize].iter().filter(|l| l.kind == Kind::Hypernym && l.rank == sense) {
            let mut p = path.clone();
            p.push(l.to);
            if l.to == kind {
                return Some(p);
            }
            queue.push_back((l.to, l.to_rank, p));
        }
    }
    None
}
