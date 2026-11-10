# Benchmarks

This fork exists to close the gap with Wispr Flow on four things: pasting the
right text, latency, formatting, and getting a dictation back when it went to
the wrong place. Each has a check here so a change can be measured rather than
felt.

| What                | How                             | Needs                                     |
| ------------------- | ------------------------------- | ----------------------------------------- |
| Formatting quality  | `formatting/run.py`             | an OpenRouter key                         |
| Latency             | `latency_from_log.py`           | a log from real use                       |
| Paste correctness   | `paste/run.sh`                  | the app running with Accessibility access |
| Transcription speed | `handy --transcribe-file x.wav` | an installed model                        |

## Formatting

`formatting/cases.json` holds raw dictations (as the speech model emits them)
with deterministic checks: is there a list when one was spoken and none when
not, did names/numbers/negations survive, were fillers and superseded
self-corrections removed, was a dictated question or request typed rather than
answered. `run.py` sends each case the way the app does and reports the pass
rate per category and the request latency.

```bash
python3 bench/formatting/run.py --repeat 2 \
  --prompts current flow \
  --models openai/gpt-4.1-mini anthropic/claude-haiku-5.5
```

`prompts/flow.txt` is the prompt the app ships as "Smart Formatting"
(`settings.rs` includes the file), so the benchmark always scores the shipped
text. `prompts/current.txt` and `prompts/handy_default.txt` are baselines.

Results on 2026-11-10 through OpenRouter (28 cases; 2 runs for the first two
rows, 3 for the last two):

| Prompt  | Model                        | Passed | Lists | p50 latency |
| ------- | ---------------------------- | ------ | ----- | ----------- |
| current | openai/gpt-4.1-mini          | 33/56  | 0/16  | 0.98 s      |
| flow    | openai/gpt-4.1-mini          | 46/56  | 9/16  | 0.89 s      |
| flow    | google/gemini-3.5-flash-lite | 77/84  | 19/24 | 0.95 s      |
| flow    | anthropic/claude-haiku-5.5   | 83/84  | 23/24 | 1.21 s      |

Latency varies between runs by a few hundred milliseconds; rerun before
drawing conclusions from small differences.

The checks are strict string checks, so a failure is a prompt for reading the
output (`--only <id> --show`), not proof the output is bad.

## Latency

The app logs one line per dictation, in this form:

```
dictation timing: audio=9.76s transcribe=171ms post_process=1040ms paste=118ms total=1335ms
```

`total` is key release to pasted. `python3 bench/latency_from_log.py` prints
percentiles per step. On a log from an upstream build it falls back to the
per-step debug lines.

Baseline (upstream 0.10.0, 29 dictations, no post-processing): total p50
402 ms, of which paste 232 ms and transcription 136 ms.

## Paste correctness

`paste/run.sh` puts a sentinel on the clipboard, has the running app paste a
unique string into a target app (`handy --paste-text`, the path a finished
dictation takes), and checks that the target got that string and that the
clipboard holds the sentinel again afterwards.

```bash
bench/paste/run.sh 20 TextEdit Notes
```

The target is 100% on both counts. A wrong-text failure is the bug this fork
set out to fix: the old clipboard pasted instead of the dictation.

## Against Wispr Flow

Wispr Flow stores the raw speech-recognition text and the text it inserted for
every dictation (`History.asrText` / `formattedText` / `pastedText` in
`~/Library/Application Support/Wispr Flow/flow.sqlite`), plus its end-to-end
latency. `compare/wispr_compare.py` runs our prompt on its raw text and
compares the result with what it inserted: same words in, two formatters out.
The export holds real dictations and stays out of the repository.

On 2026-11-10, with `anthropic/claude-haiku-5.5`:

| Sample                                | Measure                      | Result |
| ------------------------------------- | ---------------------------- | ------ |
| 62 most recent dictations (12+ words) | median word match with Wispr | 0.98   |
|                                       | list where Wispr made none   | 0/59   |
| 30 list or multi-paragraph dictations | list where Wispr made one    | 14/17  |
|                                       | list where Wispr made none   | 0/13   |

Latency is the open gap. Wispr Flow's recorded end-to-end median is 0.48 s
(0.69 s for dictations over 40 words, 546 dictations). Our formatting request
alone takes about 1.2 s on Haiku 5.5 and about 0.5 s on
`openai/gpt-oss-120b:nitro`, which scores lower on the strict cases (59/70
against 65/70).

`compare/script.md` is a read-aloud script for a live side-by-side.
