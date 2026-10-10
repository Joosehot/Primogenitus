//! MBLM, a motor based language model: Primo as a language model, word by
//! word, with nothing trained.
//!
//! A language model gives, for any text so far, a distribution over the next
//! word, and continues text by choosing from it. Here the distribution comes
//! from the motor:
//!
//!   the grammar   a word-level automaton of Primo's sentence grammar says
//!                 which words may come next at all (a determiner, an
//!                 adjective, the noun, a verb in the form that agrees, "is",
//!                 "has", a full stop); everything else has probability 0
//!   the meaning   the frozen network's lines say which of those words fit
//!                 the topic (what it is, does, has, is like; what that verb
//!                 is done to), with the familiarity of each word
//!   the judges    a word already said in the paragraph is held back
//!
//! The scores become probabilities by a softmax (temperature in rules.toml);
//! every probability comes with the line it stands on. `complete` continues a
//! prompt by beam search, sentence by sentence, about one topic; `next` gives
//! the distribution of the next word; `score` gives every word of a text its
//! probability, the text's perplexity, and how much of it lies inside the
//! motor at all (coverage), honestly. `questions` is the model asking: every
//! question of the question grammar it could ask about the person or a thing
//! of theirs, filled from the lines, each with its probability; Raven's
//! follow-up questions are these, not sentences written down beforehand.

use crate::config::Config;
use crate::lexicon;
use crate::network::{Form, Kind, Network, Pos};
use crate::parser::{INTRANSITIVE, TRANSITIVE};
use crate::rules::is_a::along;
use crate::search::{predicative, Ctx, Num};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum D {
    A,
    An,
    The,
}

/// Where the sentence is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum St {
    /// a new sentence
    Start,
    /// after the subject's determiner
    Det(D),
    /// after the subject's adjective
    Adj(D),
    /// after the subject
    Subj(Num),
    /// after "is" / "are"
    Cop(Num),
    /// after "is a"
    CopDet(D),
    /// after "has" / "have"
    Have(Num),
    /// after a verb: the verb, the sense the topic does it in, whether it may end the sentence
    Verb { verb: u32, sense: u8, intr: bool },
    /// after an object's determiner (of a verb, or of "has")
    ObjDet { det: D, verb: Option<(u32, u8)> },
    /// the sentence is whole: only a full stop
    Done,
}

/// A word that may come next.
#[derive(Clone, Debug)]
pub struct Tok {
    pub text: String,
    pub next: St,
    pub score: f64,
    /// the line (or the rule) it stands on
    pub why: String,
    /// the content word it is (for "said already")
    pub content: Option<u32>,
    /// the probability, once the candidates are weighed together
    pub p: f64,
}

/// A paragraph about one topic, as it goes.
pub struct Para<'a> {
    pub cx: Ctx<'a>,
    /// a person is not "it": the noun is said again instead
    pub person: bool,
    /// the number the topic was first named in (the pronoun follows it)
    pub num: Option<Num>,
    pub sentences: usize,
    pub used: HashSet<u32>,
}

fn det_text(d: D) -> &'static str {
    match d {
        D::A => "a",
        D::An => "an",
        D::The => "the",
    }
}

pub struct Mblm<'a> {
    pub net: &'a Network,
    pub cfg: &'a Config,
}

