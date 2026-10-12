//! A word's variations (plural, -s, past, -ing, -er, -est), each to become its
//! own node. Irregular forms come from the dictionary's exception lists; the
//! rest from spelling rules. A form the rules cannot be sure of is marked
//! unsure, and the generator never uses an unsure form.

use crate::network::{Form, Pos};

fn vowel(c: char) -> bool {
    "aeiou".contains(c)
}

fn ends_consonant_y(w: &str) -> bool {
    let c: Vec<char> = w.chars().collect();
    c.len() >= 2 && c[c.len() - 1] == 'y' && !vowel(c[c.len() - 2])
}

fn ends_consonant_o(w: &str) -> bool {
    let c: Vec<char> = w.chars().collect();
    c.len() >= 2 && c[c.len() - 1] == 'o' && !vowel(c[c.len() - 2])
}

fn sibilant(w: &str) -> bool {
    w.ends_with('s') || w.ends_with('x') || w.ends_with('z') || w.ends_with("ch") || w.ends_with("sh")
}

/// A noun that may already be plural or uncountable-looking ("trousers",
/// "news", "physics"): its number cannot be known from spelling.
pub fn plural_looking(w: &str) -> bool {
    w.ends_with('s') && !(w.ends_with("ss") || w.ends_with("us") || w.ends_with("is"))
}

fn s_form(w: &str) -> String {
    if ends_consonant_y(w) {
        format!("{}ies", &w[..w.len() - 1])
    } else if sibilant(w) {
        format!("{w}es")
    } else {
        format!("{w}s")
    }
}

fn last_word(w: &str) -> (&str, &str) {
    match w.rfind(' ') {
        Some(k) => (&w[..=k], &w[k + 1..]),
        None => ("", w),
    }
}

/// The variations of a base word: (text, form, sure).
/// Nouns in -man whose plural is -mans.
const NOT_MEN: &[&str] = &["human", "german", "roman", "shaman", "talisman", "caiman", "cayman", "ottoman", "dragoman", "hetman",
                           "doberman", "walkman", "brahman", "mussulman", "firman", "desman", "ataman", "pullman", "superhuman", "inhuman"];

/// Verbs whose past and past participle are the base itself.
const UNCHANGED: &[&str] = &["hurt", "cut", "put", "hit", "let", "set", "shut", "cost", "quit", "split", "spread", "burst", "cast",
                             "bet", "bid", "broadcast", "thrust", "upset", "beat", "fit", "knit", "rid", "shed", "slit", "wet"];

