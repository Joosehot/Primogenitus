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

## The chatbot

```
$ primogenitus chat
Primo: Hello. Ask me about a thing, an animal or a person: ...
You: Is a dog an animal?
Primo: Yes. Dogs are animals.
You: Does it have a tail?
Primo: Yes. Dogs have tails.
You: What does a dog do?
Primo: Dogs snatch a bone.
You: What else?
Primo: Dogs lick a hand.
You: Is a whale a fish?
Primo: Not that I know: no line in my network says that whales are fishes.
```

`primogenitus ask "question"` answers one question; `--explain` shows the
lines each answer stands on.

- `question.rs` reads the question whole, word for word: what is / who is /
  what does ... do / have / what is ... like / what do dogs eat / what eats
  fish / which animals swim / what animals have wings / what kind of animal is
  a dog / how big is a whale / is X a Y / is X ADJ / does X have a Y / can X
  VERB / tell me about X / who are you / what else, with pronouns (it, they)
  for what was talked about. Where, when, why and how are answered plainly:
  the network has no lines for places, times, reasons or ways yet. A word that
  reads two ways ("wings": the plural, or the pilot's badge) is read the way
  whose number is sure and fits its determiner. A word left over means not understood, never a
  guess; an unknown word gets "did you mean".
- `chat.rs` answers: open questions by the generator limited to the fitting
  patterns, yes/no questions by walking the lines (dog -> domestic animal ->
  animal, never jumping senses; a part or a deed inherited from a kind). The
  conversation remembers its topic and what was said, so "what else" gives
  something new until "That is all I know".
- Every sentence about the world passes the grammar judges and the check.
  No line means "Not that I know", never a guess.

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
("Dogs barks.", "An dog barks.", "A dog are a canine."); `tests/chat.rs`
holds a whole conversation (known facts yes, made-up ones not, pronouns,
"what else" to its end, every sentence checked).

## Known limits

The example-sentence parser is simple; a misread example gives an odd but
grammatical sentence ("Birds land a lot."). Some odd ones are the
dictionary's own ("Houses bearded the top of the heights").