impl<'a> Mblm<'a> {
    pub fn new(net: &'a Network, cfg: &'a Config) -> Mblm<'a> {
        Mblm { net, cfg }
    }

    pub fn para(&self, topic: u32) -> Para<'a> {
        let mut cx = Ctx::new(self.net, self.cfg, topic);
        cx.limit = self.cfg.mblm.candidates;
        // a person in the word's most common sense ("dog" is also a dull person, rarely)
        let person = self
            .net
            .base("person", Pos::Noun)
            .map(|k| {
                crate::chat::kinds(self.net, topic, 16).iter().any(|&(n, _)| {
                    // the kind may be named by another word of its sense ("individual")
                    n == k || self.net.lines[n as usize].iter().any(|l| l.kind == Kind::Synonym && l.to == k)
                })
            })
            .unwrap_or(false);
        Para { cx, person, num: None, sentences: 0, used: HashSet::new() }
    }

    /// A verb that says almost nothing on its own ("act", "make", "get").
    fn light(&self, v: u32) -> bool {
        self.cfg.mblm.light_verbs.iter().any(|w| w == &self.net.node(v).text)
    }

    fn text(&self, n: u32) -> &str {
        &self.net.node(n).text
    }

    fn sure_form(&self, n: u32, f: Form) -> Option<String> {
        self.net.form(n, f).filter(|x| x.sure).map(|x| x.text.clone())
    }

    /// "a" or "an" for this word.
    fn det_for(&self, word: &str) -> D {
        if lexicon::article(word, &self.cfg.article) == "an" {
            D::An
        } else {
            D::A
        }
    }

    /// A noun as a sentence goes on after it: singular after a/an, plural or
    /// mass bare, either after "the".
    fn noun_toks(&self, list: &[(u32, f64, String)], det: Option<D>, next: St, p: &Para) -> Vec<Tok> {
        let mut out = Vec::new();
        for (n, s, why) in list {
            let node = self.net.node(*n);
            let base = node.text.clone();
            let plural = self.sure_form(*n, Form::Plural).filter(|_| !node.mass);
            let forms: Vec<String> = match det {
                Some(D::A) | Some(D::An) if !node.mass && self.det_for(&base) == det.unwrap() => vec![base],
                Some(D::A) | Some(D::An) => vec![],
                Some(D::The) => std::iter::once(base).chain(plural).collect(),
                None if node.mass => vec![base],
                None => plural.into_iter().collect(),
            };
            for f in forms {
                out.push(self.tok(f, next, *s, why.clone(), Some(*n), p));
            }
        }
        out
    }

    fn tok(&self, text: String, next: St, score: f64, why: String, content: Option<u32>, p: &Para) -> Tok {
        let mut score = score;
        if let Some(c) = content {
            let opposite = self.net.lines[c as usize].iter().any(|l| l.kind == Kind::Antonym && p.used.contains(&l.to));
            if (p.used.contains(&c) && c != p.cx.topic) || opposite {
                score += self.cfg.mblm.said;
            }
        }
        Tok { text, next, score, why, content, p: 0.0 }
    }

    /// "a" / "an" before a list of singular nouns: as likely as the best noun it leads to.
    fn det_toks(&self, list: &[(u32, f64, String)], next: impl Fn(D) -> St, p: &Para) -> Vec<Tok> {
        let mut out = Vec::new();
        for d in [D::A, D::An] {
            let best = list
                .iter()
                .filter(|(n, _, _)| !self.net.node(*n).mass && self.det_for(self.text(*n)) == d)
                .map(|(n, s, _)| s + if p.used.contains(n) { self.cfg.mblm.said } else { 0.0 })
                .fold(f64::NEG_INFINITY, f64::max);
            if best.is_finite() {
                out.push(Tok { text: det_text(d).into(), next: next(d), score: best, why: "the best noun it leads to".into(), content: None, p: 0.0 });
            }
        }
        out
    }

    /// What the topic is (one step or two along the is-a lines).
    fn kinds(&self, p: &Para) -> Vec<(u32, f64, String)> {
        along(&p.cx, Kind::Hypernym)
    }

    /// The topic's parts.
    fn parts(&self, p: &Para) -> Vec<(u32, f64, String)> {
        along(&p.cx, Kind::HasPart)
    }

    /// What the verb, in the sense the topic does it in, is done to.
    fn objects(&self, verb: u32, sense: u8, p: &Para) -> Vec<(u32, f64, String)> {
        let cx = &p.cx;
        let mut out: Vec<(u32, f64, String)> = Vec::new();
        for l in &self.net.lines[verb as usize] {
            if l.kind == Kind::HasObject && l.rank == sense && cx.usable(l.to, Pos::Noun) && !out.iter().any(|x| x.0 == l.to) {
                out.push((l.to, cx.familiarity(l.to) + 0.5 * cx.meaning(l.to).max(-6.0), format!("{} -done to-> {}", self.text(verb), self.text(l.to))));
            }
        }
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        out.truncate(p.cx.limit);
        out
    }

    /// Every word that may come next, scored, with its probability.
    pub fn next(&self, st: St, p: &Para) -> Vec<Tok> {
        let pr = &self.cfg.mblm;
        let topic = p.cx.topic;
        let t = self.net.node(topic);
        let mut out: Vec<Tok> = Vec::new();
        let base = t.text.clone();
        match st {
            St::Start => {
                if p.sentences > 0 && !(p.person && p.num != Some(Num::Pl)) {
                    let (pron, num) = match p.num {
                        Some(Num::Pl) => ("they", Num::Pl),
                        _ => ("it", Num::Sg),
                    };
                    out.push(Tok { text: pron.into(), next: St::Subj(num), score: pr.pronoun, why: format!("{pron} = {base}"), content: None, p: 0.0 });
                }
                let again = if p.sentences > 0 { pr.noun_again } else { 0.0 };
                if t.mass {
                    out.push(Tok { text: base.clone(), next: St::Subj(Num::Sg), score: pr.bare + again, why: "the topic".into(), content: None, p: 0.0 });
                } else {
                    let d = self.det_for(&base);
                    out.push(Tok { text: det_text(d).into(), next: St::Det(d), score: pr.a + again, why: "the topic, one of them".into(), content: None, p: 0.0 });
                    if let Some(pl) = self.sure_form(topic, Form::Plural) {
                        out.push(Tok { text: pl, next: St::Subj(Num::Pl), score: pr.bare + again, why: "the topic, all of them".into(), content: None, p: 0.0 });
                    }
                }
                out.push(Tok { text: "the".into(), next: St::Det(D::The), score: pr.the + again, why: "the topic, that one".into(), content: None, p: 0.0 });
            }
            St::Det(d) => {
                // the topic itself, or first an adjective the examples describe it with
                let me = vec![(topic, 0.0, "the topic".to_string())];
                for tk in self.noun_toks(&me, Some(d), St::Subj(Num::Sg), p) {
                    out.push(tk);
                }
                if d == D::The {
                    if let Some(pl) = self.sure_form(topic, Form::Plural).filter(|_| !t.mass) {
                        out.push(Tok { text: pl, next: St::Subj(Num::Pl), score: 0.0, why: "the topic".into(), content: None, p: 0.0 });
                    }
                }
                for (a, s, why) in p.cx.lined(Kind::ModifiedBy, Pos::Adj, |n| n.marker != crate::network::PREDICATIVE_ONLY) {
                    let word = self.text(a).to_string();
                    if d != D::The && self.det_for(&word) != d {
                        continue;
                    }
                    out.push(self.tok(word, St::Adj(d), s + pr.adjective, why, Some(a), p));
                }
            }
            St::Adj(d) => {
                out.push(Tok { text: base.clone(), next: St::Subj(Num::Sg), score: 0.0, why: "the topic".into(), content: None, p: 0.0 });
                if d == D::The {
                    if let Some(pl) = self.sure_form(topic, Form::Plural).filter(|_| !t.mass) {
                        out.push(Tok { text: pl, next: St::Subj(Num::Pl), score: 0.0, why: "the topic".into(), content: None, p: 0.0 });
                    }
                }
            }
            St::Subj(num) => {
                let (be, have) = if num == Num::Sg { ("is", "has") } else { ("are", "have") };
                let kinds = self.kinds(p);
                let adjs = p.cx.lined(Kind::ModifiedBy, Pos::Adj, predicative);
                let best_cop = kinds.iter().map(|x| x.1).chain(adjs.iter().map(|x| x.1)).fold(f64::NEG_INFINITY, f64::max);
                if best_cop.is_finite() {
                    out.push(Tok { text: be.into(), next: St::Cop(num), score: best_cop + pr.is, why: "what it is, what it is like".into(), content: None, p: 0.0 });
                }
                let parts = self.parts(p);
                if let Some(best) = parts.iter().map(|x| x.1).reduce(f64::max) {
                    out.push(Tok { text: have.into(), next: St::Have(num), score: best + pr.has, why: "what it has".into(), content: None, p: 0.0 });
                }
                let form = if num == Num::Sg { Form::ThirdSg } else { Form::Base };
                for (v, s, why, sense) in p.cx.lined_ranks(Kind::SubjectOf, Pos::Verb, |n| n.frames & (INTRANSITIVE | TRANSITIVE) != 0) {
                    if self.light(v) {
                        continue;
                    }
                    let Some(word) = self.sure_form(v, form) else { continue };
                    let intr = self.net.node(v).frames & INTRANSITIVE != 0;
                    out.push(self.tok(word, St::Verb { verb: v, sense, intr }, s + pr.does, why, Some(v), p));
                }
            }
            St::Cop(num) => {
                for (a, s, why) in p.cx.lined(Kind::ModifiedBy, Pos::Adj, predicative) {
                    out.push(self.tok(self.text(a).to_string(), St::Done, s, why, Some(a), p));
                }
                let kinds = self.kinds(p);
                if num == Num::Sg {
                    out.extend(self.det_toks(&kinds, St::CopDet, p));
                    let mass: Vec<_> = kinds.iter().filter(|x| self.net.node(x.0).mass).cloned().collect();
                    out.extend(self.noun_toks(&mass, None, St::Done, p));
                } else {
                    out.extend(self.noun_toks(&kinds, None, St::Done, p));
                }
            }
            St::CopDet(d) => {
                let kinds = self.kinds(p);
                out.extend(self.noun_toks(&kinds, Some(d), St::Done, p));
            }
            St::Have(_) => {
                let parts = self.parts(p);
                out.extend(self.det_toks(&parts, |d| St::ObjDet { det: d, verb: None }, p));
                out.extend(self.noun_toks(&parts, None, St::Done, p));
            }
            St::Verb { verb, sense, intr } => {
                if intr {
                    let has_objects = self.net.lines[verb as usize].iter().any(|l| l.kind == Kind::HasObject && l.rank == sense);
                    let end = pr.end_intransitive + if has_objects { pr.end_with_objects } else { 0.0 };
                    out.push(Tok { text: ".".into(), next: St::Start, score: end, why: "the verb may stand alone".into(), content: None, p: 0.0 });
                }
                if self.net.node(verb).frames & TRANSITIVE != 0 {
                    let objs = self.objects(verb, sense, p);
                    out.extend(self.det_toks(&objs, |d| St::ObjDet { det: d, verb: Some((verb, sense)) }, p));
                    out.extend(self.noun_toks(&objs, None, St::Done, p));
                }
            }
            St::ObjDet { det, verb } => {
                let list = match verb {
                    Some((v, sense)) => self.objects(v, sense, p),
                    None => self.parts(p),
                };
                out.extend(self.noun_toks(&list, Some(det), St::Done, p));
            }
            St::Done => out.push(Tok { text: ".".into(), next: St::Start, score: 0.0, why: "the sentence is whole".into(), content: None, p: 0.0 }),
        }
        // the same word may come by two roads: the better one stands
        let mut seen: Vec<(String, St)> = Vec::new();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap().then(a.text.cmp(&b.text)));
        out.retain(|tk| {
            let key = (tk.text.to_lowercase(), tk.next);
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        });
        // the probabilities: a softmax of the scores
        let temp = pr.temperature.max(1e-3);
        let m = out.iter().map(|tk| tk.score).fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = out.iter().map(|tk| ((tk.score - m) / temp).exp()).sum();
        for tk in out.iter_mut() {
            tk.p = ((tk.score - m) / temp).exp() / z;
        }
        out
    }

    /// The topic of a prompt: the first noun of its last sentence, else the last noun in it.
    pub fn topic_of(&self, prompt: &str) -> Option<u32> {
        let words = crate::parser::tokens(prompt);
        let noun = |w: &str| -> Option<u32> {
            if lexicon::closed(w).is_some() {
                return None;
            }
            let mut r: Vec<_> = self
                .net
                .lookup(w)
                .iter()
                .map(|&q| self.net.node(q))
                .filter(|n| n.pos == Pos::Noun && !n.proper && matches!(n.form, Form::Base | Form::Plural) && self.net.node(n.lemma).sure)
                .collect();
            r.sort_by_key(|n| (std::cmp::Reverse(self.net.node(n.lemma).fam), n.lemma));
            r.first().map(|n| n.lemma)
        };
        let last = prompt.trim_end().trim_end_matches('.').rsplit('.').next().unwrap_or(prompt);
        crate::parser::tokens(last).iter().find_map(|w| noun(w)).or_else(|| words.iter().rev().find_map(|w| noun(w)))
    }

    /// Read text through the automaton: each word's probability where it stood
    /// (None when the motor had no place for it), and the state reached.
    pub fn read(&self, text: &str, p: &mut Para<'a>) -> (Vec<(String, Option<f64>, Vec<Tok>)>, St) {
        let mut out = Vec::new();
        let mut st = St::Start;
        let mut lost = false;
        let mut words: Vec<String> = Vec::new();
        for w in text.split_whitespace() {
            let w = w.trim_matches(|c: char| c == ',' || c == '"');
            let (core, stop) = match w.strip_suffix(['.', '!', '?']) {
                Some(c) => (c, true),
                None => (w, false),
            };
            if !core.is_empty() {
                words.push(core.to_lowercase());
            }
            if stop {
                words.push(".".into());
            }
        }
        let mut sentence: Vec<u32> = Vec::new();
        for w in words {
            if lost {
                out.push((w.clone(), None, Vec::new()));
                if w == "." {
                    lost = false;
                    st = St::Start;
                    sentence.clear();
                }
                continue;
            }
            let cands = self.next(st, p);
            match cands.iter().find(|tk| tk.text.to_lowercase() == w) {
                Some(tk) => {
                    out.push((w.clone(), Some(tk.p), cands.clone()));
                    if let Some(c) = tk.content {
                        sentence.push(c);
                    }
                    if matches!(st, St::Start) && p.num.is_none() {
                        p.num = match tk.next {
                            St::Subj(n) => Some(n),
                            St::Det(D::A) | St::Det(D::An) => Some(Num::Sg),
                            _ => None,
                        };
                    }
                    if let St::Subj(n) = tk.next {
                        p.num.get_or_insert(n);
                    }
                    st = tk.next;
                    if w == "." {
                        p.sentences += 1;
                        p.used.extend(sentence.drain(..));
                    }
                }
                None => {
                    out.push((w.clone(), None, cands));
                    lost = w != ".";
                    if w == "." {
                        st = St::Start;
                        sentence.clear();
                    }
                }
            }
        }
        (out, st)
    }

    /// One sentence from a state, by beam search over the words' log-probabilities.
    fn sentence(&self, from: St, p: &Para) -> Option<(Vec<Tok>, f64)> {
        let width = self.cfg.mblm.beam;
        let mut beams: Vec<(St, Vec<Tok>, f64)> = vec![(from, Vec::new(), 0.0)];
        let mut done: Vec<(Vec<Tok>, f64)> = Vec::new();
        for _ in 0..12 {
            let mut next: Vec<(St, Vec<Tok>, f64)> = Vec::new();
            for (st, toks, lp) in &beams {
                // within a sentence, no content word twice
                let inside: HashSet<u32> = toks.iter().filter_map(|t| t.content).collect();
                for tk in self.next(*st, p) {
                    if tk.p <= 0.0 || tk.content.map(|c| inside.contains(&c)).unwrap_or(false) {
                        continue;
                    }
                    let lp2 = lp + tk.p.ln();
                    let mut toks2 = toks.clone();
                    let st2 = tk.next;
                    toks2.push(tk);
                    if st2 == St::Start {
                        done.push((toks2, lp2));
                    } else {
                        next.push((st2, toks2, lp2));
                    }
                }
            }
            next.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
            next.truncate(width);
            beams = next;
            if beams.is_empty() {
                break;
            }
        }
        // a sentence that says something new: not one whose every content word was said
        // (the topic itself is never what is new)
        done.retain(|(toks, _)| {
            let content: Vec<u32> = toks.iter().filter_map(|t| t.content).filter(|&c| c != p.cx.topic).collect();
            content.is_empty() && p.sentences == 0 || content.iter().any(|c| !p.used.contains(c))
        });
        let mean = |d: &(Vec<Tok>, f64)| d.1 / d.0.len().max(1) as f64;
        done.into_iter().max_by(|a, b| mean(a).partial_cmp(&mean(b)).unwrap())
    }

    /// Continue the prompt: (the words added, each with its probability and why).
    pub fn complete(&self, prompt: &str, sentences: usize) -> Result<(String, Vec<Tok>), String> {
        let topic = self.topic_of(prompt).ok_or_else(|| "I found no noun in the prompt that I know well enough to go on about.".to_string())?;
        let mut p = self.para(topic);
        let (read, mut st) = self.read(prompt, &mut p);
        // a prompt the automaton cannot follow (a question, a sentence of
        // another kind) gives the topic; the continuation starts a sentence
        let lost = read.iter().rev().take_while(|(w, _, _)| w != ".").any(|(_, pr, _)| pr.is_none());
        if lost {
            st = St::Start;
        }
        let mut words: Vec<Tok> = Vec::new();
        let mut from = st;
        for _ in 0..sentences {
            let Some((toks, _)) = self.sentence(from, &p) else { break };
            for tk in &toks {
                if let St::Subj(n) = tk.next {
                    p.num.get_or_insert(n);
                }
            }
            p.used.extend(toks.iter().filter_map(|t| t.content));
            p.sentences += 1;
            words.extend(toks);
            from = St::Start;
        }
        // the words into text: a capital to start a sentence, no space before a full stop
        let mut text = String::new();
        let mut cap = st == St::Start;
        for tk in &words {
            if tk.text == "." {
                text.push('.');
                cap = true;
                continue;
            }
            text.push(' ');
            if cap {
                let mut c = tk.text.chars();
                text.push_str(&(c.next().map(|f| f.to_uppercase().collect::<String>()).unwrap_or_default() + c.as_str()));
                cap = false;
            } else {
                text.push_str(&tk.text);
            }
        }
        Ok((text, words))
    }

    /// A text's score: each word's probability, the perplexity of the words the
    /// motor had a place for, and how many it had (coverage).
    pub fn score(&self, text: &str) -> Option<Score> {
        // the topic of a text read whole: its first noun
        let first = text.split(['.', '!', '?']).next().unwrap_or(text);
        let topic = self.topic_of(first)?;
        let mut p = self.para(topic);
        let (words, _) = self.read(text, &mut p);
        let inside: Vec<f64> = words.iter().filter_map(|(_, pr, _)| *pr).collect();
        let n = words.len().max(1);
        let ppl = if inside.is_empty() { f64::INFINITY } else { (-inside.iter().map(|x| x.ln()).sum::<f64>() / inside.len() as f64).exp() };
        Some(Score {
            topic: self.text(topic).to_string(),
            words: words.into_iter().map(|(w, pr, c)| (w, pr, c.into_iter().take(3).map(|t| (t.text, t.p)).collect())).collect(),
            perplexity: ppl,
            coverage: inside.len() as f64 / n as f64,
        })
    }
}

