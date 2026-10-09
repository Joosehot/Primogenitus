//! The whole conversation, remembered. Every exchange is kept: what the user
//! said, what Raven answered, and what Raven read from it (facts). When the
//! kept text grows past a limit, the oldest exchanges are compacted: their
//! words are let go, their facts are kept, each with the turn it was told on.
//! Nothing is rewritten or made up in compacting; what was read stays.

use std::collections::VecDeque;

/// What Raven read from a turn.
#[derive(Clone, Debug, PartialEq)]
pub enum Fact {
    Name(String),
    /// a feeling and how it feels (<0 bad, >0 good)
    Feel(u32, f32),
    Has(u32),
    Likes(u32),
    Dislikes(u32),
    /// something of theirs and what it is like ("my dog is old")
    Is(u32, u32),
    /// something of theirs and what happened to it ("my dog died")
    Happened(u32, u32),
    /// what happened to them ("I broke up")
    TheyHappened(u32),
    /// a pet and its name
    PetName(u32, String),
    /// a thing and its age, as said
    Age(u32, String),
    /// what they are ("a teacher")
    Role(u32),
    /// what they asked about
    Asked(u32),
}

impl Fact {
    /// The words the fact is about, for recalling it by one of them.
    pub fn about(&self) -> Vec<u32> {
        match self {
            Fact::Has(n) | Fact::Likes(n) | Fact::Dislikes(n) | Fact::Role(n) | Fact::Asked(n) | Fact::TheyHappened(n) => vec![*n],
            Fact::Is(n, a) | Fact::Happened(n, a) => vec![*n, *a],
            Fact::PetName(n, _) | Fact::Age(n, _) | Fact::Feel(n, _) => vec![*n],
            Fact::Name(_) => vec![],
        }
    }
}

pub struct Exchange {
    pub turn: u64,
    pub user: String,
    pub reply: String,
    pub facts: Vec<Fact>,
}

impl Exchange {
    fn words(&self) -> usize {
        self.user.split_whitespace().count() + self.reply.split_whitespace().count()
    }
}

/// What is left of the compacted exchanges: how many, and every fact read from
/// them with its turn.
#[derive(Default)]
pub struct Compacted {
    pub turns: u64,
    pub words: usize,
    pub facts: Vec<(u64, Fact)>,
}

#[derive(Default)]
pub struct Transcript {
    pub exchanges: VecDeque<Exchange>,
    pub compacted: Compacted,
    /// how many times compacting has run
    pub compactions: u32,
}

impl Transcript {
    pub fn push(&mut self, e: Exchange, max_words: usize, keep_words: usize) {
        self.exchanges.push_back(e);
        if self.words() > max_words {
            self.compact(keep_words);
        }
    }

    /// The words kept as they were said.
    pub fn words(&self) -> usize {
        self.exchanges.iter().map(|e| e.words()).sum()
    }

    /// Let go of the oldest exchanges' words until `keep_words` are left,
    /// keeping every fact read from them.
    pub fn compact(&mut self, keep_words: usize) {
        while self.exchanges.len() > 1 && self.words() > keep_words {
            let e = self.exchanges.pop_front().unwrap();
            self.compacted.turns += 1;
            self.compacted.words += e.words();
            for f in e.facts {
                if !self.compacted.facts.iter().any(|(_, g)| *g == f) {
                    self.compacted.facts.push((e.turn, f));
                }
            }
        }
        self.compactions += 1;
    }

    /// Every fact of the whole conversation, the oldest first, with its turn.
    pub fn facts(&self) -> Vec<(u64, Fact)> {
        let mut out = self.compacted.facts.clone();
        for e in &self.exchanges {
            for f in &e.facts {
                out.push((e.turn, f.clone()));
            }
        }
        out
    }

    pub fn turns(&self) -> u64 {
        self.compacted.turns + self.exchanges.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacting_keeps_the_facts() {
        let mut t = Transcript::default();
        t.push(Exchange { turn: 1, user: "my name is Joose".into(), reply: "Nice to meet you, Joose.".into(), facts: vec![Fact::Name("Joose".into())] }, 50, 20);
        for k in 2..40 {
            t.push(Exchange { turn: k, user: "this is a turn with some words in it".into(), reply: "and a reply with words".into(), facts: vec![] }, 50, 20);
        }
        assert!(t.compactions > 0 && t.words() <= 50);
        assert_eq!(t.turns(), 39);
        assert!(t.facts().iter().any(|(turn, f)| *turn == 1 && *f == Fact::Name("Joose".into())));
    }
}
