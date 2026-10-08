//! The sentence patterns, one file each. A pattern offers heads (the verb,
//! the adjective, the kind, the part) reached from the topic; the beam fills
//! the rest. The judges (judges.rs) carry the grammar.

pub mod does;
pub mod does_to;
pub mod has_part;
pub mod is_a;
pub mod is_adj;
pub mod judges;

use crate::search::{Cand, Ctx};

pub trait Pattern: Sync {
    fn name(&self) -> &'static str;
    fn heads(&self, cx: &Ctx) -> Vec<Cand>;
}

pub enum Verdict {
    /// passes, with a score term (0 for none)
    Ok(f64),
    /// against the grammar: the candidate is dropped
    Refuse(&'static str),
}

pub static ALL: [&dyn Pattern; 5] = [&is_a::IsA, &has_part::HasPart, &does::Does, &does_to::DoesTo, &is_adj::IsAdj];