pub struct Score {
    pub topic: String,
    /// each word, its probability (None: the motor had no place for it), and the three likeliest there
    pub words: Vec<(String, Option<f64>, Vec<(String, f64)>)>,
    pub perplexity: f64,
    pub coverage: f64,
}

// ---------------------------------------------------------------- questions

/// Who a question is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    /// the person: how they feel (a feeling word), a thing (they may have,
    /// like), a deed (they do)
    You { feel: Option<u32>, thing: Option<u32>, deed: Option<u32> },
    /// something of theirs (past: it is gone)
    Your { noun: u32, past: bool },
}

/// What a yes says (Raven remembers it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    Feel(u32),
    Has(u32),
    Likes(u32),
    Is(u32, u32),
    Does(u32, u32),
    Kind(u32, u32),
}

/// What the answer to a question will be.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Waits {
    Confirm(Confirm),
    Story,
    Describe(u32),
    PetName(u32),
}

/// A question the model asks: its words, its probability among every
/// question it could ask here, the lines it stands on, what it waits for.
#[derive(Clone, Debug)]
pub struct Asked {
    pub text: String,
    pub p: f64,
    pub why: Vec<String>,
    pub waits: Waits,
    pub about: Vec<u32>,
}

impl<'a> Mblm<'a> {
    fn object_word(&self, n: u32) -> String {
        let node = self.net.node(n);
        if node.mass {
            node.text.clone()
        } else if let Some(pl) = self.sure_form(n, Form::Plural) {
            pl
        } else {
            format!("{} {}", det_text(self.det_for(&node.text)), node.text)
        }
    }

