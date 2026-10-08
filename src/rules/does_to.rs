//! "The dog chases a cat." The topic does something to something: a
//! transitive verb. Only what a line says: the parsed examples show the topic
//! (or its kind) doing it; the object is chosen in the beam, from what that
//! sense of the verb is done to in the examples.

use super::Pattern;
use crate::network::{Kind, Pos};
use crate::parser::TRANSITIVE;
use crate::search::{Cand, Ctx, Pred, Tense};

pub struct DoesTo;

impl Pattern for DoesTo {
    fn name(&self) -> &'static str {
        "does_to"
    }

    fn heads(&self, cx: &Ctx) -> Vec<Cand> {
        cx.lined_ranks(Kind::SubjectOf, Pos::Verb, |n| n.frames & TRANSITIVE != 0)
            .into_iter()
            .map(|(v, s, why, sense)| Cand {
                pattern: self.name(),
                subj: None,
                pred: Pred::Tr { verb: v, tense: Tense::Present, obj: None, sense },
                parts: vec![("pattern does_to".into(), cx.cfg.patterns.does_to), (format!("verb {}", cx.node(v).text), s), ("seed".into(), cx.jit(v))],
                why: vec![why],
            })
            .collect()
    }
}
