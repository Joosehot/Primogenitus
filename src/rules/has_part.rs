//! "A car has a bumper." A part of the topic, read off its has-part lines
//! (or inherited from its kind, with a cost).

use super::is_a::along;
use super::Pattern;
use crate::network::Kind;
use crate::search::{Cand, Ctx, Pred};

pub struct HasPart;

impl Pattern for HasPart {
    fn name(&self) -> &'static str {
        "has_part"
    }

    fn heads(&self, cx: &Ctx) -> Vec<Cand> {
        along(cx, Kind::HasPart)
            .into_iter()
            .map(|(n, s, why)| Cand {
                pattern: self.name(),
                subj: None,
                pred: Pred::Have { part: n, np: None },
                parts: vec![("pattern has_part".into(), cx.cfg.patterns.has_part), (format!("part {}", cx.node(n).text), s), ("seed".into(), cx.jit(n))],
                why: vec![why],
            })
            .collect()
    }
}
