//! "A dog is a canine." The kind the topic is, read straight off its is-a
//! lines (one step, or two with a cost), in its most common sense only.

use super::Pattern;
use crate::network::{Kind, Pos};
use crate::search::{Cand, Ctx, Pred};

pub struct IsA;

/// Words one step along `kind` from the topic's most common sense, or two
/// (through its kind, with a cost): (word, score, the path).
pub fn along(cx: &Ctx, kind: Kind) -> Vec<(u32, f64, String)> {
    let decay = cx.cfg.spread.rank_decay as f64;
    let topic = cx.topic;
    let name = |i: u32| cx.node(i).text.clone();
    let mut out: Vec<(u32, f64, String)> = Vec::new();
    let mut push = |w: u32, s: f64, why: String| {
        if !cx.usable(w, Pos::Noun) {
            return;
        }
        match out.iter_mut().find(|x| x.0 == w) {
            Some(x) if x.1 >= s => {}
            Some(x) => *x = (w, s, why),
            None => out.push((w, s, why)),
        }
    };
    for l in step(cx, topic, kind).filter(|l| l.rank == 0) {
        let s = cx.familiarity(l.to);
        push(l.to, s, format!("{} -{}-> {}", name(topic), kind.name(), name(l.to)));
    }
    for h in step(cx, topic, Kind::Hypernym).filter(|h| h.rank == 0) {
        for m in step(cx, h.to, kind).filter(|m| m.rank == h.to_rank) {
            let s = cx.cfg.score.hop2 + decay.powi(m.rank as i32).ln() + cx.familiarity(m.to);
            push(m.to, s, format!("{} -is a-> {} -{}-> {}", name(topic), name(h.to), kind.name(), name(m.to)));
        }
    }
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
    out.truncate(cx.cfg.search.candidates);
    out
}

fn step<'a>(cx: &'a Ctx, from: u32, kind: Kind) -> impl Iterator<Item = &'a crate::network::Line> {
    cx.net.lines[from as usize].iter().filter(move |l| l.kind == kind)
}

impl Pattern for IsA {
    fn name(&self) -> &'static str {
        "is_a"
    }

    fn heads(&self, cx: &Ctx) -> Vec<Cand> {
        along(cx, Kind::Hypernym)
            .into_iter()
            .map(|(n, s, why)| Cand {
                pattern: self.name(),
                subj: None,
                pred: Pred::CopNp { noun: n, np: None },
                parts: vec![("pattern is_a".into(), cx.cfg.patterns.is_a), (format!("kind {}", cx.node(n).text), s), ("seed".into(), cx.jit(n))],
                why: vec![why],
            })
            .collect()
    }
}
