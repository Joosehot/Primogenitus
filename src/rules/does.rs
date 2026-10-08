//! "Dogs bark." The topic does something: an intransitive verb.
//! Only what a line says: the parsed examples show the topic (or its kind) doing it.

use super::Pattern;
use crate::network::{Kind, Pos};
use crate::parser::INTRANSITIVE;
use crate::search::{Cand, Ctx, Pred, Tense};

pub struct Does;

impl Pattern for Does {
    fn name(&self) -> &'static str {
        "does"
    }

    fn heads(&self, cx: &Ctx) -> Vec<Cand> {
        cx.lined(Kind::SubjectOf, Pos::Verb, |n| n.frames & INTRANSITIVE != 0)
            .into_iter()
            .map(|(v, s, why)| Cand {
                pattern: self.name(),
                subj: None,
                pred: Pred::Intr { verb: v, tense: Tense::Present },
                parts: vec![("pattern does".into(), cx.cfg.patterns.does), (format!("verb {}", cx.node(v).text), s), ("seed".into(), cx.jit(v))],
                why: vec![why],
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::INTRANSITIVE;

    #[test]
    fn needs_an_intransitive_frame() {
        assert!(INTRANSITIVE & (1 << 2) != 0); // "Somebody ----s"
        assert!(INTRANSITIVE & (1 << 8) == 0); // "Somebody ----s something"
    }
}