    fn a(&self, n: u32) -> String {
        let t = &self.net.node(n).text;
        format!("{} {t}", det_text(self.det_for(t)))
    }

    /// Feeling words that may go with this one: the feeling words of the same
    /// feel (the anchors in rules.toml are the vocabulary), each scored by how
    /// near the network finds it from this one, and how familiar it is.
    fn near_feelings(&self, feel: u32) -> Vec<(u32, f64, String)> {
        let f = &self.cfg.raven.feeling;
        let v = crate::raven::feeling::valence(self.net, f, feel).unwrap_or(-1.0);
        let words = if v < 0.0 { &f.bad_adjectives } else { &f.good_adjectives };
        let cx = Ctx::new(self.net, self.cfg, feel);
        let mut out: Vec<(u32, f64, String)> = Vec::new();
        for w in words {
            let Some(a) = self.net.base(w, Pos::Adj) else { continue };
            if a == feel || !self.net.node(a).sure || out.iter().any(|x| x.0 == a) {
                continue;
            }
            let near = cx.meaning(a).max(-8.0);
            out.push((a, cx.familiarity(a) + 0.5 * near, format!("{} -near-> {}", self.text(feel), cx.path(a))));
        }
        out
    }

    /// Every question the model could ask here, each with its probability (a
    /// softmax over all of them), the likeliest first; none that touches a
    /// word in `asked` (asked about already).
    pub fn questions(&self, who: Who, pet_name: Option<&str>, asked: &HashSet<u32>, k: usize) -> Vec<Asked> {
        let q = &self.cfg.mblm;
        let mut all: Vec<(f64, Asked)> = Vec::new();
        let mut add = |score: f64, words: Vec<String>, why: Vec<String>, waits: Waits, about: Vec<u32>| {
            if about.iter().any(|a| asked.contains(a)) {
                return;
            }
            let mut text = words.join(" ");
            text = text[..1].to_uppercase() + &text[1..];
            text.push('?');
            all.push((score, Asked { text, p: 0.0, why, waits, about }));
        };
        match who {
            Who::Your { noun, past } => {
                let p = self.para(noun);
                let subject = match pet_name {
                    Some(n) => n.to_string(),
                    None => format!("your {}", self.text(noun)),
                };
                let (aux, be) = if past { ("did", "was") } else { ("does", "is") };
                // does your cat chase mice? / what does your cat chase?
                // (of something lost, only how it was is asked: never what it did)
                for (v, s, why, sense) in p.cx.lined_ranks(Kind::SubjectOf, Pos::Verb, |n| n.frames & (INTRANSITIVE | TRANSITIVE) != 0) {
                    if past || self.light(v) {
                        continue;
                    }
                    let verb = self.text(v).to_string();
                    let objs = self.objects(v, sense, &p);
                    if let Some((o, os, owhy)) = objs.first() {
                        add(s + os + q.ask_does, vec![aux.into(), subject.clone(), verb.clone(), self.object_word(*o)],
                            vec![why.clone(), owhy.clone()], Waits::Confirm(Confirm::Does(noun, v)), vec![v, *o]);
                        add(s + q.ask_what, vec!["what".into(), aux.into(), subject.clone(), verb.clone()], vec![why.clone()], Waits::Story, vec![v]);
                    } else if self.net.node(v).frames & INTRANSITIVE != 0 {
                        add(s + q.ask_does, vec![aux.into(), subject.clone(), verb], vec![why], Waits::Confirm(Confirm::Does(noun, v)), vec![v]);
                    }
                }
                // is your cat playful?
                for (a, s, why) in p.cx.lined(Kind::ModifiedBy, Pos::Adj, predicative) {
                    add(s + q.ask_is, vec![be.into(), subject.clone(), self.text(a).into()], vec![why], Waits::Confirm(Confirm::Is(noun, a)), vec![a]);
                }
                // is your cat a siamese? (a kind of it)
                for l in self.net.lines[noun as usize].iter().filter(|l| l.kind == Kind::Hyponym && l.rank == 0 && !past) {
                    if !p.cx.usable(l.to, Pos::Noun) || self.net.node(l.to).mass {
                        continue;
                    }
                    let why = format!("{} -has kind-> {}", self.text(noun), self.text(l.to));
                    add(p.cx.familiarity(l.to) + q.ask_kind, vec![be.into(), subject.clone(), self.a(l.to)], vec![why],
                        Waits::Confirm(Confirm::Kind(noun, l.to)), vec![l.to]);
                }
                // what is your cat like?
                add(q.ask_like, vec!["what".into(), be.into(), subject.clone(), "like".into()], vec![], Waits::Describe(noun), vec![]);
                // what is your cat's name? (an animal of theirs whose name was not told)
                if self.net.node(noun).animate && pet_name.is_none() && !past {
                    let poss = format!("your {}{}s", self.text(noun), '\u{27}');
                    add(q.ask_name, vec!["what".into(), "is".into(), poss, "name".into()], vec![], Waits::PetName(noun), vec![]);
                }
            }
            Who::You { feel, thing, deed } => {
                if let Some(m) = feel {
                    // do you feel lonely? / are you lonely? / what makes you feel sad?
                    for (a, s, why) in self.near_feelings(m) {
                        let word = self.text(a).to_string();
                        add(s + q.ask_feel, vec!["do".into(), "you".into(), "feel".into(), word], vec![why], Waits::Confirm(Confirm::Feel(a)), vec![a]);
                    }
                    let mood = self.text(m).to_string();
                    add(q.ask_makes, vec!["what".into(), "makes".into(), "you".into(), "feel".into(), mood.clone()], vec![], Waits::Story, vec![m]);
                    add(q.ask_since, vec!["how".into(), "long".into(), "have".into(), "you".into(), "felt".into(), mood], vec![], Waits::Story, vec![]);
                    let bad = crate::raven::feeling::valence(self.net, &self.cfg.raven.feeling, m).map(|v| v < 0.0).unwrap_or(false);
                    if bad {
                        add(q.ask_happened, vec!["what".into(), "happened".into()], vec![], Waits::Story, vec![]);
                    }
                }
                if let Some(x) = thing {
                    // do you have a cat? / do you like cats?
                    let s = Ctx::new(self.net, self.cfg, x).familiarity(x);
                    if !self.net.node(x).mass {
                        add(s + q.ask_have, vec!["do".into(), "you".into(), "have".into(), self.a(x)], vec![], Waits::Confirm(Confirm::Has(x)), vec![]);
                    }
                    add(s + q.ask_like_it, vec!["do".into(), "you".into(), "like".into(), self.object_word(x)], vec![], Waits::Confirm(Confirm::Likes(x)), vec![]);
                }
                if let Some(v) = deed {
                    // what do you code? / do you code programs?
                    let p = self.para(v);
                    let objs = self.objects(v, 0, &p);
                    for (o, os, owhy) in objs.iter().take(6) {
                        add(os + q.ask_does, vec!["do".into(), "you".into(), self.text(v).into(), self.object_word(*o)], vec![owhy.clone()],
                            Waits::Story, vec![*o]);
                    }
                    if !objs.is_empty() {
                        add(q.ask_what_deed, vec!["what".into(), "do".into(), "you".into(), self.text(v).into()], vec![], Waits::Story, vec![]);
                    }
                }
            }
        }
        // the probabilities: a softmax over every question it could ask
        let temp = q.temperature.max(1e-3);
        let m = all.iter().map(|x| x.0).fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = all.iter().map(|x| ((x.0 - m) / temp).exp()).sum();
        let mut out: Vec<Asked> = all
            .into_iter()
            .map(|(s, mut a)| {
                a.p = ((s - m) / temp).exp() / z;
                a
            })
            .collect();
        out.sort_by(|a, b| b.p.partial_cmp(&a.p).unwrap().then(a.text.cmp(&b.text)));
        out.dedup_by(|a, b| a.text == b.text);
        out.truncate(k);
        out
    }
}
