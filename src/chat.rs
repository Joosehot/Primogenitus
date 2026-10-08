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
    /// the last open question, for "what else"
    last: Option<(u32, Intent)>,
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
        let c = &self.cfg.chat;
        match question::read(self.net, input) {
            Question::Greeting => Reply::plain(c.greeting.clone()),
            Question::Goodbye => Reply::plain(c.goodbye.clone()),
            Question::Thanks => Reply::plain(c.thanks.clone()),
            Question::Help => Reply::plain(c.help.clone()),
            Question::Unknown(why) => Reply::plain(why),
            Question::More => match self.last {
                Some((topic, intent)) => self.ask(topic, intent),
                None => Reply::plain(c.nothing_yet.clone()),
            },
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
        self.last = Some((topic, intent));
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
