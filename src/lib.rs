//! Primogenitus: a motor that produces sentences.
//!
//! A frozen word network is built once from a downloaded dictionary
//! (WordNet 3.1): every word and every variation of a word is a node, lines
//! between words carry the meaning. Sentences come from the HotMotor
//! pipeline: lexicon, parser, rules (one sentence pattern per file), scoring,
//! beam search with judges, checks. Nothing is trained, nothing updates.
//!
//!   wordnet   the dictionary, downloaded and read
//!   morph     a word's variations
//!   network   the frozen network (nodes, lines, freeze, load)
//!   build     the dictionary into the network
//!   lexicon   the closed words, the article rule
//!   parser    example sentences into lines; the input; the grammar check
//!   input     what starts a sentence
//!   rules     the sentence patterns and the judges
//!   search    meaning spreading, the beam, the seeds

pub mod build;
pub mod config;
pub mod input;
pub mod lexicon;
pub mod morph;
pub mod network;
pub mod parser;
pub mod rules;
pub mod search;
pub mod wordnet;

use config::Config;
use network::Network;
use search::{Cand, Ctx};

pub struct Seed {
    pub seed: u64,
    pub cand: Cand,
    pub text: String,
    /// the judges' score (the seed's own shake left out)
    pub score: f64,
    pub check: Result<String, String>,
    /// the sentence's content words (seeds must differ in them)
    pub words: Vec<u32>,
}

fn words(cx: &Ctx, c: &Cand) -> Vec<u32> {
    use search::Pred::*;
    let mut w = vec![cx.topic];
    if let Some(a) = c.subj.as_ref().and_then(|s| s.adj) {
        w.push(a);
    }
    match &c.pred {
        Intr { verb, .. } => w.push(*verb),
        Tr { verb, obj, .. } => {
            w.push(*verb);
            w.extend(obj.as_ref().map(|o| o.noun));
        }
        CopAdj { adj, .. } => w.push(*adj),
        CopNp { noun, .. } => w.push(*noun),
        Have { part, .. } => w.push(*part),
    }
    w.sort();
    w
}

/// Generate: one sentence per seed, then the judges pick. Returns the seeds
/// (best first) and the index of the winner (None when no seed passed the check).
pub fn generate(net: &Network, cfg: &Config, topic: u32, seeds: usize) -> (Vec<Seed>, Option<usize>) {
    let mut cx = Ctx::new(net, cfg, topic);
    let mut out: Vec<Seed> = Vec::new();
    for s in 0..seeds as u64 {
        cx.seed = s;
        for c in search::beam(&cx) {
            let Some(text) = search::realize(&cx, &c) else { continue };
            let w = words(&cx, &c);
            if out.iter().any(|o| o.words == w) {
                continue;
            }
            let score = c.parts.iter().filter(|p| p.0 != "seed").map(|p| p.1).sum();
            let check = parser::check(net, cfg, &text).map_err(|e| e.to_string());
            out.push(Seed { seed: s, cand: c, text, score, check, words: w });
            break;
        }
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap().then(a.seed.cmp(&b.seed)));
    let winner = out.iter().position(|s| s.check.is_ok());
    (out, winner)
}
