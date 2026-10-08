//! How a word feels: good (+1), bad (-1), or not a feeling (None). Read from
//! the network, never trained: a word on the anchor lists in rules.toml, else
//! a similar word that is (glad ~ happy), the opposite of one (unhappy !
//! happy, flipped), or the feeling it names up the is-a lines (grief -> sorrow).

use crate::chat::kind_path;
use crate::config::Feeling;
use crate::network::{Kind, Network, Pos};

fn listed(net: &Network, w: u32, list: &[String], pos: Pos) -> bool {
    list.iter().any(|a| net.base(a, pos) == Some(w))
}

fn direct(net: &Network, f: &Feeling, w: u32, pos: Pos) -> Option<f32> {
    let (good, bad) = match pos {
        Pos::Adj => (&f.good_adjectives, &f.bad_adjectives),
        Pos::Noun => (&f.good_nouns, &f.bad_nouns),
        Pos::Verb => (&f.good_events, &f.bad_events),
        Pos::Adv => return None,
    };
    if listed(net, w, good, pos) {
        Some(1.0)
    } else if listed(net, w, bad, pos) {
        Some(-1.0)
    } else {
        None
    }
}

pub fn valence(net: &Network, f: &Feeling, w: u32) -> Option<f32> {
    let pos = net.node(w).pos;
    if let Some(v) = direct(net, f, w, pos) {
        return Some(v);
    }
    let lines = &net.lines[w as usize];
    // a similar word in a common sense, then an opposite (flipped)
    for l in lines.iter().filter(|l| l.kind == Kind::Similar && l.rank <= 1) {
        if let Some(v) = direct(net, f, l.to, net.node(l.to).pos) {
            return Some(v);
        }
    }
    for l in lines.iter().filter(|l| l.kind == Kind::Antonym && l.rank <= 1) {
        if let Some(v) = direct(net, f, l.to, net.node(l.to).pos) {
            return Some(-v);
        }
    }
    // the feeling it names: an adjective's noun (sad -> sadness), or a noun
    // that is a kind of an anchor feeling (grief -> sorrow)
    let nouns: Vec<u32> = match pos {
        Pos::Noun => vec![w],
        _ => lines
            .iter()
            .filter(|l| matches!(l.kind, Kind::Derived | Kind::Attribute) && l.rank == 0 && net.node(l.to).pos == Pos::Noun)
            .map(|l| l.to)
            .collect(),
    };
    for n in nouns {
        if let Some(v) = direct(net, f, n, Pos::Noun) {
            return Some(v);
        }
        for (anchors, v) in [(&f.good_nouns, 1.0), (&f.bad_nouns, -1.0)] {
            for a in anchors {
                if let Some(k) = net.base(a, Pos::Noun) {
                    if kind_path(net, n, k).is_some() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}
