# Primogenitus

A motor that produces sentences. A frozen word network is built once from a
downloaded dictionary, and sentences come from the HotMotor pipeline. Nothing
is trained and nothing updates.

```
$ primogenitus build
     4.72 s  the dictionary downloaded and unpacked (or found)
     ...
built in 1.5 s: 271207 nodes (155467 words, 115740 variations), 1411759 lines (29295 from 48530 parsed example sentences)

$ primogenitus dog
input: the topic "dog" (one noun; the sentence is about it)
seeds (the judges' score; the check parses each under the grammar):
-> seed 0   +3.74  Dogs snatch a bone.          [does_to; parses: NP Verb NP]
   seed 3   +3.59  Dogs lick a hand.            [does_to; parses: NP Verb NP]
   seed 5   +3.18  Dogs chase a rabbit.         [does_to; parses: NP Verb NP]
   ...
Dogs snatch a bone.
```

`--explain` shows every score term and the line each sentence stands on.

## The frozen word network

- **Source**: WordNet 3.1, downloaded once into `data/wordnet/`.
- **Nodes**: every word, and every variation of a word as its own node
  (`dogs`, `barks`, `barked`, `barking`), with a line to its base. A node
  carries its familiarity (WordNet's tag counts), a verb's sentence frames
  (from its most common senses), a noun's animacy, an adjective's position
  marker, whether a noun may be uncountable, and whether its form is sure.
- **Lines** carry the meaning: the dictionary's own (same as, is a, has part,
  opposite, similar, derived, ...), and the ones the parser reads from the
  dictionary's example sentences ("the dog barked": *dog does bark*; "toast
  the bread": *toast is done to bread*). Every line knows which sense of its
  word it comes from and which sense of the target it lands in, so a walk of
  two steps never jumps senses (wolf -> canine the animal, not the tooth).
- **Frozen** to `data/network.bin`; never updated after the build.

## Generation

| Step | File | What it does |
|---|---|---|
| input | `input.rs` | one noun (the simplest option; swap here) |
| lexicon | `lexicon.rs` | closed words, the a/an rule |
| meaning | `search.rs` | spreads from the topic along the lines |
| rules | `rules/*.rs` | one sentence pattern per file: `is_a`, `has_part`, `does`, `does_to`, `is_adj`; each says only what a line says |
| beam | `search.rs` | heads, then subject and tense, then object |
| judges | `rules/judges.rs` | the grammar, built in: frames, determiner and number, agreement, sure forms only, adjective place, no word twice, a/an; scoring only: animacy, role |
| seeds | `lib.rs` | each seed shakes the scores and gives one sentence; the judges pick |
| check | `parser.rs` | every sentence is parsed again under `S -> NP VP .`; one that fails is never said |

Every number is in `rules.toml`.

## The proof

`cargo test`: the spelling rules, the article rule, and on the frozen network
(`tests/sentences.rs`) 50 topics, each generated twice: the same sentences
both times, every seed's sentence parses, and the check refuses bad grammar
("Dogs barks.", "An dog barks.", "A dog are a canine.").

## Known limits

The example-sentence parser is simple; a misread example gives an odd but
grammatical sentence ("Birds land a lot."). Some odd ones are the
dictionary's own ("Houses bearded the top of the heights").