pub fn forms(base: &str, pos: Pos, irregular: &[String]) -> Vec<(String, Form, bool)> {
    let multi = base.contains(' ');
    let mut out = Vec::new();
    match pos {
        Pos::Noun => {
            let unsure = plural_looking(base);
            if let Some(p) = irregular.first() {
                out.push((p.replace('_', " "), Form::Plural, !unsure));
            } else {
                let (head, last) = last_word(base);
                let sure = !unsure && !ends_consonant_o(last);
                // "woman" -> "women", "fireman" -> "firemen" (WordNet's morphy has it as a rule, not an exception)
                let man = last.ends_with("man") && !NOT_MEN.contains(&last);
                let plural = if man { format!("{}men", &last[..last.len() - 3]) } else { s_form(last) };
                out.push((format!("{head}{plural}"), Form::Plural, sure));
            }
        }
        Pos::Verb => {
            if multi {
                return out;
            }
            let third = match base {
                "be" => "is".to_string(),
                "have" => "has".to_string(),
                "do" => "does".to_string(),
                "go" => "goes".to_string(),
                w if ends_consonant_o(w) => format!("{w}es"),
                w => s_form(w),
            };
            out.push((third, Form::ThirdSg, base != "be"));
            // the irregular forms: -ing, -s, and the past and the past participle
            let mut pasts = Vec::new();
            let mut ing = None;
            for f in irregular {
                if f.ends_with("ing") {
                    ing = Some(f.clone());
                } else if f.ends_with('s') && !f.ends_with("ss") && f.len() > base.len() {
                    continue;
                } else {
                    pasts.push(f.clone());
                }
            }
            let (past, part, sure) = match pasts.len() {
                // past and participle the same as the base ("it hurt", "I cut it"): not in the exceptions, which list only changes
                0 if UNCHANGED.contains(&base) => (base.to_string(), base.to_string(), true),
                0 => {
                    let p = if base.ends_with('e') {
                        format!("{base}d")
                    } else if ends_consonant_y(base) {
                        format!("{}ied", &base[..base.len() - 1])
                    } else {
                        format!("{base}ed")
                    };
                    (p.clone(), p, true)
                }
                1 => (pasts[0].clone(), pasts[0].clone(), true),
                2 => {
                    // the participle is the one in -n ("eaten", "seen", "gone") or with u ("sung")
                    let is_part = |w: &str| w.ends_with('n') || w.ends_with("ne") || w.contains('u');
                    let (a, b) = (&pasts[0], &pasts[1]);
                    match (is_part(a), is_part(b)) {
                        (false, true) => (a.clone(), b.clone(), true),
                        (true, false) => (b.clone(), a.clone(), true),
                        _ => (a.clone(), b.clone(), false),
                    }
                }
                _ => (pasts[0].clone(), pasts[1].clone(), false),
            };
            let sure = sure && base != "be";
            out.push((past, Form::Past, sure));
            out.push((part, Form::PastPart, sure));
            let ing = ing.unwrap_or_else(|| {
                if base.ends_with("ie") {
                    format!("{}ying", &base[..base.len() - 2])
                } else if base.ends_with('e') && !(base.ends_with("ee") || base.ends_with("ye") || base.ends_with("oe")) && base.len() > 2 {
                    format!("{}ing", &base[..base.len() - 1])
                } else {
                    format!("{base}ing")
                }
            });
            out.push((ing, Form::Ing, true));
        }
        Pos::Adj => {
            if multi {
                return out;
            }
            let mut comp = None;
            let mut sup = None;
            for f in irregular {
                if f.ends_with("est") {
                    sup = Some(f.clone());
                } else {
                    comp = Some(f.clone());
                }
            }
            let one_syllable = {
                let c: Vec<char> = base.chars().collect();
                let mut groups = 0;
                let mut prev = false;
                for ch in &c {
                    let v = vowel(*ch);
                    if v && !prev {
                        groups += 1;
                    }
                    prev = v;
                }
                groups == 1
            };
            if comp.is_none() && sup.is_none() && !one_syllable {
                return out;
            }
            let stem = |suffix: &str| {
                if base.ends_with('e') {
                    format!("{base}{}", &suffix[1..])
                } else if ends_consonant_y(base) {
                    format!("{}i{suffix}", &base[..base.len() - 1])
                } else {
                    format!("{base}{suffix}")
                }
            };
            out.push((comp.unwrap_or_else(|| stem("er")), Form::Comparative, false));
            out.push((sup.unwrap_or_else(|| stem("est")), Form::Superlative, false));
        }
        Pos::Adv => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(base: &str, pos: Pos, irr: &[&str], form: Form) -> (String, bool) {
        let irr: Vec<String> = irr.iter().map(|s| s.to_string()).collect();
        let f = forms(base, pos, &irr).into_iter().find(|f| f.1 == form).unwrap();
        (f.0, f.2)
    }

    #[test]
    fn plurals() {
        assert_eq!(get("dog", Pos::Noun, &[], Form::Plural), ("dogs".into(), true));
        assert_eq!(get("box", Pos::Noun, &[], Form::Plural), ("boxes".into(), true));
        assert_eq!(get("city", Pos::Noun, &[], Form::Plural), ("cities".into(), true));
        assert_eq!(get("mouse", Pos::Noun, &["mice"], Form::Plural), ("mice".into(), true));
        assert_eq!(get("domestic animal", Pos::Noun, &[], Form::Plural), ("domestic animals".into(), true));
        assert!(!get("potato", Pos::Noun, &[], Form::Plural).1);
        assert!(!get("trousers", Pos::Noun, &[], Form::Plural).1);
    }

    #[test]
    fn verbs() {
        assert_eq!(get("bark", Pos::Verb, &[], Form::ThirdSg).0, "barks");
        assert_eq!(get("carry", Pos::Verb, &[], Form::ThirdSg).0, "carries");
        assert_eq!(get("go", Pos::Verb, &["went", "gone"], Form::ThirdSg).0, "goes");
        assert_eq!(get("bark", Pos::Verb, &[], Form::Past), ("barked".into(), true));
        assert_eq!(get("eat", Pos::Verb, &["ate", "eaten"], Form::Past), ("ate".into(), true));
        assert_eq!(get("sing", Pos::Verb, &["sang", "sung"], Form::Past), ("sang".into(), true));
        assert_eq!(get("go", Pos::Verb, &["went", "gone"], Form::Past), ("went".into(), true));
        assert_eq!(get("stop", Pos::Verb, &["stopped", "stopping"], Form::Past), ("stopped".into(), true));
        assert_eq!(get("make", Pos::Verb, &[], Form::Ing).0, "making");
        assert_eq!(get("lie", Pos::Verb, &[], Form::Ing).0, "lying");
    }
}
