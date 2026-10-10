//! Generation: meaning spreads from the input word along the frozen lines;
//! the patterns (src/rules) offer sentence heads from what is reached; the
//! beam fills the slots (subject, tense, object) with the judges scoring and
//! refusing at every step; every seed gives one sentence; the judges score the
//! seeds and the check parses the winner.

use crate::config::Config;
use crate::lexicon;
use crate::network::{Form, Kind, Network, Node, Pos, ATTRIBUTIVE_ONLY, POSTNOMINAL, PREDICATIVE_ONLY};
use crate::rules::{self, judges, Verdict};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Det {
    A,
    The,
    Bare,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Num {
    Sg,
    Pl,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Tense {
    Present,
    Past,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Np {
    pub det: Det,
    pub num: Num,
    pub adj: Option<u32>,
    pub noun: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pred {
    /// "barks"
    Intr { verb: u32, tense: Tense },
    /// "chases a cat"; sense: which sense of the verb the topic does (its objects come from it)
    Tr { verb: u32, tense: Tense, obj: Option<Np>, sense: u8 },
    /// "is loyal"
    CopAdj { adj: u32, tense: Tense },
    /// "is a canine": the complement noun, its phrase once chosen
    CopNp { noun: u32, np: Option<Np> },
    /// "has a tail"
    Have { part: u32, np: Option<Np> },
}

#[derive(Clone, Debug)]
pub struct Cand {
    pub pattern: &'static str,
    pub subj: Option<Np>,
    pub pred: Pred,
    /// (what, how much): every term of the score
    pub parts: Vec<(String, f64)>,
    /// the lines the sentence stands on
    pub why: Vec<String>,
}

impl Cand {
    pub fn score(&self) -> f64 {
        self.parts.iter().map(|p| p.1).sum()
    }

    pub fn complete(&self) -> bool {
        self.subj.is_some()
            && match &self.pred {
                Pred::Tr { obj, .. } => obj.is_some(),
                Pred::CopNp { np, .. } | Pred::Have { np, .. } => np.is_some(),
                _ => true,
            }
    }
}

pub struct Ctx<'a> {
    pub net: &'a Network,
    pub cfg: &'a Config,
    pub topic: u32,
    pub act: Vec<f32>,
    parent: Vec<(u32, Kind)>,
    best: HashMap<Pos, f32>,
    pub seed: u64,
    /// how many choices a slot offers (the generator's few; MBLM's whole distribution)
    pub limit: usize,
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

impl<'a> Ctx<'a> {
    /// Meaning spreads from the topic: every hop passes a node's activation
    /// along its lines, weighted by the line's kind, the sense it came from
    /// and how many lines the node has.
    pub fn new(net: &'a Network, cfg: &'a Config, topic: u32) -> Ctx<'a> {
        let n = net.nodes.len();
        let mut act = vec![0f32; n];
        let mut parent = vec![(u32::MAX, Kind::FormOf); n];
        let mut contrib = vec![0f32; n];
        act[topic as usize] = 1.0;
        let mut frontier = vec![(topic, 1.0f32)];
        for _ in 0..cfg.spread.hops {
            let mut next: HashMap<u32, f32> = HashMap::new();
            for &(from, a) in &frontier {
                let ls = &net.lines[from as usize];
                let deg = (ls.len().max(1) as f32).powf(cfg.spread.degree_power);
                for l in ls {
                    if l.kind == Kind::FormOf {
                        continue;
                    }
                    let v = a * cfg.line_weight(l.kind) * cfg.spread.rank_decay.powi(l.rank as i32) / deg;
                    if v <= 0.0 {
                        continue;
                    }
                    *next.entry(l.to).or_default() += v;
                    if v > contrib[l.to as usize] && l.to != topic {
                        contrib[l.to as usize] = v;
                        if parent[l.to as usize].0 == u32::MAX || act[l.to as usize] == 0.0 {
                            parent[l.to as usize] = (from, l.kind);
                        }
                    }
                }
            }
            let mut nx: Vec<(u32, f32)> = next.into_iter().collect();
            nx.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
            nx.truncate(cfg.spread.keep);
            for &(m, v) in &nx {
                act[m as usize] += v;
            }
            frontier = nx;
        }
        let mut best: HashMap<Pos, f32> = HashMap::new();
        for (i, &a) in act.iter().enumerate() {
            if i as u32 == topic {
                continue;
            }
            let e = best.entry(net.nodes[i].pos).or_default();
            *e = e.max(a);
        }
        Ctx { net, cfg, topic, act, parent, best, seed: 0, limit: cfg.search.candidates }
    }

    pub fn node(&self, i: u32) -> &Node {
        self.net.node(i)
    }

    /// A seed's shake of a word's score (none for seed 0); kept as its own
    /// score term so the judges can leave it out when they pick among seeds.
    pub fn jit(&self, i: u32) -> f64 {
        if self.seed == 0 {
            return 0.0;
        }
        let h = splitmix(self.seed.wrapping_mul(0x100000001B3) ^ i as u64);
        self.cfg.search.jitter * ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0)
    }

    /// How strongly the topic reaches this word (log, 0 = the strongest of its kind).
    pub fn meaning(&self, i: u32) -> f64 {
        let n = self.node(i);
        let best = *self.best.get(&n.pos).unwrap_or(&1.0) as f64;
        let a = self.act[i as usize] as f64;
        self.cfg.score.meaning * (a.max(1e-9) / best.max(1e-9)).ln().max(-15.0)
    }

    pub fn familiarity(&self, i: u32) -> f64 {
        let f = self.node(i).fam as f64;
        self.cfg.score.familiarity * (1.0 + f).ln() + if f == 0.0 { self.cfg.score.rare } else { 0.0 }
    }

    /// The path the meaning took from the topic to this word.
    pub fn path(&self, i: u32) -> String {
        let mut steps = vec![self.node(i).text.clone()];
        let mut cur = i;
        for _ in 0..self.cfg.spread.hops {
            let (p, k) = self.parent[cur as usize];
            if p == u32::MAX {
                break;
            }
            steps.push(format!(" -{}-> ", k.name()));
            steps.push(self.node(p).text.clone());
            cur = p;
            if p == self.topic {
                break;
            }
        }
        steps.reverse();
        steps.concat()
    }

    /// A word the generator may use: a sure, plain, single base word of this kind.
    pub fn usable(&self, i: u32, pos: Pos) -> bool {
        let n = self.node(i);
        n.pos == pos && n.form == Form::Base && n.sure && !n.proper && i != self.topic && (pos == Pos::Noun || !n.text.contains(' '))
    }

    /// The best-reached usable words of a kind, filtered: (word, meaning + familiarity).
    pub fn reached(&self, pos: Pos, keep: impl Fn(&Node) -> bool, k: usize) -> Vec<(u32, f64)> {
        let mut c: Vec<(u32, f64)> = (0..self.act.len() as u32)
            .filter(|&i| self.act[i as usize] > 0.0 && self.usable(i, pos) && keep(self.node(i)))
            .map(|i| (i, self.meaning(i) + self.familiarity(i)))
            .collect();
        c.sort_by(|a, b| (b.1 + self.jit(b.0)).partial_cmp(&(a.1 + self.jit(a.0))).unwrap().then(a.0.cmp(&b.0)));
        c.truncate(k);
        c
    }

    /// The topic and what it is: its other names in its most common sense, and
    /// its kinds up the is-a lines (dog, domestic dog, canine, carnivore):
    /// (word, how many steps up).
    pub fn chain(&self) -> Vec<(u32, usize)> {
        let mut out = vec![(self.topic, 0)];
        for l in &self.net.lines[self.topic as usize] {
            if l.kind == Kind::Synonym && l.rank == 0 {
                out.push((l.to, 0));
            }
        }
        let (mut cur, mut sense) = (self.topic, 0u8);
        for depth in 1..=3 {
            match self.net.lines[cur as usize].iter().find(|l| l.kind == Kind::Hypernym && l.rank == sense) {
                Some(l) => {
                    out.push((l.to, depth));
                    (cur, sense) = (l.to, l.to_rank);
                }
                None => break,
            }
        }
        out
    }

    /// Words the topic's chain has a `kind` line to (verbs it does, adjectives
    /// that describe it): (word, score, the line), the nearest link counting.
    pub fn lined(&self, kind: Kind, pos: Pos, keep: impl Fn(&Node) -> bool) -> Vec<(u32, f64, String)> {
        self.lined_ranks(kind, pos, keep).into_iter().map(|(w, s, why, _)| (w, s, why)).collect()
    }

    /// As `lined`, with the sense each line came from.
    pub fn lined_ranks(&self, kind: Kind, pos: Pos, keep: impl Fn(&Node) -> bool) -> Vec<(u32, f64, String, u8)> {
        let mut out: Vec<(u32, f64, String, u8)> = Vec::new();
        for (from, depth) in self.chain() {
            for l in &self.net.lines[from as usize] {
                if l.kind != kind || !self.usable(l.to, pos) || !keep(self.node(l.to)) {
                    continue;
                }
                let s = self.cfg.score.hop2 * depth as f64 + self.meaning(l.to).max(-6.0) * 0.5 + self.familiarity(l.to);
                let why = format!("{} -{}-> {}", self.node(from).text, kind.name(), self.node(l.to).text);
                match out.iter_mut().find(|x| x.0 == l.to) {
                    Some(x) if x.1 >= s => {}
                    Some(x) => *x = (l.to, s, why, l.rank),
                    None => out.push((l.to, s, why, l.rank)),
                }
            }
        }
        out.sort_by(|a, b| (b.1 + self.jit(b.0)).partial_cmp(&(a.1 + self.jit(a.0))).unwrap().then(a.0.cmp(&b.0)));
        out.truncate(self.limit);
        out
    }

    pub fn has_line(&self, from: u32, kind: Kind, to: u32) -> bool {
        self.net.lines[from as usize].iter().any(|l| l.kind == kind && l.to == to)
    }
}

// ---------------------------------------------------------------- realization

pub fn noun_text(cx: &Ctx, np: &Np) -> Option<String> {
    let form = if np.num == Num::Sg { Form::Base } else { Form::Plural };
    let n = cx.net.form(np.noun, form)?;
    if !n.sure {
        return None;
    }
    let head = match np.adj {
        Some(a) => format!("{} {}", cx.node(a).text, n.text),
        None => n.text.clone(),
    };
    Some(match np.det {
        Det::A => format!("{} {head}", lexicon::article(&head, &cx.cfg.article)),
        Det::The => format!("the {head}"),
        Det::Bare => head,
    })
}

pub fn verb_text(cx: &Ctx, verb: u32, tense: Tense, num: Num) -> Option<String> {
    let form = match (tense, num) {
        (Tense::Past, _) => Form::Past,
        (Tense::Present, Num::Sg) => Form::ThirdSg,
        (Tense::Present, Num::Pl) => Form::Base,
    };
    let n = cx.net.form(verb, form)?;
    n.sure.then(|| n.text.clone())
}

fn be(tense: Tense, num: Num) -> &'static str {
    match (tense, num) {
        (Tense::Present, Num::Sg) => "is",
        (Tense::Present, Num::Pl) => "are",
        (Tense::Past, Num::Sg) => "was",
        (Tense::Past, Num::Pl) => "were",
    }
}

/// The sentence's words, or None when a form it needs is unknown or unsure.
pub fn realize(cx: &Ctx, c: &Cand) -> Option<String> {
    let s = c.subj.as_ref()?;
    let subj = noun_text(cx, s)?;
    let pred = match &c.pred {
        Pred::Intr { verb, tense } => verb_text(cx, *verb, *tense, s.num)?,
        Pred::Tr { verb, tense, obj, .. } => format!("{} {}", verb_text(cx, *verb, *tense, s.num)?, noun_text(cx, obj.as_ref()?)?),
        Pred::CopAdj { adj, tense } => format!("{} {}", be(*tense, s.num), cx.node(*adj).text),
        Pred::CopNp { np, .. } => format!("{} {}", be(Tense::Present, s.num), noun_text(cx, np.as_ref()?)?),
        Pred::Have { np, .. } => format!("{} {}", if s.num == Num::Sg { "has" } else { "have" }, noun_text(cx, np.as_ref()?)?),
    };
    let text = format!("{subj} {pred}.");
    let mut ch = text.chars();
    let first = ch.next()?.to_uppercase().collect::<String>();
    Some(first + ch.as_str())
}

// ---------------------------------------------------------------- the beam

fn judged(cx: &Ctx, mut c: Cand) -> Option<Cand> {
    c.parts.retain(|p| !p.0.starts_with("judge "));
    for (name, j) in judges::ALL {
        match j(cx, &c) {
            Verdict::Ok(0.0) => {}
            Verdict::Ok(p) => c.parts.push((format!("judge {name}"), p)),
            Verdict::Refuse(_) => return None,
        }
    }
    Some(c)
}

fn prune(mut v: Vec<Cand>, width: usize) -> Vec<Cand> {
    v.sort_by(|a, b| b.score().partial_cmp(&a.score()).unwrap().then_with(|| format!("{:?}{:?}", a.subj, a.pred).cmp(&format!("{:?}{:?}", b.subj, b.pred))));
    v.truncate(width);
    v
}

/// The subject's phrase and the tense.
fn subjects(cx: &Ctx, c: &Cand) -> Vec<Cand> {
    let generic = matches!(c.pred, Pred::CopNp { .. } | Pred::Have { .. });
    let tenses: &[Tense] = match c.pred {
        Pred::Intr { .. } | Pred::Tr { .. } | Pred::CopAdj { .. } => &[Tense::Present, Tense::Past],
        _ => &[Tense::Present],
    };
    let mut adjs: Vec<(Option<u32>, f64, String)> = vec![(None, 0.0, String::new())];
    // "is fresh" says the adjective; a second one in the subject could contradict it
    if !generic && !matches!(c.pred, Pred::CopAdj { .. }) {
        for (a, s, why) in cx.lined(Kind::ModifiedBy, Pos::Adj, |n| n.marker != PREDICATIVE_ONLY && n.marker != POSTNOMINAL)
            .into_iter()
            .take(cx.cfg.search.subject_adjectives)
        {
            adjs.push((Some(a), s + cx.cfg.score.adjective, why));
        }
    }
    let mut out = Vec::new();
    let mass = cx.node(cx.topic).mass;
    let forms: &[(Det, Num)] = if mass {
        &[(Det::Bare, Num::Sg), (Det::The, Num::Sg)]
    } else {
        &[(Det::The, Num::Sg), (Det::A, Num::Sg), (Det::Bare, Num::Pl), (Det::The, Num::Pl)]
    };
    for &(det, num) in forms {
        // a general truth is said of all of them ("a dog is", "dogs are", "bread is")
        if generic && det == Det::The {
            continue;
        }
        for &tense in tenses {
            for (adj, s, why) in &adjs {
                let (adj, s) = (*adj, *s);
                let mut n = c.clone();
                n.subj = Some(Np { det, num, adj, noun: cx.topic });
                match &mut n.pred {
                    Pred::Intr { tense: t, .. } | Pred::Tr { tense: t, .. } | Pred::CopAdj { tense: t, .. } => *t = tense,
                    _ => {}
                }
                if tense == Tense::Past {
                    n.parts.push(("past".into(), cx.cfg.score.past));
                }
                if det == Det::A && !generic {
                    n.parts.push(("generic a".into(), cx.cfg.score.generic_a));
                }
                if let Some(a) = adj {
                    n.parts.push((format!("adjective {}", cx.node(a).text), s));
                    n.parts.push(("seed".into(), cx.jit(a)));
                    n.why.push(why.clone());
                }
                out.push(n);
            }
        }
    }
    out
}

/// The object's (or the complement's) phrase.
fn objects(cx: &Ctx, c: &Cand) -> Vec<Cand> {
    let sn = c.subj.as_ref().map(|s| s.num).unwrap_or(Num::Sg);
    let mut out = Vec::new();
    match &c.pred {
        Pred::Tr { verb, tense, obj: None, sense } => {
            let (verb, sense) = (*verb, *sense);
            // the objects the verb has in the parsed examples (its own, or a more
            // general verb's), the topic's closeness to each a bonus
            let mut sources = vec![(verb, 0.0, Some(sense))];
            for l in &cx.net.lines[verb as usize] {
                if l.kind == Kind::Hypernym && l.rank == sense {
                    sources.push((l.to, cx.cfg.score.hop2, None));
                }
            }
            let mut nouns: Vec<(u32, f64, u32)> = Vec::new();
            for &(src, cost, want) in &sources {
                for l in &cx.net.lines[src as usize] {
                    let same_sense = want.map(|r| l.rank == r).unwrap_or(l.rank == 0);
                    if l.kind == Kind::HasObject && same_sense && cx.usable(l.to, Pos::Noun) && !nouns.iter().any(|x| x.0 == l.to) {
                        let s = cost + cx.familiarity(l.to) + 0.5 * cx.meaning(l.to).max(-6.0);
                        nouns.push((l.to, s, src));
                    }
                }
            }
            nouns.sort_by(|a, b| (b.1 + cx.jit(b.0)).partial_cmp(&(a.1 + cx.jit(a.0))).unwrap().then(a.0.cmp(&b.0)));
            nouns.truncate(cx.cfg.search.candidates);
            for (o, s, src) in nouns {
                let line = true;
                let dets: &[(Det, Num)] = if cx.node(o).mass {
                    &[(Det::Bare, Num::Sg), (Det::The, Num::Sg)]
                } else {
                    &[(Det::A, Num::Sg), (Det::The, Num::Sg), (Det::Bare, Num::Pl), (Det::The, Num::Pl)]
                };
                for &(det, num) in dets {
                    let mut n = c.clone();
                    n.pred = Pred::Tr { verb, tense: *tense, obj: Some(Np { det, num, adj: None, noun: o }), sense };
                    n.parts.push((format!("object {}", cx.node(o).text), s));
                    n.parts.push(("seed".into(), cx.jit(o)));
                    if line {
                        n.parts.push(("object line".into(), cx.cfg.score.object_line));
                        n.why.push(format!("{} -{}-> {}", cx.node(src).text, Kind::HasObject.name(), cx.node(o).text));
                    }
                    out.push(n);
                }
            }
        }
        Pred::CopNp { noun, np: None } => {
            let (det, num) = if cx.node(*noun).mass {
                (Det::Bare, Num::Sg)
            } else if sn == Num::Sg {
                (Det::A, Num::Sg)
            } else {
                (Det::Bare, Num::Pl)
            };
            let mut n = c.clone();
            n.pred = Pred::CopNp { noun: *noun, np: Some(Np { det, num, adj: None, noun: *noun }) };
            out.push(n);
        }
        Pred::Have { part, np: None } => {
            let dets: &[(Det, Num)] = if cx.node(*part).mass { &[(Det::Bare, Num::Sg)] } else { &[(Det::A, Num::Sg), (Det::Bare, Num::Pl)] };
            for &(det, num) in dets {
                let mut n = c.clone();
                n.pred = Pred::Have { part: *part, np: Some(Np { det, num, adj: None, noun: *part }) };
                out.push(n);
            }
        }
        _ => out.push(c.clone()),
    }
    out
}

/// One seed: the beam from the allowed patterns' heads to whole sentences.
pub fn beam(cx: &Ctx, patterns: &[&str]) -> Vec<Cand> {
    let width = cx.cfg.search.beam;
    let mut states: Vec<Cand> = Vec::new();
    for p in rules::ALL.iter().filter(|p| patterns.contains(&p.name())) {
        states.extend(p.heads(cx).into_iter().filter_map(|c| judged(cx, c)));
    }
    let states = prune(states, width * 4);
    let states = prune(states.iter().flat_map(|c| subjects(cx, c)).filter_map(|c| judged(cx, c)).collect(), width);
    let done = prune(states.iter().flat_map(|c| objects(cx, c)).filter_map(|c| judged(cx, c)).collect(), width);
    done.into_iter().filter(|c| c.complete()).collect()
}

/// Keep only what an adjective may do in its place.
pub fn predicative(n: &Node) -> bool {
    n.marker != ATTRIBUTIVE_ONLY && n.marker != POSTNOMINAL
}
