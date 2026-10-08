//! "The dog is loyal." An adjective that may stand after "is".
//! Only what a line says: the parsed examples show it describing the topic (or its kind).

use super::Pattern;
use crate::network::{Kind, Pos};
use crate::search::{predicative, Cand, Ctx, Pred, Tense};

pub struct IsAdj;

impl Pattern for IsAdj {
    fn name(&self) -> &'static str {
        "is_adj"
    }

    fn heads(&self, cx: &Ctx) -> Vec<Cand> {
        cx.lined(Kind::ModifiedBy, Pos::Adj, predicative)
            .into_iter()
            .map(|(a, s, why)| Cand {
                pattern: self.name(),
                subj: None,
                pred: Pred::CopAdj { adj: a, tense: Tense::Present },
                parts: vec![("pattern is_adj".into(), cx.cfg.patterns.is_adj), (format!("adjective {}", cx.node(a).text), s), ("seed".into(), cx.jit(a))],
                why: vec![why],
            })
            .collect()
    }
}
