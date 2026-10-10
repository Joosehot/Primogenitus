//! The humane judges. A reply is a few parts (an opener, a body, a
//! question); the judges score the whole of it against what the user said and
//! the conversation so far:
//!
//!   feelings first   pain is met with sympathy before anything else, and no
//!                    facts while someone hurts; joy is shared
//!   answers          a question asked gets its answer
//!   listens          the reply takes up what the user talked about (and
//!                    greets a name just told); no fact nobody asked for
//!   remembers        it brings back what the user told before, and uses
//!                    their name now and then (not every time)
//!   keeps going      it asks back, but does not interrogate (a fourth
//!                    question in a row)
//!   stays with them  while they are low (a bad feeling told a short while
//!                    ago), no cheerful question that would fit anyone and no
//!                    fact: the conversation stays on them
//!   natural          nothing said twice, not the same opening as last time,
//!                    not the same kind of question twice running, not too long

use super::{History, Part, TurnCtx};
use crate::config::RavenJudges;

/// A question's form: its first three words ("do you feel").
pub fn form_of(q: &str) -> String {
    q.to_lowercase().split_whitespace().take(3).collect::<Vec<_>>().join(" ")
}

pub fn score(parts: &[&Part], ctx: &TurnCtx, hist: &History, w: &RavenJudges) -> (f64, Vec<&'static str>) {
    let mut s = 0.0;
    let mut why = Vec::new();
    let has = |f: &dyn Fn(&Part) -> bool| parts.iter().any(|p| f(p));

    // feelings first
    if ctx.valence < 0.0 {
        // (sympathy said, before any question: a sad question is not sympathy)
        if parts.first().map(|p| p.feel < 0.0 && !p.question).unwrap_or(false) {
            s += w.feelings_first;
            why.push("feelings first");
        } else {
            s -= w.feelings_first;
        }
        if has(&|p| p.fact) {
            s += w.facts_in_pain;
        }
    } else if ctx.valence > 0.0 && has(&|p| p.feel > 0.0) {
        s += w.shares_joy;
        why.push("shares the joy");
    }
    // answers
    if ctx.asked {
        if has(&|p| p.answer) {
            s += w.answers;
            why.push("answers the question");
        } else {
            s -= w.answers;
        }
    }
    // listens
    if !ctx.nouns.is_empty() && has(&|p| p.mentions.iter().any(|m| ctx.nouns.contains(m))) {
        s += w.listens;
        why.push("takes up what you said");
    }
    if ctx.new_name {
        if has(&|p| p.meets) {
            s += w.listens;
            why.push("greets you by name");
        } else {
            s -= w.listens;
        }
    }
    // a fact nobody asked for, when someone tells about themselves
    if !ctx.asked && has(&|p| p.fact) {
        s += w.unasked_fact;
    }
    // remembers
    if has(&|p| p.memory) {
        s += w.remembers;
        why.push("remembers what you told");
    }
    if has(&|p| p.name) {
        if hist.name_used.iter().rev().take(2).any(|&u| u) {
            s += w.name_overuse;
        } else {
            s += w.name_use;
            why.push("uses your name");
        }
    }
    // keeps going, without interrogating
    let asks = parts.iter().filter(|p| p.question).count();
    // (a gentle question about how they feel, while they are low, is support, not interrogation)
    let gentle = ctx.low && parts.iter().any(|p| p.question && p.feel < 0.0);
    if asks == 1 {
        if hist.asked.iter().rev().take(3).filter(|&&a| a).count() == 3 && !gentle {
            s += w.interrogation;
        } else {
            s += w.keeps_going;
            why.push("asks back");
        }
    }
    if ctx.closing && asks > 0 {
        s += w.interrogation;
    }
    // stays with them while they are low
    if ctx.low {
        if has(&|p| p.generic) {
            s += w.off_thread;
        }
        if has(&|p| p.fact) {
            s += w.facts_in_pain;
        }
        if has(&|p| p.feel < 0.0) {
            s += w.stays;
            why.push("stays with how you feel");
        }
    }
    // the model's own questions: as likely as it found them
    for p in parts.iter().filter(|p| p.prob > 0.0) {
        s += w.model * p.prob.ln();
    }
    // natural
    for p in parts {
        // (an answer to a question asked again is said again)
        if hist.said.contains(&p.text) && !(p.answer && ctx.asked) {
            s += w.repeat;
        }
        if p.generic {
            s += w.generic;
        }
    }
    if let (Some(first), Some(last)) = (parts.first(), hist.first.last()) {
        if &first.text == last {
            s += w.repeat;
        }
    }
    // not the same kind of question twice running ("do you feel ...?" again)
    if let (Some(q), Some(last)) = (parts.iter().find(|p| p.question), hist.last_form.as_ref()) {
        if &form_of(&q.text) == last {
            s += w.same_form;
        }
    }
    let words: usize = parts.iter().map(|p| p.text.split_whitespace().count()).sum();
    if words > w.max_words {
        s += w.too_long * (words - w.max_words) as f64;
    }
    (s, why)
}
