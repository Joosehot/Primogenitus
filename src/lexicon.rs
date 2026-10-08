//! The closed vocabulary the dictionary does not carry: determiners, pronouns,
//! prepositions, auxiliaries, conjunctions. These words are only ever what
//! this list says they are (the dictionary's "a" as vitamin A is ignored).
//! And the article rule: "a" or "an".

use crate::config::Article;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Closed {
    Det,
    Pron,
    Prep,
    Aux,
    Conj,
    Not,
}

pub fn closed(w: &str) -> Option<Closed> {
    use Closed::*;
    Some(match w {
        "a" | "an" | "the" | "this" | "that" | "these" | "those" | "every" | "each" | "some" | "any" | "no" | "my" | "your"
        | "his" | "her" | "its" | "our" | "their" | "another" | "all" | "both" | "many" | "much" | "several" => Det,
        "i" | "you" | "he" | "she" | "it" | "we" | "they" | "me" | "him" | "us" | "them" | "who" | "what" | "which"
        | "whom" | "whose" | "someone" | "something" | "everyone" | "everything" | "nobody" | "nothing" | "one" => Pron,
        "of" | "in" | "on" | "at" | "to" | "for" | "with" | "from" | "by" | "about" | "into" | "over" | "under"
        | "through" | "after" | "before" | "between" | "against" | "during" | "without" | "within" | "along"
        | "across" | "behind" | "beyond" | "near" | "around" | "upon" | "onto" | "toward" | "towards" | "as" | "than"
        | "like" | "off" | "out" | "up" | "down" => Prep,
        "is" | "are" | "was" | "were" | "be" | "been" | "being" | "am" | "has" | "have" | "had" | "do" | "does" | "did"
        | "will" | "would" | "shall" | "should" | "can" | "could" | "may" | "might" | "must" => Aux,
        "and" | "or" | "but" | "nor" | "so" | "yet" | "if" | "because" | "although" | "while" | "when" | "where"
        | "whether" | "then" => Conj,
        "not" | "n't" | "never" => Not,
        _ => return None,
    })
}

/// Nouns that only measure out another ("a kind of fish", "a lot of
/// water"): the noun after "of" is the one meant.
pub fn partitive(w: &str) -> bool {
    matches!(w, "kind" | "kinds" | "sort" | "sorts" | "type" | "types" | "lot" | "lots" | "piece" | "pieces" | "number"
        | "bit" | "bits" | "group" | "couple" | "variety" | "pair" | "set" | "amount" | "deal" | "plenty" | "bunch" | "handful")
}

/// "an" before a vowel sound, "a" before a consonant sound: the longest
/// matching beginning in rules.toml decides; nothing matching means "a".
pub fn article(word: &str, rules: &Article) -> &'static str {
    let w = word.to_lowercase();
    let mut best = (0usize, "a");
    for (list, art) in [(&rules.an, "an"), (&rules.a, "a")] {
        for p in list.iter() {
            if w.starts_with(p.as_str()) && p.len() > best.0 {
                best = (p.len(), art);
            }
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Article {
        let cfg = crate::config::Config::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/rules.toml"))).unwrap();
        cfg.article
    }

    #[test]
    fn articles() {
        let r = rules();
        for (w, a) in [("dog", "a"), ("animal", "an"), ("hour", "an"), ("university", "a"), ("umbrella", "an"),
                       ("unimportant", "an"), ("user", "a"), ("european", "a"), ("house", "a"), ("egg", "an")] {
            assert_eq!(article(w, &r), a, "{w}");
        }
    }
}
