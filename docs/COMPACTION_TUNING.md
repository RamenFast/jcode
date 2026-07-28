# Compaction tuning

When a conversation approaches the model's context window, jcode **compacts**: it
summarizes the older part of the transcript and keeps the recent part verbatim.
This document covers how much conversation survives that cut, and how to tune it.

## The keep tail

Compaction keeps a **tail** of recent messages untouched. The size of that tail is
proportional to the context budget:

```
available          = token_budget - non_message_tokens
keep_target_tokens = min(
    available * keep_fraction,                        # proportional to the window
    token_budget * ANTI_THRASH_CEILING - non_message_tokens,  # never re-fire
)
```

`non_message_tokens` is everything that occupies context but is not the kept
tail: the system prompt, the tool definitions, and the summary standing in for
the compacted prefix. It is *measured* from the provider's reported input token
count rather than guessed, because the guess is often badly wrong. In a live
32k-window session the tools and system prompt alone cost about 25k, leaving
only ~7k for conversation.

Messages are kept from the end backwards until that budget is full, subject to a
floor of `min_keep_turns` messages. The cutoff is then walked back further if
necessary so a kept tool result never loses its tool call.

When overhead alone approaches the ceiling, the target goes to zero and only the
floor survives. That is the honest answer: such a window has no room left for
conversation.

Sizing against the budget matters because the tail used to be a fixed 10 messages
regardless of window size. On a 1M-token model that meant a compaction went from
~800k tokens to ~20k, discarding about 98% of the conversation in one step, which
reads as the agent abruptly forgetting the task. With a proportional tail, a
bigger window keeps proportionally more.

## Settings

All under `[compaction]` in `~/.jcode/config.toml`.

| Key | Default | Meaning |
| --- | --- | --- |
| `keep_fraction` | `0.45` | Fraction of the usable budget kept verbatim. **The main aggression dial.** |
| `min_keep_turns` | `10` | Hard floor on kept messages, even if recent turns are huge. |
| `mode` | `"reactive"` | `reactive` (compact at 80% full), `proactive` (project growth and compact early), `semantic` (relevance-scored keep set). |
| `min_turns_between_compactions` | `10` | Cooldown for proactive/semantic modes. |

Want gentler compaction (keep more, compact more often):

```toml
[compaction]
keep_fraction = 0.55
```

Want more aggressive compaction (reclaim more context per compaction):

```toml
[compaction]
keep_fraction = 0.25
```

`keep_fraction` is clamped to `MAX_KEEP_TAIL_FRACTION` (0.60). Above that, the
post-compaction transcript would land at or above the 80% trigger and compaction
would immediately fire again on its own output.

## The three compaction paths

- **Automatic** (`reactive` / `proactive` / `semantic`) fires at
  `COMPACTION_THRESHOLD` (80% of budget) and keeps the full `keep_fraction` tail.
- **Manual** (`/compact`) is an explicit ask to reclaim context now, so it keeps
  half the normal tail.
- **Emergency hard compact** fires at `CRITICAL_THRESHOLD` (95%) or after a
  provider context-limit error. It starts from the same proportional tail; if
  that does not fit it re-fits greedily against the char budget to find the
  largest tail that does, falling back to `MIN_TURNS_TO_KEEP` (2) only when a
  single message exceeds the whole target.

  This search must not halve. An earlier version stepped the kept turns
  10 -> 5 -> 2, which can only land on a power-of-two division of the floor, so
  whenever the best tail sat between two steps it overshot all the way down. A
  live 70k-window session dropped 16 of 18 messages that way.

## Measuring it

`retention_ratio_matrix` in `crates/jcode-base/src/compaction_tests.rs` is the
regression harness for compaction aggression. It prints the retention ratio
`R = post_tokens / budget` for each budget and trigger:

```
cargo test -p jcode-base retention_ratio_matrix -- --nocapture
```

Live sessions report the same figures:

```
grep 'compaction/outcome' ~/.jcode/logs/jcode-$(date +%F).log | tail -1
```
