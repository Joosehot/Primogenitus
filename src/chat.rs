//! The chatbot: a question read whole (question.rs), answered from the frozen
//! network by the same pipeline, with a memory of the conversation: what was
//! talked about last ("it", "they") and what has been said already ("what else").
//!
//! Every sentence the bot says about the world passes the judges' grammar and
//! the parser's check. A question the network has no line for gets that answer
//! ("I know of nothing that ..."), never a guess.

use crate::config::Config;
use crate::network::{Form, Kind, Network, Pos};
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
            Question::HowAreYou => Reply::plain(c.how_am_i.clone()),
            Question::YouCan(v) => Reply::plain(self.me_word(v, &self.cfg.me.can, Pos::Verb).unwrap_or_else(|| self.cfg.me.cannot.clone())),
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
            return (format!("the {}", n.text), Num::Sg);
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
        let forms: &[(Det, Num)] = if mass { &[(Det::The, Num::Sg)] } else { &[(Det::Bare, Num::Pl), (Det::A, Num::Sg), (Det::The, Num::Sg)] };
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
            Np { det: Det::The, num: Num::Sg, adj: None, noun }
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

    /// What Primo says about itself for this verb or adjective: the word
    /// itself, or a word of the same meaning (speak for talk), in rules.toml.
    fn me_word(&self, w: u32, table: &std::collections::BTreeMap<String, String>, pos: Pos) -> Option<String> {
        let keys: Vec<(u32, &String)> = table.iter().filter_map(|(k, r)| self.net.base(k, pos).map(|id| (id, r))).collect();
        if let Some((_, r)) = keys.iter().find(|(k, _)| *k == w) {
            return Some((*r).clone());
        }
        // a synonym in a sense that is the most common for one of the two and
        // common for both ("speak" for "talk"; not "see" for "learn", which
        // share only a rare sense)
        let common = |l: &crate::network::Line| l.rank.min(l.to_rank) == 0 && l.rank.max(l.to_rank) <= 2;
        keys.iter()
            .find(|(k, _)| self.net.lines[w as usize].iter().any(|l| l.kind == Kind::Synonym && l.to == *k && common(l)))
            .map(|(_, r)| (*r).clone())
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

    /// "What do dogs eat?": the topic (or its kind) does the verb in the
    /// examples, and in that sense the verb is done to these.
    fn object_of(&mut self, topic: u32, verb: u32) -> Reply {
        let cx = Ctx::new(self.net, self.cfg, topic);
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
            Np { det: Det::The, num: Num::Sg, adj: None, noun: o }
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
