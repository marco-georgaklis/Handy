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
